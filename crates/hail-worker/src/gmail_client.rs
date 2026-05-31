//! Gmail API provider-client wrapper foundation.
#![allow(dead_code)]
//!
//! This module is intentionally not wired into the worker supervisor yet. It
//! keeps provider import behind a narrow, testable boundary: a token source,
//! profile lookup, paginated message listing, and raw RFC822 fetch. Production
//! OAuth/token persistence should adapt [`GmailTokenSource`] without leaking
//! Google credentials into tests.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

use async_trait::async_trait;
use base64::Engine;
use base64::prelude::{BASE64_URL_SAFE, BASE64_URL_SAFE_NO_PAD};
use reqwest::header::RETRY_AFTER;
use reqwest::{StatusCode, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const GMAIL_API_BASE_URL: &str = "https://gmail.googleapis.com/gmail/v1/";

/// Maximum time allowed to establish provider-worker HTTP connections.
pub const PROVIDER_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum total time allowed for each provider-worker HTTP request.
pub const PROVIDER_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum idle gap allowed while reading provider-worker HTTP response bodies.
pub const PROVIDER_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the bounded HTTP client used by worker-side provider integrations.
pub fn provider_worker_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(PROVIDER_HTTP_CONNECT_TIMEOUT)
        .timeout(PROVIDER_HTTP_REQUEST_TIMEOUT)
        .read_timeout(PROVIDER_HTTP_READ_TIMEOUT)
        .build()
}

/// OAuth scopes requested for Gmail import plus outbound send.
pub const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// OAuth scope required for bidirectional read/label/trash mutations.
pub const GMAIL_MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
pub const GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";

#[derive(Debug, Error)]
pub enum GmailClientError {
    #[error("gmail token source failed: {0}")]
    Token(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("gmail request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid gmail api base url: {0}")]
    Url(#[from] url::ParseError),
    #[error("gmail api returned {status}: {message}")]
    Api {
        status: StatusCode,
        kind: GmailApiErrorKind,
        reason: Option<String>,
        message: String,
        retry_after: Option<Duration>,
    },
    #[error("gmail pagination repeated page token {page_token:?}")]
    PaginationLoop { page_token: String },
    #[error("gmail raw message response did not include raw RFC822 data")]
    MissingRawMessage,
    #[error("gmail raw message was not valid base64url: {0}")]
    RawDecode(#[from] base64::DecodeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GmailApiErrorKind {
    Unauthorized,
    PermissionDenied,
    NotFound,
    RateLimited,
    Transient,
    BadRequest,
    Other,
}

impl GmailClientError {
    pub fn token_error(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Token(Box::new(error))
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Request(error) => error.is_timeout() || error.is_connect(),
            Self::Api { kind, .. } => {
                matches!(
                    kind,
                    GmailApiErrorKind::RateLimited | GmailApiErrorKind::Transient
                )
            }
            Self::Token(_)
            | Self::Url(_)
            | Self::PaginationLoop { .. }
            | Self::MissingRawMessage
            | Self::RawDecode(_) => false,
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Api { retry_after, .. } => *retry_after,
            Self::Token(_)
            | Self::Request(_)
            | Self::Url(_)
            | Self::PaginationLoop { .. }
            | Self::MissingRawMessage
            | Self::RawDecode(_) => None,
        }
    }
}

/// Boundary between provider import and OAuth/token storage.
///
/// Implementations may wrap `yup-oauth2`, encrypted database refresh tokens, or
/// a fake token in tests. Tokens must not be logged.
#[async_trait]
pub trait GmailTokenSource: Send + Sync {
    async fn bearer_token(&self) -> Result<SecretString, GmailClientError>;
}

#[derive(Clone, Debug)]
pub struct GmailAccessToken {
    pub token: SecretString,
    pub expires_in: Duration,
}

/// Boundary for refreshing short-lived Gmail OAuth access tokens.
#[async_trait]
pub trait GmailAccessTokenProvider: Send + Sync {
    async fn refresh_access_token(&self) -> Result<GmailAccessToken, GmailClientError>;
}

#[derive(Clone, Debug)]
struct CachedBearerToken {
    token: SecretString,
    usable_until: Instant,
}

/// Gmail token source that reuses a refreshed OAuth access token until it is
/// near expiry.
///
/// The token itself stays in memory as [`SecretString`]. The wrapper never logs
/// or serializes token material; production providers should continue to keep
/// long-lived refresh tokens encrypted at rest.
#[derive(Debug)]
pub struct CachedGmailTokenSource<P> {
    provider: P,
    expiry_skew: Duration,
    cached: Arc<tokio::sync::Mutex<Option<CachedBearerToken>>>,
}

impl<P> CachedGmailTokenSource<P> {
    pub const DEFAULT_EXPIRY_SKEW: Duration = Duration::from_secs(60);

    #[must_use]
    pub fn new(provider: P) -> Self {
        Self::with_expiry_skew(provider, Self::DEFAULT_EXPIRY_SKEW)
    }

    #[must_use]
    pub fn with_expiry_skew(provider: P, expiry_skew: Duration) -> Self {
        Self {
            provider,
            expiry_skew,
            cached: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
}

impl<P> Clone for CachedGmailTokenSource<P>
where
    P: Clone,
{
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            expiry_skew: self.expiry_skew,
            cached: self.cached.clone(),
        }
    }
}

#[async_trait]
impl<P> GmailTokenSource for CachedGmailTokenSource<P>
where
    P: GmailAccessTokenProvider,
{
    async fn bearer_token(&self) -> Result<SecretString, GmailClientError> {
        let now = Instant::now();
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref()
            && token.usable_until > now
        {
            return Ok(token.token.clone());
        }

        let refreshed = self.provider.refresh_access_token().await?;
        let usable_for = refreshed.expires_in.saturating_sub(self.expiry_skew);
        let usable_until = Instant::now() + usable_for;
        let token = refreshed.token;
        *cached = Some(CachedBearerToken {
            token: token.clone(),
            usable_until,
        });
        Ok(token)
    }
}

#[derive(Clone, Debug)]
pub struct StaticGmailTokenSource {
    token: SecretString,
}

impl StaticGmailTokenSource {
    #[must_use]
    pub fn new(token: SecretString) -> Self {
        Self { token }
    }
}

#[async_trait]
impl GmailTokenSource for StaticGmailTokenSource {
    async fn bearer_token(&self) -> Result<SecretString, GmailClientError> {
        Ok(self.token.clone())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GmailRetryConfig {
    /// Total attempts per HTTP request, including the first try.
    pub max_attempts: u8,
    /// Initial exponential-backoff delay for retryable failures.
    pub base_delay: Duration,
    /// Upper bound for exponential-backoff delay.
    pub max_delay: Duration,
}

impl Default for GmailRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl GmailRetryConfig {
    #[must_use]
    pub fn no_retries() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    fn attempts(self) -> u8 {
        self.max_attempts.max(1)
    }

    fn delay_for_retry(self, retry_after: Option<Duration>, completed_attempts: u8) -> Duration {
        if let Some(delay) = retry_after {
            return delay.min(self.max_delay);
        }
        if self.base_delay.is_zero() {
            return Duration::ZERO;
        }

        let shift = u32::from(completed_attempts.saturating_sub(1)).min(16);
        let factor = 1_u32 << shift;
        let base = self.base_delay.saturating_mul(factor).min(self.max_delay);
        let max_millis = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);
        if max_millis == 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(rand::random_range(0..=max_millis))
        }
    }
}

#[derive(Clone, Debug)]
pub struct GmailClient<T> {
    http: reqwest::Client,
    token_source: T,
    base_url: Url,
    retry: GmailRetryConfig,
}

impl<T> GmailClient<T>
where
    T: GmailTokenSource,
{
    pub fn new(http: reqwest::Client, token_source: T) -> Result<Self, GmailClientError> {
        Self::with_base_url(http, token_source, GMAIL_API_BASE_URL)
    }

    pub fn with_base_url(
        http: reqwest::Client,
        token_source: T,
        base_url: &str,
    ) -> Result<Self, GmailClientError> {
        Ok(Self {
            http,
            token_source,
            base_url: Url::parse(base_url)?,
            retry: GmailRetryConfig::default(),
        })
    }

    #[must_use]
    pub fn with_retry_config(mut self, retry: GmailRetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// `GET /gmail/v1/users/me/profile`
    pub async fn profile(&self) -> Result<GmailProfile, GmailClientError> {
        self.get_json("users/me/profile", &[]).await
    }

    /// `GET /gmail/v1/users/me/labels`
    pub async fn list_labels(&self) -> Result<ListLabelsResponse, GmailClientError> {
        self.get_json("users/me/labels", &[]).await
    }

    /// List only Gmail user-created labels (`type=user`).
    ///
    /// System/category/state labels are intentionally filtered here so later
    /// import code can map provider label ids without guessing from message
    /// `labelIds` alone.
    pub async fn list_user_labels(&self) -> Result<Vec<GmailLabel>, GmailClientError> {
        Ok(self
            .list_labels()
            .await?
            .labels
            .into_iter()
            .filter(GmailLabel::is_user_created)
            .collect())
    }

    /// `GET /gmail/v1/users/me/messages`
    pub async fn list_messages(
        &self,
        params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError> {
        let max_results = params
            .max_results
            .map(|value| value.clamp(1, 500).to_string());
        let mut query = Vec::new();
        if let Some(value) = max_results.as_deref() {
            query.push(("maxResults", value));
        }
        if let Some(value) = params.page_token.as_deref() {
            query.push(("pageToken", value));
        }
        if let Some(value) = params.query.as_deref() {
            query.push(("q", value));
        }
        for label_id in &params.label_ids {
            query.push(("labelIds", label_id.as_str()));
        }
        if params.include_spam_trash {
            query.push(("includeSpamTrash", "true"));
        }

        self.get_json("users/me/messages", &query).await
    }

    /// Fetch all message ids by following `nextPageToken`.
    pub async fn list_all_messages(
        &self,
        params: &ListMessagesParams,
    ) -> Result<Vec<ListMessage>, GmailClientError> {
        let mut params = params.clone();
        let mut messages = Vec::new();
        let mut seen_page_tokens = HashSet::new();

        loop {
            if let Some(page_token) = params.page_token.as_deref()
                && !seen_page_tokens.insert(page_token.to_owned())
            {
                return Err(GmailClientError::PaginationLoop {
                    page_token: page_token.to_owned(),
                });
            }

            let response = self.list_messages(&params).await?;
            messages.extend(response.messages);
            match response.next_page_token {
                Some(next_page_token) => params.page_token = Some(next_page_token),
                None => return Ok(messages),
            }
        }
    }

    /// Stream pages to a callback without buffering every message id in memory.
    pub async fn for_each_message_page<F, Fut>(
        &self,
        params: &ListMessagesParams,
        mut handle_page: F,
    ) -> Result<(), GmailClientError>
    where
        F: FnMut(ListMessagesResponse) -> Fut,
        Fut: std::future::Future<Output = Result<(), GmailClientError>>,
    {
        let mut params = params.clone();
        let mut seen_page_tokens = HashSet::new();

        loop {
            if let Some(page_token) = params.page_token.as_deref()
                && !seen_page_tokens.insert(page_token.to_owned())
            {
                return Err(GmailClientError::PaginationLoop {
                    page_token: page_token.to_owned(),
                });
            }

            let response = self.list_messages(&params).await?;
            let next_page_token = response.next_page_token.clone();
            handle_page(response).await?;
            match next_page_token {
                Some(next_page_token) => params.page_token = Some(next_page_token),
                None => return Ok(()),
            }
        }
    }

    /// `GET /gmail/v1/users/me/history`
    pub async fn list_history(
        &self,
        params: &ListHistoryParams,
    ) -> Result<ListHistoryResponse, GmailClientError> {
        let max_results = params
            .max_results
            .map(|value| value.clamp(1, 500).to_string());
        let mut query = Vec::new();
        query.push(("startHistoryId", params.start_history_id.as_str()));
        if let Some(value) = max_results.as_deref() {
            query.push(("maxResults", value));
        }
        if let Some(value) = params.page_token.as_deref() {
            query.push(("pageToken", value));
        }
        if let Some(value) = params.label_id.as_deref() {
            query.push(("labelId", value));
        }
        for history_type in &params.history_types {
            query.push(("historyTypes", history_type.as_str()));
        }

        self.get_json("users/me/history", &query).await
    }

    /// `GET /gmail/v1/users/me/messages/{id}?format=raw`
    pub async fn get_raw_message(
        &self,
        message_id: &str,
    ) -> Result<RawGmailMessage, GmailClientError> {
        let path = format!("users/me/messages/{message_id}");
        let response: GetMessageResponse = self.get_json(&path, &[("format", "raw")]).await?;
        let raw = response.raw.ok_or(GmailClientError::MissingRawMessage)?;
        let rfc822 = decode_raw_rfc822(&raw)?;
        Ok(RawGmailMessage {
            id: response.id,
            thread_id: response.thread_id,
            history_id: response.history_id,
            label_ids: response.label_ids,
            rfc822,
        })
    }

    /// `POST /gmail/v1/users/me/messages/batchModify`
    pub async fn batch_modify_messages(
        &self,
        request: &BatchModifyMessagesRequest,
    ) -> Result<(), GmailClientError> {
        self.post_json_empty("users/me/messages/batchModify", request)
            .await
    }

    /// `POST /gmail/v1/users/me/messages/{id}/modify`
    pub async fn modify_message(
        &self,
        message_id: &str,
        request: &ModifyMessageRequest,
    ) -> Result<ModifyMessageResponse, GmailClientError> {
        let path = format!("users/me/messages/{message_id}/modify");
        self.post_json(&path, request).await
    }

    async fn get_json<R>(&self, path: &str, query: &[(&str, &str)]) -> Result<R, GmailClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        let mut url = self.base_url.join(path)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }

        let mut last_retryable_error = None;
        for attempt in 1..=self.retry.attempts() {
            let token = self.token_source.bearer_token().await?;
            match self.send_json(url.clone(), token).await {
                Ok(value) => return Ok(value),
                Err(error) if attempt < self.retry.attempts() && error.is_retryable() => {
                    let delay = self.retry.delay_for_retry(error.retry_after(), attempt);
                    last_retryable_error = Some(error);
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_retryable_error.expect("retry loop always runs at least once"))
    }

    async fn send_json<R>(&self, url: Url, token: SecretString) -> Result<R, GmailClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        let response = self
            .http
            .get(url)
            .bearer_auth(token.expose_secret())
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response.json().await?);
        }

        gmail_error_from_response(response).await
    }
    async fn post_json<B, R>(&self, path: &str, body: &B) -> Result<R, GmailClientError>
    where
        B: Serialize + ?Sized,
        R: serde::de::DeserializeOwned,
    {
        let url = self.base_url.join(path)?;
        let mut last_retryable_error = None;
        for attempt in 1..=self.retry.attempts() {
            let token = self.token_source.bearer_token().await?;
            match self.send_json_request(url.clone(), token, Some(body)).await {
                Ok(value) => return Ok(value),
                Err(error) if attempt < self.retry.attempts() && error.is_retryable() => {
                    let delay = self.retry.delay_for_retry(error.retry_after(), attempt);
                    last_retryable_error = Some(error);
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_retryable_error.expect("retry loop always runs at least once"))
    }

    async fn post_json_empty<B>(&self, path: &str, body: &B) -> Result<(), GmailClientError>
    where
        B: Serialize + ?Sized,
    {
        let url = self.base_url.join(path)?;
        let mut last_retryable_error = None;
        for attempt in 1..=self.retry.attempts() {
            let token = self.token_source.bearer_token().await?;
            match self.send_empty_request(url.clone(), token, body).await {
                Ok(()) => return Ok(()),
                Err(error) if attempt < self.retry.attempts() && error.is_retryable() => {
                    let delay = self.retry.delay_for_retry(error.retry_after(), attempt);
                    last_retryable_error = Some(error);
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_retryable_error.expect("retry loop always runs at least once"))
    }

    async fn send_json_request<B, R>(
        &self,
        url: Url,
        token: SecretString,
        body: Option<&B>,
    ) -> Result<R, GmailClientError>
    where
        B: Serialize + ?Sized,
        R: serde::de::DeserializeOwned,
    {
        let request = self.http.post(url).bearer_auth(token.expose_secret());
        let request = if let Some(body) = body {
            request.json(body)
        } else {
            request
        };
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response.json().await?);
        }
        gmail_error_from_response(response).await
    }

    async fn send_empty_request<B>(
        &self,
        url: Url,
        token: SecretString,
        body: &B,
    ) -> Result<(), GmailClientError>
    where
        B: Serialize + ?Sized,
    {
        let response = self
            .http
            .post(url)
            .bearer_auth(token.expose_secret())
            .json(body)
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        gmail_error_from_response(response).await
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchModifyMessagesRequest {
    pub ids: Vec<String>,
    pub add_label_ids: Vec<String>,
    pub remove_label_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyMessageRequest {
    pub add_label_ids: Vec<String>,
    pub remove_label_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModifyMessageResponse {
    pub id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
}

async fn gmail_error_from_response<T>(response: reqwest::Response) -> Result<T, GmailClientError> {
    let status = response.status();
    let retry_after = retry_after_duration(response.headers().get(RETRY_AFTER));
    let error_body = response.text().await.unwrap_or_default();
    let parsed = parse_gmail_error(&error_body);
    Err(GmailClientError::Api {
        status,
        kind: classify_gmail_error(status, parsed.reason.as_deref()),
        reason: parsed.reason,
        message: parsed.message.unwrap_or_else(|| {
            if error_body.is_empty() {
                status
                    .canonical_reason()
                    .unwrap_or("unknown error")
                    .to_owned()
            } else {
                error_body
            }
        }),
        retry_after,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListMessagesParams {
    pub max_results: Option<u16>,
    pub page_token: Option<String>,
    /// Gmail search query for read-only discovery/import bounds.
    ///
    /// This must not be interpreted as authoritative local state; hail-side
    /// archive/delete/read mutations are local Stalwart/JMAP mutations in v1.2.
    pub query: Option<String>,
    /// Gmail label ids for read-only discovery/import bounds.
    ///
    /// The Gmail client intentionally exposes no label mutation API for v1.2
    /// one-way import.
    pub label_ids: Vec<String>,
    pub include_spam_trash: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListHistoryParams {
    pub start_history_id: String,
    pub max_results: Option<u16>,
    pub page_token: Option<String>,
    pub label_id: Option<String>,
    pub history_types: Vec<String>,
}

impl ListHistoryParams {
    #[must_use]
    pub fn new(start_history_id: impl Into<String>) -> Self {
        Self {
            start_history_id: start_history_id.into(),
            max_results: None,
            page_token: None,
            label_id: None,
            history_types: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GmailProfile {
    pub email_address: String,
    pub messages_total: Option<u64>,
    pub threads_total: Option<u64>,
    pub history_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListLabelsResponse {
    #[serde(default)]
    pub labels: Vec<GmailLabel>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GmailLabel {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub label_type: Option<String>,
}

impl GmailLabel {
    #[must_use]
    pub fn is_user_created(&self) -> bool {
        self.label_type.as_deref() == Some("user")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListMessagesResponse {
    #[serde(default)]
    pub messages: Vec<ListMessage>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListMessage {
    pub id: String,
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListHistoryResponse {
    #[serde(default)]
    pub history: Vec<GmailHistoryRecord>,
    pub next_page_token: Option<String>,
    pub history_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryRecord {
    pub id: String,
    #[serde(default)]
    pub messages_added: Vec<GmailHistoryMessageRef>,
    #[serde(default)]
    pub labels_added: Vec<GmailHistoryLabelChange>,
    #[serde(default)]
    pub labels_removed: Vec<GmailHistoryLabelChange>,
    #[serde(default)]
    pub messages: Vec<GmailHistoryMessage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryMessageRef {
    pub message: GmailHistoryMessage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryLabelChange {
    pub message: GmailHistoryMessage,
    #[serde(default)]
    pub label_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryMessage {
    pub id: String,
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawGmailMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub history_id: Option<String>,
    pub label_ids: Vec<String>,
    pub rfc822: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetMessageResponse {
    id: String,
    thread_id: Option<String>,
    history_id: Option<String>,
    #[serde(default)]
    label_ids: Vec<String>,
    raw: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct ParsedGmailError {
    pub(super) reason: Option<String>,
    pub(super) message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailErrorEnvelope {
    error: GmailErrorBody,
}

#[derive(Debug, Deserialize)]
struct GmailErrorBody {
    message: Option<String>,
    errors: Option<Vec<GmailErrorDetail>>,
}

#[derive(Debug, Deserialize)]
struct GmailErrorDetail {
    reason: Option<String>,
    message: Option<String>,
}

fn decode_raw_rfc822(raw: &str) -> Result<Vec<u8>, base64::DecodeError> {
    BASE64_URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .or_else(|_| BASE64_URL_SAFE.decode(raw.as_bytes()))
}

pub(super) fn parse_gmail_error(body: &str) -> ParsedGmailError {
    let Ok(envelope) = serde_json::from_str::<GmailErrorEnvelope>(body) else {
        return ParsedGmailError::default();
    };

    let first_detail = envelope
        .error
        .errors
        .as_deref()
        .and_then(|errors| errors.first());

    ParsedGmailError {
        reason: first_detail.and_then(|detail| detail.reason.clone()),
        message: envelope.error.message.or_else(|| {
            first_detail.and_then(|detail| {
                detail
                    .message
                    .as_deref()
                    .map(std::borrow::ToOwned::to_owned)
            })
        }),
    }
}

pub(super) fn classify_gmail_error(status: StatusCode, reason: Option<&str>) -> GmailApiErrorKind {
    match status {
        StatusCode::UNAUTHORIZED => GmailApiErrorKind::Unauthorized,
        StatusCode::FORBIDDEN => match reason {
            Some("rateLimitExceeded" | "userRateLimitExceeded" | "quotaExceeded") => {
                GmailApiErrorKind::RateLimited
            }
            _ => GmailApiErrorKind::PermissionDenied,
        },
        StatusCode::NOT_FOUND => GmailApiErrorKind::NotFound,
        StatusCode::TOO_MANY_REQUESTS => GmailApiErrorKind::RateLimited,
        StatusCode::BAD_REQUEST => GmailApiErrorKind::BadRequest,
        status if status.is_server_error() => GmailApiErrorKind::Transient,
        _ => GmailApiErrorKind::Other,
    }
}

pub(super) fn retry_after_duration(
    value: Option<&reqwest::header::HeaderValue>,
) -> Option<Duration> {
    let seconds = value?.to_str().ok()?.parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
}

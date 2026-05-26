//! Compile-only Gmail API request-shape spike.
#![allow(dead_code)]
//!
//! This module is intentionally not wired into the worker supervisor yet. It
//! documents and compiles the narrow boundary provider import needs from Gmail:
//! a token source, profile lookup, message listing, and raw RFC822 fetch.
//! Production OAuth/token persistence should land with the provider-account
//! schema and encrypted refresh-token tasks.

use async_trait::async_trait;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use reqwest::Url;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use thiserror::Error;

const GMAIL_API_BASE_URL: &str = "https://gmail.googleapis.com/gmail/v1/";

/// OAuth scopes recommended for initial one-way Gmail import.
pub const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// Later outbound can request this separately instead of widening import scope.
pub const GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";

#[derive(Debug, Error)]
pub enum GmailClientError {
    #[error("gmail token source failed: {0}")]
    Token(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("gmail request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid gmail api base url: {0}")]
    Url(#[from] url::ParseError),
    #[error("gmail raw message response did not include raw RFC822 data")]
    MissingRawMessage,
    #[error("gmail raw message was not valid base64url: {0}")]
    RawDecode(#[from] base64::DecodeError),
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

#[derive(Clone, Debug)]
pub struct GmailClient<T> {
    http: reqwest::Client,
    token_source: T,
    base_url: Url,
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
        })
    }

    /// `GET /gmail/v1/users/me/profile`
    pub async fn profile(&self) -> Result<GmailProfile, GmailClientError> {
        self.get_json("users/me/profile", &[]).await
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

    /// `GET /gmail/v1/users/me/messages/{id}?format=raw`
    pub async fn get_raw_message(
        &self,
        message_id: &str,
    ) -> Result<RawGmailMessage, GmailClientError> {
        let path = format!("users/me/messages/{message_id}");
        let response: GetMessageResponse = self.get_json(&path, &[("format", "raw")]).await?;
        let raw = response.raw.ok_or(GmailClientError::MissingRawMessage)?;
        let rfc822 = BASE64_URL_SAFE_NO_PAD.decode(raw.as_bytes())?;
        Ok(RawGmailMessage {
            id: response.id,
            thread_id: response.thread_id,
            history_id: response.history_id,
            rfc822,
        })
    }

    async fn get_json<R>(&self, path: &str, query: &[(&str, &str)]) -> Result<R, GmailClientError>
    where
        R: serde::de::DeserializeOwned,
    {
        let mut url = self.base_url.join(path)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        let token = self.token_source.bearer_token().await?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token.expose_secret())
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListMessagesParams {
    pub max_results: Option<u16>,
    pub page_token: Option<String>,
    pub query: Option<String>,
    pub label_ids: Vec<String>,
    pub include_spam_trash: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawGmailMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub history_id: Option<String>,
    pub rfc822: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetMessageResponse {
    id: String,
    thread_id: Option<String>,
    history_id: Option<String>,
    raw: Option<String>,
}

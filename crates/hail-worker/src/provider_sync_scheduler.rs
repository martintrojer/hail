//! Cancellation-aware Gmail provider-account sync scheduler.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hail_db::provider_sync_audit::{
    NewProviderSyncAuditLog, ProviderSyncEventType, ProviderSyncOperationKind,
    ProviderSyncResultStatus, insert_provider_sync_audit_log,
};
use secrecy::SecretString;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::gmail_client::{
    GmailApiErrorKind, GmailClient, GmailClientError, GmailTokenSource, provider_worker_http_client,
};
use crate::gmail_incremental_sync::{
    GmailIncrementalSyncError, GmailIncrementalSyncOptions, run_gmail_incremental_sync,
};
use crate::gmail_initial_sync::{
    GmailInitialSyncError, GmailInitialSyncOptions, run_gmail_initial_sync,
};
use crate::provider_import_routing::{RoutingRfc822Importer, ScreenerRfc822ImportRouter};
use crate::rfc822_import::StalwartJmapRfc822Importer;

const DEFAULT_SYNC_INTERVAL_SECS: i64 = 5 * 60;
const INITIAL_RETRY_BACKOFF_SECS: i64 = 60;
const MAX_RETRY_BACKOFF_SECS: i64 = 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSyncSchedulerOptions {
    pub sync_interval: Duration,
}

impl Default for ProviderSyncSchedulerOptions {
    fn default() -> Self {
        Self {
            sync_interval: Duration::from_secs(DEFAULT_SYNC_INTERVAL_SECS as u64),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSyncAccount {
    pub id: i64,
    pub user_id: i64,
    pub jmap_account_id: String,
    pub provider_account_id: String,
    pub provider_email: String,
    pub sync_status: String,
    pub last_profile_history_id: Option<String>,
    pub initial_sync_completed_at: Option<String>,
    pub sync_backoff_secs: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSyncMode {
    Initial,
    Incremental,
}

impl ProviderSyncMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Incremental => "incremental",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSyncNextStatus {
    InitialSync,
    Active,
}

impl ProviderSyncNextStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::InitialSync => "initial_sync",
            Self::Active => "active",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSyncRunOutcome {
    pub next_status: ProviderSyncNextStatus,
    pub completed: bool,
}

impl ProviderSyncRunOutcome {
    #[must_use]
    pub fn completed_active() -> Self {
        Self {
            next_status: ProviderSyncNextStatus::Active,
            completed: true,
        }
    }

    #[must_use]
    pub fn incomplete_initial() -> Self {
        Self {
            next_status: ProviderSyncNextStatus::InitialSync,
            completed: false,
        }
    }
}

#[derive(Debug, Clone, Error)]
#[error("provider sync failed: {class}: {message}")]
pub struct ProviderSyncRunError {
    pub class: String,
    pub message: String,
    pub retryable: bool,
    pub retry_after: Option<Duration>,
}

impl ProviderSyncRunError {
    #[must_use]
    pub fn retryable(class: impl Into<String>, message: impl std::fmt::Display) -> Self {
        Self {
            class: class.into(),
            message: safe_error_message(&message),
            retryable: true,
            retry_after: None,
        }
    }

    #[must_use]
    pub fn retryable_after(
        class: impl Into<String>,
        message: impl std::fmt::Display,
        retry_after: Duration,
    ) -> Self {
        Self {
            class: class.into(),
            message: safe_error_message(&message),
            retryable: true,
            retry_after: Some(retry_after),
        }
    }

    #[must_use]
    pub fn permanent(class: impl Into<String>, message: impl std::fmt::Display) -> Self {
        Self {
            class: class.into(),
            message: safe_error_message(&message),
            retryable: false,
            retry_after: None,
        }
    }
}

#[async_trait]
pub trait ProviderSyncRunner: Send + Sync {
    async fn run_initial_sync(
        &self,
        account: ProviderSyncAccount,
        cancel: &CancellationToken,
    ) -> std::result::Result<ProviderSyncRunOutcome, ProviderSyncRunError>;

    async fn run_incremental_sync(
        &self,
        account: ProviderSyncAccount,
        cancel: &CancellationToken,
    ) -> std::result::Result<ProviderSyncRunOutcome, ProviderSyncRunError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderSyncTickSummary {
    pub considered: usize,
    pub initial_runs: usize,
    pub incremental_runs: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: bool,
}

pub async fn process_provider_sync_tick(
    db: &SqlitePool,
    runner: &dyn ProviderSyncRunner,
    now: DateTime<Utc>,
    options: ProviderSyncSchedulerOptions,
    cancel: &CancellationToken,
) -> Result<ProviderSyncTickSummary> {
    let accounts = select_due_gmail_provider_accounts(db, now, options.sync_interval).await?;
    let mut summary = ProviderSyncTickSummary {
        considered: accounts.len(),
        ..ProviderSyncTickSummary::default()
    };

    for account in accounts {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }

        let mode = sync_mode(&account);
        mark_scheduler_attempt_started(db, account.id).await?;

        let run = match mode {
            ProviderSyncMode::Initial => {
                summary.initial_runs += 1;
                cancel_or_complete(cancel, runner.run_initial_sync(account.clone(), cancel)).await
            }
            ProviderSyncMode::Incremental => {
                summary.incremental_runs += 1;
                cancel_or_complete(cancel, runner.run_incremental_sync(account.clone(), cancel))
                    .await
            }
        };

        let Some(run) = run else {
            summary.cancelled = true;
            mark_scheduler_cancelled(db, &account, mode).await?;
            break;
        };

        match run {
            Ok(outcome) => {
                summary.succeeded += 1;
                mark_scheduler_succeeded(db, &account, mode, &outcome).await?;
                info!(
                    provider_account_id = account.id,
                    user_id = account.user_id,
                    mode = mode.as_str(),
                    completed = outcome.completed,
                    "provider sync succeeded"
                );
            }
            Err(error) => {
                summary.failed += 1;
                mark_scheduler_failed(db, &account, mode, &error, now).await?;
                warn!(provider_account_id = account.id, user_id = account.user_id, mode = mode.as_str(), class = %error.class, retryable = error.retryable, "provider sync failed");
            }
        }
    }

    Ok(summary)
}

fn sync_mode(account: &ProviderSyncAccount) -> ProviderSyncMode {
    if account.initial_sync_completed_at.is_none() {
        ProviderSyncMode::Initial
    } else {
        ProviderSyncMode::Incremental
    }
}

async fn cancel_or_complete<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        output = future => Some(output),
    }
}

async fn select_due_gmail_provider_accounts(
    db: &SqlitePool,
    now: DateTime<Utc>,
    sync_interval: Duration,
) -> Result<Vec<ProviderSyncAccount>> {
    let now_s = now.to_rfc3339();
    let interval_secs = i64::try_from(sync_interval.as_secs())
        .unwrap_or(i64::MAX)
        .max(1);
    let due_before = (now - chrono::Duration::seconds(interval_secs)).to_rfc3339();

    sqlx::query_as::<
        _,
        (
            i64,
            i64,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        "SELECT id, user_id, jmap_account_id, provider_account_id, provider_email, sync_status, \
                last_profile_history_id, initial_sync_completed_at, sync_backoff_secs \
         FROM provider_accounts \
         WHERE provider_kind = 'gmail' \
           AND sync_status IN ('initial_sync', 'active', 'error') \
           AND refresh_token_enc IS NOT NULL \
           AND length(refresh_token_enc) >= 29 \
           AND refresh_token_ref IS NULL \
           AND (next_sync_after IS NULL OR next_sync_after <= ?1) \
           AND (last_sync_attempted_at IS NULL OR last_sync_attempted_at <= ?2 \
                OR sync_status IN ('initial_sync', 'error')) \
         ORDER BY CASE sync_status WHEN 'initial_sync' THEN 0 WHEN 'error' THEN 1 ELSE 2 END, \
                  COALESCE(last_sync_attempted_at, ''), id",
    )
    .bind(&now_s)
    .bind(&due_before)
    .fetch_all(db)
    .await
    .context("select due Gmail provider accounts")
    .map(|rows| {
        rows.into_iter()
            .map(
                |(
                    id,
                    user_id,
                    jmap_account_id,
                    provider_account_id,
                    provider_email,
                    sync_status,
                    last_profile_history_id,
                    initial_sync_completed_at,
                    sync_backoff_secs,
                )| ProviderSyncAccount {
                    id,
                    user_id,
                    jmap_account_id,
                    provider_account_id,
                    provider_email,
                    sync_status,
                    last_profile_history_id,
                    initial_sync_completed_at,
                    sync_backoff_secs,
                },
            )
            .collect()
    })
}

async fn mark_scheduler_attempt_started(db: &SqlitePool, provider_account_id: i64) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE provider_accounts SET last_sync_attempted_at = ?1, updated_at = ?1 WHERE id = ?2",
    )
    .bind(now)
    .bind(provider_account_id)
    .execute(db)
    .await
    .with_context(|| format!("mark provider_account {provider_account_id} sync attempted"))?;
    Ok(())
}

async fn mark_scheduler_succeeded(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    mode: ProviderSyncMode,
    outcome: &ProviderSyncRunOutcome,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE provider_accounts \
         SET sync_status = ?1, last_sync_succeeded_at = ?2, initial_sync_completed_at = CASE WHEN ?3 THEN COALESCE(initial_sync_completed_at, ?2) ELSE initial_sync_completed_at END, last_error_class = NULL, \
             last_error_message = NULL, next_sync_after = NULL, sync_backoff_secs = NULL, updated_at = ?2 \
         WHERE id = ?4",
    )
    .bind(outcome.next_status.as_str())
    .bind(&now)
    .bind(outcome.next_status == ProviderSyncNextStatus::Active)
    .bind(account.id)
    .execute(db)
    .await
    .with_context(|| format!("mark provider_account {} sync succeeded", account.id))?;

    let metadata = serde_json::json!({"mode": mode.as_str(), "completed": outcome.completed, "nextStatus": outcome.next_status.as_str()}).to_string();
    audit_scheduler_event(
        db,
        account,
        ProviderSyncOperationKind::Sync,
        ProviderSyncEventType::SyncCompleted,
        ProviderSyncResultStatus::Succeeded,
        None,
        None,
        Some(&metadata),
    )
    .await?;
    Ok(())
}

async fn mark_scheduler_failed(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    mode: ProviderSyncMode,
    error: &ProviderSyncRunError,
    now: DateTime<Utc>,
) -> Result<()> {
    let backoff_secs = if error.retryable {
        next_backoff_secs(account.sync_backoff_secs, error.retry_after)
    } else {
        None
    };
    let next_sync_after = backoff_secs.map(|secs| now + chrono::Duration::seconds(secs));
    let next_sync_after_s = next_sync_after.map(|at| at.to_rfc3339());
    let updated_at = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE provider_accounts \
         SET sync_status = 'error', last_error_class = ?1, last_error_message = ?2, \
             next_sync_after = ?3, sync_backoff_secs = ?4, updated_at = ?5 \
         WHERE id = ?6",
    )
    .bind(&error.class)
    .bind(&error.message)
    .bind(next_sync_after_s.as_deref())
    .bind(backoff_secs)
    .bind(&updated_at)
    .bind(account.id)
    .execute(db)
    .await
    .with_context(|| format!("mark provider_account {} sync failed", account.id))?;

    let metadata = serde_json::json!({"mode": mode.as_str(), "retryable": error.retryable, "nextSyncAfter": next_sync_after_s, "backoffSeconds": backoff_secs}).to_string();
    audit_scheduler_event(
        db,
        account,
        if error.retryable {
            ProviderSyncOperationKind::Retry
        } else {
            ProviderSyncOperationKind::Failure
        },
        if error.retryable {
            ProviderSyncEventType::MessageRetryScheduled
        } else {
            ProviderSyncEventType::SyncFailed
        },
        if error.retryable {
            ProviderSyncResultStatus::Retrying
        } else {
            ProviderSyncResultStatus::Failed
        },
        Some(&error.class),
        Some(&error.message),
        Some(&metadata),
    )
    .await?;
    Ok(())
}

async fn mark_scheduler_cancelled(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    mode: ProviderSyncMode,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE provider_accounts SET last_error_class = 'cancelled', last_error_message = 'sync cancelled', updated_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(account.id)
        .execute(db)
        .await
        .with_context(|| format!("mark provider_account {} sync cancelled", account.id))?;

    let metadata = serde_json::json!({ "mode": mode.as_str() }).to_string();
    audit_scheduler_event(
        db,
        account,
        ProviderSyncOperationKind::Failure,
        ProviderSyncEventType::SyncFailed,
        ProviderSyncResultStatus::Failed,
        Some("cancelled"),
        Some("sync cancelled"),
        Some(&metadata),
    )
    .await?;
    Ok(())
}

fn next_backoff_secs(previous: Option<i64>, retry_after: Option<Duration>) -> Option<i64> {
    if let Some(retry_after) = retry_after {
        let seconds = i64::try_from(retry_after.as_secs()).unwrap_or(MAX_RETRY_BACKOFF_SECS);
        return Some(seconds.clamp(1, MAX_RETRY_BACKOFF_SECS));
    }
    Some(
        previous
            .unwrap_or(INITIAL_RETRY_BACKOFF_SECS / 2)
            .saturating_mul(2)
            .clamp(INITIAL_RETRY_BACKOFF_SECS, MAX_RETRY_BACKOFF_SECS),
    )
}

async fn audit_scheduler_event(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    operation_kind: ProviderSyncOperationKind,
    event_type: ProviderSyncEventType,
    result_status: ProviderSyncResultStatus,
    class: Option<&str>,
    message: Option<&str>,
    metadata_json: Option<&str>,
) -> Result<()> {
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.id,
            operation_kind,
            event_type,
            provider_message_id: None,
            result_status,
            safe_error_code: class,
            safe_error_class: class,
            safe_error_message: message,
            metadata_json,
        },
    )
    .await?;
    Ok(())
}

fn safe_error_message(error: &impl std::fmt::Display) -> String {
    hail_db::provider_error_redaction::safe_provider_error_message(error)
}

pub mod live {
    use super::*;
    use hail_core::{ProviderOAuthTokenKind, ProviderTokenContext, open_provider_oauth_token};

    pub struct LiveProviderSyncRunner {
        db: SqlitePool,
        http: reqwest::Client,
        server_key: [u8; hail_core::KEY_LEN],
        token_decryptor: std::sync::Arc<dyn crate::crypto::TokenDecryptor>,
        client_id: Option<String>,
        client_secret: Option<secrecy::SecretString>,
        token_url: String,
        gmail_api_base_url: String,
        stalwart_jmap_url: String,
    }

    impl LiveProviderSyncRunner {
        pub fn new(
            db: SqlitePool,
            server_key: &secrecy::SecretString,
            token_decryptor: std::sync::Arc<dyn crate::crypto::TokenDecryptor>,
            client_id: Option<String>,
            client_secret: Option<secrecy::SecretString>,
            token_url: Option<String>,
            gmail_api_base_url: Option<String>,
            stalwart_jmap_url: String,
        ) -> Result<Self> {
            Ok(Self {
                db,
                http: provider_worker_http_client()
                    .map_err(|err| anyhow!("build provider sync HTTP client: {err}"))?,
                server_key: hail_core::parse_server_key(server_key)
                    .map_err(|err| anyhow!("parse server key for provider sync: {err}"))?,
                token_decryptor,
                client_id,
                client_secret,
                token_url: token_url
                    .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()),
                gmail_api_base_url: gmail_api_base_url
                    .unwrap_or_else(|| "https://gmail.googleapis.com/gmail/v1/".to_string()),
                stalwart_jmap_url,
            })
        }

        async fn gmail_client(
            &self,
            account: &ProviderSyncAccount,
        ) -> std::result::Result<GmailClient<DbGmailTokenSource>, ProviderSyncRunError> {
            let token_source = DbGmailTokenSource::load(
                &self.db,
                self.http.clone(),
                self.client_id.clone(),
                self.client_secret.clone(),
                self.token_url.clone(),
                &self.server_key,
                account,
            )
            .await
            .map_err(|err| ProviderSyncRunError::permanent("provider_token", err))?;
            GmailClient::with_base_url(self.http.clone(), token_source, &self.gmail_api_base_url)
                .map_err(|err| ProviderSyncRunError::permanent("gmail_client", err))
        }

        async fn importer(
            &self,
            account: &ProviderSyncAccount,
        ) -> std::result::Result<
            (StalwartJmapRfc822Importer, crate::screener::JmapOpsLive),
            ProviderSyncRunError,
        > {
            let token = crate::jmap_helpers::latest_active_token(
                &self.db,
                &self.token_decryptor,
                account.user_id,
            )
            .await
            .map_err(|err| ProviderSyncRunError::retryable("jmap_session", err))?;
            let route_session = hail_jmap::login_bearer(&self.stalwart_jmap_url, token.clone())
                .await
                .map_err(|err| ProviderSyncRunError::retryable("jmap_session", err))?;
            let route_jmap = crate::screener::JmapOpsLive {
                session: std::sync::Arc::new(route_session),
                account_id: account.jmap_account_id.clone(),
            };
            let session = hail_jmap::login_bearer(&self.stalwart_jmap_url, token)
                .await
                .map_err(|err| ProviderSyncRunError::retryable("jmap_session", err))?;
            Ok((StalwartJmapRfc822Importer::new(session), route_jmap))
        }
    }

    impl Drop for LiveProviderSyncRunner {
        fn drop(&mut self) {
            self.server_key.fill(0);
        }
    }

    #[async_trait]
    impl ProviderSyncRunner for LiveProviderSyncRunner {
        async fn run_initial_sync(
            &self,
            account: ProviderSyncAccount,
            cancel: &CancellationToken,
        ) -> std::result::Result<ProviderSyncRunOutcome, ProviderSyncRunError> {
            let gmail = self.gmail_client(&account).await?;
            let (importer, route_jmap) = self.importer(&account).await?;
            let router = ScreenerRfc822ImportRouter::new(&route_jmap);
            let routing_importer = RoutingRfc822Importer::new(&importer, &router);
            let inbox_id = importer
                .inbox_id()
                .await
                .map_err(|err| ProviderSyncRunError::retryable("jmap_mailbox", err))?;
            let options = GmailInitialSyncOptions::into_mailboxes([inbox_id]);
            let account =
                crate::gmail_initial_sync::load_gmail_provider_account_by_id(&self.db, account.id)
                    .await
                    .map_err(|err| ProviderSyncRunError::retryable("database", err))?
                    .ok_or_else(|| {
                        ProviderSyncRunError::permanent(
                            "provider_account_missing",
                            "provider account missing",
                        )
                    })?;
            let summary = run_gmail_initial_sync(
                &self.db,
                account,
                &gmail,
                &routing_importer,
                options,
                cancel,
            )
            .await
            .map_err(classify_initial_sync_error)?;
            Ok(if summary.import.completed {
                ProviderSyncRunOutcome::completed_active()
            } else {
                ProviderSyncRunOutcome::incomplete_initial()
            })
        }

        async fn run_incremental_sync(
            &self,
            account: ProviderSyncAccount,
            cancel: &CancellationToken,
        ) -> std::result::Result<ProviderSyncRunOutcome, ProviderSyncRunError> {
            let gmail = self.gmail_client(&account).await?;
            let (importer, route_jmap) = self.importer(&account).await?;
            let router = ScreenerRfc822ImportRouter::new(&route_jmap);
            let routing_importer = RoutingRfc822Importer::new(&importer, &router);
            let inbox_id = importer
                .inbox_id()
                .await
                .map_err(|err| ProviderSyncRunError::retryable("jmap_mailbox", err))?;
            let options = GmailIncrementalSyncOptions::into_mailboxes([inbox_id]);
            let account = crate::gmail_incremental_sync::load_gmail_incremental_sync_account_by_id(
                &self.db, account.id,
            )
            .await
            .map_err(|err| ProviderSyncRunError::retryable("database", err))?
            .ok_or_else(|| {
                ProviderSyncRunError::permanent(
                    "provider_account_missing",
                    "provider account missing",
                )
            })?;
            let summary = run_gmail_incremental_sync(
                &self.db,
                account,
                &gmail,
                &routing_importer,
                options,
                cancel,
            )
            .await
            .map_err(classify_incremental_sync_error)?;
            Ok(if summary.completed {
                ProviderSyncRunOutcome::completed_active()
            } else {
                ProviderSyncRunOutcome::incomplete_initial()
            })
        }
    }

    #[derive(Debug, Clone)]
    struct DbGmailTokenSource {
        http: reqwest::Client,
        client_id: Option<String>,
        client_secret: Option<secrecy::SecretString>,
        token_url: String,
        refresh_token: SecretString,
    }

    impl DbGmailTokenSource {
        async fn load(
            db: &SqlitePool,
            http: reqwest::Client,
            client_id: Option<String>,
            client_secret: Option<secrecy::SecretString>,
            token_url: String,
            server_key: &[u8; hail_core::KEY_LEN],
            account: &ProviderSyncAccount,
        ) -> Result<Self> {
            let ciphertext: Vec<u8> = sqlx::query_scalar(
                "SELECT refresh_token_enc FROM provider_accounts WHERE id = ?1 AND user_id = ?2",
            )
            .bind(account.id)
            .bind(account.user_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| anyhow!("provider account has no encrypted refresh token"))?;
            let context = ProviderTokenContext::new(
                account.user_id,
                account.id,
                "gmail",
                account.provider_account_id.clone(),
                ProviderOAuthTokenKind::Refresh,
            );
            let token = open_provider_oauth_token(ciphertext, server_key, &context)?;
            Ok(Self {
                http,
                client_id,
                client_secret,
                token_url,
                refresh_token: SecretString::from(token.expose_secret().to_string()),
            })
        }
    }

    #[async_trait]
    impl GmailTokenSource for DbGmailTokenSource {
        async fn bearer_token(&self) -> std::result::Result<SecretString, GmailClientError> {
            use secrecy::ExposeSecret;

            let client_id = self.client_id.as_deref().ok_or_else(|| {
                GmailClientError::token_error(std::io::Error::other(
                    "gmail oauth client id is not configured",
                ))
            })?;
            let client_secret = self.client_secret.as_ref().ok_or_else(|| {
                GmailClientError::token_error(std::io::Error::other(
                    "gmail oauth client secret is not configured",
                ))
            })?;
            let body = {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                form.append_pair("client_id", client_id);
                form.append_pair("client_secret", client_secret.expose_secret());
                form.append_pair("refresh_token", self.refresh_token.expose_secret());
                form.append_pair("grant_type", "refresh_token");
                form.finish()
            };
            let token: GoogleRefreshTokenResponse = self
                .http
                .post(&self.token_url)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(body)
                .send()
                .await
                .map_err(GmailClientError::Request)?
                .error_for_status()
                .map_err(GmailClientError::Request)?
                .json()
                .await
                .map_err(GmailClientError::Request)?;
            Ok(token.access_token)
        }
    }

    #[derive(Debug, serde::Deserialize)]
    struct GoogleRefreshTokenResponse {
        #[serde(deserialize_with = "deserialize_secret")]
        access_token: SecretString,
    }

    fn deserialize_secret<'de, D>(deserializer: D) -> std::result::Result<SecretString, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::Deserialize;
        String::deserialize(deserializer).map(SecretString::from)
    }

    fn classify_initial_sync_error(error: GmailInitialSyncError) -> ProviderSyncRunError {
        match error {
            GmailInitialSyncError::Profile(source) => {
                classify_gmail_client_error("gmail_profile", &source)
            }
            GmailInitialSyncError::ProfileMismatch { .. } => {
                ProviderSyncRunError::permanent("gmail_profile_mismatch", error)
            }
            GmailInitialSyncError::Import(source) => {
                ProviderSyncRunError::retryable("gmail_initial_import", source)
            }
            GmailInitialSyncError::Database(source) => {
                ProviderSyncRunError::retryable("database", source)
            }
        }
    }

    fn classify_incremental_sync_error(error: GmailIncrementalSyncError) -> ProviderSyncRunError {
        match error {
            GmailIncrementalSyncError::MissingHistoryCursor => {
                ProviderSyncRunError::retryable("missing_history_cursor", error)
            }
            GmailIncrementalSyncError::Cancelled => {
                ProviderSyncRunError::retryable("cancelled", error)
            }
            GmailIncrementalSyncError::Database(source) => {
                ProviderSyncRunError::retryable("database", source)
            }
            GmailIncrementalSyncError::GmailHistory(source) => {
                classify_gmail_client_error("gmail_history", &source)
            }
            GmailIncrementalSyncError::Import(source) => {
                ProviderSyncRunError::retryable("gmail_incremental_import", source)
            }
        }
    }

    fn classify_gmail_client_error(class: &str, error: &GmailClientError) -> ProviderSyncRunError {
        match error {
            GmailClientError::Api {
                kind: GmailApiErrorKind::RateLimited | GmailApiErrorKind::Transient,
                retry_after,
                ..
            } => {
                if let Some(retry_after) = *retry_after {
                    ProviderSyncRunError::retryable_after(class, error, retry_after)
                } else {
                    ProviderSyncRunError::retryable(class, error)
                }
            }
            GmailClientError::Request(req) if req.is_timeout() || req.is_connect() => {
                ProviderSyncRunError::retryable(class, error)
            }
            GmailClientError::Api {
                kind: GmailApiErrorKind::Unauthorized | GmailApiErrorKind::PermissionDenied,
                ..
            } => ProviderSyncRunError::permanent(class, error),
            _ => ProviderSyncRunError::retryable(class, error),
        }
    }
}

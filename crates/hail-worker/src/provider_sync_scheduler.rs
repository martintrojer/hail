//! Cancellation-aware Gmail provider-account sync scheduler.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hail_db::provider_audit_sanitizer::safe_provider_account_error_message;
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
    GMAIL_SEND_SCOPE, GmailAccessToken, GmailAccessTokenProvider, GmailApiErrorKind, GmailClient,
    GmailClientError, provider_worker_http_client,
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
const MAX_PROVIDER_RATE_LIMIT_BACKOFF_SECS: i64 = 5 * 60;
const PROVIDER_RATE_LIMIT_ABORT_AFTER_SECS: i64 = 30 * 60;
const PROVIDER_QUOTA_ABORT_FAILURES: i64 = 50;
const PROVIDER_QUOTA_ABORT_RATIO: f64 = 0.10;
const PROVIDER_QUOTA_OPERATOR_MESSAGE: &str =
    "Stalwart upload quota exceeded during initial Gmail import";
const PROVIDER_RATE_LIMIT_OPERATOR_MESSAGE: &str =
    "Stalwart rate limit hit during initial Gmail import";

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
    pub granted_scopes_json: String,
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
    process_provider_sync_accounts(db, runner, now, accounts, cancel).await
}

pub async fn process_provider_sync_accounts(
    db: &SqlitePool,
    runner: &dyn ProviderSyncRunner,
    now: DateTime<Utc>,
    accounts: Vec<ProviderSyncAccount>,
    cancel: &CancellationToken,
) -> Result<ProviderSyncTickSummary> {
    let mut summary = ProviderSyncTickSummary {
        considered: accounts.len(),
        ..ProviderSyncTickSummary::default()
    };

    for account in accounts {
        let mode = sync_mode(&account);
        if cancel.is_cancelled() {
            summary.cancelled = true;
            mark_scheduler_paused(db, &account, mode).await?;
            break;
        }

        let attempt_started_at = now;
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
            mark_scheduler_paused(db, &account, mode).await?;
            break;
        };

        match run {
            Ok(outcome) => {
                if mode == ProviderSyncMode::Initial
                    && !outcome.completed
                    && let Some(error) =
                        initial_sync_abort_error(db, &account, attempt_started_at, now).await?
                {
                    summary.failed += 1;
                    mark_scheduler_initial_sync_aborted(db, &account, &error).await?;
                    warn!(provider_account_id = account.id, user_id = account.user_id, mode = mode.as_str(), class = %error.class, "provider initial sync aborted");
                    continue;
                }
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
            Err(error) if error.class == "operator_paused" => {
                summary.failed += 1;
                mark_scheduler_paused(db, &account, mode).await?;
                info!(
                    provider_account_id = account.id,
                    user_id = account.user_id,
                    mode = mode.as_str(),
                    "provider sync paused by operator"
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
    if account.sync_status == "needs_reauth" {
        ProviderSyncMode::Incremental
    } else if account.initial_sync_completed_at.is_none() {
        ProviderSyncMode::Initial
    } else {
        ProviderSyncMode::Incremental
    }
}

#[must_use]
pub fn gmail_initial_sync_options_for_inbox(
    inbox_id: impl Into<String>,
) -> GmailInitialSyncOptions {
    GmailInitialSyncOptions::into_mailboxes([inbox_id])
}

#[must_use]
pub fn gmail_incremental_sync_options_for_inbox(
    inbox_id: impl Into<String>,
) -> GmailIncrementalSyncOptions {
    GmailIncrementalSyncOptions::into_mailboxes([inbox_id])
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
            String,
        ),
    >(
        "SELECT id, user_id, jmap_account_id, provider_account_id, provider_email, sync_status, \
                last_profile_history_id, initial_sync_completed_at, sync_backoff_secs, granted_scopes_json \
         FROM provider_accounts \
         WHERE provider_kind = 'gmail' \
           AND sync_status IN ('initial_sync', 'active', 'error', 'needs_reauth') \
           AND refresh_token_enc IS NOT NULL \
           AND length(refresh_token_enc) >= 29 \
           AND refresh_token_ref IS NULL \
           AND (next_sync_after IS NULL OR next_sync_after <= ?1) \
           AND (last_sync_attempted_at IS NULL OR last_sync_attempted_at <= ?2 \
                OR sync_status IN ('initial_sync', 'error', 'needs_reauth')) \
         ORDER BY CASE sync_status WHEN 'initial_sync' THEN 0 WHEN 'needs_reauth' THEN 1 WHEN 'error' THEN 2 ELSE 3 END, \
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
                    granted_scopes_json,
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
                    granted_scopes_json,
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
        next_backoff_secs_for_error(&error.class, account.sync_backoff_secs, error.retry_after)
    } else {
        None
    };
    let next_sync_after = backoff_secs.map(|secs| now + chrono::Duration::seconds(secs));
    let next_sync_after_s = next_sync_after.map(|at| at.to_rfc3339());
    let status = if mode == ProviderSyncMode::Initial
        && error.retryable
        && error.class == "provider_rate_limited"
    {
        "initial_sync"
    } else if error.class == "provider_scope_missing" {
        "needs_reauth"
    } else {
        "error"
    };
    let updated_at = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE provider_accounts \
         SET sync_status = ?1, last_error_class = ?2, last_error_message = ?3, \
             next_sync_after = ?4, sync_backoff_secs = ?5, updated_at = ?6 \
         WHERE id = ?7",
    )
    .bind(status)
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

async fn mark_scheduler_paused(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    mode: ProviderSyncMode,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE provider_accounts \
         SET sync_status = 'paused', last_error_class = 'operator_paused', \
             last_error_message = NULL, next_sync_after = NULL, sync_backoff_secs = NULL, updated_at = ?1 \
         WHERE id = ?2",
    )
    .bind(now)
    .bind(account.id)
    .execute(db)
    .await
    .with_context(|| format!("mark provider_account {} sync paused", account.id))?;

    let metadata = serde_json::json!({ "mode": mode.as_str() }).to_string();
    audit_scheduler_event(
        db,
        account,
        ProviderSyncOperationKind::Sync,
        ProviderSyncEventType::SyncPaused,
        ProviderSyncResultStatus::Info,
        Some("operator_paused"),
        Some("Gmail import paused by operator"),
        Some(&metadata),
    )
    .await?;
    Ok(())
}

fn next_backoff_secs_for_error(
    class: &str,
    previous: Option<i64>,
    retry_after: Option<Duration>,
) -> Option<i64> {
    let max_secs = if class == "provider_rate_limited" {
        MAX_PROVIDER_RATE_LIMIT_BACKOFF_SECS
    } else {
        MAX_RETRY_BACKOFF_SECS
    };
    next_backoff_secs_capped(previous, retry_after, max_secs)
}

fn next_backoff_secs_capped(
    previous: Option<i64>,
    retry_after: Option<Duration>,
    max_secs: i64,
) -> Option<i64> {
    if let Some(retry_after) = retry_after {
        let seconds = i64::try_from(retry_after.as_secs()).unwrap_or(max_secs);
        return Some(seconds.clamp(1, max_secs));
    }
    Some(
        previous
            .unwrap_or(INITIAL_RETRY_BACKOFF_SECS / 2)
            .saturating_mul(2)
            .clamp(INITIAL_RETRY_BACKOFF_SECS, max_secs),
    )
}

async fn initial_sync_abort_error(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    attempt_started_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Option<ProviderSyncRunError>> {
    let quota =
        initial_run_message_failure_stats(db, account.id, "provider_quota", attempt_started_at)
            .await?;
    if quota.should_abort_quota() {
        return Ok(Some(ProviderSyncRunError::permanent(
            "provider_quota",
            PROVIDER_QUOTA_OPERATOR_MESSAGE,
        )));
    }

    let rate_limited = initial_run_message_failure_stats(
        db,
        account.id,
        "provider_rate_limited",
        attempt_started_at,
    )
    .await?;
    if rate_limited.failures > 0
        && (now - attempt_started_at).num_seconds() >= PROVIDER_RATE_LIMIT_ABORT_AFTER_SECS
        || provider_rate_limit_exhausted(db, account.id, now).await?
    {
        return Ok(Some(ProviderSyncRunError::permanent(
            "provider_rate_limited",
            PROVIDER_RATE_LIMIT_OPERATOR_MESSAGE,
        )));
    }

    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MessageFailureStats {
    failures: i64,
    attempted: i64,
}

impl MessageFailureStats {
    fn should_abort_quota(self) -> bool {
        self.failures > PROVIDER_QUOTA_ABORT_FAILURES
            || (self.failures > 0
                && self.attempted > 0
                && (self.failures as f64 / self.attempted as f64) > PROVIDER_QUOTA_ABORT_RATIO)
    }
}

async fn initial_run_message_failure_stats(
    db: &SqlitePool,
    provider_account_id: i64,
    class: &str,
    attempt_started_at: DateTime<Utc>,
) -> Result<MessageFailureStats> {
    let since = attempt_started_at.to_rfc3339();
    let failures = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM provider_sync_events \
         WHERE provider_account_id = ?1 AND event_type = 'message_failed' \
           AND safe_error_class = ?2 AND created_at >= ?3",
    )
    .bind(provider_account_id)
    .bind(class)
    .bind(&since)
    .fetch_one(db)
    .await?;
    let attempted = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM provider_sync_events \
         WHERE provider_account_id = ?1 AND event_type IN ('message_failed', 'message_imported', 'message_skipped') \
           AND created_at >= ?2",
    )
    .bind(provider_account_id)
    .bind(&since)
    .fetch_one(db)
    .await?;
    Ok(MessageFailureStats {
        failures,
        attempted,
    })
}

async fn provider_rate_limit_exhausted(
    db: &SqlitePool,
    provider_account_id: i64,
    now: DateTime<Utc>,
) -> Result<bool> {
    let cutoff =
        (now - chrono::Duration::seconds(PROVIDER_RATE_LIMIT_ABORT_AFTER_SECS)).to_rfc3339();
    let failures = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM provider_sync_events \
         WHERE provider_account_id = ?1 AND event_type = 'message_failed' \
           AND safe_error_class = 'provider_rate_limited' AND created_at <= ?2",
    )
    .bind(provider_account_id)
    .bind(cutoff)
    .fetch_one(db)
    .await?;
    Ok(failures > 0)
}

async fn mark_scheduler_initial_sync_aborted(
    db: &SqlitePool,
    account: &ProviderSyncAccount,
    error: &ProviderSyncRunError,
) -> Result<()> {
    let updated_at = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE provider_accounts \
         SET sync_status = 'error', last_error_class = ?1, last_error_message = ?2, \
             next_sync_after = NULL, sync_backoff_secs = NULL, updated_at = ?3 \
         WHERE id = ?4",
    )
    .bind(&error.class)
    .bind(&error.message)
    .bind(&updated_at)
    .bind(account.id)
    .execute(db)
    .await
    .with_context(|| format!("mark provider_account {} initial sync aborted", account.id))?;

    let metadata = serde_json::json!({
        "mode": ProviderSyncMode::Initial.as_str(),
        "dominantErrorClass": error.class,
        "operatorActionRequired": true,
    })
    .to_string();
    audit_scheduler_event(
        db,
        account,
        ProviderSyncOperationKind::Failure,
        ProviderSyncEventType::InitialSyncAborted,
        ProviderSyncResultStatus::Failed,
        Some(&error.class),
        Some(&error.message),
        Some(&metadata),
    )
    .await?;
    Ok(())
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
    safe_provider_account_error_message(error)
}

pub mod live {
    use super::*;
    use crate::gmail_client::CachedGmailTokenSource;
    use crate::gmail_historical_import::GmailHistoricalImportError;
    use crate::rfc822_import::Rfc822ImportError;
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
        initial_import_max_messages: Option<usize>,
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
            initial_import_max_messages: Option<usize>,
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
                initial_import_max_messages,
            })
        }

        async fn gmail_client(
            &self,
            account: &ProviderSyncAccount,
        ) -> std::result::Result<
            GmailClient<CachedGmailTokenSource<DbGmailTokenSource>>,
            ProviderSyncRunError,
        > {
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
            let token_source = CachedGmailTokenSource::new(token_source);
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
            ensure_gmail_send_scope(&account)?;
            let gmail = self.gmail_client(&account).await?;
            let (importer, route_jmap) = self.importer(&account).await?;
            let router = ScreenerRfc822ImportRouter::new(&route_jmap);
            let routing_importer = RoutingRfc822Importer::new(&importer, &router);
            let inbox_id = importer
                .inbox_id()
                .await
                .map_err(|err| ProviderSyncRunError::retryable("jmap_mailbox", err))?;
            let mut options = gmail_initial_sync_options_for_inbox(inbox_id);
            options.historical.max_messages = self.initial_import_max_messages;
            if let Some(max_messages) = self.initial_import_max_messages {
                info!(
                    provider_account_id = account.id,
                    user_id = account.user_id,
                    max_messages,
                    "provider initial Gmail import is bounded by configuration"
                );
            }
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
            ensure_gmail_send_scope(&account)?;
            let gmail = self.gmail_client(&account).await?;
            let (importer, route_jmap) = self.importer(&account).await?;
            let router = ScreenerRfc822ImportRouter::new(&route_jmap);
            let routing_importer = RoutingRfc822Importer::new(&importer, &router);
            let inbox_id = importer
                .inbox_id()
                .await
                .map_err(|err| ProviderSyncRunError::retryable("jmap_mailbox", err))?;
            let options = gmail_incremental_sync_options_for_inbox(inbox_id);
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
    impl GmailAccessTokenProvider for DbGmailTokenSource {
        async fn refresh_access_token(
            &self,
        ) -> std::result::Result<GmailAccessToken, GmailClientError> {
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
            Ok(GmailAccessToken {
                token: token.access_token,
                expires_in: std::time::Duration::from_secs(token.expires_in.unwrap_or(3600)),
            })
        }
    }

    #[derive(Debug, serde::Deserialize)]
    struct GoogleRefreshTokenResponse {
        #[serde(deserialize_with = "deserialize_secret")]
        access_token: SecretString,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    fn deserialize_secret<'de, D>(deserializer: D) -> std::result::Result<SecretString, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::Deserialize;
        String::deserialize(deserializer).map(SecretString::from)
    }

    fn ensure_gmail_send_scope(
        account: &ProviderSyncAccount,
    ) -> std::result::Result<(), ProviderSyncRunError> {
        let scopes = serde_json::from_str::<Vec<String>>(&account.granted_scopes_json)
            .map_err(|err| ProviderSyncRunError::permanent("provider_scope_corrupt", err))?;
        if scopes.iter().any(|scope| scope == GMAIL_SEND_SCOPE) {
            Ok(())
        } else {
            Err(ProviderSyncRunError::permanent(
                "provider_scope_missing",
                "Re-authenticate Gmail to enable outbound sending",
            ))
        }
    }

    fn classify_initial_sync_error(error: GmailInitialSyncError) -> ProviderSyncRunError {
        match error {
            GmailInitialSyncError::Profile(source) => {
                classify_gmail_client_error("gmail_profile", &source)
            }
            GmailInitialSyncError::ProfileMismatch { .. } => {
                ProviderSyncRunError::permanent("gmail_profile_mismatch", error)
            }
            GmailInitialSyncError::Import(GmailHistoricalImportError::Cancelled) => {
                ProviderSyncRunError::permanent(
                    "operator_paused",
                    "Gmail import paused by operator",
                )
            }
            GmailInitialSyncError::Import(source) => classify_historical_import_error(source),
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
            GmailIncrementalSyncError::Cancelled => ProviderSyncRunError::permanent(
                "operator_paused",
                "Gmail import paused by operator",
            ),
            GmailIncrementalSyncError::Database(source) => {
                ProviderSyncRunError::retryable("database", source)
            }
            GmailIncrementalSyncError::GmailHistory(source) => {
                classify_gmail_client_error("gmail_history", &source)
            }
            GmailIncrementalSyncError::Import(source) => classify_historical_import_error(source),
        }
    }

    fn classify_historical_import_error(error: GmailHistoricalImportError) -> ProviderSyncRunError {
        match error {
            GmailHistoricalImportError::Rfc822Import(Rfc822ImportError::StalwartProviderQuota)
            | GmailHistoricalImportError::RoutedImport(
                crate::provider_import_routing::RoutedRfc822ImportError::Import(
                    Rfc822ImportError::StalwartProviderQuota,
                ),
            ) => ProviderSyncRunError::permanent("provider_quota", PROVIDER_QUOTA_OPERATOR_MESSAGE),
            GmailHistoricalImportError::Rfc822Import(
                Rfc822ImportError::StalwartProviderRateLimited,
            )
            | GmailHistoricalImportError::RoutedImport(
                crate::provider_import_routing::RoutedRfc822ImportError::Import(
                    Rfc822ImportError::StalwartProviderRateLimited,
                ),
            ) => ProviderSyncRunError::retryable_after(
                "provider_rate_limited",
                PROVIDER_RATE_LIMIT_OPERATOR_MESSAGE,
                Duration::from_secs(INITIAL_RETRY_BACKOFF_SECS as u64),
            ),
            source => ProviderSyncRunError::retryable("gmail_initial_import", source),
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

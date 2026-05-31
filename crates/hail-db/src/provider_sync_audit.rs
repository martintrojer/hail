//! Provider import/sync audit log helpers.
//!
//! The provider audit stream is intentionally UI-safe and operations-focused:
//! rows identify the hail user, provider account, optional provider message id,
//! operation/event kind, result, redacted error fields, and timestamps. All
//! caller-provided error fields and metadata pass through the centralized
//! provider audit sanitizer before persistence.

use sqlx::{Row, SqlitePool};

use crate::provider_audit_sanitizer::{SafeProviderErrorFields, safe_provider_metadata_json_value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSyncOperationKind {
    Oauth,
    Sync,
    MessageImport,
    MessageSkip,
    Retry,
    Failure,
    Token,
    Disconnect,
}

impl ProviderSyncOperationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Oauth => "oauth",
            Self::Sync => "sync",
            Self::MessageImport => "message_import",
            Self::MessageSkip => "message_skip",
            Self::Retry => "retry",
            Self::Failure => "failure",
            Self::Token => "token",
            Self::Disconnect => "disconnect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSyncEventType {
    OauthConnected,
    SyncStarted,
    SyncCompleted,
    SyncPaused,
    SyncFailed,
    MessageImported,
    MessageSkipped,
    MessageRetryScheduled,
    MessageFailed,
    TokenRevoked,
    Disconnected,
}

impl ProviderSyncEventType {
    fn as_str(self) -> &'static str {
        match self {
            Self::OauthConnected => "oauth_connected",
            Self::SyncStarted => "sync_started",
            Self::SyncCompleted => "sync_completed",
            Self::SyncPaused => "sync_paused",
            Self::SyncFailed => "sync_failed",
            Self::MessageImported => "message_imported",
            Self::MessageSkipped => "message_skipped",
            Self::MessageRetryScheduled => "message_retry_scheduled",
            Self::MessageFailed => "message_failed",
            Self::TokenRevoked => "token_revoked",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSyncResultStatus {
    Started,
    Succeeded,
    Skipped,
    Retrying,
    Failed,
    Info,
}

impl ProviderSyncResultStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Skipped => "skipped",
            Self::Retrying => "retrying",
            Self::Failed => "failed",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewProviderSyncAuditLog<'a> {
    pub user_id: i64,
    pub provider_account_id: i64,
    pub operation_kind: ProviderSyncOperationKind,
    pub event_type: ProviderSyncEventType,
    pub provider_message_id: Option<&'a str>,
    pub result_status: ProviderSyncResultStatus,
    pub safe_error_code: Option<&'a str>,
    pub safe_error_class: Option<&'a str>,
    pub safe_error_message: Option<&'a str>,
    pub metadata_json: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSyncAuditLog {
    pub id: i64,
    pub provider_account_id: i64,
    pub user_id: i64,
    pub operation_kind: String,
    pub event_type: String,
    pub provider_message_id: Option<String>,
    pub result_status: String,
    pub safe_error_code: Option<String>,
    pub safe_error_class: Option<String>,
    pub safe_error_message: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

/// Insert a provider sync/import audit row.
///
/// The database enforces that `user_id` matches the owning provider account, so
/// callers cannot accidentally create cross-user audit rows.
pub async fn insert_provider_sync_audit_log(
    db: &SqlitePool,
    log: NewProviderSyncAuditLog<'_>,
) -> Result<i64, sqlx::Error> {
    let metadata_json = log
        .metadata_json
        .map(validate_and_sanitize_metadata_json)
        .transpose()?;
    let safe_error = SafeProviderErrorFields::new(
        log.safe_error_code,
        log.safe_error_class,
        log.safe_error_message.as_ref(),
    );
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query_scalar(
        "INSERT INTO provider_sync_events \
         (provider_account_id, user_id, operation_kind, event_type, provider_message_id, \
          result_status, safe_error_code, safe_error_class, safe_error_message, metadata_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         RETURNING id",
    )
    .bind(log.provider_account_id)
    .bind(log.user_id)
    .bind(log.operation_kind.as_str())
    .bind(log.event_type.as_str())
    .bind(log.provider_message_id)
    .bind(log.result_status.as_str())
    .bind(safe_error.code.as_deref())
    .bind(safe_error.class.as_deref())
    .bind(safe_error.message.as_deref())
    .bind(metadata_json)
    .bind(now)
    .fetch_one(db)
    .await
}

fn validate_and_sanitize_metadata_json(metadata_json: &str) -> Result<String, sqlx::Error> {
    let value = serde_json::from_str::<serde_json::Value>(metadata_json).map_err(|err| {
        sqlx::Error::Protocol(format!(
            "provider sync audit metadata_json must be valid JSON: {err}"
        ))
    })?;
    Ok(safe_provider_metadata_json_value(&value))
}

/// List provider sync/import audit rows for one user's provider account, newest
/// first. The user filter is part of the query so API callers can safely scope
/// status views by authenticated hail user.
pub async fn list_provider_sync_audit_logs(
    db: &SqlitePool,
    user_id: i64,
    provider_account_id: i64,
    limit: i64,
) -> Result<Vec<ProviderSyncAuditLog>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, provider_account_id, user_id, operation_kind, event_type, \
                provider_message_id, result_status, safe_error_code, safe_error_class, \
                safe_error_message, metadata_json, created_at \
         FROM provider_sync_events \
         WHERE user_id = ?1 AND provider_account_id = ?2 \
         ORDER BY id DESC \
         LIMIT ?3",
    )
    .bind(user_id)
    .bind(provider_account_id)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ProviderSyncAuditLog {
            id: row.get("id"),
            provider_account_id: row.get("provider_account_id"),
            user_id: row.get("user_id"),
            operation_kind: row.get("operation_kind"),
            event_type: row.get("event_type"),
            provider_message_id: row.get("provider_message_id"),
            result_status: row.get("result_status"),
            safe_error_code: row.get("safe_error_code"),
            safe_error_class: row.get("safe_error_class"),
            safe_error_message: row.get("safe_error_message"),
            metadata_json: row.get("metadata_json"),
            created_at: row.get("created_at"),
        })
        .collect())
}

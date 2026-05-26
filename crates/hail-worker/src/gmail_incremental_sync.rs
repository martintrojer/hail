//! Gmail incremental history sync orchestration.

use std::collections::HashSet;

use async_trait::async_trait;
use hail_db::provider_sync_audit::{
    NewProviderSyncAuditLog, ProviderSyncEventType, ProviderSyncOperationKind,
    ProviderSyncResultStatus, insert_provider_sync_audit_log,
};
use reqwest::StatusCode;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::gmail_client::{
    GmailApiErrorKind, GmailClient, GmailClientError, GmailHistoryRecord, GmailTokenSource,
    ListHistoryParams, ListHistoryResponse, ListMessage,
};
use crate::gmail_historical_import::{
    GmailHistoricalImportAccount, GmailHistoricalImportError, GmailHistoricalImportOptions,
    GmailHistoricalImportSummary, GmailHistoricalImporter, GmailHistoricalSource,
    import_gmail_history, import_one_message,
};

const DEFAULT_HISTORY_PAGE_SIZE: u16 = 100;
const MAX_HISTORY_PAGE_SIZE: u16 = 500;
const EXPIRED_CURSOR_CLASS: &str = "gmail_history_cursor_expired";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailIncrementalSyncAccount {
    pub provider_account_id: i64,
    pub user_id: i64,
    pub history_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailIncrementalSyncOptions {
    pub page_size: u16,
    pub max_history_records: Option<usize>,
    /// Optional Gmail history label bound. Read-only import hint only.
    pub label_id: Option<String>,
    /// Gmail history event types to observe.
    ///
    /// Defaults to `messageAdded` only. v1.2 intentionally ignores
    /// labelAdded/labelRemoved history for archive/read/delete/label mirroring;
    /// provider labels are not authoritative after import.
    pub history_types: Vec<String>,
    pub historical_fallback: GmailHistoricalImportOptions,
}

impl GmailIncrementalSyncOptions {
    #[must_use]
    pub fn into_mailboxes(target_mailbox_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut historical_fallback =
            GmailHistoricalImportOptions::into_mailboxes(target_mailbox_ids);
        historical_fallback.resume = false;
        historical_fallback.max_messages = Some(500);
        Self {
            page_size: DEFAULT_HISTORY_PAGE_SIZE,
            max_history_records: Some(1_000),
            label_id: None,
            history_types: vec!["messageAdded".to_owned()],
            historical_fallback,
        }
    }

    fn normalized_page_size(&self, remaining: Option<usize>) -> u16 {
        let mut page_size = self.page_size.clamp(1, MAX_HISTORY_PAGE_SIZE);
        if let Some(remaining) = remaining {
            page_size =
                page_size.min(remaining.clamp(1, usize::from(MAX_HISTORY_PAGE_SIZE)) as u16);
        }
        page_size
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GmailIncrementalSyncSummary {
    pub history_records: usize,
    pub messages_seen: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub failed: usize,
    pub pages: usize,
    pub completed: bool,
    pub fallback_full_sync: bool,
    pub start_history_id: Option<String>,
    pub end_history_id: Option<String>,
    pub fallback_import: Option<GmailHistoricalImportSummary>,
}

#[async_trait]
pub trait GmailIncrementalSource: GmailHistoricalSource {
    async fn list_history(
        &self,
        params: &ListHistoryParams,
    ) -> Result<ListHistoryResponse, GmailClientError>;
}

#[async_trait]
impl<T> GmailIncrementalSource for GmailClient<T>
where
    T: GmailTokenSource,
{
    async fn list_history(
        &self,
        params: &ListHistoryParams,
    ) -> Result<ListHistoryResponse, GmailClientError> {
        self.list_history(params).await
    }
}

#[derive(Debug, Error)]
pub enum GmailIncrementalSyncError {
    #[error("Gmail incremental sync requires a stored history cursor")]
    MissingHistoryCursor,
    #[error("Gmail incremental sync was cancelled")]
    Cancelled,
    #[error("database error during Gmail incremental sync: {0}")]
    Database(#[from] sqlx::Error),
    #[error("gmail history list failed: {0}")]
    GmailHistory(#[source] GmailClientError),
    #[error(transparent)]
    Import(#[from] GmailHistoricalImportError),
}

#[allow(dead_code)]
pub async fn load_gmail_incremental_sync_account(
    db: &SqlitePool,
    provider_account_row_id: i64,
) -> Result<Option<GmailIncrementalSyncAccount>, sqlx::Error> {
    load_gmail_incremental_sync_account_by_id(db, provider_account_row_id).await
}

pub(crate) async fn load_gmail_incremental_sync_account_by_id(
    db: &SqlitePool,
    provider_account_row_id: i64,
) -> Result<Option<GmailIncrementalSyncAccount>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64, Option<String>)>(
        "SELECT id, user_id, last_profile_history_id \
         FROM provider_accounts \
         WHERE id = ?1 AND provider_kind = 'gmail' AND sync_status != 'disconnected'",
    )
    .bind(provider_account_row_id)
    .fetch_optional(db)
    .await
    .map(|row| {
        row.map(
            |(provider_account_id, user_id, history_id)| GmailIncrementalSyncAccount {
                provider_account_id,
                user_id,
                history_id,
            },
        )
    })
}

pub async fn run_gmail_incremental_sync<C, I>(
    db: &SqlitePool,
    account: GmailIncrementalSyncAccount,
    gmail: &C,
    importer: &I,
    options: GmailIncrementalSyncOptions,
    cancel: &CancellationToken,
) -> Result<GmailIncrementalSyncSummary, GmailIncrementalSyncError>
where
    C: GmailIncrementalSource,
    I: GmailHistoricalImporter,
{
    let Some(start_history_id) = account.history_id.clone() else {
        audit_sync_failed(
            db,
            &account,
            "missing_history_cursor",
            "provider account has no stored Gmail history cursor",
        )
        .await?;
        mark_sync_error(
            db,
            account.provider_account_id,
            "missing_history_cursor",
            "provider account has no stored Gmail history cursor",
        )
        .await?;
        return Err(GmailIncrementalSyncError::MissingHistoryCursor);
    };

    let mut summary = GmailIncrementalSyncSummary {
        start_history_id: Some(start_history_id.clone()),
        ..GmailIncrementalSyncSummary::default()
    };
    let mut page_token = None;
    let mut seen_message_ids = HashSet::new();
    let mut high_water_history_id: Option<String> = None;

    audit_sync_started(db, &account, &options, &start_history_id).await?;
    mark_sync_attempt_started(db, account.provider_account_id).await?;

    loop {
        if cancel.is_cancelled() {
            mark_sync_error(
                db,
                account.provider_account_id,
                "cancelled",
                "sync cancelled",
            )
            .await?;
            return Err(GmailIncrementalSyncError::Cancelled);
        }
        if let Some(max_records) = options.max_history_records
            && summary.history_records >= max_records
        {
            summary.completed = false;
            summary.end_history_id = high_water_history_id.clone();
            persist_history_cursor(
                db,
                account.provider_account_id,
                high_water_history_id.as_deref(),
            )
            .await?;
            mark_sync_succeeded(db, account.provider_account_id, false).await?;
            audit_sync_completed(db, &account, &summary).await?;
            return Ok(summary);
        }

        let remaining = options
            .max_history_records
            .map(|max| max.saturating_sub(summary.history_records));
        let params = ListHistoryParams {
            start_history_id: start_history_id.clone(),
            max_results: Some(options.normalized_page_size(remaining)),
            page_token: page_token.clone(),
            label_id: options.label_id.clone(),
            history_types: options.history_types.clone(),
        };

        let response = match gmail.list_history(&params).await {
            Ok(response) => response,
            Err(error) if is_expired_history_cursor(&error) => {
                return run_expired_cursor_fallback(
                    db,
                    account,
                    gmail,
                    importer,
                    options,
                    cancel,
                    &start_history_id,
                    error,
                )
                .await;
            }
            Err(error) => {
                let message = safe_error_message(&error);
                audit_sync_failed(db, &account, "gmail_history_list", &message).await?;
                mark_sync_error(
                    db,
                    account.provider_account_id,
                    "gmail_history_list",
                    &message,
                )
                .await?;
                return Err(GmailIncrementalSyncError::GmailHistory(error));
            }
        };
        summary.pages += 1;
        let response_history_id = response.history_id.clone();
        if let Some(history_id) = response_history_id.clone() {
            high_water_history_id = Some(history_id);
        }

        for record in limit_history_records(
            response.history,
            options.max_history_records,
            summary.history_records,
        ) {
            if cancel.is_cancelled() {
                mark_sync_error(
                    db,
                    account.provider_account_id,
                    "cancelled",
                    "sync cancelled",
                )
                .await?;
                return Err(GmailIncrementalSyncError::Cancelled);
            }
            high_water_history_id = Some(record.id.clone());
            summary.history_records += 1;
            for listed in history_record_messages(record) {
                if seen_message_ids.insert(listed.id.clone()) {
                    summary.messages_seen += 1;
                    let mut import_summary = GmailHistoricalImportSummary::default();
                    import_one_message(
                        db,
                        &GmailHistoricalImportAccount {
                            provider_account_id: account.provider_account_id,
                            user_id: account.user_id,
                        },
                        gmail,
                        importer,
                        &options.historical_fallback,
                        listed,
                        &mut import_summary,
                    )
                    .await?;
                    summary.imported += import_summary.imported;
                    summary.duplicates += import_summary.duplicates;
                    summary.skipped += import_summary.skipped;
                    summary.failed += import_summary.failed;
                }
            }
        }

        if let Some(history_id) = response_history_id {
            high_water_history_id = Some(history_id);
        }

        page_token = response.next_page_token;
        if page_token.is_none() {
            summary.completed = true;
            summary.end_history_id = high_water_history_id.clone().or(Some(start_history_id));
            persist_history_cursor(
                db,
                account.provider_account_id,
                summary.end_history_id.as_deref(),
            )
            .await?;
            mark_sync_succeeded(db, account.provider_account_id, true).await?;
            audit_sync_completed(db, &account, &summary).await?;
            return Ok(summary);
        }
    }
}

async fn run_expired_cursor_fallback<C, I>(
    db: &SqlitePool,
    account: GmailIncrementalSyncAccount,
    gmail: &C,
    importer: &I,
    mut options: GmailIncrementalSyncOptions,
    cancel: &CancellationToken,
    expired_history_id: &str,
    error: GmailClientError,
) -> Result<GmailIncrementalSyncSummary, GmailIncrementalSyncError>
where
    C: GmailIncrementalSource,
    I: GmailHistoricalImporter,
{
    let message = safe_error_message(&error);
    audit_expired_cursor_fallback(db, &account, expired_history_id, &message).await?;
    options.historical_fallback.resume = false;
    let fallback_import = import_gmail_history(
        db,
        GmailHistoricalImportAccount {
            provider_account_id: account.provider_account_id,
            user_id: account.user_id,
        },
        gmail,
        importer,
        options.historical_fallback,
        cancel,
    )
    .await?;

    let end_history_id = latest_provider_history_id(db, account.provider_account_id).await?;
    persist_history_cursor(db, account.provider_account_id, end_history_id.as_deref()).await?;
    mark_sync_succeeded(db, account.provider_account_id, fallback_import.completed).await?;

    let summary = GmailIncrementalSyncSummary {
        imported: fallback_import.imported,
        duplicates: fallback_import.duplicates,
        skipped: fallback_import.skipped,
        failed: fallback_import.failed,
        completed: fallback_import.completed,
        fallback_full_sync: true,
        start_history_id: Some(expired_history_id.to_owned()),
        end_history_id,
        fallback_import: Some(fallback_import),
        ..GmailIncrementalSyncSummary::default()
    };
    audit_sync_completed(db, &account, &summary).await?;
    Ok(summary)
}

fn history_record_messages(record: GmailHistoryRecord) -> Vec<ListMessage> {
    let mut messages = record
        .messages
        .into_iter()
        .map(|message| ListMessage {
            id: message.id,
            thread_id: message.thread_id,
        })
        .collect::<Vec<_>>();
    messages.extend(record.messages_added.into_iter().map(|added| ListMessage {
        id: added.message.id,
        thread_id: added.message.thread_id,
    }));
    messages
}

fn limit_history_records<T>(
    records: Vec<T>,
    max_records: Option<usize>,
    already_seen: usize,
) -> Vec<T> {
    let Some(max_records) = max_records else {
        return records;
    };
    records
        .into_iter()
        .take(max_records.saturating_sub(already_seen))
        .collect()
}

fn is_expired_history_cursor(error: &GmailClientError) -> bool {
    matches!(
        error,
        GmailClientError::Api {
            status: StatusCode::NOT_FOUND,
            kind: GmailApiErrorKind::NotFound,
            ..
        }
    )
}

async fn persist_history_cursor(
    db: &SqlitePool,
    provider_account_id: i64,
    history_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE provider_accounts SET last_profile_history_id = COALESCE(?1, last_profile_history_id), profile_synced_at = ?2, updated_at = ?2 WHERE id = ?3",
    )
    .bind(history_id)
    .bind(now)
    .bind(provider_account_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn latest_provider_history_id(
    db: &SqlitePool,
    provider_account_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT provider_history_id FROM provider_message_mappings WHERE provider_account_id = ?1 AND provider_history_id IS NOT NULL ORDER BY CAST(provider_history_id AS INTEGER) DESC, provider_history_id DESC LIMIT 1",
    )
    .bind(provider_account_id)
    .fetch_optional(db)
    .await
}

async fn mark_sync_attempt_started(
    db: &SqlitePool,
    provider_account_id: i64,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE provider_accounts SET sync_status = 'active', last_sync_attempted_at = ?1, last_error_class = NULL, last_error_message = NULL, updated_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(provider_account_id)
        .execute(db)
        .await?;
    Ok(())
}

async fn mark_sync_succeeded(
    db: &SqlitePool,
    provider_account_id: i64,
    completed: bool,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let status = if completed { "active" } else { "initial_sync" };
    sqlx::query("UPDATE provider_accounts SET sync_status = ?1, last_sync_succeeded_at = ?2, last_error_class = NULL, last_error_message = NULL, updated_at = ?2 WHERE id = ?3")
        .bind(status)
        .bind(now)
        .bind(provider_account_id)
        .execute(db)
        .await?;
    Ok(())
}

async fn mark_sync_error(
    db: &SqlitePool,
    provider_account_id: i64,
    class: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE provider_accounts SET sync_status = 'error', last_error_class = ?1, last_error_message = ?2, updated_at = ?3 WHERE id = ?4")
        .bind(class)
        .bind(message)
        .bind(now)
        .bind(provider_account_id)
        .execute(db)
        .await?;
    Ok(())
}

async fn audit_sync_started(
    db: &SqlitePool,
    account: &GmailIncrementalSyncAccount,
    options: &GmailIncrementalSyncOptions,
    start_history_id: &str,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "mode": "incremental_history",
        "startHistoryId": start_history_id,
        "pageSize": options.page_size,
        "maxHistoryRecords": options.max_history_records,
        "labelId": options.label_id,
        "historyTypes": options.history_types,
    })
    .to_string();
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::Sync,
            event_type: ProviderSyncEventType::SyncStarted,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Started,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: Some(&metadata),
        },
    )
    .await?;
    Ok(())
}

async fn audit_sync_completed(
    db: &SqlitePool,
    account: &GmailIncrementalSyncAccount,
    summary: &GmailIncrementalSyncSummary,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "mode": "incremental_history",
        "historyRecords": summary.history_records,
        "messagesSeen": summary.messages_seen,
        "imported": summary.imported,
        "duplicates": summary.duplicates,
        "skipped": summary.skipped,
        "failed": summary.failed,
        "completed": summary.completed,
        "fallbackFullSync": summary.fallback_full_sync,
        "startHistoryId": summary.start_history_id,
        "endHistoryId": summary.end_history_id,
    })
    .to_string();
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::Sync,
            event_type: ProviderSyncEventType::SyncCompleted,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Succeeded,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: Some(&metadata),
        },
    )
    .await?;
    Ok(())
}

async fn audit_sync_failed(
    db: &SqlitePool,
    account: &GmailIncrementalSyncAccount,
    class: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::Failure,
            event_type: ProviderSyncEventType::SyncFailed,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Failed,
            safe_error_code: Some(class),
            safe_error_class: Some(class),
            safe_error_message: Some(message),
            metadata_json: None,
        },
    )
    .await?;
    Ok(())
}

async fn audit_expired_cursor_fallback(
    db: &SqlitePool,
    account: &GmailIncrementalSyncAccount,
    expired_history_id: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "mode": "incremental_history",
        "expiredHistoryId": expired_history_id,
        "fallback": "bounded_full_sync",
    })
    .to_string();
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::Failure,
            event_type: ProviderSyncEventType::SyncFailed,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Failed,
            safe_error_code: Some(EXPIRED_CURSOR_CLASS),
            safe_error_class: Some(EXPIRED_CURSOR_CLASS),
            safe_error_message: Some(message),
            metadata_json: Some(&metadata),
        },
    )
    .await?;
    Ok(())
}

fn safe_error_message(error: &impl std::fmt::Display) -> String {
    error.to_string().chars().take(240).collect()
}

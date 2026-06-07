//! Backend-agnostic outbound write queue drainer.
//!
//! `hail-cache` records optimistic local mutations in `outbound_changes`. This
//! worker module drains those rows through the `MailBackend` trait and only
//! marks rows applied after the backend accepts the mutation.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use chrono::{DateTime, Utc};
use hail_backend::{
    BackendMsgId, Envelope, Error as BackendError, Keyword, MailBackend, MailboxRole,
};
use hail_db::provider_audit_sanitizer::safe_provider_account_error_message;
use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::info;

const DEFAULT_IDLE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BATCH_ROWS: usize = 1_000;
const INITIAL_RETRY_BACKOFF_SECS: i64 = 60;
const MAX_RETRY_BACKOFF_SECS: i64 = 30 * 60;
const MAX_ATTEMPTS: i64 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxDrainOptions {
    pub idle_interval: Duration,
    pub max_batch_rows: usize,
}

impl Default for OutboxDrainOptions {
    fn default() -> Self {
        Self {
            idle_interval: DEFAULT_IDLE_INTERVAL,
            max_batch_rows: MAX_BATCH_ROWS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundChange {
    pub id: i64,
    pub account_id: i64,
    pub backend_msg_id: String,
    pub change_type: String,
    pub payload_json: String,
    pub attempt_count: i64,
    pub created_at: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OutboxDrainSummary {
    pub selected: usize,
    pub applied: usize,
    pub failed: usize,
    pub cancelled: bool,
}

#[derive(Debug, Error)]
pub enum OutboxDrainError {
    #[error("database error during outbox drain: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid outbound change {id}: {message}")]
    InvalidChange { id: i64, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorClass {
    ProviderQuota,
    ProviderRateLimited,
    ProviderToken,
    ProviderScopeMissing,
    ProviderUnavailable,
    ProviderNotFound,
    ProviderRejected,
    ProviderError,
}

impl ErrorClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderQuota => "provider_quota",
            Self::ProviderRateLimited => "provider_rate_limited",
            Self::ProviderToken => "provider_token",
            Self::ProviderScopeMissing => "provider_scope_missing",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ProviderNotFound => "provider_not_found",
            Self::ProviderRejected => "provider_rejected",
            Self::ProviderError => "provider_error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DrainOperation {
    SetKeywords {
        account_id: i64,
        backend_msg_id: String,
        add: Vec<Keyword>,
        remove: Vec<Keyword>,
        change_ids: Vec<i64>,
    },
    MoveToRole {
        account_id: i64,
        backend_msg_id: String,
        role: MailboxRole,
        change_ids: Vec<i64>,
    },
    DeletePermanently {
        account_id: i64,
        backend_msg_id: String,
        change_ids: Vec<i64>,
    },
    Send {
        account_id: i64,
        rfc822: Vec<u8>,
        envelope: Envelope,
        change_ids: Vec<i64>,
    },
}

impl DrainOperation {
    fn account_id(&self) -> i64 {
        match self {
            Self::SetKeywords { account_id, .. }
            | Self::MoveToRole { account_id, .. }
            | Self::DeletePermanently { account_id, .. }
            | Self::Send { account_id, .. } => *account_id,
        }
    }

    fn change_ids(&self) -> &[i64] {
        match self {
            Self::SetKeywords { change_ids, .. }
            | Self::MoveToRole { change_ids, .. }
            | Self::DeletePermanently { change_ids, .. }
            | Self::Send { change_ids, .. } => change_ids,
        }
    }
}

#[derive(Debug, Deserialize)]
struct KeywordPayload {
    keyword: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RolePayload {
    role: Option<MailboxRole>,
}

#[derive(Debug, Deserialize)]
struct SendPayload {
    rfc822_base64: Option<String>,
    rfc822: Option<String>,
    envelope: Envelope,
}

pub async fn run_outbox_drain_once(
    db: &SqlitePool,
    backends: &BTreeMap<i64, &(dyn MailBackend + Send + Sync)>,
    now: DateTime<Utc>,
    cancel: &CancellationToken,
) -> Result<OutboxDrainSummary, OutboxDrainError> {
    let changes = load_pending_changes(db, now, MAX_BATCH_ROWS).await?;
    process_outbound_changes(db, backends, changes, cancel).await
}

pub async fn process_outbound_changes(
    db: &SqlitePool,
    backends: &BTreeMap<i64, &(dyn MailBackend + Send + Sync)>,
    changes: Vec<OutboundChange>,
    cancel: &CancellationToken,
) -> Result<OutboxDrainSummary, OutboxDrainError> {
    let mut summary = OutboxDrainSummary {
        selected: changes.len(),
        ..OutboxDrainSummary::default()
    };
    let operations = build_operations(changes)?;

    for operation in operations {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let account_id = operation.account_id();
        let change_ids = operation.change_ids().to_vec();
        let Some(backend) = backends.get(&account_id) else {
            mark_failed(
                db,
                account_id,
                &change_ids,
                ErrorClass::ProviderError,
                "mail backend not available for account",
            )
            .await?;
            summary.failed += change_ids.len();
            continue;
        };

        match cancel_or_complete(cancel, apply_operation(*backend, &operation)).await {
            None => {
                summary.cancelled = true;
                break;
            }
            Some(Ok(())) => {
                mark_applied(db, account_id, &change_ids).await?;
                summary.applied += change_ids.len();
            }
            Some(Err(error)) => {
                let class = classify_backend_error(&error);
                let message = safe_provider_account_error_message(&error);
                mark_failed(db, account_id, &change_ids, class, &message).await?;
                summary.failed += change_ids.len();
            }
        }
    }

    Ok(summary)
}

async fn apply_operation(
    backend: &(dyn MailBackend + Send + Sync),
    operation: &DrainOperation,
) -> hail_backend::Result<()> {
    match operation {
        DrainOperation::SetKeywords {
            backend_msg_id,
            add,
            remove,
            ..
        } => {
            backend
                .set_keywords(&BackendMsgId::new(backend_msg_id.clone()), add, remove)
                .await
        }
        DrainOperation::MoveToRole {
            backend_msg_id,
            role,
            ..
        } => {
            backend
                .move_to_role(&BackendMsgId::new(backend_msg_id.clone()), *role)
                .await
        }
        DrainOperation::DeletePermanently { backend_msg_id, .. } => {
            backend
                .delete_permanently(&BackendMsgId::new(backend_msg_id.clone()))
                .await
        }
        DrainOperation::Send {
            rfc822, envelope, ..
        } => backend.send(rfc822, envelope).await.map(|_| ()),
    }
}

pub async fn run_outbox_drain_loop<F, Fut>(
    db: SqlitePool,
    mut backend_factory: F,
    options: OutboxDrainOptions,
    cancel: CancellationToken,
) -> Result<(), OutboxDrainError>
where
    F: FnMut(i64) -> Fut,
    Fut: Future<Output = Option<Box<dyn MailBackend + Send + Sync>>>,
{
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = load_accounts_with_pending_changes(&db, Utc::now()) => {
                let account_ids = result?;
                if account_ids.is_empty() {
                    cancel_aware_sleep(options.idle_interval, &cancel).await;
                    continue;
                }

                let mut owned = Vec::new();
                for account_id in account_ids {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Some(backend) = cancel_or_complete(&cancel, backend_factory(account_id)).await.flatten() {
                        owned.push((account_id, backend));
                    }
                }
                if cancel.is_cancelled() {
                    break;
                }

                let refs = owned
                    .iter()
                    .map(|(id, backend)| (*id, backend.as_ref()))
                    .collect::<BTreeMap<_, _>>();
                let changes = load_pending_changes(&db, Utc::now(), options.max_batch_rows).await?;
                let summary = process_outbound_changes(&db, &refs, changes, &cancel).await?;
                if summary.applied > 0 || summary.failed > 0 {
                    info!(applied = summary.applied, failed = summary.failed, "outbox drain processed");
                }
                if summary.cancelled {
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn load_accounts_with_pending_changes(
    db: &SqlitePool,
    now: DateTime<Utc>,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT account_id FROM outbound_changes \
         WHERE applied_at IS NULL \
           AND attempt_count < ?1 \
           AND datetime(created_at, '+' || MIN(?2, (?3 << MIN(attempt_count, 10))) || ' seconds') <= datetime(?4) \
         ORDER BY account_id",
    )
    .bind(MAX_ATTEMPTS)
    .bind(MAX_RETRY_BACKOFF_SECS)
    .bind(INITIAL_RETRY_BACKOFF_SECS)
    .bind(now.to_rfc3339())
    .fetch_all(db)
    .await
}

async fn load_pending_changes(
    db: &SqlitePool,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<OutboundChange>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, account_id, backend_msg_id, change_type, payload_json, attempt_count, created_at \
         FROM outbound_changes \
         WHERE applied_at IS NULL \
           AND attempt_count < ?1 \
           AND datetime(created_at, '+' || MIN(?2, (?3 << MIN(attempt_count, 10))) || ' seconds') <= datetime(?4) \
         ORDER BY account_id, id \
         LIMIT ?5",
    )
    .bind(MAX_ATTEMPTS)
    .bind(MAX_RETRY_BACKOFF_SECS)
    .bind(INITIAL_RETRY_BACKOFF_SECS)
    .bind(now.to_rfc3339())
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| OutboundChange {
            id: row.get("id"),
            account_id: row.get("account_id"),
            backend_msg_id: row.get("backend_msg_id"),
            change_type: row.get("change_type"),
            payload_json: row.get("payload_json"),
            attempt_count: row.get("attempt_count"),
            created_at: row.get("created_at"),
        })
        .collect())
}

fn build_operations(changes: Vec<OutboundChange>) -> Result<Vec<DrainOperation>, OutboxDrainError> {
    let mut keyword_groups: BTreeMap<(i64, String), KeywordDeltaBuilder> = BTreeMap::new();
    let mut operations = Vec::new();

    for change in changes {
        match change.change_type.as_str() {
            "read" => keyword_groups
                .entry((change.account_id, change.backend_msg_id.clone()))
                .or_insert_with(|| {
                    KeywordDeltaBuilder::new(change.account_id, change.backend_msg_id.clone())
                })
                .push(change.id, Some(Keyword::new("$seen")), None),
            "unread" => keyword_groups
                .entry((change.account_id, change.backend_msg_id.clone()))
                .or_insert_with(|| {
                    KeywordDeltaBuilder::new(change.account_id, change.backend_msg_id.clone())
                })
                .push(change.id, None, Some(Keyword::new("$seen"))),
            "keyword_add" | "keyword_remove" => {
                let keyword = parse_keyword(&change)?;
                let add = (change.change_type == "keyword_add").then_some(keyword.clone());
                let remove = (change.change_type == "keyword_remove").then_some(keyword);
                keyword_groups
                    .entry((change.account_id, change.backend_msg_id.clone()))
                    .or_insert_with(|| {
                        KeywordDeltaBuilder::new(change.account_id, change.backend_msg_id.clone())
                    })
                    .push(change.id, add, remove);
            }
            "role_move" | "trash" | "untrash" => {
                let role = parse_role(&change)?;
                operations.push(DrainOperation::MoveToRole {
                    account_id: change.account_id,
                    backend_msg_id: change.backend_msg_id,
                    role,
                    change_ids: vec![change.id],
                });
            }
            "permanent_delete" => operations.push(DrainOperation::DeletePermanently {
                account_id: change.account_id,
                backend_msg_id: change.backend_msg_id,
                change_ids: vec![change.id],
            }),
            "send" => {
                let payload =
                    serde_json::from_str::<SendPayload>(&change.payload_json).map_err(|err| {
                        OutboxDrainError::InvalidChange {
                            id: change.id,
                            message: err.to_string(),
                        }
                    })?;
                let rfc822 =
                    decode_rfc822_payload(change.id, payload.rfc822_base64, payload.rfc822)?;
                operations.push(DrainOperation::Send {
                    account_id: change.account_id,
                    rfc822,
                    envelope: payload.envelope,
                    change_ids: vec![change.id],
                });
            }
            other => {
                return Err(OutboxDrainError::InvalidChange {
                    id: change.id,
                    message: format!("unsupported change_type {other}"),
                });
            }
        }
    }

    operations.extend(
        keyword_groups
            .into_values()
            .map(KeywordDeltaBuilder::finish),
    );
    operations.sort_by_key(|operation| {
        operation
            .change_ids()
            .iter()
            .min()
            .copied()
            .unwrap_or(i64::MAX)
    });
    Ok(operations)
}

#[derive(Debug, Clone)]
struct KeywordDeltaBuilder {
    account_id: i64,
    backend_msg_id: String,
    add: Vec<Keyword>,
    remove: Vec<Keyword>,
    change_ids: Vec<i64>,
}

impl KeywordDeltaBuilder {
    fn new(account_id: i64, backend_msg_id: String) -> Self {
        Self {
            account_id,
            backend_msg_id,
            add: Vec::new(),
            remove: Vec::new(),
            change_ids: Vec::new(),
        }
    }

    fn push(&mut self, id: i64, add: Option<Keyword>, remove: Option<Keyword>) {
        self.change_ids.push(id);
        if let Some(keyword) = add {
            self.add.push(keyword);
        }
        if let Some(keyword) = remove {
            self.remove.push(keyword);
        }
    }

    fn finish(self) -> DrainOperation {
        DrainOperation::SetKeywords {
            account_id: self.account_id,
            backend_msg_id: self.backend_msg_id,
            add: unique_keywords(self.add),
            remove: unique_keywords(self.remove),
            change_ids: self.change_ids,
        }
    }
}

fn unique_keywords(keywords: Vec<Keyword>) -> Vec<Keyword> {
    let mut values = keywords
        .into_iter()
        .map(|k| k.as_str().to_owned())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values.into_iter().map(Keyword::new).collect()
}

fn parse_keyword(change: &OutboundChange) -> Result<Keyword, OutboxDrainError> {
    let payload = serde_json::from_str::<KeywordPayload>(&change.payload_json).map_err(|err| {
        OutboxDrainError::InvalidChange {
            id: change.id,
            message: err.to_string(),
        }
    })?;
    payload
        .keyword
        .filter(|keyword| !keyword.trim().is_empty())
        .map(Keyword::new)
        .ok_or_else(|| OutboxDrainError::InvalidChange {
            id: change.id,
            message: "missing keyword".to_owned(),
        })
}

fn parse_role(change: &OutboundChange) -> Result<MailboxRole, OutboxDrainError> {
    match change.change_type.as_str() {
        "trash" => Ok(MailboxRole::Trash),
        "untrash" => Ok(MailboxRole::Inbox),
        _ => serde_json::from_str::<RolePayload>(&change.payload_json)
            .map_err(|err| OutboxDrainError::InvalidChange {
                id: change.id,
                message: err.to_string(),
            })?
            .role
            .ok_or_else(|| OutboxDrainError::InvalidChange {
                id: change.id,
                message: "missing role".to_owned(),
            }),
    }
}

fn decode_rfc822_payload(
    id: i64,
    rfc822_base64: Option<String>,
    rfc822: Option<String>,
) -> Result<Vec<u8>, OutboxDrainError> {
    if let Some(encoded) = rfc822_base64 {
        BASE64_STANDARD
            .decode(encoded)
            .map_err(|err| OutboxDrainError::InvalidChange {
                id,
                message: err.to_string(),
            })
    } else if let Some(value) = rfc822 {
        Ok(value.into_bytes())
    } else {
        Err(OutboxDrainError::InvalidChange {
            id,
            message: "missing rfc822".to_owned(),
        })
    }
}

fn classify_backend_error(error: &BackendError) -> ErrorClass {
    match error {
        BackendError::Authentication => ErrorClass::ProviderToken,
        BackendError::RateLimited => ErrorClass::ProviderRateLimited,
        BackendError::TemporarilyUnavailable => ErrorClass::ProviderUnavailable,
        BackendError::UnsupportedCapability { .. } => ErrorClass::ProviderScopeMissing,
        BackendError::NotFound { .. } => ErrorClass::ProviderNotFound,
        BackendError::InvalidRequest(_) => ErrorClass::ProviderRejected,
        BackendError::NotConnected => ErrorClass::ProviderUnavailable,
        BackendError::Other(message) => classify_other_error(message),
    }
}

fn classify_other_error(message: &str) -> ErrorClass {
    let lower = message.to_ascii_lowercase();
    if lower.contains("quota") {
        ErrorClass::ProviderQuota
    } else if lower.contains("rate") || lower.contains("429") {
        ErrorClass::ProviderRateLimited
    } else if lower.contains("scope") || lower.contains("permission") || lower.contains("forbidden")
    {
        ErrorClass::ProviderScopeMissing
    } else if lower.contains("auth") || lower.contains("token") || lower.contains("unauthorized") {
        ErrorClass::ProviderToken
    } else {
        ErrorClass::ProviderError
    }
}

async fn mark_applied(db: &SqlitePool, account_id: i64, ids: &[i64]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    let mut builder =
        sqlx::QueryBuilder::<sqlx::Sqlite>::new("UPDATE outbound_changes SET applied_at = ");
    builder.push_bind(now.clone());
    builder.push(", last_error = NULL WHERE id IN (");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.build().execute(db).await?;

    sqlx::query(
        "UPDATE mail_accounts SET last_error_class = NULL, last_error_message = NULL, updated_at = ?1 \
         WHERE id = ?2 AND NOT EXISTS (SELECT 1 FROM outbound_changes WHERE account_id = ?2 AND applied_at IS NULL AND last_error IS NOT NULL)",
    )
    .bind(now)
    .bind(account_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn mark_failed(
    db: &SqlitePool,
    account_id: i64,
    ids: &[i64],
    class: ErrorClass,
    error: &str,
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "UPDATE outbound_changes SET attempt_count = MIN(attempt_count + 1, ",
    );
    builder.push_bind(MAX_ATTEMPTS);
    builder.push("), last_error = ");
    builder.push_bind(error);
    builder.push(", created_at = ");
    builder.push_bind(Utc::now().to_rfc3339());
    builder.push(" WHERE id IN (");
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.build().execute(db).await?;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE mail_accounts SET last_error_class = ?1, last_error_message = ?2, updated_at = ?3 WHERE id = ?4",
    )
    .bind(class.as_str())
    .bind(error)
    .bind(now)
    .bind(account_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn cancel_or_complete<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        output = future => Some(output),
    }
}

async fn cancel_aware_sleep(delay: Duration, cancel: &CancellationToken) {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {}
        _ = tokio::time::sleep(delay) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use bytes::Bytes;
    use futures_util::stream;
    use hail_backend::{
        BlobRef, Capabilities, Change, Mailbox, Page, PageRequest, Principal, Query, RawMessage,
        SubmissionId, SyncCursor,
    };
    use hail_test::{TempDb, fresh_db_url};
    use std::sync::{Arc, Mutex};

    static CAPS: Capabilities = Capabilities {
        supports_initial_import: false,
        supports_eventsource: false,
        supports_principals_admin: false,
        supports_send: true,
        native_threading: true,
        max_attachment_size: u64::MAX,
        label_path_separator: '/',
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum BackendCall {
        SetKeywords {
            id: String,
            add: Vec<String>,
            remove: Vec<String>,
        },
        MoveToRole {
            id: String,
            role: MailboxRole,
        },
        DeletePermanently {
            id: String,
        },
        Send {
            rfc822: Vec<u8>,
            envelope: Envelope,
        },
    }

    #[derive(Debug, Default)]
    struct FakeBackend {
        calls: Arc<Mutex<Vec<BackendCall>>>,
        fail: Option<BackendError>,
    }

    #[async_trait]
    impl MailBackend for FakeBackend {
        fn capabilities(&self) -> &'static Capabilities {
            &CAPS
        }
        async fn list_message_ids(
            &self,
            _query: &Query,
            _page: &PageRequest,
        ) -> hail_backend::Result<Page<BackendMsgId>> {
            Ok(Page::empty())
        }
        async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
            Ok(RawMessage {
                id: id.clone(),
                thread_id: None,
                rfc822: Bytes::new(),
                keywords: Vec::new(),
                envelope: None,
                received_at_epoch_secs: None,
                size_bytes: None,
                blob_refs: Vec::new(),
                attachments: Vec::new(),
                metadata: Default::default(),
            })
        }
        async fn fetch_blob(&self, _id: &BlobRef) -> hail_backend::Result<Bytes> {
            Ok(Bytes::new())
        }
        async fn set_keywords(
            &self,
            id: &BackendMsgId,
            add: &[Keyword],
            remove: &[Keyword],
        ) -> hail_backend::Result<()> {
            self.calls
                .lock()
                .expect("calls")
                .push(BackendCall::SetKeywords {
                    id: id.as_str().to_owned(),
                    add: add
                        .iter()
                        .map(|keyword| keyword.as_str().to_owned())
                        .collect(),
                    remove: remove
                        .iter()
                        .map(|keyword| keyword.as_str().to_owned())
                        .collect(),
                });
            self.maybe_fail()
        }
        async fn move_to_role(
            &self,
            id: &BackendMsgId,
            role: MailboxRole,
        ) -> hail_backend::Result<()> {
            self.calls
                .lock()
                .expect("calls")
                .push(BackendCall::MoveToRole {
                    id: id.as_str().to_owned(),
                    role,
                });
            self.maybe_fail()
        }
        async fn delete_permanently(&self, id: &BackendMsgId) -> hail_backend::Result<()> {
            self.calls
                .lock()
                .expect("calls")
                .push(BackendCall::DeletePermanently {
                    id: id.as_str().to_owned(),
                });
            self.maybe_fail()
        }
        async fn send(
            &self,
            rfc822: &[u8],
            envelope: &Envelope,
        ) -> hail_backend::Result<SubmissionId> {
            self.calls.lock().expect("calls").push(BackendCall::Send {
                rfc822: rfc822.to_vec(),
                envelope: envelope.clone(),
            });
            self.maybe_fail()?;
            Ok(SubmissionId::new("fake"))
        }
        async fn poll_changes(
            &self,
            cursor: &SyncCursor,
        ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
            Ok((Vec::new(), cursor.clone()))
        }
        async fn watch_changes(&self) -> futures_core::stream::BoxStream<'static, Change> {
            Box::pin(stream::empty())
        }
        async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
            Ok(Vec::new())
        }
        async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
            Ok(Vec::new())
        }
    }

    impl FakeBackend {
        fn maybe_fail(&self) -> hail_backend::Result<()> {
            if let Some(error) = &self.fail {
                return Err(match error {
                    BackendError::Authentication => BackendError::Authentication,
                    BackendError::RateLimited => BackendError::RateLimited,
                    BackendError::TemporarilyUnavailable => BackendError::TemporarilyUnavailable,
                    BackendError::UnsupportedCapability { capability } => {
                        BackendError::UnsupportedCapability { capability }
                    }
                    BackendError::NotFound { kind, id } => BackendError::NotFound {
                        kind,
                        id: id.clone(),
                    },
                    BackendError::InvalidRequest(message) => {
                        BackendError::InvalidRequest(message.clone())
                    }
                    BackendError::NotConnected => BackendError::NotConnected,
                    BackendError::Other(message) => BackendError::Other(message.clone()),
                });
            }
            Ok(())
        }
    }

    async fn setup() -> (SqlitePool, TempDb, i64) {
        let (url, guard) = fresh_db_url("hail-worker-outbox-drain-test");
        let pool = hail_db::connect(&url).await.expect("connect");
        hail_db::migrate(&pool).await.expect("migrate");
        sqlx::query("INSERT INTO users (email, jmap_account_id, created_at) VALUES (?1, ?2, ?3)")
            .bind("outbox@example.com")
            .bind("acct-outbox")
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("insert user");
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?1")
            .bind("outbox@example.com")
            .fetch_one(&pool)
            .await
            .expect("user id");
        let account_id = sqlx::query(
            "INSERT INTO mail_accounts \
             (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
             VALUES (?1, 'acct-outbox', 'gmail', 'gmail', 'gmail-outbox', 'outbox@gmail.example', ?2, 'active', ?3, ?3)",
        )
        .bind(user_id)
        .bind(vec![1_u8; 29])
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("insert account")
        .last_insert_rowid();
        (pool, guard, account_id)
    }

    async fn enqueue(
        pool: &SqlitePool,
        account_id: i64,
        backend_msg_id: &str,
        change_type: &str,
        payload: &str,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO outbound_changes (account_id, backend_msg_id, change_type, payload_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(account_id)
        .bind(backend_msg_id)
        .bind(change_type)
        .bind(payload)
        .bind("2026-01-01T00:00:00Z")
        .execute(pool)
        .await
        .expect("enqueue")
        .last_insert_rowid()
    }

    #[tokio::test]
    async fn pending_rows_are_drained_and_marked_applied() {
        let (pool, _guard, account_id) = setup().await;
        enqueue(&pool, account_id, "m1", "read", "{}").await;
        enqueue(
            &pool,
            account_id,
            "m1",
            "keyword_add",
            r#"{"keyword":"Work"}"#,
        )
        .await;
        enqueue(
            &pool,
            account_id,
            "m1",
            "keyword_remove",
            r#"{"keyword":"Snoozed"}"#,
        )
        .await;
        enqueue(&pool, account_id, "m2", "unread", "{}").await;
        enqueue(
            &pool,
            account_id,
            "m3",
            "role_move",
            r#"{"role":"archive"}"#,
        )
        .await;
        enqueue(&pool, account_id, "m4", "trash", "{}").await;
        enqueue(&pool, account_id, "m5", "untrash", "{}").await;
        enqueue(&pool, account_id, "m6", "permanent_delete", "{}").await;
        enqueue(
            &pool,
            account_id,
            "draft-1",
            "send",
            r#"{"rfc822_base64":"UmF3IG1lc3NhZ2U=","envelope":{"mail_from":"outbox@example.com","rcpt_to":["to@example.com"]}}"#,
        )
        .await;

        let backend = FakeBackend::default();
        let mut backends: BTreeMap<i64, &(dyn MailBackend + Send + Sync)> = BTreeMap::new();
        backends.insert(account_id, &backend);

        let summary =
            run_outbox_drain_once(&pool, &backends, Utc::now(), &CancellationToken::new())
                .await
                .expect("drain");
        assert_eq!(summary.applied, 9);
        assert_eq!(summary.failed, 0);
        assert_eq!(
            backend.calls.lock().expect("calls").clone(),
            vec![
                BackendCall::SetKeywords {
                    id: "m1".to_owned(),
                    add: vec!["$seen".to_owned(), "Work".to_owned()],
                    remove: vec!["Snoozed".to_owned()],
                },
                BackendCall::SetKeywords {
                    id: "m2".to_owned(),
                    add: Vec::new(),
                    remove: vec!["$seen".to_owned()],
                },
                BackendCall::MoveToRole {
                    id: "m3".to_owned(),
                    role: MailboxRole::Archive,
                },
                BackendCall::MoveToRole {
                    id: "m4".to_owned(),
                    role: MailboxRole::Trash,
                },
                BackendCall::MoveToRole {
                    id: "m5".to_owned(),
                    role: MailboxRole::Inbox,
                },
                BackendCall::DeletePermanently {
                    id: "m6".to_owned(),
                },
                BackendCall::Send {
                    rfc822: b"Raw message".to_vec(),
                    envelope: Envelope {
                        mail_from: "outbox@example.com".to_owned(),
                        rcpt_to: vec!["to@example.com".to_owned()],
                    },
                },
            ]
        );
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM outbound_changes WHERE applied_at IS NULL")
                .fetch_one(&pool)
                .await
                .expect("pending");
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn failure_increments_attempt_count_sets_error_class_and_backs_off() {
        let (pool, _guard, account_id) = setup().await;
        let id = enqueue(&pool, account_id, "m1", "read", "{}").await;
        let backend = FakeBackend {
            calls: Arc::default(),
            fail: Some(BackendError::RateLimited),
        };
        let mut backends: BTreeMap<i64, &(dyn MailBackend + Send + Sync)> = BTreeMap::new();
        backends.insert(account_id, &backend);

        let summary =
            run_outbox_drain_once(&pool, &backends, Utc::now(), &CancellationToken::new())
                .await
                .expect("drain");
        assert_eq!(summary.failed, 1);
        let (attempt_count, last_error): (i64, Option<String>) =
            sqlx::query_as("SELECT attempt_count, last_error FROM outbound_changes WHERE id = ?1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("outbound row");
        assert_eq!(attempt_count, 1);
        assert_eq!(
            last_error.as_deref(),
            Some("backend rate limited the operation")
        );
        let class: Option<String> =
            sqlx::query_scalar("SELECT last_error_class FROM mail_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("account error");
        assert_eq!(class.as_deref(), Some("provider_rate_limited"));

        let calls_before = backend.calls.lock().expect("calls").len();
        let second = run_outbox_drain_once(&pool, &backends, Utc::now(), &CancellationToken::new())
            .await
            .expect("second drain");
        assert_eq!(second.selected, 0);
        assert_eq!(backend.calls.lock().expect("calls").len(), calls_before);
    }

    #[tokio::test]
    async fn cancellation_stops_loop_promptly() {
        let (pool, _guard, _account_id) = setup().await;
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        let task = tokio::spawn(async move {
            run_outbox_drain_loop(
                pool,
                |_account_id| async { None },
                OutboxDrainOptions {
                    idle_interval: Duration::from_secs(60),
                    max_batch_rows: 100,
                },
                child,
            )
            .await
        });
        tokio::task::yield_now().await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("loop stopped promptly")
            .expect("join")
            .expect("loop ok");
    }
}

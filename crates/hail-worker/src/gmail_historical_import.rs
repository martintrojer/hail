//! Historical Gmail import orchestration.
//!
//! This module is the worker-side foundation for provider import mode. It keeps
//! Gmail API access, Stalwart/JMAP RFC822 creation, and hail.db idempotency
//! behind narrow traits so tests can use in-memory fakes and production can wire
//! the real Gmail wrapper plus [`crate::rfc822_import::StalwartJmapRfc822Importer`].
//!
//! v1.2 import is intentionally one-way. Gmail labels, archive/read state,
//! Trash, Spam, and Sent are provider-owned signals. This importer may use
//! provider labels and queries as bounded discovery hints, but it must not write
//! Gmail labels or derive authoritative hail/Stalwart state from them after the
//! raw RFC822 import boundary.

use async_trait::async_trait;
use hail_db::provider_audit_sanitizer::safe_provider_account_error_message;
use hail_db::provider_message_mappings::{
    DuplicateProviderMessageMapping, FailedProviderMessageMapping, ImportedProviderMessageMapping,
    ProviderImportStatus, ProviderMessageSeen, clear_provider_message_route_error,
    find_local_mapping_by_content_sha256, find_local_mapping_by_rfc822_message_id,
    get_provider_message_mapping, mark_provider_message_duplicate, mark_provider_message_failed,
    mark_provider_message_imported, mark_provider_message_route_failed,
    record_provider_message_seen,
};
use hail_db::provider_sync_audit::{
    NewProviderSyncAuditLog, ProviderSyncEventType, ProviderSyncOperationKind,
    ProviderSyncResultStatus, insert_provider_sync_audit_log,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::gmail_client::{
    GmailClient, GmailClientError, GmailTokenSource, ListMessage, ListMessagesParams,
    ListMessagesResponse, RawGmailMessage,
};
use crate::rfc822_import::{ImportedRfc822Message, Rfc822ImportRequest, Rfc822Importer};

const DEFAULT_PAGE_SIZE: u16 = 100;
const MAX_PAGE_SIZE: u16 = 500;
const CURSOR_KIND: &str = "gmail_historical_v1";
const SKIP_REASON_ALREADY_MAPPED: &str = "provider_message_already_mapped";
const SKIP_REASON_RFC822_DUPLICATE: &str = "rfc822_message_id_duplicate";
const SKIP_REASON_CONTENT_DUPLICATE: &str = "content_sha256_duplicate";
const SKIP_REASON_PARTIAL_MAPPING: &str = "partial_mapping_without_local_jmap_id";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailHistoricalImportOptions {
    /// Gmail label ids used only to bound/discover import candidates.
    ///
    /// Provider labels are not converted into hail/Stalwart keywords and hail
    /// never adds/removes labels on Gmail during v1.2 import.
    pub label_ids: Vec<String>,
    /// Optional Gmail search query used only as an import/discovery bound.
    pub query: Option<String>,
    pub max_messages: Option<usize>,
    pub page_size: u16,
    /// Include Gmail Spam and Trash in the read/import window.
    ///
    /// The v1.2 default is `false`; local spam/trash/delete decisions remain
    /// Stalwart/hail state and are not mirrored back to Gmail.
    pub include_spam_trash: bool,
    /// Exclude Gmail Sent from the default inbound import window.
    ///
    /// Provider-created Sent copies are handled by outbound sent-copy dedupe
    /// rather than normal inbox import. Set this to `false` only for an
    /// explicit sent-copy import/dedupe pass.
    pub exclude_sent: bool,
    pub target_mailbox_ids: Vec<String>,
    /// Local Stalwart/JMAP keywords to apply at import time.
    ///
    /// These are hail/Stalwart keywords chosen by hail. They are not derived
    /// from Gmail/provider labels in v1.2.
    pub keywords: Vec<String>,
    pub resume: bool,
}

impl GmailHistoricalImportOptions {
    #[must_use]
    pub fn into_mailboxes(target_mailbox_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            label_ids: Vec::new(),
            query: None,
            max_messages: None,
            page_size: DEFAULT_PAGE_SIZE,
            include_spam_trash: false,
            exclude_sent: true,
            target_mailbox_ids: target_mailbox_ids.into_iter().map(Into::into).collect(),
            keywords: Vec::new(),
            resume: true,
        }
    }

    #[must_use]
    fn normalized_page_size(&self, remaining: Option<usize>) -> u16 {
        let mut page_size = self.page_size.clamp(1, MAX_PAGE_SIZE);
        if let Some(remaining) = remaining {
            page_size = page_size.min(remaining.clamp(1, usize::from(MAX_PAGE_SIZE)) as u16);
        }
        page_size
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GmailHistoricalImportSummary {
    pub listed: usize,
    pub fetched: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub failed: usize,
    pub pages: usize,
    pub completed: bool,
    pub bounded: bool,
    pub bound_max_messages: Option<usize>,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GmailHistoricalImportAccount {
    pub provider_account_id: i64,
    pub user_id: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct GmailHistoricalBackfillCursor {
    kind: String,
    label_ids: Vec<String>,
    query: Option<String>,
    include_spam_trash: bool,
    #[serde(default = "default_exclude_sent")]
    exclude_sent: bool,
    #[serde(default)]
    max_messages: Option<usize>,
    next_page_token: Option<String>,
    processed: usize,
    completed: bool,
}

impl GmailHistoricalBackfillCursor {
    fn new(options: &GmailHistoricalImportOptions) -> Self {
        Self {
            kind: CURSOR_KIND.to_string(),
            label_ids: options.label_ids.clone(),
            query: options.query.clone(),
            include_spam_trash: options.include_spam_trash,
            exclude_sent: options.exclude_sent,
            max_messages: options.max_messages,
            next_page_token: None,
            processed: 0,
            completed: false,
        }
    }

    fn matches_options(&self, options: &GmailHistoricalImportOptions) -> bool {
        self.kind == CURSOR_KIND
            && self.label_ids == options.label_ids
            && self.query == options.query
            && self.include_spam_trash == options.include_spam_trash
            && self.exclude_sent == options.exclude_sent
    }
}

const GMAIL_SENT_QUERY_EXCLUSION: &str = "-in:sent";

fn default_exclude_sent() -> bool {
    true
}

fn effective_gmail_query(options: &GmailHistoricalImportOptions) -> Option<String> {
    match (options.query.as_deref(), options.exclude_sent) {
        (Some(query), true) if query_mentions_sent_exclusion(query) => Some(query.to_owned()),
        (Some(query), true) if query.trim().is_empty() => {
            Some(GMAIL_SENT_QUERY_EXCLUSION.to_owned())
        }
        (Some(query), true) => Some(format!("{} {}", query.trim(), GMAIL_SENT_QUERY_EXCLUSION)),
        (None, true) => Some(GMAIL_SENT_QUERY_EXCLUSION.to_owned()),
        (Some(query), false) if query.trim().is_empty() => None,
        (Some(query), false) => Some(query.to_owned()),
        (None, false) => None,
    }
}

fn query_mentions_sent_exclusion(query: &str) -> bool {
    query
        .split_whitespace()
        .any(|term| term.eq_ignore_ascii_case(GMAIL_SENT_QUERY_EXCLUSION))
}

#[async_trait]
pub trait GmailHistoricalSource: Send + Sync {
    async fn list_messages(
        &self,
        params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError>;

    async fn get_raw_message(&self, message_id: &str) -> Result<RawGmailMessage, GmailClientError>;
}

#[async_trait]
impl<T> GmailHistoricalSource for GmailClient<T>
where
    T: GmailTokenSource,
{
    async fn list_messages(
        &self,
        params: &ListMessagesParams,
    ) -> Result<ListMessagesResponse, GmailClientError> {
        self.list_messages(params).await
    }

    async fn get_raw_message(&self, message_id: &str) -> Result<RawGmailMessage, GmailClientError> {
        self.get_raw_message(message_id).await
    }
}

#[derive(Debug, Error)]
pub enum GmailHistoricalImportError {
    #[error("Gmail historical import requires at least one target Stalwart mailbox")]
    NoTargetMailbox,
    #[error("Gmail historical import was cancelled")]
    Cancelled,
    #[error("database error during Gmail historical import: {0}")]
    Database(#[from] sqlx::Error),
    #[error("failed to serialize Gmail historical import cursor: {0}")]
    CursorSerialize(#[from] serde_json::Error),
    #[error("failed to deserialize Gmail historical import cursor: {0}")]
    CursorDeserialize(#[source] serde_json::Error),
    #[error("gmail list failed: {0}")]
    GmailList(#[source] GmailClientError),
    #[error(transparent)]
    Rfc822Import(#[from] crate::rfc822_import::Rfc822ImportError),
    #[error(transparent)]
    RoutedImport(#[from] crate::provider_import_routing::RoutedRfc822ImportError),
}

#[async_trait]
pub trait GmailHistoricalImporter: Send + Sync {
    async fn import_gmail_rfc822(
        &self,
        db: &SqlitePool,
        user_id: i64,
        request: Rfc822ImportRequest,
    ) -> Result<
        crate::provider_import_routing::RoutedImportedRfc822Message,
        GmailHistoricalImportError,
    >;

    async fn route_imported_gmail_rfc822(
        &self,
        _db: &SqlitePool,
        _user_id: i64,
        _imported: &crate::rfc822_import::ImportedRfc822Message,
        _request: &Rfc822ImportRequest,
    ) -> Result<(), GmailHistoricalImportError> {
        Ok(())
    }
}

#[async_trait]
impl<T> GmailHistoricalImporter for T
where
    T: Rfc822Importer,
{
    async fn import_gmail_rfc822(
        &self,
        _db: &SqlitePool,
        _user_id: i64,
        request: Rfc822ImportRequest,
    ) -> Result<
        crate::provider_import_routing::RoutedImportedRfc822Message,
        GmailHistoricalImportError,
    > {
        Ok(
            crate::provider_import_routing::RoutedImportedRfc822Message {
                imported: self.import_rfc822(request).await?,
                route_outcome: None,
            },
        )
    }
}

#[async_trait]
impl<I, R> GmailHistoricalImporter
    for crate::provider_import_routing::RoutingRfc822Importer<'_, I, R>
where
    I: Rfc822Importer,
    R: crate::provider_import_routing::Rfc822ImportRouter,
{
    async fn import_gmail_rfc822(
        &self,
        _db: &SqlitePool,
        _user_id: i64,
        request: Rfc822ImportRequest,
    ) -> Result<
        crate::provider_import_routing::RoutedImportedRfc822Message,
        GmailHistoricalImportError,
    > {
        Ok(
            crate::provider_import_routing::RoutedImportedRfc822Message {
                imported: self.import_rfc822_only(request).await?,
                route_outcome: None,
            },
        )
    }
    async fn route_imported_gmail_rfc822(
        &self,
        db: &SqlitePool,
        user_id: i64,
        imported: &crate::rfc822_import::ImportedRfc822Message,
        request: &Rfc822ImportRequest,
    ) -> Result<(), GmailHistoricalImportError> {
        self.route_imported_rfc822(db, user_id, imported, request)
            .await?;
        Ok(())
    }
}

pub async fn import_gmail_history<C, I>(
    db: &SqlitePool,
    account: GmailHistoricalImportAccount,
    gmail: &C,
    importer: &I,
    options: GmailHistoricalImportOptions,
    cancel: &CancellationToken,
) -> Result<GmailHistoricalImportSummary, GmailHistoricalImportError>
where
    C: GmailHistoricalSource,
    I: GmailHistoricalImporter,
{
    if options.target_mailbox_ids.is_empty() {
        return Err(GmailHistoricalImportError::NoTargetMailbox);
    }

    let mut summary = GmailHistoricalImportSummary::default();
    let mut cursor = if options.resume {
        match load_cursor(db, account.provider_account_id).await {
            Ok(Some(cursor)) if cursor.matches_options(&options) && !cursor.completed => cursor,
            Ok(_) => GmailHistoricalBackfillCursor::new(&options),
            Err(error) => {
                let message = safe_error_message(&error);
                audit_sync_failed(db, &account, "provider_cursor_corrupt", &message).await?;
                mark_sync_error(
                    db,
                    account.provider_account_id,
                    "provider_cursor_corrupt",
                    &message,
                )
                .await?;
                return Err(error);
            }
        }
    } else {
        GmailHistoricalBackfillCursor::new(&options)
    };

    audit_sync_started(db, &account, &options, cursor.next_page_token.as_deref()).await?;
    mark_sync_attempt_started(db, account.provider_account_id).await?;

    loop {
        if cancel.is_cancelled() {
            mark_sync_error(
                db,
                account.provider_account_id,
                "cancelled",
                "import cancelled",
            )
            .await?;
            return Err(GmailHistoricalImportError::Cancelled);
        }
        if let Some(max_messages) = options.max_messages
            && cursor.processed >= max_messages
        {
            summary.completed = false;
            summary.bounded = true;
            summary.bound_max_messages = Some(max_messages);
            summary.next_page_token = cursor.next_page_token.clone();
            save_cursor(db, account.provider_account_id, &cursor).await?;
            audit_sync_bounded(db, &account, &summary).await?;
            audit_sync_completed(db, &account, &summary).await?;
            mark_sync_succeeded(db, account.provider_account_id, &summary).await?;
            return Ok(summary);
        }

        let remaining = options.max_messages.map(|max| max - cursor.processed);
        let params = ListMessagesParams {
            max_results: Some(options.normalized_page_size(remaining)),
            page_token: cursor.next_page_token.clone(),
            query: effective_gmail_query(&options),
            label_ids: options.label_ids.clone(),
            include_spam_trash: options.include_spam_trash,
        };

        let response = match cancel_or_complete(cancel, gmail.list_messages(&params)).await {
            None => {
                mark_sync_error(
                    db,
                    account.provider_account_id,
                    "cancelled",
                    "import cancelled",
                )
                .await?;
                return Err(GmailHistoricalImportError::Cancelled);
            }
            Some(Ok(response)) => response,
            Some(Err(error)) => {
                let message = safe_error_message(&error);
                audit_sync_failed(db, &account, "gmail_list", &message).await?;
                mark_sync_error(db, account.provider_account_id, "gmail_list", &message).await?;
                return Err(GmailHistoricalImportError::GmailList(error));
            }
        };
        summary.pages += 1;

        let already_processed = cursor.processed;
        let next_page_token = response.next_page_token.clone();
        for listed in
            limit_page_messages(response.messages, options.max_messages, already_processed)
        {
            if cancel.is_cancelled() {
                mark_sync_error(
                    db,
                    account.provider_account_id,
                    "cancelled",
                    "import cancelled",
                )
                .await?;
                return Err(GmailHistoricalImportError::Cancelled);
            }
            summary.listed += 1;
            import_one_message(
                db,
                &account,
                gmail,
                importer,
                &options,
                listed,
                &mut summary,
                cancel,
            )
            .await?;
            cursor.processed += 1;
        }

        cursor.next_page_token = next_page_token;
        cursor.completed = cursor.next_page_token.is_none();
        save_cursor(db, account.provider_account_id, &cursor).await?;

        if cursor.completed {
            summary.completed = true;
            summary.next_page_token = None;
            audit_sync_completed(db, &account, &summary).await?;
            mark_sync_succeeded(db, account.provider_account_id, &summary).await?;
            return Ok(summary);
        }
        summary.next_page_token = cursor.next_page_token.clone();
    }
}

pub(crate) async fn import_one_message<C, I>(
    db: &SqlitePool,
    account: &GmailHistoricalImportAccount,
    gmail: &C,
    importer: &I,
    options: &GmailHistoricalImportOptions,
    listed: ListMessage,
    summary: &mut GmailHistoricalImportSummary,
    cancel: &CancellationToken,
) -> Result<(), GmailHistoricalImportError>
where
    C: GmailHistoricalSource,
    I: GmailHistoricalImporter,
{
    let mut route_retry_mapping = None;
    if let Some(existing) =
        get_provider_message_mapping(db, account.provider_account_id, &listed.id).await?
    {
        match existing.import_status {
            ProviderImportStatus::Imported | ProviderImportStatus::Duplicate
                if existing.jmap_email_id.is_some() && existing.error_class.is_none() =>
            {
                summary.skipped += 1;
                audit_message_skipped(
                    db,
                    account,
                    &listed.id,
                    SKIP_REASON_ALREADY_MAPPED,
                    Some(existing.import_status.as_str()),
                )
                .await?;
                return Ok(());
            }
            ProviderImportStatus::Imported | ProviderImportStatus::Duplicate
                if existing.jmap_email_id.is_some() =>
            {
                audit_message_skipped(
                    db,
                    account,
                    &listed.id,
                    "route_retry_after_import",
                    existing.error_class.as_deref(),
                )
                .await?;
                route_retry_mapping = Some(existing);
            }
            ProviderImportStatus::Skipped => {
                summary.skipped += 1;
                audit_message_skipped(
                    db,
                    account,
                    &listed.id,
                    SKIP_REASON_ALREADY_MAPPED,
                    Some(existing.import_status.as_str()),
                )
                .await?;
                return Ok(());
            }
            ProviderImportStatus::Imported | ProviderImportStatus::Duplicate => {
                audit_message_skipped(
                    db,
                    account,
                    &listed.id,
                    SKIP_REASON_PARTIAL_MAPPING,
                    Some("terminal mapping has no local JMAP id; attempting provider-header/RFC822 reconciliation"),
                )
                .await?;
            }
            ProviderImportStatus::Pending | ProviderImportStatus::Failed => {}
        }
    }

    let raw = match cancel_or_complete(cancel, gmail.get_raw_message(&listed.id)).await {
        None => return Err(GmailHistoricalImportError::Cancelled),
        Some(Ok(raw)) => raw,
        Some(Err(error)) => {
            let message = safe_error_message(&error);
            summary.failed += 1;
            mark_message_failed(
                db,
                account.provider_account_id,
                &listed,
                None,
                None,
                None,
                "gmail_get_raw",
                &message,
            )
            .await?;
            audit_message_failed(db, account, &listed.id, "gmail_get_raw", &message).await?;
            return Ok(());
        }
    };
    summary.fetched += 1;

    let rfc822_message_id = crate::rfc822_import::first_message_id(&raw.rfc822);
    let content_sha256 = Sha256::digest(&raw.rfc822).to_vec();
    let provider_thread_id = raw.thread_id.as_deref().or(listed.thread_id.as_deref());
    let provider_history_id = raw.history_id.as_deref();

    record_provider_message_seen(
        db,
        ProviderMessageSeen {
            provider_account_id: account.provider_account_id,
            provider_message_id: &raw.id,
            provider_thread_id,
            provider_history_id,
            rfc822_message_id: rfc822_message_id.as_deref(),
            content_sha256: Some(&content_sha256),
        },
    )
    .await?;

    if let Some(message_id) = rfc822_message_id.as_deref()
        && let Some(existing) =
            find_local_mapping_by_rfc822_message_id(db, account.provider_account_id, message_id)
                .await?
        && existing.provider_message_id != raw.id
    {
        mark_provider_message_duplicate(
            db,
            DuplicateProviderMessageMapping {
                provider_account_id: account.provider_account_id,
                provider_message_id: &raw.id,
                provider_thread_id,
                provider_history_id,
                rfc822_message_id: Some(message_id),
                content_sha256: Some(&content_sha256),
                duplicate_jmap_email_id: existing.jmap_email_id.as_deref(),
                duplicate_jmap_thread_id: existing.jmap_thread_id.as_deref(),
                duplicate_jmap_mailbox_ids_json: existing.jmap_mailbox_ids_json.as_deref(),
            },
        )
        .await?;
        summary.duplicates += 1;
        audit_message_skipped(
            db,
            account,
            &raw.id,
            SKIP_REASON_RFC822_DUPLICATE,
            Some(message_id),
        )
        .await?;
        return Ok(());
    }

    if let Some(existing) =
        find_local_mapping_by_content_sha256(db, account.provider_account_id, &content_sha256)
            .await?
        && existing.provider_message_id != raw.id
    {
        mark_provider_message_duplicate(
            db,
            DuplicateProviderMessageMapping {
                provider_account_id: account.provider_account_id,
                provider_message_id: &raw.id,
                provider_thread_id,
                provider_history_id,
                rfc822_message_id: rfc822_message_id.as_deref(),
                content_sha256: Some(&content_sha256),
                duplicate_jmap_email_id: existing.jmap_email_id.as_deref(),
                duplicate_jmap_thread_id: existing.jmap_thread_id.as_deref(),
                duplicate_jmap_mailbox_ids_json: existing.jmap_mailbox_ids_json.as_deref(),
            },
        )
        .await?;
        summary.duplicates += 1;
        audit_message_skipped(
            db,
            account,
            &raw.id,
            SKIP_REASON_CONTENT_DUPLICATE,
            Some("raw RFC822 content fingerprint matched an existing local mapping"),
        )
        .await?;
        return Ok(());
    }

    let request = Rfc822ImportRequest {
        raw_rfc822: raw.rfc822,
        mailbox_ids: options.target_mailbox_ids.clone(),
        keywords: options.keywords.clone(),
        received_at: None,
        provider_message_id: Some(raw.id.clone()),
    };

    if let Some(existing) = route_retry_mapping {
        let mailbox_ids = existing
            .jmap_mailbox_ids_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
            .filter(|mailbox_ids| !mailbox_ids.is_empty())
            .unwrap_or_else(|| options.target_mailbox_ids.clone());
        let imported = ImportedRfc822Message {
            jmap_email_id: existing
                .jmap_email_id
                .expect("route retry mapping has local JMAP id"),
            jmap_thread_id: existing.jmap_thread_id,
            jmap_mailbox_ids: mailbox_ids,
            rfc822_message_ids: existing.rfc822_message_id.into_iter().collect(),
            duplicate: existing.import_status == ProviderImportStatus::Duplicate,
        };
        match cancel_or_complete(
            cancel,
            importer.route_imported_gmail_rfc822(db, account.user_id, &imported, &request),
        )
        .await
        {
            None => return Err(GmailHistoricalImportError::Cancelled),
            Some(Ok(())) => {
                clear_provider_message_route_error(db, account.provider_account_id, &raw.id)
                    .await?;
                summary.skipped += 1;
                audit_message_skipped(
                    db,
                    account,
                    &raw.id,
                    "route_retry_succeeded",
                    Some(&imported.jmap_email_id),
                )
                .await?;
                return Ok(());
            }
            Some(Err(error)) => {
                let message = safe_error_message(&error);
                mark_provider_message_route_failed(
                    db,
                    account.provider_account_id,
                    &raw.id,
                    &message,
                )
                .await?;
                summary.failed += 1;
                audit_message_failed(db, account, &raw.id, "route_import", &message).await?;
                return Ok(());
            }
        }
    }

    let routed_import = match cancel_or_complete(
        cancel,
        importer.import_gmail_rfc822(db, account.user_id, request.clone()),
    )
    .await
    {
        None => return Err(GmailHistoricalImportError::Cancelled),
        Some(Ok(routed_import)) => routed_import,
        Some(Err(error)) => {
            let message = safe_error_message(&error);
            summary.failed += 1;
            mark_message_failed(
                db,
                account.provider_account_id,
                &listed,
                provider_history_id,
                rfc822_message_id.as_deref(),
                Some(&content_sha256),
                "stalwart_import",
                &message,
            )
            .await?;
            audit_message_failed(db, account, &raw.id, "stalwart_import", &message).await?;
            return Ok(());
        }
    };
    let imported = routed_import.imported;

    let mailbox_ids_json = serde_json::to_string(&imported.jmap_mailbox_ids)?;
    let stored_message_id = imported
        .rfc822_message_ids
        .first()
        .cloned()
        .or(rfc822_message_id);

    if imported.duplicate {
        mark_provider_message_duplicate(
            db,
            DuplicateProviderMessageMapping {
                provider_account_id: account.provider_account_id,
                provider_message_id: &raw.id,
                provider_thread_id,
                provider_history_id,
                rfc822_message_id: stored_message_id.as_deref(),
                content_sha256: Some(&content_sha256),
                duplicate_jmap_email_id: Some(&imported.jmap_email_id),
                duplicate_jmap_thread_id: imported.jmap_thread_id.as_deref(),
                duplicate_jmap_mailbox_ids_json: Some(&mailbox_ids_json),
            },
        )
        .await?;
    } else {
        mark_provider_message_imported(
            db,
            ImportedProviderMessageMapping {
                provider_account_id: account.provider_account_id,
                provider_message_id: &raw.id,
                provider_thread_id,
                provider_history_id,
                rfc822_message_id: stored_message_id.as_deref(),
                content_sha256: Some(&content_sha256),
                jmap_email_id: &imported.jmap_email_id,
                jmap_thread_id: imported.jmap_thread_id.as_deref(),
                jmap_mailbox_ids_json: Some(&mailbox_ids_json),
            },
        )
        .await?;
    }

    if routed_import.route_outcome.is_none() {
        match cancel_or_complete(
            cancel,
            importer.route_imported_gmail_rfc822(db, account.user_id, &imported, &request),
        )
        .await
        {
            None => return Err(GmailHistoricalImportError::Cancelled),
            Some(Ok(())) => {}
            Some(Err(error)) => {
                let message = safe_error_message(&error);
                mark_provider_message_route_failed(
                    db,
                    account.provider_account_id,
                    &raw.id,
                    &message,
                )
                .await?;
                summary.failed += 1;
                audit_message_failed(db, account, &raw.id, "route_import", &message).await?;
                return Ok(());
            }
        }
    }

    clear_provider_message_route_error(db, account.provider_account_id, &raw.id).await?;
    if imported.duplicate {
        summary.duplicates += 1;
        audit_message_imported(db, account, &raw.id, &imported.jmap_email_id, true).await?;
    } else {
        summary.imported += 1;
        audit_message_imported(db, account, &raw.id, &imported.jmap_email_id, false).await?;
    }

    Ok(())
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

fn limit_page_messages(
    messages: Vec<ListMessage>,
    max_messages: Option<usize>,
    already_listed: usize,
) -> Vec<ListMessage> {
    let Some(max_messages) = max_messages else {
        return messages;
    };
    let remaining = max_messages.saturating_sub(already_listed);
    if remaining == 0 {
        return Vec::new();
    }
    messages.into_iter().take(remaining).collect()
}

async fn mark_message_failed(
    db: &SqlitePool,
    provider_account_id: i64,
    listed: &ListMessage,
    provider_history_id: Option<&str>,
    rfc822_message_id: Option<&str>,
    content_sha256: Option<&[u8]>,
    class: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    mark_provider_message_failed(
        db,
        FailedProviderMessageMapping {
            provider_account_id,
            provider_message_id: &listed.id,
            provider_thread_id: listed.thread_id.as_deref(),
            provider_history_id,
            rfc822_message_id,
            content_sha256,
            error_class: class,
            error_message: Some(message),
        },
    )
    .await?;
    Ok(())
}

async fn load_cursor(
    db: &SqlitePool,
    provider_account_id: i64,
) -> Result<Option<GmailHistoricalBackfillCursor>, GmailHistoricalImportError> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT backfill_cursor_json FROM provider_accounts WHERE id = ?1")
            .bind(provider_account_id)
            .fetch_one(db)
            .await?;
    value
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(GmailHistoricalImportError::CursorDeserialize)
}

async fn save_cursor(
    db: &SqlitePool,
    provider_account_id: i64,
    cursor: &GmailHistoricalBackfillCursor,
) -> Result<(), GmailHistoricalImportError> {
    let now = chrono::Utc::now().to_rfc3339();
    let cursor_json = serde_json::to_string(cursor)?;
    sqlx::query(
        "UPDATE provider_accounts SET backfill_cursor_json = ?1, updated_at = ?2 WHERE id = ?3",
    )
    .bind(cursor_json)
    .bind(now)
    .bind(provider_account_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn mark_sync_attempt_started(
    db: &SqlitePool,
    provider_account_id: i64,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE provider_accounts SET sync_status = 'initial_sync', last_sync_attempted_at = ?1, last_error_class = NULL, last_error_message = NULL, updated_at = ?1 WHERE id = ?2",
    )
    .bind(now)
    .bind(provider_account_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn mark_sync_succeeded(
    db: &SqlitePool,
    provider_account_id: i64,
    summary: &GmailHistoricalImportSummary,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let status = if summary.completed {
        "active"
    } else {
        "initial_sync"
    };
    sqlx::query(
        "UPDATE provider_accounts SET sync_status = ?1, initial_sync_completed_at = CASE WHEN ?2 THEN COALESCE(initial_sync_completed_at, ?3) ELSE initial_sync_completed_at END, last_sync_succeeded_at = ?3, last_error_class = NULL, last_error_message = NULL, updated_at = ?3 WHERE id = ?4",
    )
    .bind(status)
    .bind(summary.completed)
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
    sqlx::query(
        "UPDATE provider_accounts SET sync_status = 'error', last_error_class = ?1, last_error_message = ?2, updated_at = ?3 WHERE id = ?4",
    )
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
    account: &GmailHistoricalImportAccount,
    options: &GmailHistoricalImportOptions,
    page_token: Option<&str>,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "labels": options.label_ids,
        "query": options.query,
        "effectiveQuery": effective_gmail_query(options),
        "maxMessages": options.max_messages,
        "pageSize": options.page_size,
        "includeSpamTrash": options.include_spam_trash,
        "excludeSent": options.exclude_sent,
        "maxMessages": options.max_messages,
        "resuming": page_token.is_some(),
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
    account: &GmailHistoricalImportAccount,
    summary: &GmailHistoricalImportSummary,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "listed": summary.listed,
        "fetched": summary.fetched,
        "imported": summary.imported,
        "duplicates": summary.duplicates,
        "skipped": summary.skipped,
        "failed": summary.failed,
        "pages": summary.pages,
        "completed": summary.completed,
        "bounded": summary.bounded,
        "boundMaxMessages": summary.bound_max_messages,
        "nextPageTokenPresent": summary.next_page_token.is_some(),
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

async fn audit_sync_bounded(
    db: &SqlitePool,
    account: &GmailHistoricalImportAccount,
    summary: &GmailHistoricalImportSummary,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "reason": "configured_initial_import_bound",
        "listed": summary.listed,
        "maxMessages": summary.bound_max_messages,
        "nextPageTokenPresent": summary.next_page_token.is_some(),
    })
    .to_string();
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::MessageSkip,
            event_type: ProviderSyncEventType::MessageSkipped,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Skipped,
            safe_error_code: Some("configured_initial_import_bound"),
            safe_error_class: Some("configured_initial_import_bound"),
            safe_error_message: Some(
                "initial Gmail import stopped at configured max_messages bound",
            ),
            metadata_json: Some(&metadata),
        },
    )
    .await?;
    Ok(())
}

async fn audit_sync_failed(
    db: &SqlitePool,
    account: &GmailHistoricalImportAccount,
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

async fn audit_message_imported(
    db: &SqlitePool,
    account: &GmailHistoricalImportAccount,
    provider_message_id: &str,
    jmap_email_id: &str,
    duplicate: bool,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({
        "jmapEmailId": jmap_email_id,
        "duplicate": duplicate,
    })
    .to_string();
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::MessageImport,
            event_type: ProviderSyncEventType::MessageImported,
            provider_message_id: Some(provider_message_id),
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

async fn audit_message_skipped(
    db: &SqlitePool,
    account: &GmailHistoricalImportAccount,
    provider_message_id: &str,
    reason: &str,
    detail: Option<&str>,
) -> Result<(), sqlx::Error> {
    let metadata = serde_json::json!({ "reason": reason, "detail": detail }).to_string();
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::MessageSkip,
            event_type: ProviderSyncEventType::MessageSkipped,
            provider_message_id: Some(provider_message_id),
            result_status: ProviderSyncResultStatus::Skipped,
            safe_error_code: Some(reason),
            safe_error_class: Some(reason),
            safe_error_message: None,
            metadata_json: Some(&metadata),
        },
    )
    .await?;
    Ok(())
}

async fn audit_message_failed(
    db: &SqlitePool,
    account: &GmailHistoricalImportAccount,
    provider_message_id: &str,
    class: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    insert_provider_sync_audit_log(
        db,
        NewProviderSyncAuditLog {
            user_id: account.user_id,
            provider_account_id: account.provider_account_id,
            operation_kind: ProviderSyncOperationKind::Failure,
            event_type: ProviderSyncEventType::MessageFailed,
            provider_message_id: Some(provider_message_id),
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

fn safe_error_message(error: &impl std::fmt::Display) -> String {
    safe_provider_account_error_message(error)
}

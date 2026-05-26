//! Provider message-to-local-JMAP dedupe mapping helpers.
//!
//! `provider_message_mappings` is the durable idempotency table for provider
//! imports. Provider account id plus provider message id is the primary key used
//! to survive retries and crashes. RFC822 `Message-ID` is a secondary,
//! account-scoped dedupe signal for cases where Gmail exposes the same message
//! under a different provider id.
//!
//! Provider labels, Gmail archive/read/delete state, and Spam/Trash/Sent
//! placement are not mirrored into hail from this table and are not written back
//! to Gmail in v1.2. Provider metadata stored here is import/dedupe evidence;
//! visible mail state after import belongs to local Stalwart/JMAP.
//!
//! Do not store message bodies, raw RFC822, tokens, or other secrets here.

use sqlx::{Row, SqlitePool};

use crate::provider_error_redaction::safe_provider_error_message;

macro_rules! mapping_select_sql {
    ($where_clause:literal) => {
        concat!(
            "SELECT id, provider_account_id, provider_message_id, provider_thread_id, ",
            "provider_history_id, rfc822_message_id, content_sha256, jmap_email_id, ",
            "jmap_thread_id, jmap_mailbox_ids_json, import_status, imported_at, ",
            "last_seen_at, error_class, error_message, created_at, updated_at ",
            "FROM provider_message_mappings ",
            $where_clause
        )
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderImportStatus {
    Pending,
    Imported,
    Duplicate,
    Skipped,
    Failed,
}

impl ProviderImportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Imported => "imported",
            Self::Duplicate => "duplicate",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    fn from_db(value: String) -> Self {
        match value.as_str() {
            "pending" => Self::Pending,
            "imported" => Self::Imported,
            "duplicate" => Self::Duplicate,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => unreachable!("provider_message_mappings.import_status is DB-constrained"),
        }
    }
}

pub const SENT_COPY_REASON_PROVIDER_MESSAGE_ALREADY_MAPPED: &str =
    "provider_message_already_mapped";
pub const SENT_COPY_REASON_LOCAL_SENT_MESSAGE_ID_MATCH: &str = "local_sent_message_id_match";
pub const SENT_COPY_REASON_EXISTING_LOCAL_MESSAGE_ID_MATCH: &str =
    "existing_local_message_id_match";
pub const SENT_COPY_REASON_NO_LOCAL_SENT_MATCH: &str = "no_local_sent_match";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSentCopyImportDecision {
    /// The provider Sent copy is already represented in `provider_message_mappings`.
    SkipAlreadyMapped {
        existing: ProviderMessageMapping,
        reason_class: &'static str,
    },
    /// The provider Sent copy corresponds to an existing local Stalwart sent
    /// object and should be recorded as a duplicate mapping, not imported as a
    /// visible second message.
    DeduplicateToLocal {
        jmap_email_id: String,
        jmap_thread_id: Option<String>,
        jmap_mailbox_ids_json: Option<String>,
        reason_class: &'static str,
    },
    /// No safe local match exists. The caller may import the provider message
    /// through the normal RFC822 import primitive and record it as imported.
    ImportAsProviderMessage { reason_class: &'static str },
}

#[derive(Debug, Clone)]
pub struct LocalSentMessageRef<'a> {
    pub rfc822_message_id: &'a str,
    pub jmap_email_id: &'a str,
    pub jmap_thread_id: Option<&'a str>,
    pub jmap_mailbox_ids_json: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ProviderSentCopyImportInput<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub rfc822_message_id: Option<&'a str>,
    /// Existing local Sent object found from Stalwart/JMAP by the same
    /// account-scoped RFC822 Message-ID or future `X-Hail-Outbound-Id` lookup.
    pub local_sent: Option<LocalSentMessageRef<'a>>,
}

#[derive(Debug, Clone)]
pub struct DedupedProviderSentCopyMapping<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub provider_thread_id: Option<&'a str>,
    pub provider_history_id: Option<&'a str>,
    pub rfc822_message_id: Option<&'a str>,
    pub content_sha256: Option<&'a [u8]>,
    pub duplicate_jmap_email_id: &'a str,
    pub duplicate_jmap_thread_id: Option<&'a str>,
    pub duplicate_jmap_mailbox_ids_json: Option<&'a str>,
    pub reason_class: &'a str,
    pub reason_message: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMessageMapping {
    pub id: i64,
    pub provider_account_id: i64,
    pub provider_message_id: String,
    pub provider_thread_id: Option<String>,
    pub provider_history_id: Option<String>,
    pub rfc822_message_id: Option<String>,
    pub content_sha256: Option<Vec<u8>>,
    pub jmap_email_id: Option<String>,
    pub jmap_thread_id: Option<String>,
    pub jmap_mailbox_ids_json: Option<String>,
    pub import_status: ProviderImportStatus,
    pub imported_at: Option<String>,
    pub last_seen_at: Option<String>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ProviderMessageSeen<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub provider_thread_id: Option<&'a str>,
    pub provider_history_id: Option<&'a str>,
    pub rfc822_message_id: Option<&'a str>,
    pub content_sha256: Option<&'a [u8]>,
}

#[derive(Debug, Clone)]
pub struct ImportedProviderMessageMapping<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub provider_thread_id: Option<&'a str>,
    pub provider_history_id: Option<&'a str>,
    pub rfc822_message_id: Option<&'a str>,
    pub content_sha256: Option<&'a [u8]>,
    pub jmap_email_id: &'a str,
    pub jmap_thread_id: Option<&'a str>,
    pub jmap_mailbox_ids_json: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct DuplicateProviderMessageMapping<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub provider_thread_id: Option<&'a str>,
    pub provider_history_id: Option<&'a str>,
    pub rfc822_message_id: Option<&'a str>,
    pub content_sha256: Option<&'a [u8]>,
    pub duplicate_jmap_email_id: Option<&'a str>,
    pub duplicate_jmap_thread_id: Option<&'a str>,
    pub duplicate_jmap_mailbox_ids_json: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct FailedProviderMessageMapping<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub provider_thread_id: Option<&'a str>,
    pub provider_history_id: Option<&'a str>,
    pub rfc822_message_id: Option<&'a str>,
    pub content_sha256: Option<&'a [u8]>,
    pub error_class: &'a str,
    pub error_message: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct SkippedProviderMessageMapping<'a> {
    pub provider_account_id: i64,
    pub provider_message_id: &'a str,
    pub provider_thread_id: Option<&'a str>,
    pub provider_history_id: Option<&'a str>,
    pub rfc822_message_id: Option<&'a str>,
    pub content_sha256: Option<&'a [u8]>,
    pub reason_class: &'a str,
    pub reason_message: Option<&'a str>,
}

pub async fn record_provider_message_seen(
    db: &SqlitePool,
    seen: ProviderMessageSeen<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    let now = now();
    sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, provider_thread_id, provider_history_id, \
          rfc822_message_id, content_sha256, import_status, last_seen_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?7, ?7) \
         ON CONFLICT(provider_account_id, provider_message_id) DO UPDATE SET \
           provider_thread_id = COALESCE(excluded.provider_thread_id, provider_message_mappings.provider_thread_id), \
           provider_history_id = COALESCE(excluded.provider_history_id, provider_message_mappings.provider_history_id), \
           rfc822_message_id = COALESCE(excluded.rfc822_message_id, provider_message_mappings.rfc822_message_id), \
           content_sha256 = COALESCE(excluded.content_sha256, provider_message_mappings.content_sha256), \
           last_seen_at = excluded.last_seen_at, updated_at = excluded.updated_at",
    )
    .bind(seen.provider_account_id)
    .bind(seen.provider_message_id)
    .bind(seen.provider_thread_id)
    .bind(seen.provider_history_id)
    .bind(seen.rfc822_message_id)
    .bind(seen.content_sha256)
    .bind(&now)
    .execute(db)
    .await?;

    get_provider_message_mapping(db, seen.provider_account_id, seen.provider_message_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn mark_provider_message_imported(
    db: &SqlitePool,
    imported: ImportedProviderMessageMapping<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    let now = now();
    sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, provider_thread_id, provider_history_id, \
          rfc822_message_id, content_sha256, jmap_email_id, jmap_thread_id, jmap_mailbox_ids_json, \
          import_status, imported_at, last_seen_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'imported', ?10, ?10, ?10, ?10) \
         ON CONFLICT(provider_account_id, provider_message_id) DO UPDATE SET \
           provider_thread_id = COALESCE(excluded.provider_thread_id, provider_message_mappings.provider_thread_id), \
           provider_history_id = COALESCE(excluded.provider_history_id, provider_message_mappings.provider_history_id), \
           rfc822_message_id = COALESCE(excluded.rfc822_message_id, provider_message_mappings.rfc822_message_id), \
           content_sha256 = COALESCE(excluded.content_sha256, provider_message_mappings.content_sha256), \
           jmap_email_id = excluded.jmap_email_id, jmap_thread_id = excluded.jmap_thread_id, \
           jmap_mailbox_ids_json = excluded.jmap_mailbox_ids_json, \
           import_status = 'imported', imported_at = excluded.imported_at, \
           last_seen_at = excluded.last_seen_at, error_class = NULL, error_message = NULL, \
           updated_at = excluded.updated_at",
    )
    .bind(imported.provider_account_id)
    .bind(imported.provider_message_id)
    .bind(imported.provider_thread_id)
    .bind(imported.provider_history_id)
    .bind(imported.rfc822_message_id)
    .bind(imported.content_sha256)
    .bind(imported.jmap_email_id)
    .bind(imported.jmap_thread_id)
    .bind(imported.jmap_mailbox_ids_json)
    .bind(&now)
    .execute(db)
    .await?;

    get_provider_message_mapping(
        db,
        imported.provider_account_id,
        imported.provider_message_id,
    )
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn mark_provider_message_duplicate(
    db: &SqlitePool,
    duplicate: DuplicateProviderMessageMapping<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    let now = now();
    sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, provider_thread_id, provider_history_id, \
          rfc822_message_id, content_sha256, jmap_email_id, jmap_thread_id, jmap_mailbox_ids_json, \
          import_status, last_seen_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'duplicate', ?10, ?10, ?10) \
         ON CONFLICT(provider_account_id, provider_message_id) DO UPDATE SET \
           provider_thread_id = COALESCE(excluded.provider_thread_id, provider_message_mappings.provider_thread_id), \
           provider_history_id = COALESCE(excluded.provider_history_id, provider_message_mappings.provider_history_id), \
           rfc822_message_id = COALESCE(excluded.rfc822_message_id, provider_message_mappings.rfc822_message_id), \
           content_sha256 = COALESCE(excluded.content_sha256, provider_message_mappings.content_sha256), \
           jmap_email_id = COALESCE(excluded.jmap_email_id, provider_message_mappings.jmap_email_id), \
           jmap_thread_id = COALESCE(excluded.jmap_thread_id, provider_message_mappings.jmap_thread_id), \
           jmap_mailbox_ids_json = COALESCE(excluded.jmap_mailbox_ids_json, provider_message_mappings.jmap_mailbox_ids_json), \
           import_status = 'duplicate', last_seen_at = excluded.last_seen_at, \
           error_class = NULL, error_message = NULL, updated_at = excluded.updated_at",
    )
    .bind(duplicate.provider_account_id)
    .bind(duplicate.provider_message_id)
    .bind(duplicate.provider_thread_id)
    .bind(duplicate.provider_history_id)
    .bind(duplicate.rfc822_message_id)
    .bind(duplicate.content_sha256)
    .bind(duplicate.duplicate_jmap_email_id)
    .bind(duplicate.duplicate_jmap_thread_id)
    .bind(duplicate.duplicate_jmap_mailbox_ids_json)
    .bind(&now)
    .execute(db)
    .await?;

    get_provider_message_mapping(
        db,
        duplicate.provider_account_id,
        duplicate.provider_message_id,
    )
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn mark_provider_sent_copy_deduped(
    db: &SqlitePool,
    deduped: DedupedProviderSentCopyMapping<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    let now = now();
    sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, provider_thread_id, provider_history_id, \
          rfc822_message_id, content_sha256, jmap_email_id, jmap_thread_id, jmap_mailbox_ids_json, \
          import_status, last_seen_at, error_class, error_message, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'duplicate', ?10, ?11, ?12, ?10, ?10) \
         ON CONFLICT(provider_account_id, provider_message_id) DO UPDATE SET \
           provider_thread_id = COALESCE(excluded.provider_thread_id, provider_message_mappings.provider_thread_id), \
           provider_history_id = COALESCE(excluded.provider_history_id, provider_message_mappings.provider_history_id), \
           rfc822_message_id = COALESCE(excluded.rfc822_message_id, provider_message_mappings.rfc822_message_id), \
           content_sha256 = COALESCE(excluded.content_sha256, provider_message_mappings.content_sha256), \
           jmap_email_id = excluded.jmap_email_id, jmap_thread_id = excluded.jmap_thread_id, \
           jmap_mailbox_ids_json = excluded.jmap_mailbox_ids_json, \
           import_status = 'duplicate', last_seen_at = excluded.last_seen_at, \
           error_class = excluded.error_class, error_message = excluded.error_message, \
           updated_at = excluded.updated_at",
    )
    .bind(deduped.provider_account_id)
    .bind(deduped.provider_message_id)
    .bind(deduped.provider_thread_id)
    .bind(deduped.provider_history_id)
    .bind(deduped.rfc822_message_id)
    .bind(deduped.content_sha256)
    .bind(deduped.duplicate_jmap_email_id)
    .bind(deduped.duplicate_jmap_thread_id)
    .bind(deduped.duplicate_jmap_mailbox_ids_json)
    .bind(&now)
    .bind(deduped.reason_class)
    .bind(deduped.reason_message)
    .execute(db)
    .await?;

    get_provider_message_mapping(db, deduped.provider_account_id, deduped.provider_message_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn decide_provider_sent_copy_import(
    db: &SqlitePool,
    input: ProviderSentCopyImportInput<'_>,
) -> Result<ProviderSentCopyImportDecision, sqlx::Error> {
    if let Some(existing) =
        get_provider_message_mapping(db, input.provider_account_id, input.provider_message_id)
            .await?
    {
        return Ok(ProviderSentCopyImportDecision::SkipAlreadyMapped {
            existing,
            reason_class: SENT_COPY_REASON_PROVIDER_MESSAGE_ALREADY_MAPPED,
        });
    }

    if let (Some(provider_message_id), Some(local_sent)) =
        (input.rfc822_message_id, input.local_sent.as_ref())
    {
        if provider_message_id == local_sent.rfc822_message_id {
            return Ok(ProviderSentCopyImportDecision::DeduplicateToLocal {
                jmap_email_id: local_sent.jmap_email_id.to_string(),
                jmap_thread_id: local_sent.jmap_thread_id.map(str::to_string),
                jmap_mailbox_ids_json: local_sent.jmap_mailbox_ids_json.map(str::to_string),
                reason_class: SENT_COPY_REASON_LOCAL_SENT_MESSAGE_ID_MATCH,
            });
        }
    }

    if let Some(message_id) = input.rfc822_message_id {
        if let Some(existing) =
            find_local_mapping_by_rfc822_message_id(db, input.provider_account_id, message_id)
                .await?
        {
            return Ok(ProviderSentCopyImportDecision::DeduplicateToLocal {
                jmap_email_id: existing
                    .jmap_email_id
                    .expect("find_local_mapping_by_rfc822_message_id returns local ids"),
                jmap_thread_id: existing.jmap_thread_id,
                jmap_mailbox_ids_json: existing.jmap_mailbox_ids_json,
                reason_class: SENT_COPY_REASON_EXISTING_LOCAL_MESSAGE_ID_MATCH,
            });
        }
    }

    Ok(ProviderSentCopyImportDecision::ImportAsProviderMessage {
        reason_class: SENT_COPY_REASON_NO_LOCAL_SENT_MATCH,
    })
}

pub async fn mark_provider_message_skipped(
    db: &SqlitePool,
    skipped: SkippedProviderMessageMapping<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    upsert_non_imported_mapping(
        db,
        NonImportedMapping {
            provider_account_id: skipped.provider_account_id,
            provider_message_id: skipped.provider_message_id,
            provider_thread_id: skipped.provider_thread_id,
            provider_history_id: skipped.provider_history_id,
            rfc822_message_id: skipped.rfc822_message_id,
            content_sha256: skipped.content_sha256,
            import_status: ProviderImportStatus::Skipped,
            error_class: skipped.reason_class,
            error_message: skipped
                .reason_message
                .map(|message| safe_provider_error_message(&message)),
        },
    )
    .await
}

pub async fn mark_provider_message_failed(
    db: &SqlitePool,
    failed: FailedProviderMessageMapping<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    upsert_non_imported_mapping(
        db,
        NonImportedMapping {
            provider_account_id: failed.provider_account_id,
            provider_message_id: failed.provider_message_id,
            provider_thread_id: failed.provider_thread_id,
            provider_history_id: failed.provider_history_id,
            rfc822_message_id: failed.rfc822_message_id,
            content_sha256: failed.content_sha256,
            import_status: ProviderImportStatus::Failed,
            error_class: failed.error_class,
            error_message: failed
                .error_message
                .map(|message| safe_provider_error_message(&message)),
        },
    )
    .await
}

struct NonImportedMapping<'a> {
    provider_account_id: i64,
    provider_message_id: &'a str,
    provider_thread_id: Option<&'a str>,
    provider_history_id: Option<&'a str>,
    rfc822_message_id: Option<&'a str>,
    content_sha256: Option<&'a [u8]>,
    import_status: ProviderImportStatus,
    error_class: &'a str,
    error_message: Option<String>,
}

async fn upsert_non_imported_mapping(
    db: &SqlitePool,
    mapping: NonImportedMapping<'_>,
) -> Result<ProviderMessageMapping, sqlx::Error> {
    let now = now();
    sqlx::query(
        "INSERT INTO provider_message_mappings \
         (provider_account_id, provider_message_id, provider_thread_id, provider_history_id, \
          rfc822_message_id, content_sha256, import_status, last_seen_at, error_class, error_message, \
          created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?8, ?8) \
         ON CONFLICT(provider_account_id, provider_message_id) DO UPDATE SET \
           provider_thread_id = COALESCE(excluded.provider_thread_id, provider_message_mappings.provider_thread_id), \
           provider_history_id = COALESCE(excluded.provider_history_id, provider_message_mappings.provider_history_id), \
           rfc822_message_id = COALESCE(excluded.rfc822_message_id, provider_message_mappings.rfc822_message_id), \
           content_sha256 = COALESCE(excluded.content_sha256, provider_message_mappings.content_sha256), \
           import_status = excluded.import_status, last_seen_at = excluded.last_seen_at, \
           error_class = excluded.error_class, error_message = excluded.error_message, \
           updated_at = excluded.updated_at",
    )
    .bind(mapping.provider_account_id)
    .bind(mapping.provider_message_id)
    .bind(mapping.provider_thread_id)
    .bind(mapping.provider_history_id)
    .bind(mapping.rfc822_message_id)
    .bind(mapping.content_sha256)
    .bind(mapping.import_status.as_str())
    .bind(&now)
    .bind(mapping.error_class)
    .bind(mapping.error_message.as_deref())
    .execute(db)
    .await?;

    get_provider_message_mapping(db, mapping.provider_account_id, mapping.provider_message_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_provider_message_mapping(
    db: &SqlitePool,
    provider_account_id: i64,
    provider_message_id: &str,
) -> Result<Option<ProviderMessageMapping>, sqlx::Error> {
    let row = sqlx::query(mapping_select_sql!(
        "WHERE provider_account_id = ?1 AND provider_message_id = ?2"
    ))
    .bind(provider_account_id)
    .bind(provider_message_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(mapping_from_row))
}

/// Fetch an already-localized mapping by account-scoped RFC822 Message-ID.
///
/// This is a secondary duplicate signal. It only returns imported or duplicate
/// rows with a known local `jmap_email_id`, so pending/failed rows do not cause
/// false-positive skips before Stalwart has a stable local identity.
pub async fn find_local_mapping_by_rfc822_message_id(
    db: &SqlitePool,
    provider_account_id: i64,
    rfc822_message_id: &str,
) -> Result<Option<ProviderMessageMapping>, sqlx::Error> {
    let row = sqlx::query(mapping_select_sql!(
        "WHERE provider_account_id = ?1 AND rfc822_message_id = ?2 \
           AND jmap_email_id IS NOT NULL AND import_status IN ('imported', 'duplicate') \
         ORDER BY CASE import_status WHEN 'imported' THEN 0 ELSE 1 END, id ASC \
         LIMIT 1"
    ))
    .bind(provider_account_id)
    .bind(rfc822_message_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(mapping_from_row))
}

/// Fetch an already-localized mapping by account-scoped RFC822 content digest.
///
/// This is the defensive fallback when a provider message has no usable
/// `Message-ID` or when a retry must reconcile a partial mapping. Like the
/// Message-ID lookup, it only returns rows that already have a stable local JMAP
/// identity.
pub async fn find_local_mapping_by_content_sha256(
    db: &SqlitePool,
    provider_account_id: i64,
    content_sha256: &[u8],
) -> Result<Option<ProviderMessageMapping>, sqlx::Error> {
    let row = sqlx::query(mapping_select_sql!(
        "WHERE provider_account_id = ?1 AND content_sha256 = ?2 \
           AND jmap_email_id IS NOT NULL AND import_status IN ('imported', 'duplicate') \
         ORDER BY CASE import_status WHEN 'imported' THEN 0 ELSE 1 END, id ASC \
         LIMIT 1"
    ))
    .bind(provider_account_id)
    .bind(content_sha256)
    .fetch_optional(db)
    .await?;
    Ok(row.map(mapping_from_row))
}

pub async fn list_provider_thread_mappings(
    db: &SqlitePool,
    provider_account_id: i64,
    provider_thread_id: &str,
) -> Result<Vec<ProviderMessageMapping>, sqlx::Error> {
    let rows = sqlx::query(mapping_select_sql!(
        "WHERE provider_account_id = ?1 AND provider_thread_id = ?2 ORDER BY id ASC"
    ))
    .bind(provider_account_id)
    .bind(provider_thread_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(mapping_from_row).collect())
}

fn mapping_from_row(row: sqlx::sqlite::SqliteRow) -> ProviderMessageMapping {
    ProviderMessageMapping {
        id: row.get("id"),
        provider_account_id: row.get("provider_account_id"),
        provider_message_id: row.get("provider_message_id"),
        provider_thread_id: row.get("provider_thread_id"),
        provider_history_id: row.get("provider_history_id"),
        rfc822_message_id: row.get("rfc822_message_id"),
        content_sha256: row.get("content_sha256"),
        jmap_email_id: row.get("jmap_email_id"),
        jmap_thread_id: row.get("jmap_thread_id"),
        jmap_mailbox_ids_json: row.get("jmap_mailbox_ids_json"),
        import_status: ProviderImportStatus::from_db(row.get("import_status")),
        imported_at: row.get("imported_at"),
        last_seen_at: row.get("last_seen_at"),
        error_class: row.get("error_class"),
        error_message: row.get("error_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

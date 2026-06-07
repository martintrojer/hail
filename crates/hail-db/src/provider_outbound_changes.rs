//! Provider outbound-change queue helpers for bidirectional sync.
//!
//! Rows in `provider_outbound_changes` are the durable handoff between local
//! hail/JMAP mutations and provider-specific worker push loops. Helpers here
//! intentionally store only ids, change types, and small JSON payloads; never
//! message bodies or OAuth token material.

use serde_json::json;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOutboundChangeType {
    Read,
    Unread,
    LabelAdd,
    LabelRemove,
    Trash,
    Untrash,
}

impl ProviderOutboundChangeType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Unread => "unread",
            Self::LabelAdd => "label_add",
            Self::LabelRemove => "label_remove",
            Self::Trash => "trash",
            Self::Untrash => "untrash",
        }
    }
}

pub async fn enqueue_if_bidi_enabled(
    db: &SqlitePool,
    user_id: i64,
    jmap_email_id: &str,
    change_type: ProviderOutboundChangeType,
    payload_json: &str,
) -> Result<bool, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let result = sqlx::query(
        "INSERT INTO provider_outbound_changes \
         (provider_account_id, jmap_email_id, change_type, payload_json, created_at) \
         SELECT pa.id, ?1, ?2, ?3, ?4 \
         FROM mail_accounts pa \
         INNER JOIN provider_message_mappings pmm ON pmm.provider_account_id = pa.id \
         WHERE pa.user_id = ?5 AND pa.backend_kind = 'gmail' \
           AND pa.sync_status != 'disconnected' \
           AND pa.bidirectional_sync_enabled = 1 \
           AND pmm.jmap_email_id = ?1 \
           AND pmm.import_status IN ('imported','duplicate')",
    )
    .bind(jmap_email_id)
    .bind(change_type.as_str())
    .bind(payload_json)
    .bind(now)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn enqueue_thread_if_bidi_enabled(
    db: &SqlitePool,
    user_id: i64,
    jmap_thread_id: &str,
    change_type: ProviderOutboundChangeType,
    payload_json: &str,
) -> Result<u64, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let result = sqlx::query(
        "INSERT INTO provider_outbound_changes \
         (provider_account_id, jmap_email_id, change_type, payload_json, created_at) \
         SELECT pa.id, pmm.jmap_email_id, ?1, ?2, ?3 \
         FROM mail_accounts pa \
         INNER JOIN provider_message_mappings pmm ON pmm.provider_account_id = pa.id \
         WHERE pa.user_id = ?4 AND pa.backend_kind = 'gmail' \
           AND pa.sync_status != 'disconnected' \
           AND pa.bidirectional_sync_enabled = 1 \
           AND pmm.jmap_thread_id = ?5 \
           AND pmm.jmap_email_id IS NOT NULL \
           AND pmm.import_status IN ('imported','duplicate')",
    )
    .bind(change_type.as_str())
    .bind(payload_json)
    .bind(now)
    .bind(user_id)
    .bind(jmap_thread_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn enqueue_read_state_if_bidi_enabled(
    db: &SqlitePool,
    user_id: i64,
    jmap_email_id: &str,
    read: bool,
) -> Result<bool, sqlx::Error> {
    enqueue_if_bidi_enabled(
        db,
        user_id,
        jmap_email_id,
        if read {
            ProviderOutboundChangeType::Read
        } else {
            ProviderOutboundChangeType::Unread
        },
        "{}",
    )
    .await
}

pub async fn enqueue_thread_read_state_if_bidi_enabled(
    db: &SqlitePool,
    user_id: i64,
    jmap_thread_id: &str,
    read: bool,
) -> Result<u64, sqlx::Error> {
    enqueue_thread_if_bidi_enabled(
        db,
        user_id,
        jmap_thread_id,
        if read {
            ProviderOutboundChangeType::Read
        } else {
            ProviderOutboundChangeType::Unread
        },
        "{}",
    )
    .await
}

pub async fn enqueue_thread_label_change_if_bidi_enabled(
    db: &SqlitePool,
    user_id: i64,
    jmap_thread_id: &str,
    label_name: &str,
    added: bool,
) -> Result<u64, sqlx::Error> {
    let payload = json!({ "label_name": label_name }).to_string();
    enqueue_thread_if_bidi_enabled(
        db,
        user_id,
        jmap_thread_id,
        if added {
            ProviderOutboundChangeType::LabelAdd
        } else {
            ProviderOutboundChangeType::LabelRemove
        },
        &payload,
    )
    .await
}

pub async fn enqueue_thread_trash_change_if_bidi_enabled(
    db: &SqlitePool,
    user_id: i64,
    jmap_thread_id: &str,
    trashed: bool,
) -> Result<u64, sqlx::Error> {
    enqueue_thread_if_bidi_enabled(
        db,
        user_id,
        jmap_thread_id,
        if trashed {
            ProviderOutboundChangeType::Trash
        } else {
            ProviderOutboundChangeType::Untrash
        },
        "{}",
    )
    .await
}

pub async fn pending_outbound_change_count(
    db: &SqlitePool,
    provider_account_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_outbound_changes WHERE provider_account_id = ?1 AND applied_at IS NULL",
    )
    .bind(provider_account_id)
    .fetch_one(db)
    .await
}

pub async fn recently_applied_outbound_change_exists(
    db: &SqlitePool,
    provider_account_id: i64,
    provider_message_id: &str,
    change_type: &str,
    within_seconds: i64,
) -> Result<bool, sqlx::Error> {
    let threshold = (chrono::Utc::now() - chrono::Duration::seconds(within_seconds.max(0)))
        .to_rfc3339();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM provider_outbound_changes poc \
         INNER JOIN provider_message_mappings pmm \
           ON pmm.provider_account_id = poc.provider_account_id \
          AND pmm.jmap_email_id = poc.jmap_email_id \
         WHERE poc.provider_account_id = ?1 \
           AND pmm.provider_message_id = ?2 \
           AND poc.change_type = ?3 \
           AND poc.applied_at IS NOT NULL \
           AND poc.applied_at >= ?4",
    )
    .bind(provider_account_id)
    .bind(provider_message_id)
    .bind(change_type)
    .bind(threshold)
    .fetch_one(db)
    .await?;
    Ok(count > 0)
}

pub async fn outbound_rows_for_tests(db: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query("SELECT change_type, payload_json FROM provider_outbound_changes ORDER BY id")
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.get("change_type"), row.get("payload_json")))
        .collect())
}

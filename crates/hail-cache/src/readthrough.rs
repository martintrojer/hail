//! Metadata read-through implementation for cached mail.

use crate::{CachedMail, Result};
use chrono::{DateTime, TimeZone, Utc};
use hail_backend::{BackendMsgId, BlobRef, Envelope, PageRequest, Query, RawMessage};
use hail_core::{MailCacheMode, MailClassification, SPAM_KEYWORD};
use sqlx::{Row, Sqlite, Transaction};

use crate::{CachedMessage, MailView, MailViewItem, MailViewListOpts, MailViewPage};

const SEEN_KEYWORD: &str = "$seen";
const TRASH_KEYWORD: &str = "$trash";
const DRAFT_KEYWORD: &str = "$draft";
const SPAM_ALIAS_KEYWORD: &str = "$spam";
const ARCHIVE_KEYWORD: &str = "$archive";

impl CachedMail {
    /// List a collapsed mail view from cached SQLite metadata.
    pub async fn list_view(
        &self,
        view: MailView,
        cursor: Option<String>,
        limit: usize,
        opts: MailViewListOpts,
    ) -> Result<MailViewPage> {
        let _ = opts;
        if self.policy.mode == MailCacheMode::Off {
            let page = self
                .backend
                .list_message_ids(
                    &Query::all(),
                    &PageRequest {
                        limit: limit_as_u32(limit),
                        cursor,
                    },
                )
                .await?;
            let mut items = Vec::with_capacity(page.items.len());
            for id in page.items {
                let message = self.backend.get_message(&id).await?;
                let cached = cached_message_from_raw(message);
                if message_matches_view(&cached, view) {
                    items.push(view_item_from_message(cached, view));
                }
            }
            return Ok(MailViewPage {
                items,
                next_cursor: page.next_cursor,
            });
        }

        if !has_cached_metadata(self.db(), self.account_id).await? {
            self.populate_metadata_page(limit).await?;
        }
        let items = select_view_items(self.db(), self.account_id, view, limit).await?;
        Ok(MailViewPage {
            items,
            next_cursor: None,
        })
    }

    /// Count a collapsed mail view from cached SQLite metadata.
    pub async fn count_view(&self, view: MailView, unread_only: bool) -> Result<usize> {
        if self.policy.mode == MailCacheMode::Off {
            let page = self
                .backend
                .list_message_ids(&Query::all(), &PageRequest::first(u32::MAX))
                .await?;
            let mut count = 0_usize;
            for id in page.items {
                let cached = cached_message_from_raw(self.backend.get_message(&id).await?);
                if message_matches_view(&cached, view) && (!unread_only || cached.unread) {
                    count += 1;
                }
            }
            return Ok(count);
        }

        if !has_cached_metadata(self.db(), self.account_id).await? {
            self.populate_metadata_page(usize::MAX).await?;
        }
        count_cached_view(self.db(), self.account_id, view, unread_only).await
    }

    /// Fetch cached or backend metadata for one message.
    pub async fn get_message(&self, id: &BackendMsgId) -> Result<CachedMessage> {
        if self.policy.mode == MailCacheMode::Off {
            return Ok(cached_message_from_raw(self.backend.get_message(id).await?));
        }

        if let Some(message) = select_cached_message(self.db(), self.account_id, id).await? {
            return Ok(message);
        }

        let listed = self
            .backend
            .list_message_ids(&Query::all(), &PageRequest::first(1_000))
            .await?;
        for listed_id in listed.items {
            if listed_id == *id {
                let raw = self.backend.get_message(&listed_id).await?;
                upsert_raw_metadata(self.db(), self.account_id, raw).await?;
                break;
            }
        }

        if let Some(message) = select_cached_message(self.db(), self.account_id, id).await? {
            return Ok(message);
        }

        let raw = self.backend.get_message(id).await?;
        let cached = cached_message_from_raw(raw.clone());
        upsert_raw_metadata(self.db(), self.account_id, raw).await?;
        Ok(cached)
    }

    async fn populate_metadata_page(&self, limit: usize) -> Result<()> {
        let page = self
            .backend
            .list_message_ids(&Query::all(), &PageRequest::first(limit_as_u32(limit)))
            .await?;
        for id in page.items {
            if select_cached_message(self.db(), self.account_id, &id)
                .await?
                .is_none()
            {
                let raw = self.backend.get_message(&id).await?;
                upsert_raw_metadata(self.db(), self.account_id, raw).await?;
            }
        }
        Ok(())
    }
}

fn limit_as_u32(limit: usize) -> u32 {
    u32::try_from(limit).unwrap_or(u32::MAX).max(1)
}

async fn upsert_raw_metadata(
    db: &sqlx::SqlitePool,
    account_id: i64,
    raw: RawMessage,
) -> Result<i64> {
    let mut tx = db.begin().await?;
    let message_id = upsert_raw_metadata_tx(&mut tx, account_id, raw).await?;
    tx.commit().await?;
    Ok(message_id)
}

async fn upsert_raw_metadata_tx(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    raw: RawMessage,
) -> Result<i64> {
    let cached = cached_message_from_raw(raw);
    let now = Utc::now().to_rfc3339();
    let thread_id = cached
        .thread_id
        .as_deref()
        .unwrap_or_else(|| cached.id.as_str());
    let internal_date = cached.received_at.map_or(0, |dt| dt.timestamp());
    let size_bytes = i64::try_from(cached.size_bytes.unwrap_or(0)).unwrap_or(i64::MAX);

    sqlx::query(
        "INSERT INTO messages \
         (account_id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes, inserted_at, accessed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9) \
         ON CONFLICT(account_id, backend_msg_id) DO UPDATE SET \
         thread_id=excluded.thread_id, internal_date=excluded.internal_date, from_addr=excluded.from_addr, \
         subject=excluded.subject, preview=excluded.preview, size_bytes=excluded.size_bytes, accessed_at=excluded.accessed_at",
    )
    .bind(account_id)
    .bind(cached.id.as_str())
    .bind(thread_id)
    .bind(internal_date)
    .bind(&cached.from)
    .bind(&cached.subject)
    .bind(&cached.preview)
    .bind(size_bytes)
    .bind(&now)
    .execute(&mut **tx)
    .await?;

    let message_id: i64 =
        sqlx::query_scalar("SELECT id FROM messages WHERE account_id = ?1 AND backend_msg_id = ?2")
            .bind(account_id)
            .bind(cached.id.as_str())
            .fetch_one(&mut **tx)
            .await?;

    sqlx::query("DELETE FROM message_keywords WHERE message_id = ?1")
        .bind(message_id)
        .execute(&mut **tx)
        .await?;
    for keyword in &cached.keywords {
        sqlx::query("INSERT OR IGNORE INTO message_keywords (message_id, keyword) VALUES (?1, ?2)")
            .bind(message_id)
            .bind(keyword.as_str())
            .execute(&mut **tx)
            .await?;
    }

    sqlx::query("DELETE FROM attachments WHERE message_id = ?1")
        .bind(message_id)
        .execute(&mut **tx)
        .await?;
    for blob_ref in &cached.blob_refs {
        sqlx::query(
            "INSERT INTO attachments (message_id, filename, mime_type, size_bytes, blob_id, inline) \
             VALUES (?1, ?2, ?3, 0, NULL, 0)",
        )
        .bind(message_id)
        .bind(blob_ref.as_str())
        .bind("application/octet-stream")
        .execute(&mut **tx)
        .await?;
    }

    Ok(message_id)
}

async fn has_cached_metadata(db: &sqlx::SqlitePool, account_id: i64) -> Result<bool> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE account_id = ?1")
        .bind(account_id)
        .fetch_one(db)
        .await?;
    Ok(count > 0)
}

async fn select_cached_message(
    db: &sqlx::SqlitePool,
    account_id: i64,
    id: &BackendMsgId,
) -> Result<Option<CachedMessage>> {
    let Some(row) = sqlx::query(
        "SELECT id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes \
         FROM messages WHERE account_id = ?1 AND backend_msg_id = ?2",
    )
    .bind(account_id)
    .bind(id.as_str())
    .fetch_optional(db)
    .await? else {
        return Ok(None);
    };

    let message_id: i64 = row.get("id");
    touch_message(db, message_id).await?;
    Ok(Some(message_from_row(db, row).await?))
}

async fn touch_message(db: &sqlx::SqlitePool, message_id: i64) -> Result<()> {
    sqlx::query("UPDATE messages SET accessed_at = ?1 WHERE id = ?2")
        .bind(Utc::now().to_rfc3339())
        .bind(message_id)
        .execute(db)
        .await?;
    Ok(())
}

async fn message_from_row(
    db: &sqlx::SqlitePool,
    row: sqlx::sqlite::SqliteRow,
) -> Result<CachedMessage> {
    let message_id: i64 = row.get("id");
    let keywords = sqlx::query_scalar::<_, String>(
        "SELECT keyword FROM message_keywords WHERE message_id = ?1 ORDER BY keyword",
    )
    .bind(message_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(hail_backend::Keyword::new)
    .collect::<Vec<_>>();
    let blob_refs = sqlx::query_scalar::<_, String>(
        "SELECT filename FROM attachments WHERE message_id = ?1 ORDER BY id",
    )
    .bind(message_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(BlobRef::new)
    .collect::<Vec<_>>();
    let received_at = epoch_to_datetime(row.get::<i64, _>("internal_date"));
    let size_i64: i64 = row.get("size_bytes");

    Ok(CachedMessage {
        id: BackendMsgId::new(row.get::<String, _>("backend_msg_id")),
        thread_id: Some(row.get::<String, _>("thread_id")),
        from: row.get("from_addr"),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: row.get("subject"),
        preview: row.get("preview"),
        received_at,
        unread: !keywords
            .iter()
            .any(|kw| kw.as_str().eq_ignore_ascii_case(SEEN_KEYWORD)),
        keywords,
        size_bytes: u64::try_from(size_i64).ok(),
        blob_refs,
    })
}

async fn select_view_items(
    db: &sqlx::SqlitePool,
    account_id: i64,
    view: MailView,
    limit: usize,
) -> Result<Vec<MailViewItem>> {
    let rows = sqlx::query(
        "SELECT id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes \
         FROM messages WHERE account_id = ?1 ORDER BY internal_date DESC LIMIT ?2",
    )
    .bind(account_id)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(db)
    .await?;

    let mut items = Vec::new();
    for row in rows {
        let message = message_from_row(db, row).await?;
        if message_matches_view(&message, view) {
            items.push(view_item_from_message(message, view));
        }
    }
    Ok(items)
}

async fn count_cached_view(
    db: &sqlx::SqlitePool,
    account_id: i64,
    view: MailView,
    unread_only: bool,
) -> Result<usize> {
    let rows = sqlx::query(
        "SELECT id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes \
         FROM messages WHERE account_id = ?1",
    )
    .bind(account_id)
    .fetch_all(db)
    .await?;
    let mut count = 0_usize;
    for row in rows {
        let message = message_from_row(db, row).await?;
        if message_matches_view(&message, view) && (!unread_only || message.unread) {
            count += 1;
        }
    }
    Ok(count)
}

fn cached_message_from_raw(raw: RawMessage) -> CachedMessage {
    let from = raw
        .envelope
        .as_ref()
        .map_or_else(String::new, |env| env.mail_from.clone());
    let subject = raw.metadata.get("subject").cloned().unwrap_or_default();
    let preview = raw.metadata.get("preview").cloned().unwrap_or_default();
    let received_at = raw.received_at_epoch_secs.and_then(epoch_to_datetime);
    let unread = !raw
        .keywords
        .iter()
        .any(|kw| kw.as_str().eq_ignore_ascii_case(SEEN_KEYWORD));

    CachedMessage {
        id: raw.id,
        thread_id: raw.thread_id,
        from,
        to: recipients(raw.envelope.as_ref()),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject,
        preview,
        received_at,
        unread,
        keywords: raw.keywords,
        size_bytes: raw.size_bytes,
        blob_refs: raw.blob_refs,
    }
}

fn recipients(envelope: Option<&Envelope>) -> Vec<String> {
    envelope.map_or_else(Vec::new, |env| env.rcpt_to.clone())
}

fn epoch_to_datetime(epoch_secs: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(epoch_secs, 0).single()
}

fn message_matches_view(message: &CachedMessage, view: MailView) -> bool {
    let has = |keyword: &str| {
        message
            .keywords
            .iter()
            .any(|kw| kw.as_str().eq_ignore_ascii_case(keyword))
    };
    match view {
        MailView::Imbox => has(MailClassification::Imbox.keyword()),
        MailView::Feed => has(MailClassification::Feed.keyword()),
        MailView::Papertrail => has(MailClassification::Papertrail.keyword()),
        MailView::Drafts => has(DRAFT_KEYWORD),
        MailView::Trash => has(TRASH_KEYWORD),
        MailView::Spam => has(SPAM_KEYWORD) || has(SPAM_ALIAS_KEYWORD),
        MailView::Archive => has(ARCHIVE_KEYWORD),
    }
}

fn view_item_from_message(message: CachedMessage, view: MailView) -> MailViewItem {
    MailViewItem {
        thread_id: message
            .thread_id
            .clone()
            .unwrap_or_else(|| message.id.as_str().to_owned()),
        email_id: message.id.as_str().to_owned(),
        from: message.from,
        to: message.to,
        cc: message.cc,
        bcc: message.bcc,
        subject: message.subject,
        preview: message.preview,
        received_at: message.received_at,
        unread: message.unread,
        message_count: 1,
        unread_count: usize::from(message.unread),
        classification: view,
        labels: Vec::new(),
        feed_html: None,
        feed_html_with_images: None,
        feed_blocked_trackers: None,
        feed_blocked_images: None,
    }
}

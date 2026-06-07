//! Body and attachment blob read-through implementation for cached mail.

use bytes::Bytes;
use chrono::Utc;
use hail_backend::{BackendMsgId, BlobRef, RawMessage};
use hail_core::{BlobId, BlobKind, MailCacheMode, mail_render::html_fragment_to_text};
use sqlx::Row;

use crate::{CachedMail, Result, readthrough};

impl CachedMail {
    pub(crate) async fn get_message_body_readthrough(&self, id: &BackendMsgId) -> Result<Bytes> {
        if self.policy().mode == MailCacheMode::Off {
            return Ok(self.backend().get_message(id).await?.rfc822);
        }

        if let Some((message_row_id, blob_id)) =
            cached_body_ref(self.db(), self.account_id(), id).await?
        {
            touch_message(self.db(), message_row_id).await?;
            if let Ok(blob_id) = BlobId::parse(&blob_id) {
                return Ok(Bytes::from(self.blobs().get(&blob_id).await?));
            }
        }

        let raw = self.backend().get_message(id).await?;
        let rfc822 = raw.rfc822.clone();
        store_message_body(self, raw).await?;
        Ok(rfc822)
    }

    pub(crate) async fn get_blob_readthrough(&self, id: &BlobRef) -> Result<Bytes> {
        if self.policy().mode == MailCacheMode::Off {
            return self.backend().fetch_blob(id).await.map_err(Into::into);
        }

        if let Some((message_row_id, stored_ref)) =
            cached_attachment_ref(self.db(), self.account_id(), id).await?
        {
            touch_message(self.db(), message_row_id).await?;
            if let Ok(blob_id) = BlobId::parse(&stored_ref) {
                return Ok(Bytes::from(self.blobs().get(&blob_id).await?));
            }
        }

        let bytes = self.backend().fetch_blob(id).await?;
        let blob_id = self.blobs().put(BlobKind::Att, &bytes).await?;
        update_attachment_blob_ref(self.db(), self.account_id(), id, &blob_id.to_string()).await?;
        Ok(bytes)
    }
}

async fn cached_body_ref(
    db: &sqlx::SqlitePool,
    account_id: i64,
    id: &BackendMsgId,
) -> Result<Option<(i64, String)>> {
    let row = sqlx::query(
        "SELECT id, body_blob_id FROM messages \
         WHERE account_id = ?1 AND backend_msg_id = ?2 AND body_blob_id IS NOT NULL",
    )
    .bind(account_id)
    .bind(id.as_str())
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| (row.get("id"), row.get("body_blob_id"))))
}

async fn cached_attachment_ref(
    db: &sqlx::SqlitePool,
    account_id: i64,
    id: &BlobRef,
) -> Result<Option<(i64, String)>> {
    let row = sqlx::query(
        "SELECT messages.id AS message_id, attachments.blob_id AS blob_id \
         FROM attachments \
         JOIN messages ON messages.id = attachments.message_id \
         WHERE messages.account_id = ?1 AND attachments.blob_id = ?2 \
         LIMIT 1",
    )
    .bind(account_id)
    .bind(id.as_str())
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| (row.get("message_id"), row.get("blob_id"))))
}

async fn store_message_body(cache: &CachedMail, raw: RawMessage) -> Result<()> {
    let rfc822 = raw.rfc822.clone();
    let body_text = decoded_plaintext(&rfc822);
    let message_id = readthrough::upsert_raw_metadata(cache.db(), cache.account_id(), raw).await?;
    let blob_id = cache.blobs().put(BlobKind::Eml, &rfc822).await?;

    sqlx::query(
        "UPDATE messages SET body_blob_id = ?1, body_text = ?2, accessed_at = ?3 WHERE id = ?4",
    )
    .bind(blob_id.to_string())
    .bind(body_text)
    .bind(Utc::now().to_rfc3339())
    .bind(message_id)
    .execute(cache.db())
    .await?;
    Ok(())
}

async fn update_attachment_blob_ref(
    db: &sqlx::SqlitePool,
    account_id: i64,
    backend_ref: &BlobRef,
    blob_id: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE attachments SET blob_id = ?1 \
         WHERE id IN ( \
           SELECT attachments.id FROM attachments \
           JOIN messages ON messages.id = attachments.message_id \
           WHERE messages.account_id = ?2 AND attachments.blob_id = ?3 \
         )",
    )
    .bind(blob_id)
    .bind(account_id)
    .bind(backend_ref.as_str())
    .execute(db)
    .await?;

    sqlx::query(
        "UPDATE messages SET accessed_at = ?1 \
         WHERE account_id = ?2 AND id IN ( \
           SELECT message_id FROM attachments WHERE blob_id = ?3 \
         )",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(account_id)
    .bind(blob_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn touch_message(db: &sqlx::SqlitePool, message_id: i64) -> Result<()> {
    sqlx::query("UPDATE messages SET accessed_at = ?1 WHERE id = ?2")
        .bind(Utc::now().to_rfc3339())
        .bind(message_id)
        .execute(db)
        .await?;
    Ok(())
}

fn decoded_plaintext(rfc822: &[u8]) -> String {
    let Some(message) = mail_parser::MessageParser::default().parse(rfc822) else {
        return String::from_utf8_lossy(rfc822).into_owned();
    };

    let mut text = Vec::new();
    for index in 0..message.text_body_count() {
        if let Some(body) = message.body_text(index) {
            text.push(body.into_owned());
        }
    }
    if text.is_empty() {
        for index in 0..message.html_body_count() {
            if let Some(body) = message.body_html(index) {
                text.push(html_fragment_to_text(&body));
            }
        }
    }
    text.join("\n\n")
}

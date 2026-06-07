//! Backend change application for cache sync.
//!
//! The worker owns polling/watching. This module is the short-lived apply step
//! that translates backend-neutral [`Change`] values into durable SQLite cache
//! state.

use chrono::{DateTime, Duration, Utc};
use hail_backend::{BackendMsgId, Change, Keyword, MailboxRole, RawMessage};
use hail_core::{MailCacheMode, SPAM_KEYWORD};
use sqlx::{Row, Sqlite, Transaction};

use crate::{CachedMail, Result, readthrough};

const SELF_APPLIED_WINDOW_SECS: i64 = 60;
const SEEN_KEYWORD: &str = "$seen";
const TRASH_KEYWORD: &str = "$trash";
const JUNK_ALIAS_KEYWORD: &str = "$junk";
const HAIL_IMBOX_KEYWORD: &str = "$hail_imbox";
const HAIL_FEED_KEYWORD: &str = "$hail_feed";
const HAIL_PAPERTRAIL_KEYWORD: &str = "$hail_papertrail";
const ARCHIVE_KEYWORD: &str = "$archive";

impl CachedMail {
    /// Apply one backend sync change to the local cache.
    pub async fn apply_change(&self, change: Change) -> Result<()> {
        if self.policy().mode == MailCacheMode::Off {
            return Ok(());
        }

        match change {
            Change::MessageCreated { id, raw_ref } => {
                let raw = match raw_ref {
                    Some(raw) => raw,
                    None => self.backend().get_message(&id).await?,
                };
                if should_apply_change(
                    self.db(),
                    self.account_id(),
                    &raw.id,
                    &change_types_for_created(&raw),
                    Utc::now(),
                )
                .await?
                {
                    readthrough::upsert_raw_metadata(self.db(), self.account_id(), raw).await?;
                }
            }
            Change::MessageUpdated {
                id,
                keywords,
                keywords_added,
                keywords_removed,
            } => {
                let change_types = change_types_for_keywords(&keywords_added, &keywords_removed);
                if should_apply_change(self.db(), self.account_id(), &id, &change_types, Utc::now())
                    .await?
                {
                    let mut tx = self.db().begin().await?;
                    if let Some(authoritative) = keywords.as_deref() {
                        // Backend hydrated the full keyword state; treat it as
                        // authoritative over the add/remove delta.
                        replace_keywords(&mut tx, self.account_id(), &id, authoritative).await?;
                    } else {
                        apply_keyword_delta(
                            &mut tx,
                            self.account_id(),
                            &id,
                            &keywords_added,
                            &keywords_removed,
                        )
                        .await?;
                    }
                    tx.commit().await?;
                }
            }
            Change::MessageDeleted { id } => {
                if should_apply_change(
                    self.db(),
                    self.account_id(),
                    &id,
                    &["permanent_delete"],
                    Utc::now(),
                )
                .await?
                {
                    let mut tx = self.db().begin().await?;
                    delete_if_not_locally_trashed(&mut tx, self.account_id(), &id).await?;
                    tx.commit().await?;
                }
            }
            Change::MailboxRoleChanged { id, role } => {
                let change_types = [change_type_for_role(role)];
                if should_apply_change(self.db(), self.account_id(), &id, &change_types, Utc::now())
                    .await?
                {
                    let mut tx = self.db().begin().await?;
                    apply_role_move(&mut tx, self.account_id(), &id, role).await?;
                    tx.commit().await?;
                }
            }
        }
        Ok(())
    }

    /// Apply a batch of backend sync changes in order.
    pub async fn apply_changes(&self, changes: impl IntoIterator<Item = Change>) -> Result<()> {
        for change in changes {
            self.apply_change(change).await?;
        }
        Ok(())
    }
}

async fn should_apply_change(
    db: &sqlx::SqlitePool,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    change_types: &[&str],
    incoming_at: DateTime<Utc>,
) -> Result<bool> {
    for change_type in change_types {
        if recently_applied_outbound_change_exists(
            db,
            account_id,
            backend_msg_id,
            change_type,
            incoming_at,
        )
        .await?
        {
            return Ok(false);
        }
    }

    let Some(local_created_at) =
        newest_pending_outbound_created_at(db, account_id, backend_msg_id, change_types).await?
    else {
        return Ok(true);
    };

    Ok(incoming_at >= local_created_at)
}

async fn recently_applied_outbound_change_exists(
    db: &sqlx::SqlitePool,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    change_type: &str,
    incoming_at: DateTime<Utc>,
) -> Result<bool> {
    let rows = sqlx::query(
        "SELECT applied_at FROM outbound_changes \
         WHERE account_id = ?1 AND backend_msg_id = ?2 AND change_type = ?3 AND applied_at IS NOT NULL",
    )
    .bind(account_id)
    .bind(backend_msg_id.as_str())
    .bind(change_type)
    .fetch_all(db)
    .await?;

    for row in rows {
        let applied_at: String = row.get("applied_at");
        if let Some(applied_at) = parse_rfc3339_utc(&applied_at) {
            let age = incoming_at.signed_duration_since(applied_at);
            if age >= Duration::zero() && age <= Duration::seconds(SELF_APPLIED_WINDOW_SECS) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn newest_pending_outbound_created_at(
    db: &sqlx::SqlitePool,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    change_types: &[&str],
) -> Result<Option<DateTime<Utc>>> {
    let rows = sqlx::query(
        "SELECT change_type, created_at FROM outbound_changes \
         WHERE account_id = ?1 AND backend_msg_id = ?2 AND applied_at IS NULL",
    )
    .bind(account_id)
    .bind(backend_msg_id.as_str())
    .fetch_all(db)
    .await?;

    let newest = rows
        .into_iter()
        .filter_map(|row| {
            let change_type: String = row.get("change_type");
            if !change_types
                .iter()
                .any(|candidate| *candidate == change_type)
            {
                return None;
            }
            let created_at: String = row.get("created_at");
            parse_rfc3339_utc(&created_at)
        })
        .max();
    Ok(newest)
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

async fn replace_keywords(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    keywords: &[Keyword],
) -> Result<()> {
    let Some(message_id) = cached_message_row_id(tx, account_id, backend_msg_id).await? else {
        return Ok(());
    };

    sqlx::query("DELETE FROM message_keywords WHERE message_id = ?1")
        .bind(message_id)
        .execute(&mut **tx)
        .await?;
    for keyword in keywords {
        sqlx::query("INSERT OR IGNORE INTO message_keywords (message_id, keyword) VALUES (?1, ?2)")
            .bind(message_id)
            .bind(keyword.as_str())
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn apply_keyword_delta(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    add: &[Keyword],
    remove: &[Keyword],
) -> Result<()> {
    let Some(message_id) = cached_message_row_id(tx, account_id, backend_msg_id).await? else {
        return Ok(());
    };

    for keyword in remove {
        sqlx::query("DELETE FROM message_keywords WHERE message_id = ?1 AND keyword = ?2")
            .bind(message_id)
            .bind(keyword.as_str())
            .execute(&mut **tx)
            .await?;
    }
    for keyword in add {
        sqlx::query("INSERT OR IGNORE INTO message_keywords (message_id, keyword) VALUES (?1, ?2)")
            .bind(message_id)
            .bind(keyword.as_str())
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn apply_role_move(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    role: MailboxRole,
) -> Result<()> {
    let Some(message_id) = cached_message_row_id(tx, account_id, backend_msg_id).await? else {
        return Ok(());
    };

    if role != MailboxRole::Trash && has_keyword(tx, message_id, TRASH_KEYWORD).await? {
        return Ok(());
    }

    for keyword in role_keywords_to_remove(role) {
        sqlx::query("DELETE FROM message_keywords WHERE message_id = ?1 AND keyword = ?2")
            .bind(message_id)
            .bind(keyword)
            .execute(&mut **tx)
            .await?;
    }
    for keyword in role_keywords_to_add(role) {
        sqlx::query("INSERT OR IGNORE INTO message_keywords (message_id, keyword) VALUES (?1, ?2)")
            .bind(message_id)
            .bind(keyword)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn delete_if_not_locally_trashed(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
) -> Result<()> {
    let Some(message_id) = cached_message_row_id(tx, account_id, backend_msg_id).await? else {
        return Ok(());
    };
    if has_keyword(tx, message_id, TRASH_KEYWORD).await? {
        return Ok(());
    }
    sqlx::query("DELETE FROM messages WHERE id = ?1")
        .bind(message_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn cached_message_row_id(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
) -> Result<Option<i64>> {
    let id =
        sqlx::query_scalar("SELECT id FROM messages WHERE account_id = ?1 AND backend_msg_id = ?2")
            .bind(account_id)
            .bind(backend_msg_id.as_str())
            .fetch_optional(&mut **tx)
            .await?;
    Ok(id)
}

async fn has_keyword(
    tx: &mut Transaction<'_, Sqlite>,
    message_id: i64,
    keyword: &str,
) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message_keywords WHERE message_id = ?1 AND keyword = ?2",
    )
    .bind(message_id)
    .bind(keyword)
    .fetch_one(&mut **tx)
    .await?;
    Ok(count > 0)
}

fn change_types_for_created(raw: &RawMessage) -> Vec<&'static str> {
    let mut change_types = change_types_for_keywords(&raw.keywords, &[]);
    if change_types.is_empty() {
        change_types.push("keyword_add");
    }
    change_types
}

fn change_types_for_keywords(add: &[Keyword], remove: &[Keyword]) -> Vec<&'static str> {
    let mut change_types = Vec::new();
    for keyword in remove {
        push_unique(&mut change_types, keyword_change_type(keyword, false));
    }
    for keyword in add {
        push_unique(&mut change_types, keyword_change_type(keyword, true));
    }
    change_types
}

fn push_unique(values: &mut Vec<&'static str>, value: &'static str) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn keyword_change_type(keyword: &Keyword, added: bool) -> &'static str {
    if keyword.as_str().eq_ignore_ascii_case(SEEN_KEYWORD) {
        if added { "read" } else { "unread" }
    } else if added {
        "keyword_add"
    } else {
        "keyword_remove"
    }
}

fn change_type_for_role(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Trash => "trash",
        MailboxRole::Inbox => "untrash",
        _ => "role_move",
    }
}

fn role_keywords_to_remove(role: MailboxRole) -> &'static [&'static str] {
    match role {
        MailboxRole::Inbox => &[
            HAIL_FEED_KEYWORD,
            HAIL_PAPERTRAIL_KEYWORD,
            ARCHIVE_KEYWORD,
            TRASH_KEYWORD,
            SPAM_KEYWORD,
            JUNK_ALIAS_KEYWORD,
        ],
        MailboxRole::Archive | MailboxRole::Trash | MailboxRole::Junk => &[
            HAIL_IMBOX_KEYWORD,
            HAIL_FEED_KEYWORD,
            HAIL_PAPERTRAIL_KEYWORD,
            ARCHIVE_KEYWORD,
            TRASH_KEYWORD,
            SPAM_KEYWORD,
            JUNK_ALIAS_KEYWORD,
        ],
        _ => &[],
    }
}

fn role_keywords_to_add(role: MailboxRole) -> &'static [&'static str] {
    match role {
        MailboxRole::Inbox => &[HAIL_IMBOX_KEYWORD],
        MailboxRole::Archive => &[ARCHIVE_KEYWORD],
        MailboxRole::Trash => &[TRASH_KEYWORD],
        MailboxRole::Junk => &[SPAM_KEYWORD],
        _ => &[],
    }
}

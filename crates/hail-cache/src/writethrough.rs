//! Write-through mutation queue and optimistic cache updates.
//!
//! This module intentionally only performs durable local storage work: update
//! cached rows optimistically when cache metadata is enabled, then enqueue rows
//! in `outbound_changes` for a later worker to drain. It never talks to the
//! upstream backend.

use chrono::Utc;
use hail_backend::{BackendMsgId, Keyword, MailboxRole};
use hail_core::{MailCacheMode, SPAM_KEYWORD};
use sqlx::{Row, Sqlite, Transaction};

use crate::{CachedMail, MailTarget, Result};

const SEEN_KEYWORD: &str = "$seen";
const TRASH_KEYWORD: &str = "$trash";
const JUNK_ALIAS_KEYWORD: &str = "$junk";
const HAIL_IMBOX_KEYWORD: &str = "$hail_imbox";
const HAIL_FEED_KEYWORD: &str = "$hail_feed";
const HAIL_PAPERTRAIL_KEYWORD: &str = "$hail_papertrail";
const ARCHIVE_KEYWORD: &str = "$archive";

pub(crate) async fn mutate_keywords(
    cache: &CachedMail,
    target: MailTarget<'_>,
    add: &[Keyword],
    remove: &[Keyword],
) -> Result<()> {
    let mut tx = cache.db().begin().await?;
    let message_ids = backend_message_ids_for_target(&mut tx, cache.account_id(), target).await?;

    for backend_msg_id in &message_ids {
        if cache.policy().mode != MailCacheMode::Off {
            apply_keyword_delta(&mut tx, cache.account_id(), backend_msg_id, add, remove).await?;
        }
        enqueue_keyword_changes(&mut tx, cache.account_id(), backend_msg_id, add, remove).await?;
    }

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn move_to_role(
    cache: &CachedMail,
    target: MailTarget<'_>,
    role: MailboxRole,
) -> Result<()> {
    let mut tx = cache.db().begin().await?;
    let message_ids = backend_message_ids_for_target(&mut tx, cache.account_id(), target).await?;

    for backend_msg_id in &message_ids {
        if cache.policy().mode != MailCacheMode::Off {
            apply_role_move(&mut tx, cache.account_id(), backend_msg_id, role).await?;
        }
        enqueue_role_move(&mut tx, cache.account_id(), backend_msg_id, role).await?;
    }

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn pending_sync_count(cache: &CachedMail) -> Result<i64> {
    let count = sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbound_changes WHERE account_id = ?1 AND applied_at IS NULL",
    )
    .bind(cache.account_id())
    .fetch_one(cache.db())
    .await?;
    Ok(count)
}

async fn backend_message_ids_for_target(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    target: MailTarget<'_>,
) -> Result<Vec<BackendMsgId>> {
    match target {
        MailTarget::Message(id) => Ok(vec![id.clone()]),
        MailTarget::Thread(thread_id) => {
            let rows = sqlx::query(
                "SELECT backend_msg_id FROM messages WHERE account_id = ?1 AND thread_id = ?2 ORDER BY id",
            )
            .bind(account_id)
            .bind(thread_id)
            .fetch_all(&mut **tx)
            .await?;
            Ok(rows
                .into_iter()
                .map(|row| BackendMsgId::new(row.get::<String, _>("backend_msg_id")))
                .collect())
        }
    }
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

async fn enqueue_keyword_changes(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    add: &[Keyword],
    remove: &[Keyword],
) -> Result<()> {
    for keyword in remove {
        let change_type = keyword_change_type(keyword, false);
        let payload = keyword_payload(keyword);
        enqueue_outbound_change(tx, account_id, backend_msg_id, change_type, &payload).await?;
    }
    for keyword in add {
        let change_type = keyword_change_type(keyword, true);
        let payload = keyword_payload(keyword);
        enqueue_outbound_change(tx, account_id, backend_msg_id, change_type, &payload).await?;
    }
    Ok(())
}

async fn enqueue_role_move(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    role: MailboxRole,
) -> Result<()> {
    let change_type = match role {
        MailboxRole::Trash => "trash",
        MailboxRole::Inbox => "untrash",
        _ => "role_move",
    };
    let payload = role_payload(role);
    enqueue_outbound_change(tx, account_id, backend_msg_id, change_type, &payload).await
}

async fn enqueue_outbound_change(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    backend_msg_id: &BackendMsgId,
    change_type: &str,
    payload_json: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO outbound_changes (account_id, backend_msg_id, change_type, payload_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(account_id)
    .bind(backend_msg_id.as_str())
    .bind(change_type)
    .bind(payload_json)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn keyword_payload(keyword: &Keyword) -> String {
    format!(
        r#"{{"keyword":"{}"}}"#,
        escape_json_string(keyword.as_str())
    )
}

fn role_payload(role: MailboxRole) -> String {
    format!(r#"{{"role":"{}"}}"#, role_name(role))
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str(r#"\""#),
            '\\' => escaped.push_str(r#"\\"#),
            '\n' => escaped.push_str(r#"\n"#),
            '\r' => escaped.push_str(r#"\r"#),
            '\t' => escaped.push_str(r#"\t"#),
            ch if ch.is_control() => escaped.push_str(&format!(r#"\u{:04x}"#, ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
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

fn role_name(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "inbox",
        MailboxRole::Archive => "archive",
        MailboxRole::Drafts => "drafts",
        MailboxRole::Sent => "sent",
        MailboxRole::Trash => "trash",
        MailboxRole::Junk => "junk",
        MailboxRole::Important => "important",
        MailboxRole::AllMail => "all_mail",
        MailboxRole::Custom => "custom",
    }
}

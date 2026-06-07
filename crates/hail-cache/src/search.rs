//! SQLite FTS5-backed search with backend fallthrough for sparse cache hits.

use std::collections::HashSet;

use chrono::{DateTime, TimeZone, Utc};
use hail_backend::{Keyword, MailboxRole, PageRequest, Query, QueryScope};
use hail_core::{MailCacheMode, MailClassification};
use sqlx::Row;

use crate::{CachedLabel, CachedMail, MailSearchResult, Result, SearchMailbox, SearchResultSource};

const LOCAL_SPARSE_THRESHOLD: usize = 10;
const SEEN_KEYWORD: &str = "$seen";
const ARCHIVE_KEYWORD: &str = "$archive";
const TRASH_KEYWORD: &str = "$trash";
const DRAFT_KEYWORD: &str = "$draft";

impl CachedMail {
    pub(crate) async fn search_cached_mail(
        &self,
        q: &str,
        mailbox: Option<SearchMailbox>,
        limit: usize,
    ) -> Result<Vec<MailSearchResult>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let query = search_query(q, mailbox);
        if self.policy().mode == MailCacheMode::Off {
            return self.search_backend(&query, limit).await;
        }

        let effective_limit = limit.max(1);
        let mut results = search_local_fts(self, q, mailbox, effective_limit).await?;
        let needs_backend = results.len() < effective_limit.min(LOCAL_SPARSE_THRESHOLD);
        if needs_backend {
            merge_backend_results(self, &query, effective_limit, &mut results).await?;
        }
        results.truncate(effective_limit);
        Ok(results)
    }

    async fn search_backend(&self, query: &Query, limit: usize) -> Result<Vec<MailSearchResult>> {
        let page = self
            .backend()
            .list_message_ids(query, &PageRequest::first(limit_as_u32(limit)))
            .await?;
        let mut results = Vec::with_capacity(page.items.len());
        for id in page.items {
            let message = self.backend().get_message(&id).await?;
            results.push(search_result_from_message(
                crate::readthrough::cached_message_from_raw(message),
                SearchResultSource::Backend,
            ));
        }
        Ok(results)
    }
}

async fn merge_backend_results(
    cache: &CachedMail,
    query: &Query,
    limit: usize,
    results: &mut Vec<MailSearchResult>,
) -> Result<()> {
    let mut seen = results
        .iter()
        .map(|result| result.email_id.clone())
        .collect::<HashSet<_>>();
    let page = cache
        .backend()
        .list_message_ids(query, &PageRequest::first(limit_as_u32(limit)))
        .await?;
    for id in page.items {
        if results.len() >= limit {
            break;
        }
        if !seen.insert(id.as_str().to_owned()) {
            continue;
        }
        let message = cache.backend().get_message(&id).await?;
        results.push(search_result_from_message(
            crate::readthrough::cached_message_from_raw(message),
            SearchResultSource::Backend,
        ));
    }
    Ok(())
}

async fn search_local_fts(
    cache: &CachedMail,
    q: &str,
    mailbox: Option<SearchMailbox>,
    limit: usize,
) -> Result<Vec<MailSearchResult>> {
    if q.trim().is_empty() {
        return Ok(Vec::new());
    }

    let rows = sqlx::query(
        "SELECT messages.id, messages.backend_msg_id, messages.thread_id, messages.internal_date, \
                messages.from_addr, messages.subject, messages.preview \
         FROM messages_fts \
         JOIN messages ON messages.id = messages_fts.rowid \
         WHERE messages.account_id = ?1 AND messages_fts MATCH ?2 \
         ORDER BY rank, messages.internal_date DESC \
         LIMIT ?3",
    )
    .bind(cache.account_id())
    .bind(fts_query(q))
    .bind(i64::try_from(limit.saturating_mul(5)).unwrap_or(i64::MAX))
    .fetch_all(cache.db())
    .await?;

    let mut results = Vec::new();
    for row in rows {
        if results.len() >= limit {
            break;
        }
        let message_id: i64 = row.get("id");
        let keywords = keywords_for_message(cache.db(), message_id).await?;
        if !matches_mailbox(&keywords, mailbox) {
            continue;
        }
        results.push(search_result_from_row(row, keywords));
    }
    Ok(results)
}

fn search_query(q: &str, mailbox: Option<SearchMailbox>) -> Query {
    let (mailbox_role, keywords) = mailbox_filter(mailbox);
    Query {
        scope: QueryScope::Search,
        text: Some(q.trim().to_owned()),
        mailbox_role,
        keywords,
        newer_than_epoch_secs: None,
        older_than_epoch_secs: None,
    }
}

fn mailbox_filter(mailbox: Option<SearchMailbox>) -> (Option<MailboxRole>, Vec<Keyword>) {
    match mailbox {
        Some(SearchMailbox::Imbox) => (
            None,
            vec![Keyword::new(MailClassification::Imbox.keyword())],
        ),
        Some(SearchMailbox::Feed) => (None, vec![Keyword::new(MailClassification::Feed.keyword())]),
        Some(SearchMailbox::Papertrail) => (
            None,
            vec![Keyword::new(MailClassification::Papertrail.keyword())],
        ),
        Some(SearchMailbox::Archive) => (Some(MailboxRole::Archive), Vec::new()),
        Some(SearchMailbox::Trash) => (Some(MailboxRole::Trash), Vec::new()),
        Some(SearchMailbox::Drafts) => (Some(MailboxRole::Drafts), Vec::new()),
        None => (None, Vec::new()),
    }
}

fn matches_mailbox(keywords: &[Keyword], mailbox: Option<SearchMailbox>) -> bool {
    match mailbox {
        Some(SearchMailbox::Imbox) => has_keyword(keywords, MailClassification::Imbox.keyword()),
        Some(SearchMailbox::Feed) => has_keyword(keywords, MailClassification::Feed.keyword()),
        Some(SearchMailbox::Papertrail) => {
            has_keyword(keywords, MailClassification::Papertrail.keyword())
        }
        Some(SearchMailbox::Archive) => has_keyword(keywords, ARCHIVE_KEYWORD),
        Some(SearchMailbox::Trash) => has_keyword(keywords, TRASH_KEYWORD),
        Some(SearchMailbox::Drafts) => has_keyword(keywords, DRAFT_KEYWORD),
        None => true,
    }
}

fn has_keyword(keywords: &[Keyword], needle: &str) -> bool {
    keywords
        .iter()
        .any(|keyword| keyword.as_str().eq_ignore_ascii_case(needle))
}

async fn keywords_for_message(db: &sqlx::SqlitePool, message_id: i64) -> Result<Vec<Keyword>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT keyword FROM message_keywords WHERE message_id = ?1 ORDER BY keyword",
    )
    .bind(message_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(Keyword::new)
    .collect())
}

fn search_result_from_row(
    row: sqlx::sqlite::SqliteRow,
    keywords: Vec<Keyword>,
) -> MailSearchResult {
    let thread_id: String = row.get("thread_id");
    let email_id: String = row.get("backend_msg_id");
    MailSearchResult {
        thread_id,
        email_id,
        from: row.get("from_addr"),
        subject: row.get("subject"),
        preview: row.get("preview"),
        message_count: 1,
        unread_count: usize::from(!has_keyword(&keywords, SEEN_KEYWORD)),
        unread: !has_keyword(&keywords, SEEN_KEYWORD),
        received_at: epoch_to_datetime(row.get("internal_date")),
        labels: Vec::new(),
        source: SearchResultSource::Local,
    }
}

fn search_result_from_message(
    message: crate::CachedMessage,
    source: SearchResultSource,
) -> MailSearchResult {
    let unread = message.unread;
    MailSearchResult {
        thread_id: message
            .thread_id
            .unwrap_or_else(|| message.id.as_str().to_owned()),
        email_id: message.id.as_str().to_owned(),
        from: message.from,
        subject: message.subject,
        preview: message.preview,
        message_count: 1,
        unread_count: usize::from(unread),
        unread,
        received_at: message.received_at,
        labels: Vec::<CachedLabel>::new(),
        source,
    }
}

fn epoch_to_datetime(epoch_secs: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(epoch_secs, 0).single()
}

fn limit_as_u32(limit: usize) -> u32 {
    u32::try_from(limit).unwrap_or(u32::MAX).max(1)
}

fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|term| {
            let escaped = term.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

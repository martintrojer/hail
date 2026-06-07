use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend, Mailbox,
    MailboxRole, Page, PageRequest, Principal, Query, QueryScope, RawMessage, SubmissionId,
    SyncCursor,
};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, CachedMail, SearchMailbox, SearchResultSource};
use hail_core::{MailBackfill, MailCacheMode, MailClassification};
use sqlx::SqlitePool;
use tempfile::TempDir;

static CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: false,
    max_attachment_size: 0,
    label_path_separator: '/',
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BackendStats {
    list_calls: usize,
    get_calls: usize,
    queries: Vec<Query>,
}

#[derive(Clone)]
struct FakeBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    order: Arc<Vec<BackendMsgId>>,
    stats: Arc<Mutex<BackendStats>>,
}

impl FakeBackend {
    fn new(messages: Vec<RawMessage>) -> Self {
        let order = messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let messages = messages
            .into_iter()
            .map(|message| (message.id.clone(), message))
            .collect::<HashMap<_, _>>();
        Self {
            messages: Arc::new(messages),
            order: Arc::new(order),
            stats: Arc::new(Mutex::new(BackendStats::default())),
        }
    }

    fn stats(&self) -> BackendStats {
        self.stats.lock().expect("stats lock").clone()
    }
}

#[async_trait]
impl MailBackend for FakeBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        let mut stats = self.stats.lock().expect("stats lock");
        stats.list_calls += 1;
        stats.queries.push(query.clone());
        drop(stats);

        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        let items = self
            .order
            .iter()
            .filter(|id| {
                self.messages
                    .get(*id)
                    .is_some_and(|message| backend_matches(message, query))
            })
            .take(limit)
            .cloned()
            .collect();
        Ok(Page {
            items,
            next_cursor: None,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.stats.lock().expect("stats lock").get_calls += 1;
        self.messages
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
    }

    async fn fetch_blob(&self, _id: &BlobRef) -> hail_backend::Result<Bytes> {
        Ok(Bytes::new())
    }

    async fn set_keywords(
        &self,
        _id: &BackendMsgId,
        _add: &[Keyword],
        _remove: &[Keyword],
    ) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn move_to_role(
        &self,
        _id: &BackendMsgId,
        _role: MailboxRole,
    ) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> hail_backend::Result<()> {
        Ok(())
    }

    async fn send(
        &self,
        _rfc822: &[u8],
        _envelope: &Envelope,
    ) -> hail_backend::Result<SubmissionId> {
        Ok(SubmissionId::new("fake-submission"))
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(&self) -> futures_util::stream::BoxStream<'static, Change> {
        Box::pin(stream::empty())
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Ok(Vec::new())
    }
}

fn backend_matches(message: &RawMessage, query: &Query) -> bool {
    if query.scope != QueryScope::Search {
        return true;
    }
    let text_matches = query.text.as_ref().is_none_or(|needle| {
        let needle = needle.to_ascii_lowercase();
        message
            .metadata
            .get("subject")
            .is_some_and(|subject| subject.to_ascii_lowercase().contains(&needle))
            || message
                .metadata
                .get("preview")
                .is_some_and(|preview| preview.to_ascii_lowercase().contains(&needle))
    });
    text_matches
        && query.keywords.iter().all(|keyword| {
            message
                .keywords
                .iter()
                .any(|candidate| candidate.as_str() == keyword.as_str())
        })
        && query.mailbox_role.is_none_or(|role| match role {
            MailboxRole::Archive => has_raw_keyword(message, "$archive"),
            MailboxRole::Trash => has_raw_keyword(message, "$trash"),
            MailboxRole::Drafts => has_raw_keyword(message, "$draft"),
            _ => true,
        })
}

fn has_raw_keyword(message: &RawMessage, keyword: &str) -> bool {
    message
        .keywords
        .iter()
        .any(|candidate| candidate.as_str() == keyword)
}

async fn fixture(
    backend_messages: Vec<RawMessage>,
    mode: MailCacheMode,
) -> (CachedMail, FakeBackend, TempDir) {
    let pool = hail_db::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    hail_db::migrate(&pool).await.expect("run migrations");
    ensure_default_account(&pool).await;

    let tempdir = tempfile::tempdir().expect("create temp blob dir");
    let blobs = Arc::new(FilesystemBlobStore::new(tempdir.path())) as Arc<dyn BlobStore>;
    let backend = FakeBackend::new(backend_messages);
    let policy = CachePolicy::new(mode, 90, 50_000, 5 * 1024 * 1024, MailBackfill::Off);
    let cache = CachedMail::new(pool, blobs, Box::new(backend.clone()), policy);
    (cache, backend, tempdir)
}

async fn ensure_default_account(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO users (id, email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (1, 'cache-search@example.test', 'acct', NULL, 1, '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (1, 1, 'acct', 'gmail', 'gmail', 'provider-acct', 'cache-search@example.test', ?1, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("insert account");
}

fn raw_message(id: &str, subject: &str, body: &str, keywords: Vec<&str>) -> RawMessage {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_owned(), subject.to_owned());
    metadata.insert("preview".to_owned(), format!("preview {subject}"));
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(format!("thread-{id}")),
        rfc822: Bytes::from(format!(
            "From: {id}@example.test\r\nTo: me@example.test\r\nSubject: {subject}\r\n\r\n{body}"
        )),
        keywords: keywords.into_iter().map(Keyword::new).collect(),
        envelope: Some(Envelope {
            mail_from: format!("{id}@example.test"),
            rcpt_to: vec!["me@example.test".to_owned()],
        }),
        received_at_epoch_secs: Some(1_700_000_000),
        size_bytes: Some(1234),
        blob_refs: Vec::new(),
        attachments: Vec::new(),
        metadata,
    }
}

async fn cache_body(cache: &CachedMail, id: &str) {
    cache
        .get_message_body(&BackendMsgId::new(id))
        .await
        .expect("cache body text");
}

#[tokio::test]
async fn fts5_match_returns_expected_local_messages() {
    let local = raw_message(
        "local-1",
        "Quarterly update",
        "the launch codename is narwhal",
        vec![MailClassification::Imbox.keyword()],
    );
    let miss = raw_message(
        "local-2",
        "Unrelated",
        "nothing to see here",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, _backend, _tempdir) = fixture(vec![local, miss], MailCacheMode::Bounded).await;
    cache_body(&cache, "local-1").await;
    cache_body(&cache, "local-2").await;

    let results = cache
        .search("narwhal", None, 1)
        .await
        .expect("search local fts");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].email_id, "local-1");
    assert_eq!(results[0].source, SearchResultSource::Local);
}

#[tokio::test]
async fn sparse_local_results_merge_backend_and_dedup() {
    let local = raw_message(
        "shared-1",
        "Needle local",
        "cached needle body",
        vec![MailClassification::Imbox.keyword()],
    );
    let backend_only = raw_message(
        "backend-1",
        "Needle backend",
        "older remote body",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, backend, _tempdir) =
        fixture(vec![local, backend_only], MailCacheMode::Bounded).await;
    cache_body(&cache, "shared-1").await;

    let results = cache
        .search("Needle", None, 10)
        .await
        .expect("merge backend search");

    assert_eq!(
        results
            .iter()
            .map(|item| item.email_id.as_str())
            .collect::<Vec<_>>(),
        vec!["shared-1", "backend-1"]
    );
    assert_eq!(results[0].source, SearchResultSource::Local);
    assert_eq!(results[1].source, SearchResultSource::Backend);
    assert_eq!(backend.stats().list_calls, 1);
}

#[tokio::test]
async fn mailbox_filter_is_applied_to_local_and_backend_results() {
    let feed = raw_message(
        "feed-1",
        "Needle feed",
        "cached needle body",
        vec![MailClassification::Feed.keyword()],
    );
    let imbox = raw_message(
        "imbox-1",
        "Needle imbox",
        "cached needle body",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, backend, _tempdir) = fixture(vec![feed, imbox], MailCacheMode::Bounded).await;
    cache_body(&cache, "feed-1").await;
    cache_body(&cache, "imbox-1").await;

    let results = cache
        .search("Needle", Some(SearchMailbox::Feed), 10)
        .await
        .expect("filtered search");

    assert_eq!(
        results
            .iter()
            .map(|item| item.email_id.as_str())
            .collect::<Vec<_>>(),
        vec!["feed-1"]
    );
    assert_eq!(
        backend.stats().queries[0].keywords,
        vec![Keyword::new(MailClassification::Feed.keyword())]
    );
}

#[tokio::test]
async fn mode_off_proxies_directly_to_backend_search() {
    let message = raw_message(
        "backend-1",
        "Needle remote",
        "remote body",
        vec![MailClassification::Imbox.keyword()],
    );
    let (cache, backend, _tempdir) = fixture(vec![message], MailCacheMode::Off).await;

    let empty = cache
        .search("Needle", Some(SearchMailbox::Imbox), 0)
        .await
        .expect("zero-limit backend search returns empty");
    assert!(empty.is_empty());
    assert_eq!(backend.stats().list_calls, 0);

    let results = cache
        .search("Needle", Some(SearchMailbox::Imbox), 10)
        .await
        .expect("mode off backend search");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].email_id, "backend-1");
    assert_eq!(results[0].source, SearchResultSource::Backend);
    let stats = backend.stats();
    assert_eq!(stats.list_calls, 1);
    assert_eq!(stats.get_calls, 1);
    assert_eq!(stats.queries[0].scope, QueryScope::Search);
    assert_eq!(stats.queries[0].text.as_deref(), Some("Needle"));
    assert_eq!(
        stats.queries[0].keywords,
        vec![Keyword::new(MailClassification::Imbox.keyword())]
    );
}

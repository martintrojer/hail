use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend, Mailbox,
    MailboxRole, Page, PageRequest, Principal, Query, RawMessage, SubmissionId, SyncCursor,
};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, CachedMail, FeedRenderMode, MailView, MailViewListOpts};
use hail_core::{BlobId, MailBackfill, MailCacheMode, MailClassification};
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

#[derive(Clone, Default)]
struct BackendStats {
    list_calls: usize,
    get_calls: usize,
    fetch_blob_calls: usize,
}

#[derive(Clone)]
struct FakeBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    blobs: Arc<HashMap<BlobRef, Bytes>>,
    order: Arc<Vec<BackendMsgId>>,
    stats: Arc<Mutex<BackendStats>>,
}

impl FakeBackend {
    fn with_blobs(messages: Vec<RawMessage>, blobs: HashMap<BlobRef, Bytes>) -> Self {
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
            blobs: Arc::new(blobs),
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
        _query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        self.stats.lock().expect("stats lock").list_calls += 1;
        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        Ok(Page {
            items: self.order.iter().take(limit).cloned().collect(),
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

    async fn fetch_blob(&self, id: &BlobRef) -> hail_backend::Result<Bytes> {
        self.stats.lock().expect("stats lock").fetch_blob_calls += 1;
        self.blobs
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "blob",
                id: id.as_str().to_owned(),
            })
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

async fn fixture(
    messages: Vec<RawMessage>,
    mode: MailCacheMode,
) -> (CachedMail, FakeBackend, TempDir) {
    fixture_with_blobs(messages, HashMap::new(), mode).await
}

async fn fixture_with_blobs(
    messages: Vec<RawMessage>,
    blobs_by_ref: HashMap<BlobRef, Bytes>,
    mode: MailCacheMode,
) -> (CachedMail, FakeBackend, TempDir) {
    let pool = hail_db::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    hail_db::migrate(&pool).await.expect("run migrations");
    ensure_default_account(&pool).await;

    let tempdir = tempfile::tempdir().expect("create temp blob dir");
    let blobs = Arc::new(FilesystemBlobStore::new(tempdir.path())) as Arc<dyn BlobStore>;
    let backend = FakeBackend::with_blobs(messages, blobs_by_ref);
    let policy = CachePolicy::new(mode, 90, 50_000, 5 * 1024 * 1024, MailBackfill::Off);
    let cache = CachedMail::new(pool, blobs, Box::new(backend.clone()), policy);
    (cache, backend, tempdir)
}

async fn ensure_default_account(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO users (id, email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (1, 'cache@example.test', 'acct', NULL, 1, '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (1, 1, 'acct', 'gmail', 'gmail', 'provider-acct', 'cache@example.test', ?1, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .expect("insert account");
}

fn raw_message(id: &str, subject: &str, keywords: Vec<&str>) -> RawMessage {
    raw_message_with_body(
        id,
        subject,
        keywords,
        Bytes::from_static(b"Subject: ignored\r\n\r\nbody intentionally not cached"),
    )
}

fn raw_message_with_body(
    id: &str,
    subject: &str,
    keywords: Vec<&str>,
    rfc822: Bytes,
) -> RawMessage {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_owned(), subject.to_owned());
    metadata.insert("preview".to_owned(), format!("preview {subject}"));
    let attachment_blob = BlobRef::new(format!("blob-{id}"));
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(format!("thread-{id}")),
        rfc822,
        keywords: keywords.into_iter().map(Keyword::new).collect(),
        envelope: Some(Envelope {
            mail_from: format!("{id}@example.test"),
            rcpt_to: vec!["me@example.test".to_owned()],
        }),
        received_at_epoch_secs: Some(1_700_000_000),
        size_bytes: Some(1234),
        blob_refs: vec![attachment_blob.clone()],
        attachments: vec![hail_backend::AttachmentMeta {
            filename: format!("{id}.txt"),
            mime_type: "text/plain".to_owned(),
            size_bytes: 42,
            blob_ref: Some(attachment_blob),
            inline: false,
            content_id: None,
        }],
        metadata,
    }
}

async fn assert_blob_file_round_trips(
    root: &Path,
    store: &dyn BlobStore,
    stored_blob_id: &str,
    expected: &Bytes,
) {
    let blob_id = BlobId::parse(stored_blob_id).expect("stored blob id should be parseable");
    let path = root
        .join(&blob_id.hex()[0..2])
        .join(&blob_id.hex()[2..4])
        .join(blob_id.file_name());
    assert!(
        path.is_file(),
        "expected blob file {} to exist on disk",
        path.display()
    );

    let round_tripped = store
        .get(&blob_id)
        .await
        .expect("read blob file through store");
    assert_eq!(round_tripped, expected.as_ref());
}

async fn assert_blob_root_empty(root: &Path) {
    let entries = std::fs::read_dir(root)
        .expect("read temp blob root")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect temp blob root entries");
    assert!(
        entries.is_empty(),
        "expected no blob files under {}, found {} entries",
        root.display(),
        entries.len()
    );
}

#[tokio::test]
async fn constructs_cached_mail_with_sqlite_blob_store_and_backend() {
    let (cache, _backend, _tempdir) = fixture(Vec::new(), MailCacheMode::Bounded).await;

    assert_eq!(cache.backend().capabilities(), &CAPABILITIES);
    let _: &SqlitePool = cache.db();
    let _: &dyn BlobStore = cache.blobs();
}

#[tokio::test]
async fn feed_view_renders_sanitized_inline_html_from_cached_body() {
    let feed_body = Bytes::from_static(
        br#"From: News <feed@example.test>
To: Me <me@example.test>
Subject: Feed HTML
Content-Type: text/html; charset=utf-8

<article><p>Feed body</p><script>alert(1)</script><img src="https://cdn.example/hero.png"><img src="https://track.mailgun.net/open.gif" width="1" height="1"></article>"#,
    );
    let feed = raw_message_with_body(
        "feed-html-1",
        "Feed HTML",
        vec![MailClassification::Feed.keyword()],
        feed_body,
    );
    let imbox = raw_message_with_body(
        "imbox-html-1",
        "Imbox HTML",
        vec![MailClassification::Imbox.keyword()],
        Bytes::from_static(
            br#"Subject: Imbox HTML
Content-Type: text/html; charset=utf-8

<p>Imbox body</p>"#,
        ),
    );
    let (cache, backend, _tempdir) = fixture(vec![feed, imbox], MailCacheMode::Bounded).await;

    let feed_page = cache
        .list_view(MailView::Feed, None, 10, MailViewListOpts::default())
        .await
        .expect("feed list renders inline html");
    assert_eq!(feed_page.items.len(), 1);
    let item = &feed_page.items[0];
    let feed_html = item.feed_html.as_deref().expect("feed_html populated");
    assert!(feed_html.contains("Feed body"));
    assert!(!feed_html.contains("script"));
    assert!(!feed_html.contains("alert"));
    assert!(!feed_html.contains("cdn.example/hero.png"));
    assert!(!feed_html.contains("track.mailgun.net"));
    let feed_html_with_images = item
        .feed_html_with_images
        .as_deref()
        .expect("feed_html_with_images populated");
    assert!(feed_html_with_images.contains("cdn.example/hero.png"));
    assert!(!feed_html_with_images.contains("track.mailgun.net"));
    assert_eq!(item.feed_blocked_images, Some(1));
    let trackers = item
        .feed_blocked_trackers
        .as_ref()
        .expect("feed blocked trackers populated");
    assert_eq!(trackers.len(), 1);
    assert!(trackers[0].src.contains("track.mailgun.net/open.gif"));

    let imbox_page = cache
        .list_view(MailView::Imbox, None, 10, MailViewListOpts::default())
        .await
        .expect("imbox still lists compact preview only");
    assert_eq!(imbox_page.items.len(), 1);
    assert!(imbox_page.items[0].feed_html.is_none());
    assert!(imbox_page.items[0].feed_html_with_images.is_none());
    assert_eq!(backend.stats().get_calls, 3);
}

#[tokio::test]
async fn feed_view_remote_image_pref_selects_with_images_variant() {
    let feed_body = Bytes::from_static(
        br#"Subject: Feed Images
Content-Type: text/html; charset=utf-8

<p>Feed body</p><img src="https://cdn.example/hero.png"><img src="https://track.mailgun.net/open.gif" width="1" height="1">"#,
    );
    let feed = raw_message_with_body(
        "feed-images-1",
        "Feed Images",
        vec![MailClassification::Feed.keyword()],
        feed_body,
    );
    let (cache, _backend, _tempdir) = fixture(vec![feed], MailCacheMode::Bounded).await;

    let page = cache
        .list_view(
            MailView::Feed,
            None,
            10,
            MailViewListOpts {
                feed_render: FeedRenderMode::WithRemoteImages,
            },
        )
        .await
        .expect("feed list renders with remote image pref");

    let item = &page.items[0];
    assert!(
        item.feed_html
            .as_deref()
            .expect("feed_html populated")
            .contains("cdn.example/hero.png")
    );
    assert!(
        !item
            .feed_html
            .as_deref()
            .expect("feed_html populated")
            .contains("track.mailgun.net")
    );
    assert_eq!(item.feed_blocked_images, Some(1));
}

#[tokio::test]
async fn feed_view_plaintext_body_falls_back_to_safe_html() {
    let feed = raw_message_with_body(
        "feed-text-1",
        "Feed Text",
        vec![MailClassification::Feed.keyword()],
        Bytes::from_static(b"Subject: Feed Text\r\n\r\nPlain <body>\nsecond line"),
    );
    let (cache, _backend, _tempdir) = fixture(vec![feed], MailCacheMode::Bounded).await;

    let page = cache
        .list_view(MailView::Feed, None, 10, MailViewListOpts::default())
        .await
        .expect("feed list renders text fallback");

    assert_eq!(page.items.len(), 1);
    let html = page.items[0]
        .feed_html
        .as_deref()
        .expect("feed_html populated");
    assert!(html.contains("Plain &lt;body&gt;<br>second line"));
}

#[tokio::test]
async fn feed_view_degrades_to_preview_when_body_readthrough_fails() {
    let feed = raw_message(
        "feed-missing-body",
        "Feed Missing Body",
        vec![MailClassification::Feed.keyword()],
    );
    let (cache, _backend, _tempdir) = fixture(vec![feed], MailCacheMode::Bounded).await;
    cache
        .get_message(&BackendMsgId::new("feed-missing-body"))
        .await
        .expect("seed metadata row");
    let missing_blob_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.eml";
    sqlx::query("UPDATE messages SET body_blob_id = ?1 WHERE backend_msg_id = ?2")
        .bind(missing_blob_id)
        .bind("feed-missing-body")
        .execute(cache.db())
        .await
        .expect("poison body cache ref");

    let page = cache
        .list_view(MailView::Feed, None, 10, MailViewListOpts::default())
        .await
        .expect("feed list tolerates body cache failure");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].preview, "preview Feed Missing Body");
    assert!(page.items[0].feed_html.is_none());
}

#[tokio::test]
async fn metadata_miss_populates_sqlite_rows_and_second_get_hits_cache() {
    let message = raw_message("msg-1", "Hello", vec![MailClassification::Imbox.keyword()]);
    let (cache, backend, _tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;

    let first = cache
        .get_message(&BackendMsgId::new("msg-1"))
        .await
        .expect("first read populates metadata");
    assert_eq!(first.subject, "Hello");
    assert_eq!(first.blob_refs, vec![BlobRef::new("blob-msg-1")]);
    assert_eq!(backend.stats().list_calls, 1);
    assert_eq!(backend.stats().get_calls, 1);

    let (message_rows, keyword_rows, attachment_rows): (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT COUNT(*) FROM messages), \
           (SELECT COUNT(*) FROM message_keywords), \
           (SELECT COUNT(*) FROM attachments)",
    )
    .fetch_one(cache.db())
    .await
    .expect("count cache rows");
    assert_eq!(message_rows, 1);
    assert_eq!(keyword_rows, 1);
    assert_eq!(attachment_rows, 1);

    let attachment: (String, String, i64, Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT filename, mime_type, size_bytes, backend_blob_ref, cached_blob_id, inline FROM attachments",
    )
    .fetch_one(cache.db())
    .await
    .expect("read attachment row");
    assert_eq!(attachment.0, "msg-1.txt");
    assert_eq!(attachment.1, "text/plain");
    assert_eq!(attachment.2, 42);
    assert_eq!(attachment.3.as_deref(), Some("blob-msg-1"));
    assert!(attachment.4.is_none());
    assert_eq!(attachment.5, 0);

    let body_blob: Option<String> = sqlx::query_scalar("SELECT body_blob_id FROM messages")
        .fetch_one(cache.db())
        .await
        .expect("read body blob marker");
    assert!(
        body_blob.is_none(),
        "metadata task must not cache raw bodies"
    );

    let second = cache
        .get_message(&BackendMsgId::new("msg-1"))
        .await
        .expect("second read hits sqlite");
    assert_eq!(second.subject, "Hello");
    assert_eq!(backend.stats().list_calls, 1);
    assert_eq!(backend.stats().get_calls, 1);
}

#[tokio::test]
async fn list_and_count_populate_then_read_from_sqlite() {
    let imbox = raw_message(
        "imbox-1",
        "Imbox",
        vec![MailClassification::Imbox.keyword()],
    );
    let feed = raw_message(
        "feed-1",
        "Feed",
        vec![MailClassification::Feed.keyword(), "$seen"],
    );
    let (cache, backend, _tempdir) = fixture(vec![imbox, feed], MailCacheMode::Bounded).await;

    let page = cache
        .list_view(MailView::Imbox, None, 10, MailViewListOpts::default())
        .await
        .expect("list view");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].email_id, "imbox-1");
    assert_eq!(backend.stats().list_calls, 1);
    assert_eq!(backend.stats().get_calls, 2);

    let feed_count = cache
        .count_view(MailView::Feed, false)
        .await
        .expect("count from existing sqlite metadata");
    assert_eq!(feed_count, 1);
    assert_eq!(backend.stats().list_calls, 1);
    assert_eq!(backend.stats().get_calls, 2);

    let unread_imbox = cache
        .count_view(MailView::Imbox, true)
        .await
        .expect("count view");
    assert_eq!(unread_imbox, 1);
    assert_eq!(backend.stats().list_calls, 1);
    assert_eq!(backend.stats().get_calls, 2);
}

#[tokio::test]
async fn cache_off_proxies_backend_and_writes_no_rows() {
    let message = raw_message("live-1", "Live", vec![MailClassification::Imbox.keyword()]);
    let (cache, backend, _tempdir) = fixture(vec![message], MailCacheMode::Off).await;

    let got = cache
        .get_message(&BackendMsgId::new("live-1"))
        .await
        .expect("live get");
    assert_eq!(got.subject, "Live");

    let page = cache
        .list_view(MailView::Imbox, None, 10, MailViewListOpts::default())
        .await
        .expect("live list");
    assert_eq!(page.items.len(), 1);
    assert!(backend.stats().list_calls >= 1);
    assert!(backend.stats().get_calls >= 2);

    let message_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(cache.db())
        .await
        .expect("count messages");
    assert_eq!(message_rows, 0);
}

#[tokio::test]
async fn bodies_body_miss_fetches_stores_and_second_read_hits_blob_store_without_backend() {
    let message = raw_message("body-1", "Body", vec![MailClassification::Imbox.keyword()]);
    let expected = message.rfc822.clone();
    let (cache, backend, tempdir) = fixture(vec![message], MailCacheMode::Bounded).await;

    let first = cache
        .get_message_body(&BackendMsgId::new("body-1"))
        .await
        .expect("first body read fetches backend");
    assert_eq!(first, expected);
    assert_eq!(backend.stats().get_calls, 1);

    let row: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT body_blob_id, body_text FROM messages WHERE backend_msg_id = 'body-1'",
    )
    .fetch_one(cache.db())
    .await
    .expect("read body cache row");
    let body_blob_id = row
        .0
        .expect("body_blob_id should be set after body read-through");
    assert!(body_blob_id.ends_with(".eml"));
    assert_blob_file_round_trips(tempdir.path(), cache.blobs(), &body_blob_id, &expected).await;
    assert!(
        row.1
            .as_deref()
            .is_some_and(|text| text.contains("body intentionally not cached"))
    );

    let second = cache
        .get_message_body(&BackendMsgId::new("body-1"))
        .await
        .expect("second body read hits blob store");
    assert_eq!(second, expected);
    assert_eq!(backend.stats().get_calls, 1);
}

#[tokio::test]
async fn bodies_full_mode_body_miss_writes_blob_file() {
    let message = raw_message(
        "full-body-1",
        "Full Body",
        vec![MailClassification::Imbox.keyword()],
    );
    let expected = message.rfc822.clone();
    let (cache, backend, tempdir) = fixture(vec![message], MailCacheMode::Full).await;

    let got = cache
        .get_message_body(&BackendMsgId::new("full-body-1"))
        .await
        .expect("full mode body read fetches backend");
    assert_eq!(got, expected);
    assert_eq!(backend.stats().get_calls, 1);

    let body_blob_id: String = sqlx::query_scalar(
        "SELECT body_blob_id FROM messages WHERE backend_msg_id = 'full-body-1'",
    )
    .fetch_one(cache.db())
    .await
    .expect("read full mode body blob id");
    assert!(body_blob_id.ends_with(".eml"));
    assert_blob_file_round_trips(tempdir.path(), cache.blobs(), &body_blob_id, &expected).await;
}

#[tokio::test]
async fn bodies_cache_off_fetches_without_storing_rows_or_blobs() {
    let message = raw_message(
        "off-body-1",
        "Off Body",
        vec![MailClassification::Imbox.keyword()],
    );
    let expected = message.rfc822.clone();
    let (cache, backend, tempdir) = fixture(vec![message], MailCacheMode::Off).await;

    let got = cache
        .get_message_body(&BackendMsgId::new("off-body-1"))
        .await
        .expect("off mode body read");
    assert_eq!(got, expected);
    assert_eq!(backend.stats().get_calls, 1);

    let message_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(cache.db())
        .await
        .expect("count messages");
    assert_eq!(message_rows, 0);
    assert_blob_root_empty(tempdir.path()).await;
}

#[tokio::test]
async fn bodies_attachment_blob_round_trips_through_blob_store() {
    let message = raw_message(
        "att-1",
        "Attachment",
        vec![MailClassification::Imbox.keyword()],
    );
    let backend_blob_ref = BlobRef::new("blob-att-1");
    let expected = Bytes::from_static(b"attachment bytes");
    let mut backend_blobs = HashMap::new();
    backend_blobs.insert(backend_blob_ref.clone(), expected.clone());
    let (cache, backend, tempdir) =
        fixture_with_blobs(vec![message], backend_blobs, MailCacheMode::Bounded).await;

    cache
        .get_message(&BackendMsgId::new("att-1"))
        .await
        .expect("populate metadata with backend blob ref");

    let first = cache
        .get_blob(&backend_blob_ref)
        .await
        .expect("first attachment read fetches backend blob");
    assert_eq!(first, expected);
    assert_eq!(backend.stats().fetch_blob_calls, 1);

    let (preserved_backend_ref, stored_blob_id): (String, String) = sqlx::query_as(
        "SELECT attachments.backend_blob_ref, attachments.cached_blob_id FROM attachments \
         JOIN messages ON messages.id = attachments.message_id \
         WHERE messages.backend_msg_id = 'att-1'",
    )
    .fetch_one(cache.db())
    .await
    .expect("read stored attachment refs");
    assert_eq!(preserved_backend_ref, backend_blob_ref.as_str());
    assert!(stored_blob_id.ends_with(".att"));
    assert_blob_file_round_trips(tempdir.path(), cache.blobs(), &stored_blob_id, &expected).await;

    let second = cache
        .get_blob(&backend_blob_ref)
        .await
        .expect("second attachment read by backend ref hits blob store");
    assert_eq!(second, expected);
    assert_eq!(backend.stats().fetch_blob_calls, 1);
}

#[tokio::test]
async fn attachment_cache_separates_backend_ref_from_local_blob_id_collision() {
    let backend_blob_ref =
        BlobRef::new("1242a0f78c81455c13652c0f76f8910a6787b1b2eda990992736d294cc3a4206.att");
    let mut message = raw_message(
        "collision-att-1",
        "Attachment collision",
        vec![MailClassification::Imbox.keyword()],
    );
    message.blob_refs = vec![backend_blob_ref.clone()];
    message.attachments[0].blob_ref = Some(backend_blob_ref.clone());

    let backend_expected = Bytes::from_static(b"backend attachment bytes");
    let wrong_local = Bytes::from_static(b"evil local bytes");
    let mut backend_blobs = HashMap::new();
    backend_blobs.insert(backend_blob_ref.clone(), backend_expected.clone());
    let (cache, backend, _tempdir) =
        fixture_with_blobs(vec![message], backend_blobs, MailCacheMode::Bounded).await;

    let wrong_local_id = cache
        .blobs()
        .put(hail_core::BlobKind::Att, &wrong_local)
        .await
        .expect("seed colliding local blob id");
    assert_eq!(wrong_local_id.to_string(), backend_blob_ref.as_str());

    cache
        .get_message(&BackendMsgId::new("collision-att-1"))
        .await
        .expect("populate metadata with backend blob ref that parses as local id");

    let first = cache
        .get_blob(&backend_blob_ref)
        .await
        .expect("first attachment read fetches backend blob despite local-id collision");
    assert_eq!(first, backend_expected);
    assert_ne!(first, wrong_local);
    assert_eq!(backend.stats().fetch_blob_calls, 1);

    let (preserved_backend_ref, cached_blob_id): (String, String) = sqlx::query_as(
        "SELECT attachments.backend_blob_ref, attachments.cached_blob_id FROM attachments \
         JOIN messages ON messages.id = attachments.message_id \
         WHERE messages.backend_msg_id = 'collision-att-1'",
    )
    .fetch_one(cache.db())
    .await
    .expect("read collision attachment refs");
    assert_eq!(preserved_backend_ref, backend_blob_ref.as_str());
    assert_ne!(cached_blob_id, backend_blob_ref.as_str());

    let second = cache
        .get_blob(&backend_blob_ref)
        .await
        .expect("second attachment read still looks up by backend ref");
    assert_eq!(second, backend_expected);
    assert_eq!(backend.stats().fetch_blob_calls, 1);
}

#[tokio::test]
async fn bodies_full_mode_attachment_blob_writes_blob_file() {
    let message = raw_message(
        "full-att-1",
        "Full Attachment",
        vec![MailClassification::Imbox.keyword()],
    );
    let backend_blob_ref = BlobRef::new("blob-full-att-1");
    let expected = Bytes::from_static(b"full mode attachment bytes");
    let mut backend_blobs = HashMap::new();
    backend_blobs.insert(backend_blob_ref.clone(), expected.clone());
    let (cache, backend, tempdir) =
        fixture_with_blobs(vec![message], backend_blobs, MailCacheMode::Full).await;

    cache
        .get_message(&BackendMsgId::new("full-att-1"))
        .await
        .expect("populate full mode metadata with backend blob ref");

    let got = cache
        .get_blob(&backend_blob_ref)
        .await
        .expect("full mode attachment read fetches backend blob");
    assert_eq!(got, expected);
    assert_eq!(backend.stats().fetch_blob_calls, 1);

    let (preserved_backend_ref, stored_blob_id): (String, String) = sqlx::query_as(
        "SELECT attachments.backend_blob_ref, attachments.cached_blob_id FROM attachments \
         JOIN messages ON messages.id = attachments.message_id \
         WHERE messages.backend_msg_id = 'full-att-1'",
    )
    .fetch_one(cache.db())
    .await
    .expect("read full mode stored attachment refs");
    assert_eq!(preserved_backend_ref, backend_blob_ref.as_str());
    assert!(stored_blob_id.ends_with(".att"));
    assert_blob_file_round_trips(tempdir.path(), cache.blobs(), &stored_blob_id, &expected).await;
}

#[tokio::test]
async fn get_thread_and_send_are_implemented() {
    let (cache, _backend, _tempdir) = fixture(Vec::new(), MailCacheMode::Bounded).await;

    // get_thread is implemented (read-through); an empty cache yields an empty thread.
    let thread = cache
        .get_thread("thread-1")
        .await
        .expect("get_thread is implemented and returns an empty thread on a cold cache");
    assert!(thread.messages.is_empty());

    // send_enqueue is implemented and dispatches to the backend, returning its
    // submission id.
    let envelope = Envelope {
        mail_from: "me@example.test".to_owned(),
        rcpt_to: vec!["you@example.test".to_owned()],
    };
    let submission = cache
        .send_enqueue(b"Subject: hello\r\n\r\nbody", &envelope)
        .await
        .expect("send_enqueue dispatches to the backend");
    assert_eq!(submission, SubmissionId::new("fake-submission"));
}

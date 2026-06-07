use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use bytes::Bytes;
use futures_util::stream;
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::state::AppState;
use hail_backend::{
    AttachmentMeta, BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend,
    Mailbox, MailboxRole, Page, PageRequest, Principal, Query, QueryScope, RawMessage,
    SubmissionId, SyncCursor,
};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, CachedMail};
use hail_core::{MailBackfill, MailCacheMode, MailClassification};
use hail_test::{fixture_config, json_body, seed_session};
use sqlx::{Row, SqlitePool};
use tempfile::TempDir;
use tower::ServiceExt;

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
    mutation_calls: usize,
    queries: Vec<Query>,
}

#[derive(Clone)]
struct FakeBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    blobs: Arc<HashMap<BlobRef, Bytes>>,
    order: Arc<Vec<BackendMsgId>>,
    stats: Arc<Mutex<BackendStats>>,
}

impl FakeBackend {
    fn new(messages: Vec<RawMessage>, blobs: HashMap<BlobRef, Bytes>) -> Self {
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
        self.stats.lock().expect("backend stats lock").clone()
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
        let mut stats = self.stats.lock().expect("backend stats lock");
        stats.list_calls += 1;
        stats.queries.push(query.clone());
        drop(stats);

        let mut items = self.order.iter().cloned().collect::<Vec<_>>();
        if query.scope == QueryScope::Thread {
            let wanted = query.text.as_deref().unwrap_or_default();
            items.retain(|id| {
                self.messages
                    .get(id)
                    .and_then(|message| message.thread_id.as_deref())
                    == Some(wanted)
            });
        }
        if query.scope == QueryScope::Search {
            if let Some(text) = query.text.as_deref() {
                let needle = text.to_ascii_lowercase();
                items.retain(|id| {
                    self.messages.get(id).is_some_and(|message| {
                        message
                            .metadata
                            .get("subject")
                            .is_some_and(|subject| subject.to_ascii_lowercase().contains(&needle))
                            || message.metadata.get("preview").is_some_and(|preview| {
                                preview.to_ascii_lowercase().contains(&needle)
                            })
                    })
                });
            }
            for keyword in &query.keywords {
                items.retain(|id| {
                    self.messages.get(id).is_some_and(|message| {
                        message
                            .keywords
                            .iter()
                            .any(|candidate| candidate == keyword)
                    })
                });
            }
        }
        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        items.truncate(limit);
        Ok(Page {
            items,
            next_cursor: None,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.stats.lock().expect("backend stats lock").get_calls += 1;
        self.messages
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
    }

    async fn fetch_blob(&self, id: &BlobRef) -> hail_backend::Result<Bytes> {
        self.stats
            .lock()
            .expect("backend stats lock")
            .fetch_blob_calls += 1;
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
        self.stats
            .lock()
            .expect("backend stats lock")
            .mutation_calls += 1;
        Ok(())
    }

    async fn move_to_role(
        &self,
        _id: &BackendMsgId,
        _role: MailboxRole,
    ) -> hail_backend::Result<()> {
        self.stats
            .lock()
            .expect("backend stats lock")
            .mutation_calls += 1;
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> hail_backend::Result<()> {
        self.stats
            .lock()
            .expect("backend stats lock")
            .mutation_calls += 1;
        Ok(())
    }

    async fn send(
        &self,
        _rfc822: &[u8],
        _envelope: &Envelope,
    ) -> hail_backend::Result<SubmissionId> {
        self.stats
            .lock()
            .expect("backend stats lock")
            .mutation_calls += 1;
        Ok(SubmissionId::new("fake-submission"))
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(&self) -> futures_core::stream::BoxStream<'static, Change> {
        Box::pin(stream::empty())
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Ok(Vec::new())
    }
}

struct Fixture {
    state: AppState,
    key: [u8; hail_core::KEY_LEN],
    backend: FakeBackend,
    blob_root: TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let db_url = "sqlite::memory:";
        let db = hail_db::connect(db_url).await.expect("open in-memory db");
        hail_db::migrate(&db).await.expect("migrate db");

        let key = [0xA5; hail_core::KEY_LEN];
        let blob_root = tempfile::tempdir().expect("blob tempdir");
        let (messages, backend_blobs) = backend_data();
        let backend = FakeBackend::new(messages, backend_blobs);
        let blobs = Arc::new(FilesystemBlobStore::new(blob_root.path())) as Arc<dyn BlobStore>;
        let cache = Arc::new(CachedMail::new(
            db.clone(),
            blobs,
            Box::new(backend.clone()),
            CachePolicy::new(
                MailCacheMode::Bounded,
                90,
                50_000,
                5 * 1024 * 1024,
                MailBackfill::Off,
            ),
        ));
        let state = AppState {
            db,
            config: fixture_config(db_url, &key),
            server_key: Arc::new(key),
            auth_rate_limiter: Arc::new(hail_api::middleware::rate_limit::IpRateLimiter::default()),
            mail: cache,
            events: hail_api::events::AppEventBus::default(),
        };
        Self {
            state,
            key,
            backend,
            blob_root,
        }
    }

    async fn authed_session(&self) -> (i64, String) {
        let (user_id, sid) = seed_session(&self.state, &self.key, "cache-route@example.test").await;
        sqlx::query(
            "INSERT INTO mail_accounts \
             (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
             VALUES (?1, 'acct-cache-route', 'gmail', 'gmail', 'provider-cache-route', 'cache-route@example.test', ?2, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user_id)
        .bind(vec![7_u8; 32])
        .execute(&self.state.db)
        .await
        .expect("insert mail account");
        (user_id, sid)
    }
}

fn app(state: AppState) -> axum::Router {
    let protected = hail_api::routes::views::router()
        .merge(hail_api::routes::threads::router())
        .merge(axum::Router::from(hail_api::routes::threads_view::router()))
        .merge(axum::Router::from(hail_api::routes::attachments::router()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ));
    axum::Router::new().merge(protected).with_state(state)
}

async fn request(
    state: AppState,
    sid: Option<&str>,
    method: Method,
    uri: &str,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let req = builder
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_owned())))
        .expect("build request");
    app(state).oneshot(req).await.expect("route response")
}

async fn get_json(state: AppState, sid: &str, uri: &str) -> serde_json::Value {
    let response = request(state, Some(sid), Method::GET, uri, false, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

fn backend_data() -> (Vec<RawMessage>, HashMap<BlobRef, Bytes>) {
    let attachment_ref = BlobRef::new("backend-att-imbox");
    let imbox_body = Bytes::from_static(
        b"From: Alice <alice@example.test>\r\nTo: Me <me@example.test>\r\nSubject: Cache Route Subject\r\n\r\nHello from cached body.",
    );
    let feed_body = Bytes::from_static(
        br#"From: News <news@example.test>
To: Me <me@example.test>
Subject: Daily Cache News
Content-Type: text/html; charset=utf-8

<article><p>Feed body</p><img src="https://cdn.example/hero.png"><img src="https://track.mailgun.net/open.gif" width="1" height="1"></article>"#,
    );
    let mut blobs = HashMap::new();
    blobs.insert(
        attachment_ref.clone(),
        Bytes::from_static(b"attachment bytes from backend"),
    );

    (
        vec![
            raw_message(
                "msg-imbox",
                "thread-imbox",
                "alice@example.test",
                "Cache Route Subject",
                "preview cached route",
                Bytes::clone(&imbox_body),
                vec![MailClassification::Imbox.keyword()],
                vec![attachment_ref],
                vec![AttachmentMeta {
                    filename: "evidence.txt".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    size_bytes: 29,
                    blob_ref: Some(BlobRef::new("backend-att-imbox")),
                    inline: false,
                    content_id: None,
                }],
                1_700_000_010,
            ),
            raw_message(
                "msg-feed",
                "thread-feed",
                "news@example.test",
                "Daily Cache News",
                "preview feed cache",
                feed_body,
                vec![MailClassification::Feed.keyword(), "$seen"],
                Vec::new(),
                Vec::new(),
                1_700_000_000,
            ),
        ],
        blobs,
    )
}

#[allow(clippy::too_many_arguments)]
fn raw_message(
    id: &str,
    thread_id: &str,
    from: &str,
    subject: &str,
    preview: &str,
    rfc822: Bytes,
    keywords: Vec<&str>,
    blob_refs: Vec<BlobRef>,
    attachments: Vec<AttachmentMeta>,
    received_at_epoch_secs: i64,
) -> RawMessage {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_owned(), subject.to_owned());
    metadata.insert("preview".to_owned(), preview.to_owned());
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(thread_id.to_owned()),
        rfc822,
        keywords: keywords.into_iter().map(Keyword::new).collect(),
        envelope: Some(Envelope {
            mail_from: from.to_owned(),
            rcpt_to: vec!["me@example.test".to_owned()],
        }),
        received_at_epoch_secs: Some(received_at_epoch_secs),
        size_bytes: Some(1234),
        blob_refs,
        attachments,
        metadata,
    }
}

async fn message_keywords(pool: &SqlitePool, backend_msg_id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT mk.keyword \
         FROM message_keywords mk \
         INNER JOIN messages m ON m.id = mk.message_id \
         WHERE m.backend_msg_id = ?1 \
         ORDER BY mk.keyword",
    )
    .bind(backend_msg_id)
    .fetch_all(pool)
    .await
    .expect("read message keywords")
}

async fn outbound_rows(pool: &SqlitePool) -> Vec<(String, String, String)> {
    sqlx::query(
        "SELECT backend_msg_id, change_type, payload_json FROM outbound_changes ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("read outbound rows")
    .into_iter()
    .map(|row| {
        (
            row.get("backend_msg_id"),
            row.get("change_type"),
            row.get("payload_json"),
        )
    })
    .collect()
}

async fn cached_blob_id(pool: &SqlitePool, backend_msg_id: &str) -> String {
    sqlx::query_scalar("SELECT body_blob_id FROM messages WHERE backend_msg_id = ?1")
        .bind(backend_msg_id)
        .fetch_one(pool)
        .await
        .expect("read body blob id")
}

fn blob_file_exists(root: &std::path::Path, stored_blob_id: &str) -> bool {
    let blob_id = hail_core::BlobId::parse(stored_blob_id).expect("parse cached blob id");
    root.join(&blob_id.hex()[0..2])
        .join(&blob_id.hex()[2..4])
        .join(blob_id.file_name())
        .is_file()
}

#[tokio::test]
async fn list_open_body_attachment_and_search_use_real_cache_layers() {
    let fixture = Fixture::new().await;
    let (_user_id, sid) = fixture.authed_session().await;

    let imbox = get_json(fixture.state.clone(), &sid, "/api/views/imbox?limit=10").await;
    assert_eq!(imbox["items"].as_array().expect("items").len(), 1);
    assert_eq!(imbox["items"][0]["email_id"], "msg-imbox");
    assert_eq!(imbox["items"][0]["subject"], "Cache Route Subject");
    assert_eq!(imbox["items"][0]["classification"], "imbox");
    assert_eq!(imbox["items"][0]["unread"], true);

    let cached_message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&fixture.state.db)
        .await
        .expect("count messages");
    assert_eq!(cached_message_count, 2);

    let thread = get_json(fixture.state.clone(), &sid, "/api/threads/thread-imbox").await;
    assert_eq!(thread["thread_id"], "thread-imbox");
    assert_eq!(
        thread["messages"]
            .as_array()
            .expect("thread messages")
            .len(),
        1
    );
    assert_eq!(thread["messages"][0]["email_id"], "msg-imbox");
    assert_eq!(thread["messages"][0]["html"], "Hello from cached body.");
    assert_eq!(
        thread["messages"][0]["attachments"][0]["blob_id"],
        "backend-att-imbox"
    );

    let body_blob_id = cached_blob_id(&fixture.state.db, "msg-imbox").await;
    assert!(body_blob_id.ends_with(".eml"));
    assert!(blob_file_exists(fixture.blob_root.path(), &body_blob_id));
    let stats_after_first_thread = fixture.backend.stats();
    assert_eq!(stats_after_first_thread.fetch_blob_calls, 0);

    let thread_again = get_json(fixture.state.clone(), &sid, "/api/threads/thread-imbox").await;
    assert_eq!(
        thread_again["messages"][0]["html"],
        "Hello from cached body."
    );
    assert_eq!(fixture.backend.stats().fetch_blob_calls, 0);

    let attachments = get_json(fixture.state.clone(), &sid, "/api/attachments?limit=10").await;
    assert_eq!(
        attachments["items"]
            .as_array()
            .expect("attachment items")
            .len(),
        1
    );
    assert_eq!(attachments["items"][0]["name"], "evidence.txt");
    assert_eq!(attachments["items"][0]["context"]["email_id"], "msg-imbox");

    let download = request(
        fixture.state.clone(),
        Some(&sid),
        Method::GET,
        "/api/attachments/backend-att-imbox/download",
        false,
        None,
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    let bytes = http_body_util::BodyExt::collect(download.into_body())
        .await
        .expect("collect download")
        .to_bytes();
    assert_eq!(&bytes[..], b"attachment bytes from backend");
    // Local blob-store id lives in cached_blob_id after the
    // backend_blob_ref/cached_blob_id split; backend_blob_ref holds the
    // provider-native ref.
    let stored_attachment_ref: String = sqlx::query_scalar(
        "SELECT attachments.cached_blob_id \
         FROM attachments INNER JOIN messages ON messages.id = attachments.message_id \
         WHERE messages.backend_msg_id = 'msg-imbox'",
    )
    .fetch_one(&fixture.state.db)
    .await
    .expect("read attachment cached blob id");
    assert!(stored_attachment_ref.ends_with(".att"));
    assert!(blob_file_exists(
        fixture.blob_root.path(),
        &stored_attachment_ref
    ));

    let feed = get_json(fixture.state.clone(), &sid, "/api/views/feed?limit=10").await;
    assert_eq!(feed["items"].as_array().expect("feed items").len(), 1);
    assert_eq!(feed["items"][0]["email_id"], "msg-feed");
    assert_eq!(feed["items"][0]["feed_html"], serde_json::Value::Null);
    assert_eq!(feed["items"][0]["feed_html_with_images"], serde_json::Value::Null);

    let search = get_json(
        fixture.state.clone(),
        &sid,
        "/api/views/search?q=Cache%20Route&scope=mail&mailbox=imbox",
    )
    .await;
    assert_eq!(
        search["results"].as_array().expect("search results").len(),
        1
    );
    assert_eq!(search["results"][0]["type"], "mail");
    assert_eq!(search["results"][0]["email_id"], "msg-imbox");

    let stats = fixture.backend.stats();
    assert!(stats.list_calls > 0);
    assert!(stats.get_calls >= 2);
}

#[tokio::test]
async fn thread_mutations_update_cache_enqueue_outbound_and_require_csrf() {
    let fixture = Fixture::new().await;
    let (_user_id, sid) = fixture.authed_session().await;

    let initial = get_json(fixture.state.clone(), &sid, "/api/views/imbox?limit=10").await;
    assert_eq!(initial["items"].as_array().expect("initial items").len(), 1);

    let no_auth = request(
        fixture.state.clone(),
        None,
        Method::POST,
        "/api/threads/thread-imbox/mark",
        true,
        Some(r#"{"read":true}"#),
    )
    .await;
    assert_eq!(no_auth.status(), StatusCode::UNAUTHORIZED);

    let no_csrf = request(
        fixture.state.clone(),
        Some(&sid),
        Method::POST,
        "/api/threads/thread-imbox/mark",
        false,
        Some(r#"{"read":true}"#),
    )
    .await;
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    let mark = request(
        fixture.state.clone(),
        Some(&sid),
        Method::POST,
        "/api/threads/thread-imbox/mark",
        true,
        Some(r#"{"read":true}"#),
    )
    .await;
    assert_eq!(mark.status(), StatusCode::NO_CONTENT);
    assert!(
        message_keywords(&fixture.state.db, "msg-imbox")
            .await
            .contains(&"$seen".to_owned())
    );
    assert_eq!(
        outbound_rows(&fixture.state.db).await,
        vec![(
            "msg-imbox".to_owned(),
            "read".to_owned(),
            r#"{"keyword":"$seen"}"#.to_owned()
        )]
    );
    assert_eq!(
        fixture
            .state
            .mail
            .pending_sync_count()
            .await
            .expect("pending sync count"),
        1
    );
    assert_eq!(fixture.backend.stats().mutation_calls, 0);

    let archive = request(
        fixture.state.clone(),
        Some(&sid),
        Method::POST,
        "/api/threads/thread-imbox/archive",
        true,
        None,
    )
    .await;
    assert_eq!(archive.status(), StatusCode::OK);
    let archived_keywords = message_keywords(&fixture.state.db, "msg-imbox").await;
    assert!(archived_keywords.contains(&"$archive".to_owned()));
    assert!(!archived_keywords.contains(&MailClassification::Imbox.keyword().to_owned()));

    let rows = outbound_rows(&fixture.state.db).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].0, "msg-imbox");
    assert_eq!(rows[1].1, "role_move");
    assert_eq!(rows[1].2, r#"{"role":"archive"}"#);

    let imbox_after_archive =
        get_json(fixture.state.clone(), &sid, "/api/views/imbox?limit=10").await;
    assert_eq!(
        imbox_after_archive["items"]
            .as_array()
            .expect("imbox after archive")
            .len(),
        0
    );
    let archive_view = get_json(fixture.state.clone(), &sid, "/api/views/archive?limit=10").await;
    assert_eq!(
        archive_view["items"]
            .as_array()
            .expect("archive items")
            .len(),
        1
    );
    assert_eq!(archive_view["items"][0]["email_id"], "msg-imbox");
    assert_eq!(archive_view["items"][0]["classification"], "archive");
    assert_eq!(
        fixture
            .state
            .mail
            .pending_sync_count()
            .await
            .expect("pending sync count"),
        2
    );
}

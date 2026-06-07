use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use bytes::Bytes;
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::drafts::{
    CacheDraftStore, DraftCreate, DraftDetails, DraftStore, DraftStoreError, DraftUpdate,
};
use hail_api::state::AppState;
use hail_backend::{BackendMsgId, Envelope, Keyword, RawMessage};
use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{CachePolicy, CachedMail};
use hail_core::{MailBackfill, MailCacheMode};
use hail_test::{fixture_state, json_body, seed_session};
use secrecy::SecretString;
use serde_json::Value;
use tower::ServiceExt;

fn app(state: AppState, store: Arc<FakeDraftStore>) -> Router {
    let protected = hail_api::routes::drafts::router_with_store(store).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn request(
    state: AppState,
    store: Arc<FakeDraftStore>,
    method: Method,
    path: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }

    app(state, store)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn production_app(state: AppState) -> Router {
    let protected = hail_api::routes::drafts::router_with_store(Arc::new(CacheDraftStore)).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn state_with_cache_mail(
    state: &AppState,
    user_id: i64,
    messages: Vec<RawMessage>,
) -> AppState {
    sqlx::query(
        "INSERT OR IGNORE INTO mail_accounts \
         (id, user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (1, ?1, 'acct', 'gmail', 'gmail', 'provider-acct', 'cache@example.test', ?2, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(user_id)
    .bind(vec![7_u8; 32])
    .execute(&state.db)
    .await
    .expect("insert cache mail account");
    let tempdir = tempfile::tempdir().expect("create temp blob dir");
    let blob_root = tempdir.keep();
    let blobs = Arc::new(FilesystemBlobStore::new(blob_root)) as Arc<dyn BlobStore>;
    AppState {
        db: state.db.clone(),
        config: state.config.clone(),
        server_key: state.server_key.clone(),
        auth_rate_limiter: state.auth_rate_limiter.clone(),
        mail: Arc::new(CachedMail::new(
            state.db.clone(),
            blobs,
            Box::new(TestMailBackend::with_messages(messages)),
            CachePolicy::new(
                MailCacheMode::Bounded,
                90,
                50_000,
                5 * 1024 * 1024,
                MailBackfill::Off,
            ),
        )),
        events: state.events.clone(),
    }
}

#[derive(Clone, Default)]
struct TestMailBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    order: Arc<Vec<BackendMsgId>>,
}

impl TestMailBackend {
    fn with_messages(messages: Vec<RawMessage>) -> Self {
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
        }
    }
}

#[async_trait]
impl hail_backend::MailBackend for TestMailBackend {
    fn capabilities(&self) -> &'static hail_backend::Capabilities {
        hail_api::test_support::FakeMailBackend::empty().capabilities()
    }

    async fn list_message_ids(
        &self,
        _query: &hail_backend::Query,
        page: &hail_backend::PageRequest,
    ) -> hail_backend::Result<hail_backend::Page<BackendMsgId>> {
        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        Ok(hail_backend::Page {
            items: self.order.iter().take(limit).cloned().collect(),
            next_cursor: None,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.messages
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
    }

    async fn fetch_blob(&self, id: &hail_backend::BlobRef) -> hail_backend::Result<Bytes> {
        Err(hail_backend::Error::NotFound {
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
        _role: hail_backend::MailboxRole,
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
    ) -> hail_backend::Result<hail_backend::SubmissionId> {
        Ok(hail_backend::SubmissionId::new("fake-submission"))
    }

    async fn poll_changes(
        &self,
        cursor: &hail_backend::SyncCursor,
    ) -> hail_backend::Result<(Vec<hail_backend::Change>, hail_backend::SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(
        &self,
    ) -> futures_util::stream::BoxStream<'static, hail_backend::Change> {
        Box::pin(futures_util::stream::empty())
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<hail_backend::Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<hail_backend::Principal>> {
        Ok(Vec::new())
    }
}

fn raw_draft_message(
    id: &str,
    from: &str,
    to: Vec<&str>,
    cc: Vec<&str>,
    bcc: Vec<&str>,
    subject: &str,
    rfc822: &str,
) -> RawMessage {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_owned(), subject.to_owned());
    metadata.insert("preview".to_owned(), String::new());
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: Some(format!("thread-{id}")),
        rfc822: Bytes::from(rfc822.to_owned()),
        keywords: vec![Keyword::new("$draft")],
        envelope: Some(Envelope {
            mail_from: from.to_owned(),
            rcpt_to: to
                .iter()
                .chain(cc.iter())
                .chain(bcc.iter())
                .map(|addr| (*addr).to_owned())
                .collect(),
        }),
        received_at_epoch_secs: Some(1_700_000_000),
        size_bytes: Some(u64::try_from(rfc822.len()).expect("rfc822 length fits u64")),
        blob_refs: Vec::new(),
        attachments: Vec::new(),
        metadata,
    }
}

fn draft_rfc822(
    from: &str,
    to: &str,
    cc: &str,
    subject: &str,
    content_type: &str,
    body: &str,
) -> String {
    format!(
        "From: {from}\r\nTo: {to}\r\nCc: {cc}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: {content_type}; charset=utf-8\r\n\r\n{body}"
    )
}

fn assert_text_inside_blockquote(html: &str, text: &str) {
    let blockquote_start = html.find("<blockquote").expect("blockquote start");
    let blockquote_end = html[blockquote_start..]
        .find("</blockquote>")
        .map(|offset| blockquote_start + offset)
        .expect("blockquote end");
    let text_position = html.find(text).expect("blockquote text");

    assert!(
        blockquote_start < text_position && text_position < blockquote_end,
        "expected {text:?} inside blockquote"
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Create {
        from: String,
        to: Vec<String>,
        cc: Vec<String>,
        bcc: Vec<String>,
        subject: String,
        body_markdown: String,
        body_html: String,
    },
    Get {
        draft_id: String,
    },
    Update {
        draft_id: String,
        to: Option<Vec<String>>,
        cc: Option<Vec<String>>,
        bcc: Option<Vec<String>>,
        subject: Option<String>,
        body_markdown: Option<String>,
        body_html: Option<String>,
    },
    Delete {
        draft_id: String,
    },
}

#[derive(Default)]
struct FakeDraftStore {
    calls: Mutex<Vec<Call>>,
    fail: Mutex<Option<DraftStoreError>>,
    get_response: Mutex<Option<DraftDetails>>,
}

impl FakeDraftStore {
    fn with_get_response(response: DraftDetails) -> Self {
        Self {
            get_response: Mutex::new(Some(response)),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }

    fn fail_next(&self, error: DraftStoreError) {
        *self.fail.lock().expect("fail mutex") = Some(error);
    }

    fn fail_next_provider(&self) {
        self.fail_next(DraftStoreError::Provider("boom".to_string()));
    }

    fn fail_next_sender_identity(&self) {
        self.fail_next(DraftStoreError::SenderIdentityUnavailable);
    }

    fn should_fail(&self) -> Option<DraftStoreError> {
        self.fail.lock().expect("fail mutex").take()
    }
}

impl DraftStore for FakeDraftStore {
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Create {
                from: from.to_string(),
                to: draft.to,
                cc: draft.cc,
                bcc: draft.bcc,
                subject: draft.subject,
                body_markdown: draft.body_markdown,
                body_html: draft.body_html,
            });
            Ok("draft-1".to_string())
        })
    }

    fn get<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DraftDetails>, DraftStoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Get {
                draft_id: draft_id.to_string(),
            });
            Ok(Some(
                self.get_response
                    .lock()
                    .expect("get response mutex")
                    .clone()
                    .unwrap_or_else(|| DraftDetails {
                        draft_id: draft_id.to_string(),
                        to: vec!["bob@example.org".to_string()],
                        cc: vec!["carol@example.org".to_string()],
                        bcc: vec!["dana@example.org".to_string()],
                        subject: "Saved draft".to_string(),
                        body_html: "<p>Saved body</p>".to_string(),
                        body_markdown: "Saved body".to_string(),
                    }),
            ))
        })
    }

    fn update<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Update {
                draft_id: draft_id.to_string(),
                to: draft.to,
                cc: draft.cc,
                bcc: draft.bcc,
                subject: draft.subject,
                body_markdown: draft.body_markdown,
                body_html: draft.body_html,
            });
            Ok(())
        })
    }

    fn delete<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(error) = self.should_fail() {
                return Err(error);
            }
            self.calls.lock().expect("calls mutex").push(Call::Delete {
                draft_id: draft_id.to_string(),
            });
            Ok(())
        })
    }
}

fn create_body() -> &'static str {
    r#"{
        "to":["bob@example.org"],
        "cc":["carol@example.org"],
        "subject":"Hello",
        "body_markdown":"Hi Bob"
    }"#
}

fn update_recipients_body(field: &str, recipients: Vec<String>) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert(
        field.to_string(),
        Value::Array(recipients.into_iter().map(Value::String).collect()),
    );
    Value::Object(payload).to_string()
}

fn recipient_list(count: usize) -> Vec<String> {
    (0..count)
        .map(|idx| format!("user{idx}@example.org"))
        .collect()
}

#[tokio::test]
async fn create_requires_auth() {
    let (state, _key) = fixture_state().await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store,
        Method::POST,
        "/api/drafts",
        None,
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_requires_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store,
        Method::POST,
        "/api/drafts",
        Some(&sid),
        false,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_requires_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store,
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        Some(r#"{"subject":"Revised"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_draft_calls_store_and_returns_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert!(body["updated_at"].as_str().is_some());
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "alice@example.org".to_string(),
            to: vec!["bob@example.org".to_string()],
            cc: vec!["carol@example.org".to_string()],
            bcc: vec![],
            subject: "Hello".to_string(),
            body_markdown: "Hi Bob".to_string(),
            body_html: "<p>Hi Bob</p>\n".to_string(),
        }]
    );
}

#[tokio::test]
async fn create_draft_accepts_body_html_and_derives_markdown() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "html-draft@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"subject":"HTML","body_html":"<p>Hello <strong>Bob</strong></p><script>alert(1)</script>"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "html-draft@example.org".to_string(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "HTML".to_string(),
            body_markdown: "Hello Bob".to_string(),
            body_html: "<p>Hello <strong>Bob</strong></p>".to_string(),
        }]
    );
}

#[tokio::test]
async fn create_draft_prefers_body_html_over_legacy_markdown() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "prefer-html-draft@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"subject":"HTML wins","body_html":"<p>HTML draft</p>","body_markdown":"Markdown draft"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "prefer-html-draft@example.org".to_string(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "HTML wins".to_string(),
            body_markdown: "HTML draft".to_string(),
            body_html: "<p>HTML draft</p>".to_string(),
        }]
    );
}

#[tokio::test]
async fn create_draft_accepts_missing_body() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"subject":"Subject-only autosave"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "alice@example.org".to_string(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Subject-only autosave".to_string(),
            body_markdown: "".to_string(),
            body_html: "".to_string(),
        }]
    );
}

#[tokio::test]
async fn create_draft_accepts_empty_body() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"to":[],"cc":[],"bcc":[],"subject":"","body_markdown":"","attachments":[]}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(
        store.calls(),
        vec![Call::Create {
            from: "alice@example.org".to_string(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "".to_string(),
            body_markdown: "".to_string(),
            body_html: "".to_string(),
        }]
    );
}

#[tokio::test]
async fn update_draft_can_clear_body() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"body_html":""}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![Call::Update {
            draft_id: "draft-1".to_string(),
            to: None,
            cc: None,
            bcc: None,
            subject: None,
            body_markdown: Some("".to_string()),
            body_html: Some("".to_string()),
        }]
    );
}

#[tokio::test]
async fn get_draft_returns_empty_body_fields() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::with_get_response(DraftDetails {
        draft_id: "draft-1".to_string(),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "Subject-only autosave".to_string(),
        body_html: "".to_string(),
        body_markdown: "".to_string(),
    }));

    let resp = request(
        state,
        store.clone(),
        Method::GET,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["body_html"], "");
    assert_eq!(body["body_markdown"], "");
}

#[tokio::test]
async fn get_draft_returns_saved_composer_fields() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::GET,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert_eq!(body["to"], serde_json::json!(["bob@example.org"]));
    assert_eq!(body["cc"], serde_json::json!(["carol@example.org"]));
    assert_eq!(body["bcc"], serde_json::json!(["dana@example.org"]));
    assert_eq!(body["subject"], "Saved draft");
    assert_eq!(body["body_html"], "<p>Saved body</p>");
    assert_eq!(body["body_markdown"], "Saved body");
    assert_eq!(
        store.calls(),
        vec![Call::Get {
            draft_id: "draft-1".to_string(),
        }]
    );
}

#[tokio::test]
async fn jmap_get_sanitizes_saved_html_before_returning_draft() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "jmap-html-draft@example.org").await;
    let unsafe_html = r#"<p>Hi <strong>Bob</strong>,</p><p onclick="alert(1)">Quick note <em>about</em> the doc.</p><blockquote><p>Prior context line 1</p><p>Prior context line 2</p></blockquote><a href="javascript:alert(1)">bad</a><img src="http://tracker/p.gif" /><script>alert(1)</script>"#;
    let rfc822 = draft_rfc822(
        "jmap-html-draft@example.org",
        "bob@example.org",
        "carol@example.org",
        "Saved unsafe draft",
        "text/html",
        unsafe_html,
    );
    let state = state_with_cache_mail(
        &state,
        user_id,
        vec![raw_draft_message(
            "draft-malicious-1",
            "jmap-html-draft@example.org",
            vec!["bob@example.org"],
            vec!["carol@example.org"],
            vec![],
            "Saved unsafe draft",
            &rfc822,
        )],
    )
    .await;

    let resp = production_app(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/drafts/draft-malicious-1")
                .header(header::COOKIE, format!("hail_session={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-malicious-1");
    assert_eq!(body["to"], serde_json::json!([]));
    assert_eq!(body["cc"], serde_json::json!([]));
    assert_eq!(body["bcc"], serde_json::json!([]));
    assert_eq!(body["subject"], "Saved unsafe draft");
    assert_eq!(
        body["body_markdown"],
        "Hi Bob,\nQuick note about the doc.\nPrior context line 1\nPrior context line 2\nbad"
    );

    let body_html = body["body_html"].as_str().expect("body_html string");
    assert!(body_html.contains("<strong>Bob</strong>"));
    assert!(body_html.contains("<em>about</em>"));
    assert!(body_html.contains(">bad</a>"));
    assert_text_inside_blockquote(body_html, "Prior context line 1");
    assert_text_inside_blockquote(body_html, "Prior context line 2");
    let lower_html = body_html.to_ascii_lowercase();
    assert!(!lower_html.contains("<script"));
    assert!(!lower_html.contains("onclick"));
    assert!(!lower_html.contains("javascript:"));
    assert!(!lower_html.contains("<img"));
    assert!(!lower_html.contains("http://tracker"));
}

#[tokio::test]
async fn jmap_get_sanitizes_legacy_text_fallback_html_before_returning_draft() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "jmap-text-draft@example.org").await;
    let text_body = "Hello [link](javascript:alert(1)) and <b onclick=\"alert(2)\">raw</b>";
    let rfc822 = draft_rfc822(
        "jmap-text-draft@example.org",
        "bob@example.org",
        "",
        "Text only draft",
        "text/plain",
        text_body,
    );
    let state = state_with_cache_mail(
        &state,
        user_id,
        vec![raw_draft_message(
            "draft-text-only-1",
            "jmap-text-draft@example.org",
            vec!["bob@example.org"],
            vec![],
            vec![],
            "Text only draft",
            &rfc822,
        )],
    )
    .await;

    let resp = production_app(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/drafts/draft-text-only-1")
                .header(header::COOKIE, format!("hail_session={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["body_markdown"], text_body);
    let body_html = body["body_html"].as_str().expect("body_html string");
    assert!(body_html.contains("Hello [link]("));
    assert!(body_html.contains("raw"));
    assert!(!body_html.contains("<b"));
    assert!(!body_html.contains("<script"));
    assert!(!body_html.contains("<a href"));
}

#[tokio::test]
async fn update_draft_calls_store_and_returns_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"to":["erin@example.org"],"cc":["carol@example.org"],"bcc":["blind@example.org"],"subject":"Revised","body_markdown":"new body"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["draft_id"], "draft-1");
    assert!(body["updated_at"].as_str().is_some());
    assert_eq!(
        store.calls(),
        vec![Call::Update {
            draft_id: "draft-1".to_string(),
            to: Some(vec!["erin@example.org".to_string()]),
            cc: Some(vec!["carol@example.org".to_string()]),
            bcc: Some(vec!["blind@example.org".to_string()]),
            subject: Some("Revised".to_string()),
            body_markdown: Some("new body".to_string()),
            body_html: Some("<p>new body</p>\n".to_string()),
        }]
    );
}

#[tokio::test]
async fn update_draft_accepts_body_html_and_derives_markdown() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "update-html-draft@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"body_html":"<p>Updated <em>HTML</em> draft</p><script>alert(1)</script>","body_markdown":"legacy text"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![Call::Update {
            draft_id: "draft-1".to_string(),
            to: None,
            cc: None,
            bcc: None,
            subject: None,
            body_markdown: Some("Updated HTML draft".to_string()),
            body_html: Some("<p>Updated <em>HTML</em> draft</p>".to_string()),
        }]
    );
}

#[tokio::test]
async fn create_update_get_round_trip_body_html() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "roundtrip-draft@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let create = request(
        state.clone(),
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"subject":"HTML","body_html":"<p>Hello <strong>Bob</strong></p><ul><li><p>One</p></li></ul>"}"#),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);

    let update = request(
        state.clone(),
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"body_html":"<p>Updated</p><blockquote><p>Quoted</p></blockquote>"}"#),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    let get = request(
        state,
        store.clone(),
        Method::GET,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    let body = json_body(get).await;
    assert_eq!(body["body_html"], "<p>Saved body</p>");

    assert_eq!(
        store.calls(),
        vec![
            Call::Create {
                from: "roundtrip-draft@example.org".to_string(),
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: "HTML".to_string(),
                body_markdown: "Hello Bob One".to_string(),
                body_html: "<p>Hello <strong>Bob</strong></p><ul><li><p>One</p></li></ul>"
                    .to_string(),
            },
            Call::Update {
                draft_id: "draft-1".to_string(),
                to: None,
                cc: None,
                bcc: None,
                subject: None,
                body_markdown: Some("Updated Quoted".to_string()),
                body_html: Some("<p>Updated</p><blockquote><p>Quoted</p></blockquote>".to_string()),
            },
            Call::Get {
                draft_id: "draft-1".to_string(),
            },
        ]
    );
}

#[tokio::test]
async fn invalid_recipient_returns_400_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(r#"{"to":["not-an-email"],"subject":"x","body_markdown":"y"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid_to");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_attachments_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(
            r#"{"to":["bob@example.org"],"subject":"x","body_markdown":"y","attachments":[{"name":"a.txt"}]}"#,
        ),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "attachments_not_supported");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_attachments_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"subject":"x","attachments":[{"name":"a.txt"}]}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "attachments_not_supported");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_invalid_cc_bcc_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [("cc", "invalid_cc"), ("bcc", "invalid_bcc")] {
        let body = serde_json::json!({
            "to": ["bob@example.org"],
            field: ["not-an-email"],
            "subject": "x",
            "body_markdown": "y",
        })
        .to_string();
        let resp = request(
            state.clone(),
            store.clone(),
            Method::POST,
            "/api/drafts",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_too_many_recipients_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [
        ("to", "too_many_to"),
        ("cc", "too_many_cc"),
        ("bcc", "too_many_bcc"),
    ] {
        let mut body = serde_json::json!({
            "to": ["bob@example.org"],
            "subject": "x",
            "body_markdown": "y",
        });
        body[field] = serde_json::json!(recipient_list(201));
        let body = body.to_string();
        let resp = request(
            state.clone(),
            store.clone(),
            Method::POST,
            "/api/drafts",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_subject_crlf_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "to": ["bob@example.org"],
        "subject": "Hi\r\nBcc: attacker@example.org",
        "body_markdown": "y",
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid_subject");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_subject_crlf_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "subject": "Hi\nInjected: header",
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "invalid_subject");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_body_too_large_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "to": ["bob@example.org"],
        "subject": "x",
        "body_markdown": "x".repeat(1024 * 1024 + 1),
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "body_too_large");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_body_too_large_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let body = serde_json::json!({
        "body_markdown": "x".repeat(1024 * 1024 + 1),
    })
    .to_string();
    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(&body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "body_too_large");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_empty_patch_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "empty_patch");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_invalid_cc_bcc_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [("cc", "invalid_cc"), ("bcc", "invalid_bcc")] {
        let body = update_recipients_body(field, vec!["not-an-email".to_string()]);
        let resp = request(
            state.clone(),
            store.clone(),
            Method::PATCH,
            "/api/drafts/draft-1",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn update_rejects_too_many_cc_bcc_without_store_call() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    for (field, expected_error) in [("cc", "too_many_cc"), ("bcc", "too_many_bcc")] {
        let body = update_recipients_body(field, recipient_list(201));
        let resp = request(
            state.clone(),
            store.clone(),
            Method::PATCH,
            "/api/drafts/draft-1",
            Some(&sid),
            true,
            Some(&body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["error"], expected_error);
    }
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn provider_error_returns_500() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next_provider();

    let resp = request(
        state,
        store,
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "internal");
}

#[tokio::test]
async fn update_not_found_returns_404() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next(DraftStoreError::NotFound);

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/missing-draft",
        Some(&sid),
        true,
        Some(r#"{"subject":"Revised"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        store.calls(),
        Vec::<Call>::new(),
        "fake store returns before recording the mutation"
    );
}

#[tokio::test]
async fn delete_not_found_returns_404() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next(DraftStoreError::NotFound);

    let resp = request(
        state,
        store.clone(),
        Method::DELETE,
        "/api/drafts/missing-draft",
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        store.calls(),
        Vec::<Call>::new(),
        "fake store returns before recording the mutation"
    );
}

#[tokio::test]
async fn delete_draft_requires_csrf_and_deletes_by_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "delete-draft@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state.clone(),
        store.clone(),
        Method::DELETE,
        "/api/drafts/draft-1",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = request(
        state,
        store.clone(),
        Method::DELETE,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        store.calls(),
        vec![Call::Delete {
            draft_id: "draft-1".to_string()
        }]
    );
}

#[tokio::test]
async fn create_returns_clear_400_when_sender_identity_missing() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "missing-draft-identity@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next_sender_identity();

    let resp = request(
        state,
        store.clone(),
        Method::POST,
        "/api/drafts",
        Some(&sid),
        true,
        Some(create_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = json_body(resp).await;
    assert_eq!(body["error"], "sender_identity_unavailable");
    assert!(store.calls().is_empty());
}

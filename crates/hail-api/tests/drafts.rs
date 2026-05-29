use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::IntoResponse;
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::drafts::{
    DraftCreate, DraftDetails, DraftStore, DraftStoreError, DraftUpdate, JmapDraftStore,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use secrecy::SecretString;
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
    let protected = hail_api::routes::drafts::router_with_store(Arc::new(JmapDraftStore)).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Debug)]
struct FakeJmapDraft {
    id: &'static str,
    to: &'static str,
    cc: &'static str,
    bcc: &'static str,
    subject: &'static str,
    html_body: Option<&'static str>,
    text_body: &'static str,
}

async fn start_fake_draft_jmap(draft: FakeJmapDraft) -> (String, tokio::task::JoinHandle<()>) {
    let draft = Arc::new(draft);
    let app = Router::new()
        .route(
            "/.well-known/jmap",
            axum::routing::get(fake_draft_jmap_session),
        )
        .route("/jmap/", axum::routing::post(fake_draft_jmap_api))
        .with_state(draft);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake draft jmap");
    let addr = listener.local_addr().expect("fake draft jmap addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .expect("fake draft jmap server");
    });
    (format!("http://{addr}"), handle)
}

async fn fake_draft_jmap_session(headers: axum::http::HeaderMap) -> impl IntoResponse {
    let base_url = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| format!("http://{host}"))
        .unwrap_or_else(|| "http://127.0.0.1:0".to_owned());
    axum::Json(serde_json::json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": {
                "maxSizeUpload": 50_000_000,
                "maxConcurrentUpload": 4,
                "maxSizeRequest": 10_000_000,
                "maxConcurrentRequests": 4,
                "maxCallsInRequest": 16,
                "maxObjectsInGet": 500,
                "maxObjectsInSet": 500,
                "collationAlgorithms": ["i;unicode-casemap"]
            },
            "urn:ietf:params:jmap:mail": {
                "maxMailboxesPerEmail": 16,
                "maxMailboxDepth": 10,
                "maxSizeMailboxName": 255,
                "maxSizeAttachmentsPerEmail": 50_000_000,
                "emailQuerySortOptions": ["receivedAt"],
                "mayCreateTopLevelMailbox": true
            }
        },
        "accounts": {
            "account-test": {
                "name": "Draft Test Account",
                "isPersonal": true,
                "isReadOnly": false,
                "accountCapabilities": { "urn:ietf:params:jmap:mail": {} }
            }
        },
        "primaryAccounts": { "urn:ietf:params:jmap:mail": "account-test" },
        "username": "drafts@example.org",
        "apiUrl": format!("{base_url}/jmap/"),
        "downloadUrl": format!("{base_url}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
        "uploadUrl": format!("{base_url}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base_url}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": "fake-state"
    }))
}

async fn fake_draft_jmap_api(
    State(draft): State<Arc<FakeJmapDraft>>,
    body: Bytes,
) -> impl IntoResponse {
    let request: Value = serde_json::from_slice(&body).expect("jmap request json");
    let call = request["methodCalls"]
        .as_array()
        .and_then(|calls| calls.first())
        .expect("single jmap method call");
    let method = call[0].as_str().expect("jmap method name");
    let tag = call[2].as_str().unwrap_or("s0");
    assert_eq!(method, "Email/get");

    let ids = call[1]["ids"]
        .as_array()
        .expect("Email/get ids")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let list = ids
        .iter()
        .filter(|id| **id == draft.id)
        .map(|_| draft_email_json(&draft))
        .collect::<Vec<_>>();

    axum::Json(serde_json::json!({
        "sessionState": "fake-state",
        "methodResponses": [[method, {
            "accountId": "account-test",
            "state": "fake-get-state",
            "list": list,
            "notFound": []
        }, tag]]
    }))
}

fn draft_email_json(draft: &FakeJmapDraft) -> Value {
    let mut body_values = serde_json::Map::new();
    body_values.insert(
        "text-1".to_owned(),
        serde_json::json!({ "value": draft.text_body, "isTruncated": false }),
    );
    let mut html_body = Vec::new();
    if let Some(html) = draft.html_body {
        body_values.insert(
            "html-1".to_owned(),
            serde_json::json!({ "value": html, "isTruncated": false }),
        );
        html_body.push(serde_json::json!({ "partId": "html-1", "type": "text/html" }));
    }

    serde_json::json!({
        "id": draft.id,
        "keywords": { "$draft": true },
        "to": [{ "name": "Bob", "email": draft.to }],
        "cc": [{ "name": "Carol", "email": draft.cc }],
        "bcc": [{ "name": "Dana", "email": draft.bcc }],
        "subject": draft.subject,
        "textBody": [{ "partId": "text-1", "type": "text/plain" }],
        "htmlBody": html_body,
        "bodyValues": body_values
    })
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
    let (mut state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "jmap-html-draft@example.org").await;
    let (_jmap_handle_guard, handle) = start_fake_draft_jmap(FakeJmapDraft {
        id: "draft-malicious-1",
        to: "bob@example.org",
        cc: "carol@example.org",
        bcc: "dana@example.org",
        subject: "Saved unsafe draft",
        html_body: Some(
            r#"<p onclick="alert(2)">Hello <strong>Bob</strong></p><script>alert(1)</script><a href="javascript:alert(3)">bad link</a><img src="http://evil">"#,
        ),
        text_body: "Hello Bob text fallback",
    })
    .await;
    state.config.stalwart.jmap_url = _jmap_handle_guard;

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
    assert_eq!(body["to"], serde_json::json!(["bob@example.org"]));
    assert_eq!(body["cc"], serde_json::json!(["carol@example.org"]));
    assert_eq!(body["bcc"], serde_json::json!(["dana@example.org"]));
    assert_eq!(body["subject"], "Saved unsafe draft");
    assert_eq!(body["body_markdown"], "Hello Bob text fallback");

    let body_html = body["body_html"].as_str().expect("body_html string");
    assert!(body_html.contains("Hello <strong>Bob</strong>"));
    assert!(body_html.contains(">bad link</a>"));
    let lower_html = body_html.to_ascii_lowercase();
    assert!(!lower_html.contains("<script"));
    assert!(!lower_html.contains("alert(1)"));
    assert!(!lower_html.contains("onclick"));
    assert!(!lower_html.contains("javascript:"));
    assert!(!lower_html.contains("<img"));
    assert!(!lower_html.contains("http://evil"));

    handle.abort();
}

#[tokio::test]
async fn jmap_get_sanitizes_legacy_text_fallback_html_before_returning_draft() {
    let (mut state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "jmap-text-draft@example.org").await;
    let (_jmap_handle_guard, handle) = start_fake_draft_jmap(FakeJmapDraft {
        id: "draft-text-only-1",
        to: "bob@example.org",
        cc: "carol@example.org",
        bcc: "dana@example.org",
        subject: "Text only draft",
        html_body: None,
        text_body: "Hello [link](javascript:alert(1)) and <b onclick=\"alert(2)\">raw</b>",
    })
    .await;
    state.config.stalwart.jmap_url = _jmap_handle_guard;

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
    assert_eq!(
        body["body_markdown"],
        "Hello [link](javascript:alert(1)) and <b onclick=\"alert(2)\">raw</b>"
    );
    let body_html = body["body_html"].as_str().expect("body_html string");
    assert!(body_html.contains(">link</a>"));
    assert!(body_html.contains("raw"));
    let lower_html = body_html.to_ascii_lowercase();
    assert!(!lower_html.contains("javascript:"));
    assert!(!lower_html.contains("onclick=\""));

    handle.abort();
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

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::drafts::{DraftCreate, DraftStore, DraftStoreError, DraftUpdate};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::Value;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_drafts_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", hex::encode(key));
    }
    let config = Config::load_from(None).expect("load config");

    let state = AppState {
        db,
        config,
        server_key: Arc::new(key),
        login_limiter: Arc::new(IpRateLimiter::default()),
        events: hail_api::events::AppEventBus::default(),
    };
    (state, key)
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

async fn seed_session(state: &AppState, key: &[u8; KEY_LEN], email: &str) -> String {
    let now = Utc::now();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
         VALUES (?1, ?2, 0, ?3) RETURNING id",
    )
    .bind(email)
    .bind(format!("account-{email}"))
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert user");

    let token_enc = hail_core::seal(b"dummy-token", key).expect("seal");
    let session_id = format!("{:064x}", user_id);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&token_enc)
    .bind(Some("test-ua"))
    .bind(now + Duration::days(30))
    .bind(now)
    .execute(&state.db)
    .await
    .expect("insert session");
    session_id
}

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

async fn json_body(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
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
    },
    Update {
        draft_id: String,
        to: Option<Vec<String>>,
        subject: Option<String>,
        body_markdown: Option<String>,
    },
}

#[derive(Default)]
struct FakeDraftStore {
    calls: Mutex<Vec<Call>>,
    fail: Mutex<bool>,
}

impl FakeDraftStore {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }

    fn fail_next(&self) {
        *self.fail.lock().expect("fail mutex") = true;
    }

    fn should_fail(&self) -> bool {
        let mut fail = self.fail.lock().expect("fail mutex");
        let value = *fail;
        *fail = false;
        value
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
            if self.should_fail() {
                return Err(DraftStoreError::Provider("boom".to_string()));
            }
            self.calls.lock().expect("calls mutex").push(Call::Create {
                from: from.to_string(),
                to: draft.to,
                cc: draft.cc,
                bcc: draft.bcc,
                subject: draft.subject,
                body_markdown: draft.body_markdown,
            });
            Ok("draft-1".to_string())
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
            if self.should_fail() {
                return Err(DraftStoreError::Provider("boom".to_string()));
            }
            self.calls.lock().expect("calls mutex").push(Call::Update {
                draft_id: draft_id.to_string(),
                to: draft.to,
                subject: draft.subject,
                body_markdown: draft.body_markdown,
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
    let sid = seed_session(&state, &key, "alice@example.org").await;
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
    let sid = seed_session(&state, &key, "alice@example.org").await;
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
    let sid = seed_session(&state, &key, "alice@example.org").await;
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
        }]
    );
}

#[tokio::test]
async fn update_draft_calls_store_and_returns_id() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());

    let resp = request(
        state,
        store.clone(),
        Method::PATCH,
        "/api/drafts/draft-1",
        Some(&sid),
        true,
        Some(r#"{"subject":"Revised","body_markdown":"new body"}"#),
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
            to: None,
            subject: Some("Revised".to_string()),
            body_markdown: Some("new body".to_string()),
        }]
    );
}

#[tokio::test]
async fn invalid_recipient_returns_400_without_store_call() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
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
async fn provider_error_returns_500() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let store = Arc::new(FakeDraftStore::default());
    store.fail_next();

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

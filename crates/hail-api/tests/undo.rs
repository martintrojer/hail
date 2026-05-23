use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::undo::{UndoActionPayload, UndoError, UndoExecutor, create_undo_action};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_undo_test_{uniq}?mode=memory&cache=shared");
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

async fn seed_session(state: &AppState, key: &[u8; KEY_LEN], email: &str) -> (i64, String) {
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
    (user_id, session_id)
}

fn app(state: AppState, executor: Arc<FakeExecutor>) -> Router {
    let protected = hail_api::routes::undo::router_with_executor(executor).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn post_undo(
    state: AppState,
    executor: Arc<FakeExecutor>,
    sid: Option<&str>,
    csrf: bool,
    id: &str,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/undo/{id}"));
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    let req = builder.body(Body::empty()).unwrap();
    app(state, executor).oneshot(req).await.unwrap()
}

#[derive(Default)]
struct FakeExecutor {
    calls: Mutex<Vec<UndoActionPayload>>,
}

impl FakeExecutor {
    fn calls(&self) -> Vec<UndoActionPayload> {
        self.calls.lock().expect("calls mutex").clone()
    }
}

#[async_trait]
impl UndoExecutor for FakeExecutor {
    async fn execute(
        &self,
        _state: &AppState,
        _user: &hail_api::middleware::auth::AuthUser,
        undo: UndoActionPayload,
    ) -> Result<(), UndoError> {
        self.calls.lock().expect("calls mutex").push(undo);
        Ok(())
    }
}

#[tokio::test]
async fn create_and_execute_undo_action() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "undo@example.org").await;
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.trash",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await
    .unwrap();
    let executor = Arc::new(FakeExecutor::default());

    let resp = post_undo(state.clone(), executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["id"], undo.id);
    assert_eq!(json["action"], "thread.trash");
    assert_eq!(
        executor.calls(),
        vec![UndoActionPayload {
            action: "thread.trash".to_string(),
            payload: serde_json::json!({ "thread_id": "thread-1" }),
        }]
    );
}

#[tokio::test]
async fn expired_undo_returns_410_and_does_not_execute() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "expired@example.org").await;
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.trash",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE undo_actions SET expires_at = ?1 WHERE id = ?2")
        .bind(Utc::now() - Duration::seconds(1))
        .bind(&undo.id)
        .execute(&state.db)
        .await
        .unwrap();
    let executor = Arc::new(FakeExecutor::default());

    let resp = post_undo(state, executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::GONE);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn wrong_user_cannot_execute_undo_action() {
    let (state, key) = fixture_state().await;
    let (alice_id, _alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (_bob_id, bob_sid) = seed_session(&state, &key, "bob@example.org").await;
    let undo = create_undo_action(
        &state,
        alice_id,
        "thread.trash",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await
    .unwrap();
    let executor = Arc::new(FakeExecutor::default());

    let resp = post_undo(state, executor.clone(), Some(&bob_sid), true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn undo_action_can_only_be_used_once() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "once@example.org").await;
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.trash",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await
    .unwrap();
    let executor = Arc::new(FakeExecutor::default());

    let first = post_undo(state.clone(), executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = post_undo(state, executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(second.status(), StatusCode::GONE);
    assert_eq!(executor.calls().len(), 1);
}

#[tokio::test]
async fn missing_csrf_returns_403() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "csrf@example.org").await;
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.trash",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state,
        Arc::new(FakeExecutor::default()),
        Some(&sid),
        false,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

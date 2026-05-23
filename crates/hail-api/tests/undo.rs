use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, Duration, Utc};
use hail_api::middleware::auth::{AuthUser, CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::undo::{
    ActionUndoExecutor, EmailMailboxSnapshot, ThreadUndoRestorer, UndoActionPayload, UndoError,
    UndoExecutor, create_undo_action,
};
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

fn app<E>(state: AppState, executor: Arc<E>) -> Router
where
    E: UndoExecutor,
{
    let protected = hail_api::routes::undo::router_with_executor(executor).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn post_undo<E>(
    state: AppState,
    executor: Arc<E>,

    sid: Option<&str>,
    csrf: bool,
    id: &str,
) -> axum::response::Response
where
    E: UndoExecutor,
{
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
        _user: &AuthUser,

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
async fn thread_stack_undo_removes_new_stack_row_and_keyword() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "stack-undo@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
         VALUES (?1, 'set_aside', 'thread-1', 3, ?2)",
    )
    .bind(user_id)
    .bind(Utc::now())
    .execute(&state.db)
    .await
    .unwrap();
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.stack",
        serde_json::json!({
            "thread_id": "thread-1",
            "stack": "set_aside",
            "keyword": "$hail_setaside",
            "previous_position": null,
        }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state.clone(),
        Arc::new(ActionUndoExecutor::new(restorer.clone())),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND stack = 'set_aside' AND thread_id = 'thread-1'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        restorer.keyword_calls(),
        vec![("thread-1".to_string(), "$hail_setaside".to_string(), false,)]
    );
}

#[tokio::test]
async fn thread_stack_undo_restores_existing_stack_position_without_clearing_keyword() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "stack-restore@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let original_added_at = Utc::now() - Duration::minutes(10);
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
         VALUES (?1, 'reply_later', 'thread-2', 9, ?2)",
    )
    .bind(user_id)
    .bind(Utc::now())
    .execute(&state.db)
    .await
    .unwrap();
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.stack",
        serde_json::json!({
            "thread_id": "thread-2",
            "stack": "reply_later",
            "keyword": "$hail_replylater",
            "previous_position": {
                "position": 4,
                "added_at": original_added_at,
            },
        }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state.clone(),
        Arc::new(ActionUndoExecutor::new(restorer.clone())),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let row: (i64, DateTime<Utc>) = sqlx::query_as(
        "SELECT position, added_at FROM stack_positions WHERE user_id = ?1 AND stack = 'reply_later' AND thread_id = 'thread-2'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row, (4, original_added_at));
    assert!(restorer.keyword_calls().is_empty());
}

#[tokio::test]
async fn malformed_thread_stack_undo_returns_400() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "bad-stack-undo@example.org").await;
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.stack",
        serde_json::json!({
            "thread_id": "thread-1",
            "stack": "set_aside",
            "keyword": "$hail_replylater",
            "previous_position": null,
        }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state,
        Arc::new(ActionUndoExecutor::new(Arc::new(
            FakeThreadRestorer::default(),
        ))),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn malformed_classify_undo_returns_400() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "badpayload@example.org").await;
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.classify",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state,
        Arc::new(ActionUndoExecutor::new(Arc::new(
            FakeThreadRestorer::default(),
        ))),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn classify_undo_restores_previous_classification() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-undo@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let undo = create_undo_action(
        &state,
        user_id,
        "thread.classify",
        serde_json::json!({
            "thread_id": "thread-1",
            "previous_classification": "feed"
        }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state,
        Arc::new(ActionUndoExecutor::new(restorer.clone())),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        restorer
            .classify_calls
            .lock()
            .expect("classify calls mutex")
            .clone(),
        vec![("thread-1".to_string(), "feed".to_string())]
    );
}

#[tokio::test]
async fn screener_decision_undo_restores_previous_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "screener-undo@example.org").await;
    let first_seen = Utc::now() - Duration::days(2);
    let decided = Utc::now() - Duration::days(1);
    sqlx::query(
        "INSERT INTO screener_rules (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?1, 'sender@example.org', 'deny', NULL, ?2, ?3)",
    )
    .bind(user_id)
    .bind(Utc::now())
    .bind(Utc::now())
    .execute(&state.db)
    .await
    .unwrap();
    let undo = create_undo_action(
        &state,
        user_id,
        "screener.decision",
        serde_json::json!({
            "sender": "sender@example.org",
            "previous_rule": {
                "decision": "allow",
                "classify_as": "papertrail",
                "decided_at": decided,
                "first_seen_at": first_seen,
            }
        }),
    )
    .await
    .unwrap();

    let resp = post_undo(
        state.clone(),
        Arc::new(ActionUndoExecutor::new(Arc::new(
            FakeThreadRestorer::default(),
        ))),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let row: (String, Option<String>, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "SELECT decision, classify_as, decided_at, first_seen_at FROM screener_rules \
         WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("papertrail"));
    assert_eq!(row.2, decided);
    assert_eq!(row.3, first_seen);
}

#[derive(Default)]
struct FakeThreadRestorer {
    classify_calls: Mutex<Vec<(String, String)>>,
    keyword_calls: Mutex<Vec<(String, String, bool)>>,
}

impl FakeThreadRestorer {
    fn keyword_calls(&self) -> Vec<(String, String, bool)> {
        self.keyword_calls
            .lock()
            .expect("keyword calls mutex")
            .clone()
    }
}

#[async_trait]
impl ThreadUndoRestorer for FakeThreadRestorer {
    async fn restore_classification(
        &self,
        _state: &AppState,
        _user: &AuthUser,
        thread_id: &str,
        previous_classification: &str,
    ) -> Result<(), UndoError> {
        self.classify_calls
            .lock()
            .expect("classify calls mutex")
            .push((thread_id.to_string(), previous_classification.to_string()));
        Ok(())
    }

    async fn restore_mailboxes(
        &self,
        _state: &AppState,
        _user: &AuthUser,
        _snapshots: Vec<EmailMailboxSnapshot>,
    ) -> Result<(), UndoError> {
        Ok(())
    }

    async fn set_keyword(
        &self,
        _state: &AppState,
        _user: &AuthUser,
        thread_id: &str,
        keyword: &str,
        enabled: bool,
    ) -> Result<(), UndoError> {
        self.keyword_calls
            .lock()
            .expect("keyword calls mutex")
            .push((thread_id.to_string(), keyword.to_string(), enabled));
        Ok(())
    }
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

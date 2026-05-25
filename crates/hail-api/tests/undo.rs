use chrono::{DateTime, Duration, Utc};
use http_body_util::BodyExt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::{AuthUser, CSRF_HEADER, require_auth};
use hail_api::routes::undo::{
    ActionUndoExecutor, EmailMailboxSnapshot, NewUndoAction, ThreadStackUndoTarget,
    ThreadUndoRestorer, UndoActionPayload, UndoError, UndoExecutor, UndoToken, create_undo_action,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, seed_session};
use tower::ServiceExt;

async fn insert_raw_undo_action(
    state: &AppState,
    user_id: i64,
    action: &str,
    payload: serde_json::Value,
) -> UndoToken {
    let now = Utc::now();
    let expires_at = now + Duration::seconds(10);
    let id = format!(
        "{:064x}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default() as u128
    );
    sqlx::query(
        "INSERT INTO undo_actions (id, user_id, action, payload_json, expires_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(action)
    .bind(serde_json::to_string(&payload).expect("serialize raw undo payload"))
    .bind(expires_at)
    .bind(now)
    .execute(&state.db)
    .await
    .expect("insert raw undo action");
    UndoToken {
        id,
        action: action.to_string(),
        expires_at,
    }
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

#[derive(Clone, Copy)]
enum ExecutorFailure {
    BadRequest,
    NotImplemented,
    Internal,
}

struct FailingExecutor {
    failure: ExecutorFailure,
    calls: Mutex<usize>,
}

impl FailingExecutor {
    fn new(failure: ExecutorFailure) -> Self {
        Self {
            failure,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("calls mutex")
    }
}

#[async_trait]
impl UndoExecutor for FailingExecutor {
    async fn execute(
        &self,
        _state: &AppState,
        _user: &AuthUser,

        _undo: UndoActionPayload,
    ) -> Result<(), UndoError> {
        *self.calls.lock().expect("calls mutex") += 1;
        match self.failure {
            ExecutorFailure::BadRequest => Err(UndoError::bad_request("bad undo")),
            ExecutorFailure::NotImplemented => Err(UndoError::not_implemented("unsupported undo")),
            ExecutorFailure::Internal => Err(UndoError::internal("executor failed")),
        }
    }
}

#[tokio::test]
async fn create_and_execute_undo_action() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "undo@example.org").await;
    let undo = create_undo_action(&state, user_id, NewUndoAction::thread_trash(Vec::new()))
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
            payload: serde_json::json!({ "email_mailbox_ids": [] }),
        }]
    );
}

#[tokio::test]
async fn expired_undo_returns_410_and_does_not_execute() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "expired@example.org").await;
    let undo = create_undo_action(&state, user_id, NewUndoAction::thread_trash(Vec::new()))
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
    let undo = create_undo_action(&state, alice_id, NewUndoAction::thread_trash(Vec::new()))
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
    let undo = create_undo_action(&state, user_id, NewUndoAction::thread_trash(Vec::new()))
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
        NewUndoAction::thread_stack(
            "thread-1",
            ThreadStackUndoTarget::SetAside,
            None::<serde_json::Value>,
        ),
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
        NewUndoAction::thread_stack(
            "thread-2",
            ThreadStackUndoTarget::ReplyLater,
            Some(serde_json::json!({
                "position": 4,
                "added_at": original_added_at,
            })),
        ),
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
    let undo = insert_raw_undo_action(
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
    .await;

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
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "thread.classify",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await;

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
        NewUndoAction::thread_classify("thread-1", "feed", "papertrail"),
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
async fn classify_undo_rejects_missing_new_classification_payload() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-missing-new@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "thread.classify",
        serde_json::json!({
            "thread_id": "thread-1",
            "previous_classification": "feed"
        }),
    )
    .await;

    let resp = post_undo(
        state,
        Arc::new(ActionUndoExecutor::new(restorer.clone())),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        restorer
            .classify_calls
            .lock()
            .expect("classify calls mutex")
            .is_empty()
    );
}

#[tokio::test]
async fn classify_undo_rejects_missing_previous_classification_payload() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-missing-previous@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "thread.classify",
        serde_json::json!({
            "thread_id": "thread-1",
            "new_classification": "feed"
        }),
    )
    .await;

    let resp = post_undo(
        state,
        Arc::new(ActionUndoExecutor::new(restorer.clone())),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        restorer
            .classify_calls
            .lock()
            .expect("classify calls mutex")
            .is_empty()
    );
}

#[tokio::test]
async fn classify_undo_rejects_noop_classification_payload() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-noop-undo@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let undo = create_undo_action(
        &state,
        user_id,
        NewUndoAction::thread_classify("thread-1", "feed", "feed"),
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
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        restorer
            .classify_calls
            .lock()
            .expect("classify calls mutex")
            .is_empty()
    );
}

#[tokio::test]
async fn thread_archive_undo_restores_mailbox_snapshots() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "archive-move-undo@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let expected_snapshots = vec![
        EmailMailboxSnapshot {
            email_id: "email-1".to_string(),
            mailbox_ids: vec!["inbox".to_string()],
        },
        EmailMailboxSnapshot {
            email_id: "email-2".to_string(),
            mailbox_ids: vec!["inbox".to_string(), "custom".to_string()],
        },
    ];
    let undo = create_undo_action(
        &state,
        user_id,
        NewUndoAction::thread_archive(expected_snapshots.clone()),
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
    assert_eq!(restorer.mailbox_calls(), vec![expected_snapshots]);
}

#[tokio::test]
async fn thread_trash_undo_restores_mailbox_snapshots() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "trash-move-undo@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let expected_snapshots = vec![EmailMailboxSnapshot {
        email_id: "email-1".to_string(),
        mailbox_ids: vec!["inbox".to_string(), "custom".to_string()],
    }];
    let undo = create_undo_action(
        &state,
        user_id,
        NewUndoAction::thread_trash(expected_snapshots.clone()),
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
    assert_eq!(restorer.mailbox_calls(), vec![expected_snapshots]);
}

#[tokio::test]
async fn thread_move_undo_without_snapshots_returns_501() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "missing-snapshot-undo@example.org").await;
    let restorer = Arc::new(FakeThreadRestorer::default());
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "thread.archive",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await;

    let resp = post_undo(
        state,
        Arc::new(ActionUndoExecutor::new(restorer.clone())),
        Some(&sid),
        true,
        &undo.id,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    assert!(restorer.mailbox_calls().is_empty());
}

#[tokio::test]
async fn unsupported_action_returns_501() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "unsupported-undo@example.org").await;
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "thread.snooze",
        serde_json::json!({ "thread_id": "thread-1" }),
    )
    .await;

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
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn executor_bad_request_failure_returns_400() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "executor-bad-request@example.org").await;
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "test.action",
        serde_json::json!({ "ok": true }),
    )
    .await;
    let executor = Arc::new(FailingExecutor::new(ExecutorFailure::BadRequest));

    let resp = post_undo(state, executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(executor.calls(), 1);
}

#[tokio::test]
async fn executor_not_implemented_failure_returns_501() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "executor-not-implemented@example.org").await;
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "test.action",
        serde_json::json!({ "ok": true }),
    )
    .await;
    let executor = Arc::new(FailingExecutor::new(ExecutorFailure::NotImplemented));

    let resp = post_undo(state, executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(executor.calls(), 1);
}

#[tokio::test]
async fn executor_internal_failure_returns_500() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "executor-internal@example.org").await;
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "test.action",
        serde_json::json!({ "ok": true }),
    )
    .await;
    let executor = Arc::new(FailingExecutor::new(ExecutorFailure::Internal));

    let resp = post_undo(state, executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(executor.calls(), 1);
}

#[tokio::test]
async fn executor_failure_consumes_undo_token() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "executor-consume@example.org").await;
    let undo = insert_raw_undo_action(
        &state,
        user_id,
        "test.action",
        serde_json::json!({ "ok": true }),
    )
    .await;
    let executor = Arc::new(FailingExecutor::new(ExecutorFailure::Internal));

    let first = post_undo(state.clone(), executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(first.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let second = post_undo(state, executor.clone(), Some(&sid), true, &undo.id).await;
    assert_eq!(second.status(), StatusCode::GONE);
    assert_eq!(executor.calls(), 1);
}

#[tokio::test]
async fn invalid_undo_id_returns_404_without_executing() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "invalid-id-undo@example.org").await;
    let executor = Arc::new(FakeExecutor::default());

    let resp = post_undo(state, executor.clone(), Some(&sid), true, "not-an-undo-id").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn no_auth_returns_401_for_well_formed_undo_id() {
    let (state, key) = fixture_state().await;
    let (user_id, _sid) = seed_session(&state, &key, "no-auth-undo@example.org").await;
    let undo = create_undo_action(&state, user_id, NewUndoAction::thread_trash(Vec::new()))
        .await
        .unwrap();
    let executor = Arc::new(FakeExecutor::default());

    let resp = post_undo(state, executor.clone(), None, true, &undo.id).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(executor.calls().is_empty());
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
        NewUndoAction::screener_decision(
            "sender@example.org",
            Some(&serde_json::json!({
                "decision": "allow",
                "classify_as": "papertrail",
                "decided_at": decided,
                "first_seen_at": first_seen,
            })),
        ),
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
    mailbox_calls: Mutex<Vec<Vec<EmailMailboxSnapshot>>>,
    keyword_calls: Mutex<Vec<(String, String, bool)>>,
}

impl FakeThreadRestorer {
    fn mailbox_calls(&self) -> Vec<Vec<EmailMailboxSnapshot>> {
        self.mailbox_calls
            .lock()
            .expect("mailbox calls mutex")
            .clone()
    }

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
        snapshots: Vec<EmailMailboxSnapshot>,
    ) -> Result<(), UndoError> {
        self.mailbox_calls
            .lock()
            .expect("mailbox calls mutex")
            .push(snapshots);
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
    let undo = create_undo_action(&state, user_id, NewUndoAction::thread_trash(Vec::new()))
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

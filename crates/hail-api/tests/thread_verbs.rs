use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::threads::{
    Classification, ThreadActionError, ThreadActions, ThreadVerifier, ThreadVerifyError,
};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_thread_verbs_test_{uniq}?mode=memory&cache=shared");
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

fn app(state: AppState, actions: Arc<FakeActions>) -> Router {
    let protected = hail_api::routes::threads::router_with_deps(
        Arc::new(FakeVerifier { exists: true }),
        actions,
    )
    .layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));
    Router::new().merge(protected).with_state(state)
}

async fn post(
    state: AppState,
    actions: Arc<FakeActions>,
    sid: Option<&str>,
    csrf: bool,
    path: &str,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(Method::POST).uri(path);
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
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    app(state, actions).oneshot(req).await.unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

struct FakeVerifier {
    exists: bool,
}

impl ThreadVerifier for FakeVerifier {
    fn exists<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>> {
        Box::pin(async move { Ok(self.exists) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Classify {
        thread_id: String,
        classification: Classification,
    },
    AddKeyword {
        thread_id: String,
        keyword: String,
    },
    Archive {
        thread_id: String,
    },
    Trash {
        thread_id: String,
    },
    Mark {
        thread_id: String,
        read: bool,
    },
}

#[derive(Default)]
struct FakeActions {
    calls: Mutex<Vec<Call>>,
    missing: Mutex<Vec<String>>,
    current_classification: Mutex<Option<Classification>>,
}

impl FakeActions {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }

    fn mark_missing(&self, thread_id: &str) {
        self.missing
            .lock()
            .expect("missing mutex")
            .push(thread_id.to_string());
    }

    fn maybe_missing(&self, thread_id: &str) -> Result<(), ThreadActionError> {
        if self
            .missing
            .lock()
            .expect("missing mutex")
            .iter()
            .any(|id| id == thread_id)
        {
            Err(ThreadActionError::NotFound)
        } else {
            Ok(())
        }
    }
}

impl ThreadActions for FakeActions {
    fn current_classification<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Classification>, ThreadActionError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.maybe_missing(thread_id)?;
            Ok(*self
                .current_classification
                .lock()
                .expect("current classification mutex"))
        })
    }

    fn classify<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
        classification: Classification,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_missing(thread_id)?;
            self.calls
                .lock()
                .expect("calls mutex")
                .push(Call::Classify {
                    thread_id: thread_id.to_string(),
                    classification,
                });
            Ok(())
        })
    }

    fn add_keyword<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_missing(thread_id)?;
            self.calls
                .lock()
                .expect("calls mutex")
                .push(Call::AddKeyword {
                    thread_id: thread_id.to_string(),
                    keyword: keyword.to_string(),
                });
            Ok(())
        })
    }

    fn archive<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Archive {
                thread_id: thread_id.to_string(),
            });
            Ok(())
        })
    }

    fn trash<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Trash {
                thread_id: thread_id.to_string(),
            });
            Ok(())
        })
    }

    fn mark<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
        read: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Mark {
                thread_id: thread_id.to_string(),
                read,
            });
            Ok(())
        })
    }
}

#[tokio::test]
async fn auth_required_for_each_thread_verb_endpoint() {
    let (state, _key) = fixture_state().await;

    for (path, body) in verb_requests() {
        let resp = post(
            state.clone(),
            Arc::new(FakeActions::default()),
            None,
            true,
            path,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[tokio::test]
async fn csrf_required_for_each_thread_verb_endpoint() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "csrf@example.org").await;

    for (path, body) in verb_requests() {
        let resp = post(
            state.clone(),
            Arc::new(FakeActions::default()),
            Some(&sid),
            false,
            path,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

fn verb_requests() -> [(&'static str, Option<&'static str>); 6] {
    [
        ("/api/threads/thread-1/classify", Some(r#"{"to":"imbox"}"#)),
        ("/api/threads/thread-1/set-aside", None),
        ("/api/threads/thread-1/reply-later", None),
        ("/api/threads/thread-1/archive", None),
        ("/api/threads/thread-1/trash", None),
        ("/api/threads/thread-1/mark", Some(r#"{"read":true}"#)),
    ]
}

#[tokio::test]
async fn classify_calls_action_and_rejects_invalid_classification() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "classify@example.org").await;
    let actions = Arc::new(FakeActions::default());

    let resp = post(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-1/classify",
        Some(r#"{"to":"feed"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["undo"].is_null());
    assert_eq!(
        actions.calls(),
        vec![Call::Classify {
            thread_id: "thread-1".to_string(),
            classification: Classification::Feed,
        }]
    );

    let bad = post(
        state,
        actions,
        Some(&sid),
        true,
        "/api/threads/thread-1/classify",
        Some(r#"{"to":"spam"}"#),
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn classify_creates_undo_with_previous_and_new_classification() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-undo-payload@example.org").await;
    let actions = Arc::new(FakeActions::default());
    *actions
        .current_classification
        .lock()
        .expect("current classification mutex") = Some(Classification::Imbox);

    let resp = post(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-1/classify",
        Some(r#"{"to":"feed"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["undo"]["action"], "thread.classify");
    let undo_id = json["undo"]["id"].as_str().expect("undo id");
    assert_eq!(undo_id.len(), 64);

    let (action, payload_json): (String, String) = sqlx::query_as(
        "SELECT action, payload_json FROM undo_actions WHERE id = ?1 AND user_id = ?2",
    )
    .bind(undo_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(action, "thread.classify");
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
    assert_eq!(payload["thread_id"], "thread-1");
    assert_eq!(payload["previous_classification"], "imbox");
    assert_eq!(payload["new_classification"], "feed");
    assert_eq!(
        actions.calls(),
        vec![Call::Classify {
            thread_id: "thread-1".to_string(),
            classification: Classification::Feed,
        }]
    );
}

#[tokio::test]
async fn classify_without_previous_classification_has_no_undo() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-no-previous@example.org").await;
    let actions = Arc::new(FakeActions::default());

    let resp = post(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-1/classify",
        Some(r#"{"to":"papertrail"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["undo"].is_null());

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM undo_actions WHERE user_id = ?1 AND action = 'thread.classify'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        actions.calls(),
        vec![Call::Classify {
            thread_id: "thread-1".to_string(),
            classification: Classification::Papertrail,
        }]
    );
}

#[tokio::test]
async fn classify_same_classification_is_noop_for_undo() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-idempotent@example.org").await;
    let actions = Arc::new(FakeActions::default());
    *actions
        .current_classification
        .lock()
        .expect("current classification mutex") = Some(Classification::Feed);

    let resp = post(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-1/classify",
        Some(r#"{"to":"feed"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["undo"].is_null());

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM undo_actions WHERE user_id = ?1 AND action = 'thread.classify'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        actions.calls(),
        vec![Call::Classify {
            thread_id: "thread-1".to_string(),
            classification: Classification::Feed,
        }]
    );
}

#[tokio::test]
async fn set_aside_inserts_and_updates_current_users_stack_position() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _bob_sid) = seed_session(&state, &key, "bob@example.org").await;
    let actions = Arc::new(FakeActions::default());
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) VALUES (?1, 'set_aside', 'existing', 7, ?2)",
    )
    .bind(alice_id)
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) VALUES (?1, 'set_aside', 'shared', 99, ?2)",
    )
    .bind(bob_id)
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();

    let first = post(
        state.clone(),
        actions.clone(),
        Some(&alice_sid),
        true,
        "/api/threads/shared/set-aside",
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_json = json_body(first).await;
    assert_eq!(first_json["undo"]["action"], "thread.stack");
    assert!(first_json["undo"]["id"].as_str().is_some());
    assert_eq!(first_json["undo"]["id"].as_str().unwrap().len(), 64);
    let undo_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM undo_actions WHERE id = ?1 AND user_id = ?2 AND action = 'thread.stack'",
    )
    .bind(first_json["undo"]["id"].as_str().unwrap())
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    let undo_payload: serde_json::Value = serde_json::from_str(&undo_payload).unwrap();
    assert_eq!(undo_payload["thread_id"], "shared");
    assert_eq!(undo_payload["stack"], "set_aside");
    assert_eq!(undo_payload["keyword"], "$hail_setaside");
    assert!(undo_payload["previous_position"].is_null());
    let second = post(
        state.clone(),
        actions.clone(),
        Some(&alice_sid),
        true,
        "/api/threads/shared/set-aside",
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_json = json_body(second).await;
    let undo_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM undo_actions WHERE id = ?1 AND user_id = ?2 AND action = 'thread.stack'",
    )
    .bind(second_json["undo"]["id"].as_str().unwrap())
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    let undo_payload: serde_json::Value = serde_json::from_str(&undo_payload).unwrap();
    assert_eq!(undo_payload["previous_position"]["position"], 8);

    let alice_row: (i64,) = sqlx::query_as(
        "SELECT position FROM stack_positions WHERE user_id = ?1 AND stack = 'set_aside' AND thread_id = 'shared'",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    let bob_row: (i64,) = sqlx::query_as(
        "SELECT position FROM stack_positions WHERE user_id = ?1 AND stack = 'set_aside' AND thread_id = 'shared'",
    )
    .bind(bob_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(alice_row.0, 8);
    assert_eq!(bob_row.0, 99);
    assert_eq!(
        actions.calls(),
        vec![
            Call::AddKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_setaside".to_string()
            },
            Call::AddKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_setaside".to_string()
            },
        ]
    );
}

#[tokio::test]
async fn reply_later_inserts_stack_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "reply@example.org").await;
    let actions = Arc::new(FakeActions::default());

    let resp = post(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-2/reply-later",
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["undo"]["action"], "thread.stack");
    assert!(json["undo"]["id"].as_str().is_some());

    let undo_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM undo_actions WHERE id = ?1 AND user_id = ?2 AND action = 'thread.stack'",
    )
    .bind(json["undo"]["id"].as_str().unwrap())
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    let undo_payload: serde_json::Value = serde_json::from_str(&undo_payload).unwrap();
    assert_eq!(undo_payload["thread_id"], "thread-2");
    assert_eq!(undo_payload["stack"], "reply_later");
    assert_eq!(undo_payload["keyword"], "$hail_replylater");
    assert!(undo_payload["previous_position"].is_null());

    let row: (String, i64) = sqlx::query_as(
        "SELECT thread_id, position FROM stack_positions WHERE user_id = ?1 AND stack = 'reply_later'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row, ("thread-2".to_string(), 1));
    assert_eq!(
        actions.calls(),
        vec![Call::AddKeyword {
            thread_id: "thread-2".to_string(),
            keyword: "$hail_replylater".to_string()
        }]
    );
}

#[tokio::test]
async fn archive_trash_and_mark_call_actions() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "verbs@example.org").await;
    let actions = Arc::new(FakeActions::default());

    for (path, body, expected) in [
        ("/api/threads/thread-3/archive", None, StatusCode::OK),
        ("/api/threads/thread-3/trash", None, StatusCode::OK),
        (
            "/api/threads/thread-3/mark",
            Some(r#"{"read":true}"#),
            StatusCode::NO_CONTENT,
        ),
        (
            "/api/threads/thread-3/mark",
            Some(r#"{"read":false}"#),
            StatusCode::NO_CONTENT,
        ),
    ] {
        let resp = post(state.clone(), actions.clone(), Some(&sid), true, path, body).await;
        assert_eq!(resp.status(), expected);
    }

    assert_eq!(
        actions.calls(),
        vec![
            Call::Archive {
                thread_id: "thread-3".to_string()
            },
            Call::Trash {
                thread_id: "thread-3".to_string()
            },
            Call::Mark {
                thread_id: "thread-3".to_string(),
                read: true
            },
            Call::Mark {
                thread_id: "thread-3".to_string(),
                read: false
            },
        ]
    );
}

#[tokio::test]
async fn invalid_thread_id_returns_400() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "badid@example.org").await;
    let resp = post(
        state,
        Arc::new(FakeActions::default()),
        Some(&sid),
        true,
        "/api/threads/bad_id/archive",
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn provider_missing_thread_returns_404_and_does_not_insert_stack_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "missing@example.org").await;
    let actions = Arc::new(FakeActions::default());
    actions.mark_missing("missing-thread");

    let resp = post(
        state.clone(),
        actions,
        Some(&sid),
        true,
        "/api/threads/missing-thread/set-aside",
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND thread_id = 'missing-thread'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

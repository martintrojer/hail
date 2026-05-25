use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::threads::{
    Classification, ThreadActionError, ThreadActions, ThreadVerifier, ThreadVerifyError,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, seed_session};
use secrecy::SecretString;
use tower::ServiceExt;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    RemoveKeyword { thread_id: String, keyword: String },
}

#[derive(Default)]
struct FakeActions {
    calls: std::sync::Mutex<Vec<Call>>,
}

impl FakeActions {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }
}

impl ThreadActions for FakeActions {
    fn current_classification<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Classification>, ThreadActionError>> + Send + 'a>>
    {
        Box::pin(async { Ok(None) })
    }

    fn classify<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
        _classification: Classification,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn set_keyword<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
        enabled: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            if !enabled {
                self.calls
                    .lock()
                    .expect("calls mutex")
                    .push(Call::RemoveKeyword {
                        thread_id: thread_id.to_string(),
                        keyword: keyword.to_string(),
                    });
            }
            Ok(())
        })
    }

    fn archive<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn trash<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn restore<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn destroy<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn mark<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
        _read: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

fn app(state: AppState, verifier: Arc<FakeVerifier>, actions: Arc<FakeActions>) -> Router {
    let protected = hail_api::routes::threads::router_with_deps(verifier, actions).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
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

#[tokio::test]
async fn valid_request_inserts_bubble_up() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let db = state.db.clone();
    let actions = Arc::new(FakeActions::default());
    let surface_at = Utc::now() + Duration::minutes(10);
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
         VALUES (?1, 'set_aside', 'thread-123', 1, ?2)",
    )
    .bind(user_id)
    .bind(Utc::now())
    .execute(&db)
    .await
    .unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/threads/thread-123/bubble-up")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            r#"{{"at":"{}"}}"#,
            surface_at.to_rfc3339()
        )))
        .unwrap();

    let resp = app(
        state,
        Arc::new(FakeVerifier { exists: true }),
        actions.clone(),
    )
    .oneshot(req)
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["bubble_id"], 1);
    assert_eq!(
        json["surface_at"],
        surface_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
    );

    let (thread_id, stored_at): (String, String) =
        sqlx::query_as("SELECT thread_id, surface_at FROM bubble_ups WHERE id = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(thread_id, "thread-123");
    assert_eq!(stored_at, surface_at.to_rfc3339());
    assert_eq!(
        actions.calls(),
        vec![
            Call::RemoveKeyword {
                thread_id: "thread-123".to_string(),
                keyword: "$hail_imbox".to_string(),
            },
            Call::RemoveKeyword {
                thread_id: "thread-123".to_string(),
                keyword: "$hail_feed".to_string(),
            },
            Call::RemoveKeyword {
                thread_id: "thread-123".to_string(),
                keyword: "$hail_papertrail".to_string(),
            },
            Call::RemoveKeyword {
                thread_id: "thread-123".to_string(),
                keyword: "$hail_setaside".to_string(),
            },
            Call::RemoveKeyword {
                thread_id: "thread-123".to_string(),
                keyword: "$hail_replylater".to_string(),
            },
        ]
    );

    let stack_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND thread_id = 'thread-123'",
    )
    .bind(user_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(stack_count, 0);
}

#[tokio::test]
async fn at_in_past_returns_400() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bob@example.org").await;
    let past = Utc::now() - Duration::minutes(1);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/threads/thread-123/bubble-up")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"at":"{}"}}"#, past.to_rfc3339())))
        .unwrap();

    let resp = app(
        state,
        Arc::new(FakeVerifier { exists: true }),
        Arc::new(FakeActions::default()),
    )
    .oneshot(req)
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_csrf_header_returns_403() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "carol@example.org").await;
    let future = Utc::now() + Duration::minutes(10);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/threads/thread-123/bubble-up")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"at":"{}"}}"#, future.to_rfc3339())))
        .unwrap();

    let resp = app(
        state,
        Arc::new(FakeVerifier { exists: true }),
        Arc::new(FakeActions::default()),
    )
    .oneshot(req)
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cancel_bubble_up_deletes_only_current_users_pending_row() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice-cancel@example.org").await;
    let (bob_id, _bob_sid) = seed_session(&state, &key, "bob-cancel@example.org").await;
    let db = state.db.clone();
    let now = Utc::now();
    let surface_at = now + Duration::minutes(10);

    for (user_id, thread_id) in [
        (alice_id, "thread-cancel"),
        (bob_id, "thread-cancel"),
        (alice_id, "other-thread"),
    ] {
        sqlx::query(
            "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(user_id)
        .bind(thread_id)
        .bind(surface_at)
        .bind(now)
        .execute(&db)
        .await
        .unwrap();
    }

    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/api/threads/thread-cancel/bubble-up")
        .header(header::COOKIE, format!("hail_session={alice_sid}"))
        .header(CSRF_HEADER, "1")
        .body(Body::empty())
        .unwrap();

    let resp = app(
        state,
        Arc::new(FakeVerifier { exists: true }),
        Arc::new(FakeActions::default()),
    )
    .oneshot(req)
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "cancelled");

    let remaining: Vec<(i64, String)> =
        sqlx::query_as("SELECT user_id, thread_id FROM bubble_ups ORDER BY user_id, thread_id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        remaining,
        vec![
            (alice_id, "other-thread".to_string()),
            (bob_id, "thread-cancel".to_string()),
        ]
    );
}

#[tokio::test]
async fn no_auth_returns_401() {
    let (state, _key) = fixture_state().await;
    let future = Utc::now() + Duration::minutes(10);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/threads/thread-123/bubble-up")
        .header(CSRF_HEADER, "1")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"at":"{}"}}"#, future.to_rfc3339())))
        .unwrap();

    let resp = app(
        state,
        Arc::new(FakeVerifier { exists: true }),
        Arc::new(FakeActions::default()),
    )
    .oneshot(req)
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

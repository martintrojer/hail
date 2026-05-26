use chrono::{Duration, Utc};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::threads::{
    Classification, ThreadActionError, ThreadActions, ThreadVerifier, ThreadVerifyError,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use secrecy::SecretString;
use tower::ServiceExt;

fn app(state: AppState, actions: Arc<FakeActions>) -> Router {
    app_with_verifier(state, Arc::new(FakeVerifier::visible()), actions)
}

fn app_with_verifier(
    state: AppState,
    verifier: Arc<FakeVerifier>,
    actions: Arc<FakeActions>,
) -> Router {
    let protected = hail_api::routes::threads::router_with_deps(verifier, actions).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn request(
    method: Method,
    state: AppState,
    actions: Arc<FakeActions>,
    sid: Option<&str>,
    csrf: bool,
    path: &str,
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
    let req = builder
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    app(state, actions).oneshot(req).await.unwrap()
}

async fn post(
    state: AppState,
    actions: Arc<FakeActions>,
    sid: Option<&str>,
    csrf: bool,
    path: &str,
    body: Option<&str>,
) -> axum::response::Response {
    request(Method::POST, state, actions, sid, csrf, path, body).await
}

async fn delete(
    state: AppState,
    actions: Arc<FakeActions>,
    sid: Option<&str>,
    csrf: bool,
    path: &str,
) -> axum::response::Response {
    request(Method::DELETE, state, actions, sid, csrf, path, None).await
}

async fn post_with_verifier(
    state: AppState,
    verifier: Arc<FakeVerifier>,
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
    app_with_verifier(state, verifier, actions)
        .oneshot(req)
        .await
        .unwrap()
}

enum VerifyMode {
    Visible,
    Hidden,
    Error,
}

struct FakeVerifier {
    mode: VerifyMode,
}

impl FakeVerifier {
    fn visible() -> Self {
        Self {
            mode: VerifyMode::Visible,
        }
    }

    fn hidden() -> Self {
        Self {
            mode: VerifyMode::Hidden,
        }
    }

    fn failing() -> Self {
        Self {
            mode: VerifyMode::Error,
        }
    }
}

impl ThreadVerifier for FakeVerifier {
    fn exists<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ThreadVerifyError>> + Send + 'a>> {
        Box::pin(async move {
            match self.mode {
                VerifyMode::Visible => Ok(true),
                VerifyMode::Hidden => Ok(false),
                VerifyMode::Error => Err(ThreadVerifyError::provider("visibility failed")),
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Classify {
        thread_id: String,
        classification: Classification,
    },
    SetKeyword {
        thread_id: String,
        keyword: String,
        enabled: bool,
    },
    Archive {
        thread_id: String,
    },
    Trash {
        thread_id: String,
    },
    Spam {
        thread_id: String,
    },
    NotSpam {
        thread_id: String,
    },
    Restore {
        thread_id: String,
    },
    Destroy {
        thread_id: String,
    },
    Mark {
        thread_id: String,
        read: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionKind {
    CurrentClassification,
    Classify,
    SetKeyword,
    Archive,
    Trash,
    Spam,
    NotSpam,
    Restore,
    Destroy,
    Mark,
}

#[derive(Default)]
struct FakeActions {
    calls: Mutex<Vec<Call>>,
    missing: Mutex<Vec<String>>,
    provider_failures: Mutex<Vec<ActionKind>>,
    current_classification: Mutex<Option<Classification>>,
}

impl FakeActions {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }

    /// Return calls excluding pile-keyword removals (noise from classify cleanup).
    fn calls_without_pile_cleanup(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| !matches!(c, Call::SetKeyword { keyword, enabled: false, .. } if keyword == "$hail_setaside" || keyword == "$hail_replylater"))
            .collect()
    }

    fn mark_missing(&self, thread_id: &str) {
        self.missing
            .lock()
            .expect("missing mutex")
            .push(thread_id.to_string());
    }

    fn fail_provider(&self, kind: ActionKind) {
        self.provider_failures
            .lock()
            .expect("provider failures mutex")
            .push(kind);
    }

    fn maybe_fail(&self, kind: ActionKind) -> Result<(), ThreadActionError> {
        if self
            .provider_failures
            .lock()
            .expect("provider failures mutex")
            .contains(&kind)
        {
            Err(ThreadActionError::Provider(format!("{kind:?} failed")))
        } else {
            Ok(())
        }
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
            self.maybe_fail(ActionKind::CurrentClassification)?;
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
            self.maybe_fail(ActionKind::Classify)?;
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

    fn set_keyword<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
        keyword: &'static str,
        enabled: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_fail(ActionKind::SetKeyword)?;
            self.maybe_missing(thread_id)?;
            self.calls
                .lock()
                .expect("calls mutex")
                .push(Call::SetKeyword {
                    thread_id: thread_id.to_string(),
                    keyword: keyword.to_string(),
                    enabled,
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
            self.maybe_fail(ActionKind::Archive)?;
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
            self.maybe_fail(ActionKind::Trash)?;
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Trash {
                thread_id: thread_id.to_string(),
            });
            Ok(())
        })
    }

    fn spam<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_fail(ActionKind::Spam)?;
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Spam {
                thread_id: thread_id.to_string(),
            });
            Ok(())
        })
    }

    fn not_spam<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_fail(ActionKind::NotSpam)?;
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::NotSpam {
                thread_id: thread_id.to_string(),
            });
            Ok(())
        })
    }

    fn restore<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_fail(ActionKind::Restore)?;
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Restore {
                thread_id: thread_id.to_string(),
            });
            Ok(())
        })
    }

    fn destroy<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ThreadActionError>> + Send + 'a>> {
        Box::pin(async move {
            self.maybe_fail(ActionKind::Destroy)?;
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Destroy {
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
            self.maybe_fail(ActionKind::Mark)?;
            self.maybe_missing(thread_id)?;
            self.calls.lock().expect("calls mutex").push(Call::Mark {
                thread_id: thread_id.to_string(),
                read,
            });
            Ok(())
        })
    }
}

async fn count_bubble_rows(state: &AppState, user_id: i64, thread_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND thread_id = ?2",
    )
    .bind(user_id)
    .bind(thread_id)
    .fetch_one(&state.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn bubble_up_hidden_or_provider_error_does_not_insert_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "bubble-errors@example.org").await;
    let hidden_at = Utc::now() + Duration::minutes(10);
    let provider_at = Utc::now() + Duration::minutes(11);

    let hidden = post_with_verifier(
        state.clone(),
        Arc::new(FakeVerifier::hidden()),
        Arc::new(FakeActions::default()),
        Some(&sid),
        true,
        "/api/threads/thread-hidden/bubble-up",
        Some(&format!(r#"{{"at":"{}"}}"#, hidden_at.to_rfc3339())),
    )
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let provider_error = post_with_verifier(
        state.clone(),
        Arc::new(FakeVerifier::failing()),
        Arc::new(FakeActions::default()),
        Some(&sid),
        true,
        "/api/threads/thread-provider/bubble-up",
        Some(&format!(r#"{{"at":"{}"}}"#, provider_at.to_rfc3339())),
    )
    .await;
    assert_eq!(provider_error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = json_body(provider_error).await;
    assert_eq!(json["error"], "internal");

    assert_eq!(count_bubble_rows(&state, user_id, "thread-hidden").await, 0);
    assert_eq!(
        count_bubble_rows(&state, user_id, "thread-provider").await,
        0
    );
}

#[tokio::test]
async fn action_provider_failures_return_500_and_do_not_insert_sidecar_rows() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "action-errors@example.org").await;

    for (kind, method, path, body, stack) in [
        (
            ActionKind::SetKeyword,
            Method::POST,
            "/api/threads/thread-set-aside/set-aside",
            None,
            Some("set_aside"),
        ),
        (
            ActionKind::SetKeyword,
            Method::POST,
            "/api/threads/thread-reply-later/reply-later",
            None,
            Some("reply_later"),
        ),
        (
            ActionKind::Archive,
            Method::POST,
            "/api/threads/thread-archive/archive",
            None,
            None,
        ),
        (
            ActionKind::Trash,
            Method::POST,
            "/api/threads/thread-trash/trash",
            None,
            None,
        ),
        (
            ActionKind::Spam,
            Method::POST,
            "/api/threads/thread-spam/spam",
            None,
            None,
        ),
        (
            ActionKind::NotSpam,
            Method::POST,
            "/api/threads/thread-not-spam/not-spam",
            None,
            None,
        ),
        (
            ActionKind::Restore,
            Method::POST,
            "/api/threads/thread-restore/restore",
            None,
            None,
        ),
        (
            ActionKind::Destroy,
            Method::DELETE,
            "/api/threads/thread-destroy/destroy",
            None,
            None,
        ),
        (
            ActionKind::Mark,
            Method::POST,
            "/api/threads/thread-mark/mark",
            Some(r#"{"read":true}"#),
            None,
        ),
        (
            ActionKind::Classify,
            Method::POST,
            "/api/threads/thread-classify/classify",
            Some(r#"{"to":"feed"}"#),
            None,
        ),
    ] {
        let actions = Arc::new(FakeActions::default());
        actions.fail_provider(kind);
        *actions
            .current_classification
            .lock()
            .expect("current classification mutex") = Some(Classification::Imbox);

        let resp = request(
            method,
            state.clone(),
            actions.clone(),
            Some(&sid),
            true,
            path,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "internal", "{path}");
        assert_eq!(actions.calls(), Vec::<Call>::new(), "{path}");

        if let Some(stack) = stack {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND stack = ?2",
            )
            .bind(user_id)
            .bind(stack)
            .fetch_one(&state.db)
            .await
            .unwrap();
            assert_eq!(count, 0, "{path}");
        }
    }

    let undo_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM undo_actions WHERE user_id = ?1 AND action = 'thread.classify'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(undo_count, 0);
}

#[derive(Debug, PartialEq, Eq)]
struct SidecarSnapshot {
    stack_positions: Vec<(i64, String, String, i64, String)>,
    bubble_ups: Vec<(i64, i64, String, String, Option<String>, String)>,
    thread_notes: Vec<(i64, i64, String, String, String, String)>,
}

async fn snapshot_sidecar_state(state: &AppState) -> SidecarSnapshot {
    let stack_positions = sqlx::query_as::<_, (i64, String, String, i64, String)>(
        "SELECT user_id, stack, thread_id, position, added_at FROM stack_positions \
         ORDER BY user_id, stack, thread_id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    let bubble_ups = sqlx::query_as::<_, (i64, i64, String, String, Option<String>, String)>(
        "SELECT id, user_id, thread_id, surface_at, fired_at, created_at FROM bubble_ups \
         ORDER BY id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    let thread_notes = sqlx::query_as::<_, (i64, i64, String, String, String, String)>(
        "SELECT id, user_id, thread_id, email_id, body, created_at FROM thread_notes \
         ORDER BY id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();

    SidecarSnapshot {
        stack_positions,
        bubble_ups,
        thread_notes,
    }
}

async fn seed_sidecar_state(state: &AppState, user_id: i64, other_user_id: i64, thread_id: &str) {
    let now = Utc::now();
    for (row_user_id, stack, row_thread_id, position) in [
        (user_id, "set_aside", thread_id, 7),
        (user_id, "reply_later", "other-thread", 8),
        (other_user_id, "set_aside", thread_id, 9),
    ] {
        sqlx::query(
            "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(row_user_id)
        .bind(stack)
        .bind(row_thread_id)
        .bind(position)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();
    }

    for (row_user_id, row_thread_id, fired) in [
        (user_id, thread_id, false),
        (user_id, thread_id, true),
        (other_user_id, thread_id, false),
    ] {
        if fired {
            sqlx::query(
                "INSERT INTO bubble_ups (user_id, thread_id, surface_at, fired_at, created_at) \
                 VALUES (?1, ?2, ?3, ?3, ?3)",
            )
            .bind(row_user_id)
            .bind(row_thread_id)
            .bind(now)
            .execute(&state.db)
            .await
            .unwrap();
        } else {
            sqlx::query(
                "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) \
                 VALUES (?1, ?2, ?3, ?3)",
            )
            .bind(row_user_id)
            .bind(row_thread_id)
            .bind(now)
            .execute(&state.db)
            .await
            .unwrap();
        }
    }

    for (row_user_id, row_thread_id, email_id, body) in [
        (user_id, thread_id, "email-1", "target note"),
        (other_user_id, thread_id, "email-2", "other note"),
    ] {
        sqlx::query(
            "INSERT INTO thread_notes (user_id, thread_id, email_id, body) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(row_user_id)
        .bind(row_thread_id)
        .bind(email_id)
        .bind(body)
        .execute(&state.db)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn action_provider_failures_preserve_existing_sidecar_state() {
    for (kind, method, thread_id, path, body) in [
        (
            ActionKind::SetKeyword,
            Method::POST,
            "thread-set-aside",
            "/api/threads/thread-set-aside/set-aside",
            None,
        ),
        (
            ActionKind::SetKeyword,
            Method::POST,
            "thread-reply-later",
            "/api/threads/thread-reply-later/reply-later",
            None,
        ),
        (
            ActionKind::Archive,
            Method::POST,
            "thread-archive",
            "/api/threads/thread-archive/archive",
            None,
        ),
        (
            ActionKind::Trash,
            Method::POST,
            "thread-trash",
            "/api/threads/thread-trash/trash",
            None,
        ),
        (
            ActionKind::Restore,
            Method::POST,
            "thread-restore",
            "/api/threads/thread-restore/restore",
            None,
        ),
        (
            ActionKind::Destroy,
            Method::DELETE,
            "thread-destroy",
            "/api/threads/thread-destroy/destroy",
            None,
        ),
        (
            ActionKind::Mark,
            Method::POST,
            "thread-mark",
            "/api/threads/thread-mark/mark",
            Some(r#"{"read":true}"#),
        ),
        (
            ActionKind::CurrentClassification,
            Method::POST,
            "thread-classify-current",
            "/api/threads/thread-classify-current/classify",
            Some(r#"{"to":"feed"}"#),
        ),
        (
            ActionKind::Classify,
            Method::POST,
            "thread-classify",
            "/api/threads/thread-classify/classify",
            Some(r#"{"to":"feed"}"#),
        ),
    ] {
        let (state, key) = fixture_state().await;
        let (user_id, sid) = seed_session(&state, &key, "action-preserve@example.org").await;
        let (other_user_id, _other_sid) =
            seed_session(&state, &key, "action-preserve-other@example.org").await;
        seed_sidecar_state(&state, user_id, other_user_id, thread_id).await;
        let before = snapshot_sidecar_state(&state).await;

        let actions = Arc::new(FakeActions::default());
        actions.fail_provider(kind);
        *actions
            .current_classification
            .lock()
            .expect("current classification mutex") = Some(Classification::Imbox);

        let resp = request(
            method,
            state.clone(),
            actions.clone(),
            Some(&sid),
            true,
            path,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "internal", "{path}");
        assert_eq!(actions.calls(), Vec::<Call>::new(), "{path}");
        assert_eq!(snapshot_sidecar_state(&state).await, before, "{path}");

        let undo_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM undo_actions WHERE user_id = ?1")
                .bind(user_id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(undo_count, 0, "{path}");
    }
}

#[tokio::test]
async fn current_classification_provider_failure_stops_classify_before_action_or_undo() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "classify-current-error@example.org").await;
    let actions = Arc::new(FakeActions::default());
    actions.fail_provider(ActionKind::CurrentClassification);

    let resp = post(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-classify-current/classify",
        Some(r#"{"to":"feed"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "internal");
    assert_eq!(actions.calls(), Vec::<Call>::new());

    let undo_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM undo_actions WHERE user_id = ?1 AND action = 'thread.classify'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(undo_count, 0);
}

#[tokio::test]
async fn malformed_mark_json_returns_400_without_calling_provider() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bad-mark@example.org").await;
    let actions = Arc::new(FakeActions::default());

    for body in [r#"{"read":"yes"}"#, r#"{"seen":true}"#, "not-json"] {
        let resp = post(
            state.clone(),
            actions.clone(),
            Some(&sid),
            true,
            "/api/threads/thread-mark/mark",
            Some(body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "invalid_mark", "{body}");
    }

    assert_eq!(actions.calls(), Vec::<Call>::new());
}

#[tokio::test]
async fn auth_required_for_each_thread_verb_endpoint() {
    let (state, _key) = fixture_state().await;

    for (method, path, body) in verb_requests() {
        let resp = request(
            method,
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

    for (method, path, body) in verb_requests() {
        let resp = request(
            method,
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

fn verb_requests() -> [(Method, &'static str, Option<&'static str>); 10] {
    [
        (
            Method::POST,
            "/api/threads/thread-1/classify",
            Some(r#"{"to":"imbox"}"#),
        ),
        (Method::POST, "/api/threads/thread-1/set-aside", None),
        (Method::POST, "/api/threads/thread-1/reply-later", None),
        (Method::POST, "/api/threads/thread-1/archive", None),
        (Method::POST, "/api/threads/thread-1/trash", None),
        (Method::POST, "/api/threads/thread-1/spam", None),
        (Method::POST, "/api/threads/thread-1/not-spam", None),
        (Method::POST, "/api/threads/thread-1/restore", None),
        (Method::DELETE, "/api/threads/thread-1/destroy", None),
        (
            Method::POST,
            "/api/threads/thread-1/mark",
            Some(r#"{"read":true}"#),
        ),
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
        actions.calls_without_pile_cleanup(),
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
        actions.calls_without_pile_cleanup(),
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
        actions.calls_without_pile_cleanup(),
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
        actions.calls_without_pile_cleanup(),
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
    sqlx::query(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) VALUES (?1, 'shared', ?2, ?2)",
    )
    .bind(alice_id)
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, fired_at, created_at) VALUES (?1, 'shared', ?2, ?2, ?2)",
    )
    .bind(alice_id)
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
    let fired_bubble_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND thread_id = 'shared' AND fired_at IS NOT NULL",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(fired_bubble_count, 1);
    let pending_bubble_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND thread_id = 'shared' AND fired_at IS NULL",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(pending_bubble_count, 0);
    assert_eq!(
        actions.calls(),
        vec![
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_setaside".to_string(),
                enabled: true,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_imbox".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_feed".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_papertrail".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_setaside".to_string(),
                enabled: true,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_imbox".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_feed".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "shared".to_string(),
                keyword: "$hail_papertrail".to_string(),
                enabled: false,
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
        vec![
            Call::SetKeyword {
                thread_id: "thread-2".to_string(),
                keyword: "$hail_replylater".to_string(),
                enabled: true,
            },
            Call::SetKeyword {
                thread_id: "thread-2".to_string(),
                keyword: "$hail_imbox".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-2".to_string(),
                keyword: "$hail_feed".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-2".to_string(),
                keyword: "$hail_papertrail".to_string(),
                enabled: false,
            },
        ]
    );
}

#[tokio::test]
async fn archive_trash_restore_destroy_and_mark_call_actions() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "verbs@example.org").await;
    let actions = Arc::new(FakeActions::default());
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) VALUES (?1, 'set_aside', 'thread-3', 1, ?2)",
    )
    .bind(user_id)
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, created_at) VALUES (?1, 'thread-3', ?2, ?2)",
    )
    .bind(user_id)
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO bubble_ups (user_id, thread_id, surface_at, fired_at, created_at) VALUES (?1, 'thread-3', ?2, ?2, ?2)",
    )
    .bind(user_id)
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO thread_notes (user_id, thread_id, email_id, body) VALUES (?1, 'thread-3', 'email-1', 'note')",
    )
    .bind(user_id)
    .execute(&state.db)
    .await
    .unwrap();

    for (path, body, expected) in [
        ("/api/threads/thread-3/archive", None, StatusCode::OK),
        ("/api/threads/thread-3/trash", None, StatusCode::OK),
        ("/api/threads/thread-3/spam", None, StatusCode::OK),
        ("/api/threads/thread-3/not-spam", None, StatusCode::OK),
        ("/api/threads/thread-3/restore", None, StatusCode::OK),
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
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_imbox".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_feed".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_papertrail".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_setaside".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_replylater".to_string(),
                enabled: false,
            },
            Call::Spam {
                thread_id: "thread-3".to_string()
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_imbox".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_feed".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_papertrail".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_setaside".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_replylater".to_string(),
                enabled: false,
            },
            Call::NotSpam {
                thread_id: "thread-3".to_string()
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_imbox".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_feed".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_papertrail".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_setaside".to_string(),
                enabled: false,
            },
            Call::SetKeyword {
                thread_id: "thread-3".to_string(),
                keyword: "$hail_replylater".to_string(),
                enabled: false,
            },
            Call::Classify {
                thread_id: "thread-3".to_string(),
                classification: Classification::Imbox
            },
            Call::Restore {
                thread_id: "thread-3".to_string()
            },
            Call::Classify {
                thread_id: "thread-3".to_string(),
                classification: Classification::Imbox
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

    let stack_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND thread_id = 'thread-3'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(stack_count, 0);
    let pending_bubble_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND thread_id = 'thread-3' AND fired_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(pending_bubble_count, 0);
    let fired_bubble_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND thread_id = 'thread-3' AND fired_at IS NOT NULL",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(fired_bubble_count, 1);

    let destroy_resp = delete(
        state.clone(),
        actions.clone(),
        Some(&sid),
        true,
        "/api/threads/thread-3/destroy",
    )
    .await;
    assert_eq!(destroy_resp.status(), StatusCode::OK);
    let destroy_json = json_body(destroy_resp).await;
    assert_eq!(destroy_json["status"], "destroyed");
    assert!(actions.calls().contains(&Call::Destroy {
        thread_id: "thread-3".to_string()
    }));

    let note_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM thread_notes WHERE user_id = ?1 AND thread_id = 'thread-3'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(note_count, 0);
    let bubble_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bubble_ups WHERE user_id = ?1 AND thread_id = 'thread-3'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(bubble_count, 0);
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
async fn provider_missing_thread_returns_404_and_does_not_mutate_sidecar_state() {
    for (method, path, body, stack) in [
        (
            Method::POST,
            "/api/threads/missing-thread/set-aside",
            None,
            Some("set_aside"),
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/reply-later",
            None,
            Some("reply_later"),
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/archive",
            None,
            None,
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/trash",
            None,
            None,
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/spam",
            None,
            None,
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/not-spam",
            None,
            None,
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/restore",
            None,
            None,
        ),
        (
            Method::DELETE,
            "/api/threads/missing-thread/destroy",
            None,
            None,
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/mark",
            Some(r#"{"read":true}"#),
            None,
        ),
        (
            Method::POST,
            "/api/threads/missing-thread/classify",
            Some(r#"{"to":"feed"}"#),
            None,
        ),
    ] {
        let (state, key) = fixture_state().await;
        let (user_id, sid) = seed_session(&state, &key, "missing@example.org").await;
        let actions = Arc::new(FakeActions::default());
        actions.mark_missing("missing-thread");
        *actions
            .current_classification
            .lock()
            .expect("current classification mutex") = Some(Classification::Imbox);

        let resp = request(
            method,
            state.clone(),
            actions.clone(),
            Some(&sid),
            true,
            path,
            body,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(actions.calls(), Vec::<Call>::new(), "{path}");

        if let Some(stack) = stack {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM stack_positions WHERE user_id = ?1 AND stack = ?2 AND thread_id = 'missing-thread'",
            )
            .bind(user_id)
            .bind(stack)
            .fetch_one(&state.db)
            .await
            .unwrap();
            assert_eq!(count, 0, "{path}");
        }

        let undo_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM undo_actions WHERE user_id = ?1 AND payload_json LIKE '%missing-thread%'",
        )
        .bind(user_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(undo_count, 0, "{path}");
    }
}

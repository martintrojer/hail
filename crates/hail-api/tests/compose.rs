use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::compose::{
    ComposeError, Composer, OutboundMessage, ReplyContext, ReplyHeaders,
};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::Value;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_compose_test_{uniq}?mode=memory&cache=shared");
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

fn app(state: AppState, composer: Arc<FakeComposer>) -> Router {
    let protected = hail_api::routes::compose::router_with_composer(composer).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn request(
    state: AppState,
    composer: Arc<FakeComposer>,
    method: Method,
    path: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<String>,
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

    app(state, composer)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, Body::from))
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
        plain_text: String,
        html: String,
        reply: Option<ReplyHeaders>,
    },
    Submit {
        from: String,
        email_id: String,
    },
    ThreadContext {
        thread_id: String,
    },
}

struct FakeComposer {
    calls: Mutex<Vec<Call>>,
    context: Mutex<Option<ReplyContext>>,
}

impl Default for FakeComposer {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            context: Mutex::new(Some(ReplyContext {
                to: vec!["sender@example.org".to_string()],
                subject: "Re: Hello".to_string(),
                in_reply_to: vec!["m1@example.org".to_string()],
                references: vec!["m0@example.org".to_string(), "m1@example.org".to_string()],
            })),
        }
    }
}

impl FakeComposer {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls mutex").clone()
    }

    fn set_not_found(&self) {
        *self.context.lock().expect("context mutex") = None;
    }
}

impl Composer for FakeComposer {
    fn create_draft<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        from: &'a str,
        message: OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<String, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().expect("calls mutex").push(Call::Create {
                from: from.to_string(),
                to: message.to,
                cc: message.cc,
                bcc: message.bcc,
                subject: message.subject,
                plain_text: message.body.plain_text,
                html: message.body.html,
                reply: message.reply,
            });
            Ok(format!(
                "draft-{}",
                self.calls.lock().expect("calls mutex").len()
            ))
        })
    }

    fn submit<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        from: &'a str,
        email_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().expect("calls mutex").push(Call::Submit {
                from: from.to_string(),
                email_id: email_id.to_string(),
            });
            Ok(Some("submission-1".to_string()))
        })
    }

    fn thread_context<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ReplyContext>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls mutex")
                .push(Call::ThreadContext {
                    thread_id: thread_id.to_string(),
                });
            Ok(self.context.lock().expect("context mutex").clone())
        })
    }
}

fn compose_body(send_at: Option<chrono::DateTime<Utc>>) -> String {
    let send_at = send_at
        .map(|dt| format!(r#", "send_at":"{}""#, dt.to_rfc3339()))
        .unwrap_or_default();
    format!(
        r#"{{
            "to":["bob@example.org"],
            "cc":["carol@example.org"],
            "subject":"Hello",
            "body_markdown":"Hi **Bob**<script>alert(1)</script>"{send_at}
        }}"#
    )
}

fn recipient_list(count: usize) -> Vec<String> {
    (0..count)
        .map(|idx| format!("user{idx}@example.org"))
        .collect()
}

#[tokio::test]
async fn compose_send_now_creates_draft_and_submits() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(compose_body(None)),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "sent");
    assert_eq!(json["email_id"], "draft-1");
    assert_eq!(json["submission_id"], "submission-1");
    let calls = composer.calls();
    assert_eq!(calls.len(), 2);
    let Call::Create {
        plain_text, html, ..
    } = &calls[0]
    else {
        panic!("first call should create draft");
    };
    assert!(plain_text.contains("Hi Bob"));
    assert!(html.contains("<strong>Bob</strong>"));
    assert!(!html.contains("script"));
    assert_eq!(
        calls[1],
        Call::Submit {
            from: "alice@example.org".to_string(),
            email_id: "draft-1".to_string()
        }
    );
}

#[tokio::test]
async fn compose_send_later_inserts_pending_row_without_submit() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    let send_at = Utc::now() + Duration::hours(2);

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(compose_body(Some(send_at))),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "pending");
    assert_eq!(json["draft_email_id"], "draft-1");
    assert_eq!(composer.calls().len(), 1);

    let row: (String, String) =
        sqlx::query_as("SELECT draft_email_id, status FROM scheduled_sends WHERE id = ?1")
            .bind(json["scheduled_send_id"].as_i64().expect("id"))
            .fetch_one(&state.db)
            .await
            .expect("scheduled row");
    assert_eq!(row, ("draft-1".to_string(), "pending".to_string()));
}

#[tokio::test]
async fn scheduled_send_list_and_get_are_scoped_to_user() {
    let (state, key) = fixture_state().await;
    let alice_sid = seed_session(&state, &key, "alice@example.org").await;
    let _bob_sid = seed_session(&state, &key, "bob@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    let alice_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("alice@example.org")
        .fetch_one(&state.db)
        .await
        .expect("alice id");
    let bob_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("bob@example.org")
        .fetch_one(&state.db)
        .await
        .expect("bob id");
    let now = Utc::now();
    let alice_later = now + Duration::hours(3);
    let alice_soon = now + Duration::hours(1);
    let bob_send_at = now + Duration::hours(2);
    let alice_later_id: i64 = sqlx::query_scalar(
        "INSERT INTO scheduled_sends (user_id, draft_email_id, send_at, status, created_at) \
         VALUES (?, 'draft-alice-later', ?, 'pending', ?) RETURNING id",
    )
    .bind(alice_id)
    .bind(alice_later)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert alice later");
    let alice_soon_id: i64 = sqlx::query_scalar(
        "INSERT INTO scheduled_sends (user_id, draft_email_id, send_at, status, created_at) \
         VALUES (?, 'draft-alice-soon', ?, 'failed', ?) RETURNING id",
    )
    .bind(alice_id)
    .bind(alice_soon)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert alice soon");
    let bob_id: i64 = sqlx::query_scalar(
        "INSERT INTO scheduled_sends (user_id, draft_email_id, send_at, status, created_at) \
         VALUES (?, 'draft-bob', ?, 'pending', ?) RETURNING id",
    )
    .bind(bob_id)
    .bind(bob_send_at)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert bob");

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::GET,
        "/api/scheduled-sends",
        Some(&alice_sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let rows = json.as_array().expect("array response");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], alice_soon_id);
    assert_eq!(rows[0]["draft_email_id"], "draft-alice-soon");
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(rows[1]["id"], alice_later_id);
    assert_eq!(rows[1]["draft_email_id"], "draft-alice-later");

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::GET,
        &format!("/api/scheduled-sends/{alice_later_id}"),
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["id"], alice_later_id);
    assert_eq!(json["send_at"], serde_json::to_value(alice_later).unwrap());

    let resp = request(
        state,
        composer,
        Method::GET,
        &format!("/api/scheduled-sends/{bob_id}"),
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn scheduled_send_cancel_marks_pending_row_cancelled_for_owner() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    let send_at = Utc::now() + Duration::hours(2);

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(compose_body(Some(send_at))),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let scheduled_send_id = json_body(resp).await["scheduled_send_id"]
        .as_i64()
        .expect("scheduled id");

    let resp = request(
        state.clone(),
        composer,
        Method::DELETE,
        &format!("/api/scheduled-sends/{scheduled_send_id}"),
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["id"], scheduled_send_id);
    assert_eq!(json["status"], "cancelled");
    assert_eq!(json["draft_email_id"], "draft-1");

    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM scheduled_sends WHERE id = ?")
            .bind(scheduled_send_id)
            .fetch_one(&state.db)
            .await
            .expect("scheduled row");
    assert_eq!(row, ("cancelled".to_string(), None));
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE action = 'compose.schedule_cancel' AND user_id = (SELECT id FROM users WHERE email = 'alice@example.org')",
    )
    .fetch_one(&state.db)
    .await
    .expect("audit count");
    assert_eq!(audit_count, 1);
}

#[tokio::test]
async fn scheduled_send_cancel_rejects_non_pending_and_non_owner_rows() {
    let (state, key) = fixture_state().await;
    let alice_sid = seed_session(&state, &key, "alice@example.org").await;
    let bob_sid = seed_session(&state, &key, "bob@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    let alice_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("alice@example.org")
        .fetch_one(&state.db)
        .await
        .expect("alice id");
    let bob_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind("bob@example.org")
        .fetch_one(&state.db)
        .await
        .expect("bob id");
    let now = Utc::now();
    let sent_id: i64 = sqlx::query_scalar(
        "INSERT INTO scheduled_sends (user_id, draft_email_id, send_at, status, sent_at, created_at) \
         VALUES (?, 'draft-sent', ?, 'sent', ?, ?) RETURNING id",
    )
    .bind(alice_id)
    .bind(now - Duration::minutes(1))
    .bind(now)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert sent");
    let bob_pending_id: i64 = sqlx::query_scalar(
        "INSERT INTO scheduled_sends (user_id, draft_email_id, send_at, status, created_at) \
         VALUES (?, 'draft-bob', ?, 'pending', ?) RETURNING id",
    )
    .bind(bob_id)
    .bind(now + Duration::hours(2))
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert bob pending");

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::DELETE,
        &format!("/api/scheduled-sends/{sent_id}"),
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "scheduled_send_not_cancellable");

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::DELETE,
        &format!("/api/scheduled-sends/{bob_pending_id}"),
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = request(
        state,
        composer,
        Method::DELETE,
        &format!("/api/scheduled-sends/{bob_pending_id}"),
        Some(&bob_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "cancelled");
}

#[tokio::test]
async fn compose_rejects_non_future_send_at_without_creating_draft() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    let send_at = Utc::now() - Duration::minutes(1);

    let resp = request(
        state.clone(),
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(compose_body(Some(send_at))),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_send_at");
    assert!(composer.calls().is_empty());

    let scheduled_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_sends")
        .fetch_one(&state.db)
        .await
        .expect("scheduled count");
    assert_eq!(scheduled_count, 0);
}

#[tokio::test]
async fn reply_rejects_non_future_send_at_before_thread_lookup() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    let send_at = (Utc::now() - Duration::minutes(1)).to_rfc3339();

    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/threads/thread-1/reply",
        Some(&sid),
        true,
        Some(format!(
            r#"{{"body_markdown":"Reply body","send_at":"{send_at}"}}"#
        )),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_send_at");
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_rejects_invalid_recipient() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(r#"{"to":["bad"],"subject":"Hi","body_markdown":"Body"}"#.to_string()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_to");
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_rejects_attachments_without_creating_draft() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(
            r#"{"to":["bob@example.org"],"subject":"Hi","body_markdown":"Body","attachments":[{"name":"a.txt"}]}"#
                .to_string(),
        ),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "attachments_not_supported");
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_rejects_invalid_cc_bcc_without_creating_draft() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    for (field, expected_error) in [("cc", "invalid_cc"), ("bcc", "invalid_bcc")] {
        let body = serde_json::json!({
            "to": ["bob@example.org"],
            field: ["not-an-email"],
            "subject": "Hi",
            "body_markdown": "Body",
        })
        .to_string();
        let resp = request(
            state.clone(),
            composer.clone(),
            Method::POST,
            "/api/compose",
            Some(&sid),
            true,
            Some(body),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = json_body(resp).await;
        assert_eq!(json["error"], expected_error);
    }
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_rejects_too_many_recipients_without_creating_draft() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    for (field, expected_error) in [
        ("to", "too_many_to"),
        ("cc", "too_many_cc"),
        ("bcc", "too_many_bcc"),
    ] {
        let mut body = serde_json::json!({
            "to": ["bob@example.org"],
            "subject": "Hi",
            "body_markdown": "Body",
        });
        body[field] = serde_json::json!(recipient_list(201));
        let resp = request(
            state.clone(),
            composer.clone(),
            Method::POST,
            "/api/compose",
            Some(&sid),
            true,
            Some(body.to_string()),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = json_body(resp).await;
        assert_eq!(json["error"], expected_error);
    }
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_rejects_subject_crlf_without_creating_draft() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    let body = serde_json::json!({
        "to": ["bob@example.org"],
        "subject": "Hi\r\nBcc: attacker@example.org",
        "body_markdown": "Body",
    })
    .to_string();
    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_subject");
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_rejects_body_too_large_without_creating_draft() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    let body = serde_json::json!({
        "to": ["bob@example.org"],
        "subject": "Hi",
        "body_markdown": "x".repeat(1024 * 1024 + 1),
    })
    .to_string();
    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/compose",
        Some(&sid),
        true,
        Some(body),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "body_too_large");
    assert!(composer.calls().is_empty());
}

#[tokio::test]
async fn compose_requires_csrf() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());

    let resp = request(
        state,
        composer,
        Method::POST,
        "/api/compose",
        Some(&sid),
        false,
        Some(compose_body(None)),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn compose_requires_auth() {
    let (state, _key) = fixture_state().await;
    let composer = Arc::new(FakeComposer::default());

    let resp = request(
        state,
        composer,
        Method::POST,
        "/api/compose",
        None,
        true,
        Some(compose_body(None)),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reply_not_found_returns_404() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let composer = Arc::new(FakeComposer::default());
    composer.set_not_found();

    let resp = request(
        state,
        composer.clone(),
        Method::POST,
        "/api/threads/thread-1/reply",
        Some(&sid),
        true,
        Some(r#"{"body_markdown":"Reply body"}"#.to_string()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        composer.calls(),
        vec![Call::ThreadContext {
            thread_id: "thread-1".to_string()
        }]
    );
}

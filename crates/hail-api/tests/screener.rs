use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use chrono::{Duration, Utc};
use hail_api::{
    middleware::{
        auth::{CSRF_HEADER, require_auth},
        rate_limit::IpRateLimiter,
    },
    routes::screener::{Classification, ScreenerBackfill, ScreenerBackfillError, ScreenerDecision},
    state::AppState,
};
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let url = format!(
        "sqlite:file:hail_screener_test_{}?mode=memory&cache=shared",
        uuid_like()
    );
    let db = connect(&url).await.unwrap();
    hail_db::migrate(&db).await.unwrap();
    let key = [0x5Au8; KEY_LEN];
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", hex::encode(key));
    }
    (
        AppState {
            db,
            config: Config::load_from(None).unwrap(),
            server_key: Arc::new(key),
            login_limiter: Arc::new(IpRateLimiter::default()),
        },
        key,
    )
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
    let user_id: i64 = sqlx::query_scalar("INSERT INTO users (email, jmap_account_id, is_admin, created_at) VALUES (?1, ?2, 0, ?3) RETURNING id")
        .bind(email).bind(format!("account-{email}")).bind(now).fetch_one(&state.db).await.unwrap();
    let sid = format!("{:064x}", user_id);
    sqlx::query("INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) VALUES (?1, ?2, ?3, 'test', ?4, ?5, ?5)")
        .bind(&sid).bind(user_id).bind(hail_core::seal(b"dummy-token", key).unwrap()).bind(now + Duration::days(30)).bind(now).execute(&state.db).await.unwrap();
    (user_id, sid)
}

async fn seed_rule(
    state: &AppState,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
    first_seen_at: chrono::DateTime<Utc>,
) {
    sqlx::query("INSERT INTO screener_rules (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) VALUES (?1, ?2, ?3, ?4, NULL, ?5)")
        .bind(user_id).bind(sender).bind(decision).bind(classify_as).bind(first_seen_at).execute(&state.db).await.unwrap();
}

fn app(state: AppState, backfill: Arc<FakeBackfill>) -> Router {
    let protected = hail_api::routes::screener::router_with_backfill(backfill).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn request(
    state: AppState,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    request_with_backfill(
        state,
        Arc::new(FakeBackfill::default()),
        method,
        uri,
        sid,
        csrf,
        body,
    )
    .await
}

async fn request_with_backfill(
    state: AppState,
    backfill: Arc<FakeBackfill>,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    app(state, backfill)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BackfillCall {
    user_id: i64,
    sender: String,
    decision: ScreenerDecision,
    classify_as: Option<Classification>,
}

#[derive(Default)]
struct FakeBackfill {
    calls: Mutex<Vec<BackfillCall>>,
}

#[async_trait]
impl ScreenerBackfill for FakeBackfill {
    async fn apply(
        &self,
        _state: &AppState,
        user: &hail_api::middleware::auth::AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError> {
        self.calls.lock().unwrap().push(BackfillCall {
            user_id: user.id,
            sender: sender.to_string(),
            decision,
            classify_as,
        });
        Ok(())
    }
}

#[tokio::test]
async fn screener_view_requires_auth() {
    let (state, _) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/views/screener", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn screener_view_returns_only_current_user_pending_rows() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule(
        &state,
        alice_id,
        "alice-pending@example.org",
        "pending",
        None,
        now,
    )
    .await;
    seed_rule(
        &state,
        bob_id,
        "bob-pending@example.org",
        "pending",
        None,
        now,
    )
    .await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "alice-pending@example.org");
    assert_eq!(senders[0]["message_count"], 1);
    assert!(senders[0]["latest_preview"].is_null());
}

#[tokio::test]
async fn screener_view_omits_allowed_and_denied_rows() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(&state, user_id, "pending@example.org", "pending", None, now).await;
    seed_rule(
        &state,
        user_id,
        "allowed@example.org",
        "allow",
        Some("feed"),
        now,
    )
    .await;
    seed_rule(&state, user_id, "denied@example.org", "deny", None, now).await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "pending@example.org");
}

#[tokio::test]
async fn screener_view_sorts_newest_first() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(
        &state,
        user_id,
        "old@example.org",
        "pending",
        None,
        now - Duration::hours(2),
    )
    .await;
    seed_rule(&state, user_id, "new@example.org", "pending", None, now).await;
    seed_rule(
        &state,
        user_id,
        "middle@example.org",
        "pending",
        None,
        now - Duration::hours(1),
    )
    .await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let got: Vec<&str> = json["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        got,
        vec!["new@example.org", "middle@example.org", "old@example.org"]
    );
}

#[tokio::test]
async fn approve_creates_or_updates_row_and_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "news@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    let resp = request(state.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":" News@Example.ORG ","decision":"approve","classify_as":"feed","apply_to_history":false}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["sender"], "news@example.org");
    assert_eq!(json["decision"], "approve");
    assert_eq!(json["classify_as"], "feed");
    let row: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT decision, classify_as, decided_at FROM screener_rules WHERE user_id = ?1 AND sender_address = 'news@example.org'").bind(user_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("feed"));
    assert!(row.2.is_some());
}

#[tokio::test]
async fn approve_without_classify_as_defaults_to_imbox() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"person@example.org","decision":"approve","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["classify_as"], "imbox");
}

#[tokio::test]
async fn deny_creates_or_updates_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "allow",
        Some("imbox"),
        Utc::now(),
    )
    .await;
    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["sender"], "spam@example.org");
    assert_eq!(json["decision"], "deny");
    assert!(json["classify_as"].is_null());
    let row: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT decision, classify_as, decided_at FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'").bind(user_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(row.0, "deny");
    assert!(row.1.is_none());
    assert!(row.2.is_some());
}

#[tokio::test]
async fn decision_missing_csrf_returns_403() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        false,
        Some(r#"{"sender":"x@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn decision_without_auth_returns_401() {
    let (state, _) = fixture_state().await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        None,
        true,
        Some(r#"{"sender":"x@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_decision_or_classify_as_returns_400() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let bad_decision = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"x@example.org","decision":"maybe","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(bad_decision.status(), StatusCode::BAD_REQUEST);
    let bad_classify = request(state, Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":"x@example.org","decision":"approve","classify_as":"other","apply_to_history":false}"#)).await;
    assert_eq!(bad_classify.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn apply_to_history_calls_fake_backfill_once() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let backfill = Arc::new(FakeBackfill::default());
    let resp = request_with_backfill(state, backfill.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":" Sender@Example.ORG ","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let calls = backfill.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        BackfillCall {
            user_id,
            sender: "sender@example.org".to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(Classification::Papertrail)
        }
    );
}

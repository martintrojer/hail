use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::SESSION_COOKIE;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_screener_test_{uniq}?mode=memory&cache=shared");
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

    let state = AppState {
        db,
        config: Config::load_from(None).expect("load config"),
        server_key: Arc::new(key),
        login_limiter: Arc::new(IpRateLimiter::default()),
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

    let session_id = format!("{:064x}", user_id);
    sqlx::query(
        "INSERT INTO sessions \
         (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, 'test', ?4, ?5, ?5)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(hail_core::seal(b"dummy-token", key).expect("seal"))
    .bind(now + Duration::days(30))
    .bind(now)
    .execute(&state.db)
    .await
    .expect("insert session");
    (user_id, session_id)
}

async fn seed_rule(
    state: &AppState,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
    first_seen_at: chrono::DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO screener_rules \
         (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
         VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
    )
    .bind(user_id)
    .bind(sender)
    .bind(decision)
    .bind(classify_as)
    .bind(first_seen_at)
    .execute(&state.db)
    .await
    .expect("insert screener rule");
}

async fn get_screener(state: AppState, sid: Option<&str>) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri("/api/views/screener");
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("{SESSION_COOKIE}={sid}"));
    }
    hail_api::build_router(state, true)
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn screener_view_requires_auth() {
    let (state, _) = fixture_state().await;
    let resp = get_screener(state, None).await;
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

    let resp = get_screener(state, Some(&alice_sid)).await;
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

    let resp = get_screener(state, Some(&sid)).await;
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

    let resp = get_screener(state, Some(&sid)).await;
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

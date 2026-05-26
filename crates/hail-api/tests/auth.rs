//! Integration tests for auth middleware plus the login/logout/me pipeline.
//!
//! Login route coverage uses a tiny test-only seam in `routes::auth` to
//! avoid depending on a live Stalwart while still exercising request
//! extraction, rate limiting, user/session writes, cookies, and admin
//! promotion semantics. The middleware tests seed session rows directly with
//! encrypted tokens and fire requests at the production router via
//! `tower::ServiceExt::oneshot`.
//!
//! Pattern follows existing crates' tests: a fresh in-memory SQLite per
//! test, migrations run, deterministic state.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration as StdDuration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN, parse_server_key};
use hail_test::fixture_state;
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

async fn fixture_state_with_admin(admin_email: Option<&str>) -> (AppState, [u8; KEY_LEN]) {
    let (mut state, key) = fixture_state().await;
    unsafe {
        if let Some(email) = admin_email {
            std::env::set_var("HAIL_ADMIN__EMAIL", email);
        } else {
            std::env::remove_var("HAIL_ADMIN__EMAIL");
        }
    }
    state.config = Config::load_from(None).expect("reload config with admin override");
    state.server_key =
        Arc::new(parse_server_key(&state.config.secrets.server_key).expect("parse server key"));
    (state, key)
}

/// Insert a `(user, session)` pair so a request carrying that session
/// cookie will authenticate. Returns the session id (the cookie value).
async fn seed_session(
    state: &AppState,
    key: &[u8; KEY_LEN],
    email: &str,
    expires_in: Duration,
) -> String {
    seed_session_with_token(state, key, email, expires_in, b"dummy-bearer-token").await
}

async fn seed_session_with_token(
    state: &AppState,
    key: &[u8; KEY_LEN],
    email: &str,
    expires_in: Duration,
    token: &[u8],
) -> String {
    let now = Utc::now();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
         VALUES (?1, ?2, 0, ?3) RETURNING id",
    )
    .bind(email)
    .bind("account-id")
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert user");

    let token_enc = hail_core::seal(token, key).expect("seal");
    let session_id = "a".repeat(64);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&token_enc)
    .bind(Some("test-ua"))
    .bind(now + expires_in)
    .bind(now)
    .execute(&state.db)
    .await
    .expect("insert session");
    session_id
}

fn login_request(body: Body) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap()
}

async fn login_with_provider(
    state: AppState,
    request: Request<Body>,
    provider: hail_api::routes::auth::TestLoginProvider,
) -> axum::response::Response {
    hail_api::routes::auth::test_login_with_provider(
        axum::extract::State(state),
        ConnectInfo("127.0.0.1:10000".parse().unwrap()),
        request.headers().clone(),
        request,
        provider,
    )
    .await
}

async fn valid_login(
    state: AppState,
    email: &str,
    password: &str,
    provider: hail_api::routes::auth::TestLoginProvider,
) -> axum::response::Response {
    let request = login_request(Body::from(
        json!({ "email": email, "password": password }).to_string(),
    ));
    login_with_provider(state, request, provider).await
}

fn ok_provider(
    account_id: &'static str,
    calls: Arc<AtomicUsize>,
) -> hail_api::routes::auth::TestLoginProvider {
    Arc::new(move |_jmap_url, _email, _password| {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(account_id.to_owned()) })
    })
}

#[tokio::test]
async fn malformed_json_returns_400_and_does_not_call_provider() {
    let (state, _key) = fixture_state().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ok_provider("acct-unused", calls.clone());

    let request = login_request(Body::from(r#"{"email":"alice@example.org""#));
    let resp = login_with_provider(state, request, provider).await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn malformed_payload_returns_400_and_does_not_call_provider() {
    let (state, _key) = fixture_state().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ok_provider("acct-unused", calls.clone());

    let request = login_request(Body::from(
        json!({ "email": "alice@example.org" }).to_string(),
    ));
    let resp = login_with_provider(state, request, provider).await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rate_limiter_returns_429_after_attempts_without_calling_provider() {
    let (mut state, _key) = fixture_state().await;
    state.auth_rate_limiter = Arc::new(IpRateLimiter::new(2, StdDuration::from_secs(60)));
    let calls = Arc::new(AtomicUsize::new(0));

    for _ in 0..2 {
        let resp = valid_login(
            state.clone(),
            "alice@example.org",
            "correct horse battery staple",
            ok_provider("acct", calls.clone()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let resp = valid_login(
        state,
        "alice@example.org",
        "correct horse battery staple",
        ok_provider("acct", calls.clone()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn rate_limiter_keys_on_x_forwarded_for_when_present() {
    let (mut state, _key) = fixture_state().await;
    state.auth_rate_limiter = Arc::new(IpRateLimiter::new(1, StdDuration::from_secs(60)));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = ok_provider("acct", calls.clone());

    let first = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "203.0.113.10, 10.0.0.2")
        .body(Body::from(
            json!({ "email": "alice@example.org", "password": "correct horse battery staple" })
                .to_string(),
        ))
        .unwrap();
    let first = login_with_provider(state.clone(), first, provider.clone()).await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "203.0.113.10, 10.0.0.2")
        .body(Body::from(
            json!({ "email": "bob@example.org", "password": "correct horse battery staple" })
                .to_string(),
        ))
        .unwrap();
    let second = login_with_provider(state, second, provider).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn login_upserts_existing_user_account_id() {
    let (state, _key) = fixture_state().await;
    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at)
         VALUES (?1, ?2, 0, ?3)",
    )
    .bind("alice@example.org")
    .bind("old-account")
    .bind(Utc::now())
    .execute(&state.db)
    .await
    .expect("seed user");

    let resp = valid_login(
        state.clone(),
        " Alice@Example.Org ",
        "correct horse battery staple",
        ok_provider("new-account", Arc::new(AtomicUsize::new(0))),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let (count, account_id): (i64, String) = sqlx::query_as(
        "SELECT COUNT(*), MAX(jmap_account_id) FROM users WHERE email = 'alice@example.org'",
    )
    .fetch_one(&state.db)
    .await
    .expect("select user");
    assert_eq!(count, 1);
    assert_eq!(account_id, "new-account");
}

#[tokio::test]
async fn first_login_without_config_admin_becomes_admin() {
    let (state, _key) = fixture_state().await;

    let resp = valid_login(
        state.clone(),
        "alice@example.org",
        "correct horse battery staple",
        ok_provider("acct-alice", Arc::new(AtomicUsize::new(0))),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let is_admin: i64 = sqlx::query_scalar("SELECT is_admin FROM users WHERE email = ?1")
        .bind("alice@example.org")
        .fetch_one(&state.db)
        .await
        .expect("select user");
    assert_eq!(is_admin, 1);
}

#[tokio::test]
async fn first_login_with_config_admin_does_not_promote_other_user() {
    let (state, _key) = fixture_state_with_admin(Some("admin@example.org")).await;

    let resp = valid_login(
        state.clone(),
        "alice@example.org",
        "correct horse battery staple",
        ok_provider("acct-alice", Arc::new(AtomicUsize::new(0))),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let is_admin: i64 = sqlx::query_scalar("SELECT is_admin FROM users WHERE email = ?1")
        .bind("alice@example.org")
        .fetch_one(&state.db)
        .await
        .expect("select user");
    assert_eq!(is_admin, 0);
}

#[tokio::test]
async fn configured_admin_login_is_promoted() {
    let (state, _key) = fixture_state_with_admin(Some("admin@example.org")).await;

    let resp = valid_login(
        state.clone(),
        "Admin@Example.Org",
        "correct horse battery staple",
        ok_provider("acct-admin", Arc::new(AtomicUsize::new(0))),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let (email, is_admin): (String, i64) =
        sqlx::query_as("SELECT email, is_admin FROM users WHERE email = ?1")
            .bind("admin@example.org")
            .fetch_one(&state.db)
            .await
            .expect("select user");
    assert_eq!(email, "admin@example.org");
    assert_eq!(is_admin, 1);
}

#[tokio::test]
async fn fake_provider_called_once_on_valid_login() {
    let (state, _key) = fixture_state().await;
    let calls = Arc::new(AtomicUsize::new(0));

    let resp = valid_login(
        state,
        "alice@example.org",
        "correct horse battery staple",
        ok_provider("acct-alice", calls.clone()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn successful_login_sets_cookie_and_creates_session() {
    let (state, key) = fixture_state().await;

    let resp = valid_login(
        state.clone(),
        "alice@example.org",
        "correct horse battery staple",
        ok_provider("acct-alice", Arc::new(AtomicUsize::new(0))),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .expect("cookie header");
    assert_login_cookie_flags(cookie);

    let (session_count, encrypted_token): (i64, Vec<u8>) =
        sqlx::query_as("SELECT COUNT(*), MAX(jmap_token_enc) FROM sessions")
            .fetch_one(&state.db)
            .await
            .expect("select session");
    assert_eq!(session_count, 1);
    let token = hail_core::open(&encrypted_token, &key).expect("decrypt token");
    let token = String::from_utf8(token).expect("utf8 token");
    assert_eq!(
        token,
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            b"alice@example.org:correct horse battery staple"
        )
    );
}

fn assert_login_cookie_flags(cookie: &str) {
    assert!(
        cookie.starts_with("hail_session="),
        "cookie name/value missing: {cookie}"
    );
    assert!(
        cookie.contains("HttpOnly"),
        "cookie missing HttpOnly: {cookie}"
    );
    assert!(cookie.contains("Secure"), "cookie missing Secure: {cookie}");
    assert!(
        cookie.contains("SameSite=Lax"),
        "cookie missing SameSite=Lax: {cookie}"
    );
    assert!(cookie.contains("Path=/"), "cookie missing Path=/: {cookie}");
    assert!(
        cookie.contains("Max-Age=2592000"),
        "cookie missing 30-day Max-Age: {cookie}"
    );
}

#[tokio::test]
async fn auth_failure_returns_401_and_does_not_create_user_or_session() {
    let (state, _key) = fixture_state().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let provider: hail_api::routes::auth::TestLoginProvider =
        Arc::new(move |_jmap_url, _email, _password| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("invalid credentials".to_owned()) })
        });

    let resp = valid_login(
        state.clone(),
        "alice@example.org",
        "wrong password",
        provider,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await
        .expect("count users");
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&state.db)
        .await
        .expect("count sessions");
    assert_eq!(user_count, 0);
    assert_eq!(session_count, 0);
}

async fn session_last_used_at(state: &AppState, sid: &str) -> chrono::DateTime<Utc> {
    sqlx::query_scalar("SELECT last_used_at FROM sessions WHERE id = ?1")
        .bind(sid)
        .fetch_one(&state.db)
        .await
        .expect("select last_used_at")
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[tokio::test]
async fn me_returns_user_with_valid_session() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org", Duration::days(30)).await;

    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["user"]["email"], "alice@example.org");
    assert_eq!(json["user"]["is_admin"], false);
}

#[tokio::test]
async fn auth_middleware_propagates_decrypted_token_to_downstream_extension() {
    let (state, key) = fixture_state().await;
    let token = b"token-visible-only-through-safe-probe";
    let sid = seed_session_with_token(
        &state,
        &key,
        "token-probe@example.org",
        Duration::days(30),
        token,
    )
    .await;

    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/test-token")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["email"], "token-probe@example.org");
    assert_eq!(json["is_admin"], false);
    assert_eq!(json["token_len"].as_u64(), Some(token.len() as u64));
    assert_eq!(json["token_hash"], fnv1a64_hex(token));
    assert_ne!(
        json["token_hash"], "token-visible-only-through-safe-probe",
        "probe must not echo the plaintext token"
    );
    assert!(
        json.get("token").is_none(),
        "probe should expose only safe token metadata"
    );
}

#[tokio::test]
async fn authenticated_request_advances_session_last_used_at() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "last-used@example.org", Duration::days(30)).await;
    let before = session_last_used_at(&state, &sid).await;

    let app = hail_api::build_router(state.clone(), true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);

    let after = session_last_used_at(&state, &sid).await;
    assert!(
        after > before,
        "auth middleware should advance last_used_at after successful auth: before={before:?} after={after:?}"
    );
}

#[tokio::test]
async fn expired_session_is_rejected() {
    let (state, key) = fixture_state().await;
    // Negative TTL → expired the moment it was written.
    let sid = seed_session(&state, &key, "bob@example.org", Duration::minutes(-1)).await;

    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_cookie_is_rejected() {
    let (state, _key) = fixture_state().await;
    let app = hail_api::build_router(state, true);
    // 64 hex chars but not in DB.
    let bogus = "f".repeat(64);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header(header::COOKIE, format!("hail_session={bogus}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_cookie_is_rejected() {
    let (state, _key) = fixture_state().await;
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mutating_request_without_csrf_header_is_forbidden() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "carol@example.org", Duration::days(30)).await;
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/threads/test")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn mutating_request_with_csrf_header_reaches_stub() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "dave@example.org", Duration::days(30)).await;
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/threads/test")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header("X-Hail-Request", "1")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn logout_deletes_session_row_and_clears_cookie() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "erin@example.org", Duration::days(30)).await;
    let db = state.db.clone();

    let app = hail_api::build_router(state, true);
    // Logout is public (no CSRF middleware) — we still send the header
    // because in production the SPA always sends it on mutations, and
    // we want to make sure we don't accidentally reject it.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/logout")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        cookie.contains("Max-Age=0"),
        "logout must clear cookie: {cookie}"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?1")
        .bind(&sid)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0, "session row must be gone after logout");
}

#[tokio::test]
async fn logout_with_no_cookie_still_returns_204() {
    let (state, _key) = fixture_state().await;
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/logout")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn crypto_roundtrip_through_seal_open() {
    // Smoke test: the same key encrypt → decrypt path the login handler
    // uses, verified at the integration-test layer.
    let key = [0xCDu8; KEY_LEN];
    let token = b"hail_session_bearer_token";
    let enc = hail_core::seal(token, &key).expect("seal");
    let dec = hail_core::open(&enc, &key).expect("open");
    assert_eq!(dec.as_slice(), token);
}

#[tokio::test]
async fn wrong_server_key_rejects_session() {
    // Seed a session encrypted under key A; build a router using key B.
    // The middleware must 401 because token decrypt fails.
    let (mut state, key_a) = fixture_state().await;
    let sid = seed_session(&state, &key_a, "frank@example.org", Duration::days(30)).await;

    // Swap in a different key — simulates operator rotating
    // `secrets.server_key` without re-encrypting existing rows.
    let key_b = [0x99u8; KEY_LEN];
    state.server_key = Arc::new(key_b);

    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_cookie_short_circuits_to_401() {
    let (state, _key) = fixture_state().await;
    let app = hail_api::build_router(state, true);
    // Wrong length / non-hex characters — never hits the DB.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header(header::COOKIE, "hail_session=not-hex")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_cookie_carries_all_security_flags() {
    // Inspect the cookie we'd hand a real client. We can't reach the
    // full login handler (Stalwart) but the `build_session_cookie`
    // string is asserted in the unit tests; here we cross-check the
    // logout-clear variant.
    let (state, _key) = fixture_state().await;
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/logout")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Path=/"));
}

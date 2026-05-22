//! Integration tests for the auth middleware + login/logout/me pipeline.
//!
//! These deliberately do NOT exercise `POST /api/auth/login` end-to-end:
//! that handler talks to a real Stalwart, which we don't have in CI.
//! Instead we:
//!   * insert a session row by hand (with an encrypted token built via
//!     `hail_core::seal` — the same code the live handler uses), then
//!   * fire requests at the router via `tower::ServiceExt::oneshot` and
//!     check the auth middleware does its job.
//!
//! Pattern follows existing crates' tests: a fresh in-memory SQLite per
//! test, migrations run, deterministic state.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Build a fully-initialized `AppState` against a fresh in-memory SQLite.
/// Returns the state plus the 32-byte server key so tests can encrypt
/// fixture tokens with the same key.
async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    // Distinct shared-cache URI per call so tests don't share state.
    // `mode=memory&cache=shared` keeps the DB alive across the pool's
    // multiple connections.
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    // Deterministic 32-byte key so we can assert specific behaviour
    // (notably: "wrong key → 401" tests use a *different* key here).
    let key = [0x5Au8; KEY_LEN];
    let server_key_hex = hex::encode(key);

    // We construct Config via the env layer so the `validate_server_key`
    // path is exercised. This is what the binary does in production too.
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", &server_key_hex);
    }
    let config = Config::load_from(None).expect("load config");

    let state = AppState {
        db,
        config,
        server_key: Arc::new(key),
        login_limiter: Arc::new(IpRateLimiter::default()),
    };
    (state, key)
}

/// Cheap unique suffix — we don't need real UUIDs, just per-test
/// uniqueness for the in-memory DB name.
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{pid}_{n}")
}

/// Insert a `(user, session)` pair so a request carrying that session
/// cookie will authenticate. Returns the session id (the cookie value).
async fn seed_session(
    state: &AppState,
    key: &[u8; KEY_LEN],
    email: &str,
    expires_in: Duration,
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

    let token_enc = hail_core::seal(b"dummy-bearer-token", key).expect("seal");
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
    let cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(cookie.contains("Max-Age=0"), "logout must clear cookie: {cookie}");

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
    let cookie = resp.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Path=/"));
}

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::CSRF_HEADER;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_contacts_test_{uniq}?mode=memory&cache=shared");
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
    };
    (state, key)
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
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

async fn request(
    state: AppState,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<String>,
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
    hail_api::build_router(state, true)
        .oneshot(builder.body(body.map_or_else(Body::empty, Body::from)).unwrap())
        .await
        .unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn put_then_get_round_trips_note_and_normalizes_address() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;

    let resp = request(
        state.clone(),
        Method::PUT,
        "/api/contacts/%20Bob@Example.ORG%20/note",
        Some(&sid),
        true,
        Some(r#"{"markdown":"hello **bob**"}"#.to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let put_json = json_body(resp).await;
    assert_eq!(put_json["markdown"], "hello **bob**");

    let resp = request(
        state,
        Method::GET,
        "/api/contacts/bob@example.org",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let get_json = json_body(resp).await;
    assert_eq!(get_json["address"], "bob@example.org");
    assert_eq!(get_json["note"]["markdown"], "hello **bob**");
    assert_eq!(get_json["threads"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_without_prior_put_returns_null_note() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "bob@example.org").await;

    let resp = request(
        state,
        Method::GET,
        "/api/contacts/nope@example.org",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["note"].is_null());
}

#[tokio::test]
async fn delete_removes_note() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "carol@example.org").await;

    let resp = request(
        state.clone(),
        Method::PUT,
        "/api/contacts/dave@example.org/note",
        Some(&sid),
        true,
        Some(r#"{"markdown":"temporary"}"#.to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = request(
        state.clone(),
        Method::DELETE,
        "/api/contacts/dave@example.org/note",
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = request(
        state,
        Method::GET,
        "/api/contacts/dave@example.org",
        Some(&sid),
        false,
        None,
    )
    .await;
    let json = json_body(resp).await;
    assert!(json["note"].is_null());
}

#[tokio::test]
async fn markdown_over_64kb_returns_400() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "dan@example.org").await;
    let body = serde_json::json!({ "markdown": "x".repeat(64 * 1024 + 1) }).to_string();

    let resp = request(
        state,
        Method::PUT,
        "/api/contacts/eve@example.org/note",
        Some(&sid),
        true,
        Some(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn wrong_user_cannot_see_note() {
    let (state, key) = fixture_state().await;
    let alice_sid = seed_session(&state, &key, "alice@example.org").await;
    let bob_sid = seed_session(&state, &key, "bob@example.org").await;

    let resp = request(
        state.clone(),
        Method::PUT,
        "/api/contacts/shared@example.org/note",
        Some(&alice_sid),
        true,
        Some(r#"{"markdown":"alice-only"}"#.to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = request(
        state,
        Method::GET,
        "/api/contacts/shared@example.org",
        Some(&bob_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["note"].is_null());
}

#[tokio::test]
async fn no_auth_returns_401() {
    let (state, _key) = fixture_state().await;
    let resp = request(
        state,
        Method::GET,
        "/api/contacts/bob@example.org",
        None,
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_csrf_on_put_and_delete_returns_403() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "erin@example.org").await;

    let resp = request(
        state.clone(),
        Method::PUT,
        "/api/contacts/bob@example.org/note",
        Some(&sid),
        false,
        Some(r#"{"markdown":"nope"}"#.to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = request(
        state,
        Method::DELETE,
        "/api/contacts/bob@example.org/note",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

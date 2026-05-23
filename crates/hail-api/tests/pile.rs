use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, TimeZone, Utc};
use hail_api::middleware::auth::require_auth;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_pile_test_{uniq}?mode=memory&cache=shared");
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

async fn insert_stack_row(
    state: &AppState,
    user_id: i64,
    stack: &str,
    thread_id: &str,
    position: i64,
) {
    let added_at =
        Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap() + Duration::seconds(position);
    sqlx::query(
        "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(user_id)
    .bind(stack)
    .bind(thread_id)
    .bind(position)
    .bind(added_at)
    .execute(&state.db)
    .await
    .expect("insert stack row");
}

fn app(state: AppState) -> Router {
    let protected = hail_api::routes::pile::router().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));
    Router::new().merge(protected).with_state(state)
}

async fn get_view(state: AppState, sid: Option<&str>, path: &str) -> axum::response::Response {
    let mut builder = Request::builder().method(Method::GET).uri(path);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    let req = builder.body(Body::empty()).unwrap();
    app(state).oneshot(req).await.unwrap()
}

async fn response_json(resp: axum::response::Response) -> Value {
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn auth_required_returns_401() {
    let (state, _key) = fixture_state().await;

    let resp = get_view(state, None, "/api/views/set-aside").await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn set_aside_returns_only_current_user_set_aside_sorted_by_position() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _bob_sid) = seed_session(&state, &key, "bob@example.org").await;
    insert_stack_row(&state, alice_id, "set_aside", "alice-third", 30).await;
    insert_stack_row(&state, alice_id, "reply_later", "alice-reply", 10).await;
    insert_stack_row(&state, bob_id, "set_aside", "bob-hidden", 5).await;
    insert_stack_row(&state, alice_id, "set_aside", "alice-first", 10).await;
    insert_stack_row(&state, alice_id, "set_aside", "alice-second", 20).await;

    let resp = get_view(state, Some(&alice_sid), "/api/views/set-aside").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    assert_eq!(json["items"].as_array().unwrap().len(), 3);
    assert_eq!(json["items"][0]["thread_id"], "alice-first");
    assert_eq!(json["items"][0]["position"], 10);
    assert!(json["items"][0]["added_at"].as_str().is_some());
    assert!(json["items"][0]["preview"].is_null());
    assert_eq!(json["items"][1]["thread_id"], "alice-second");
    assert_eq!(json["items"][2]["thread_id"], "alice-third");
}

#[tokio::test]
async fn reply_later_returns_only_reply_later_rows_sorted_by_position() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "carol@example.org").await;
    insert_stack_row(&state, user_id, "reply_later", "reply-late", 20).await;
    insert_stack_row(&state, user_id, "set_aside", "set-aside-hidden", 1).await;
    insert_stack_row(&state, user_id, "reply_later", "reply-early", 10).await;

    let resp = get_view(state, Some(&sid), "/api/views/reply-later").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    assert_eq!(json["items"][0]["thread_id"], "reply-early");
    assert_eq!(json["items"][0]["position"], 10);
    assert!(json["items"][0]["preview"].is_null());
    assert_eq!(json["items"][1]["thread_id"], "reply-late");
    assert_eq!(json["items"][1]["position"], 20);
}

#[tokio::test]
async fn wrong_user_isolation() {
    let (state, key) = fixture_state().await;
    let (alice_id, _alice_sid) = seed_session(&state, &key, "dana@example.org").await;
    let (bob_id, bob_sid) = seed_session(&state, &key, "erin@example.org").await;
    insert_stack_row(&state, alice_id, "reply_later", "alice-hidden", 1).await;
    insert_stack_row(&state, bob_id, "reply_later", "bob-visible", 2).await;

    let resp = get_view(state, Some(&bob_sid), "/api/views/reply-later").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
    assert_eq!(json["items"][0]["thread_id"], "bob-visible");
}

#[tokio::test]
async fn empty_list_returns_empty_items() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "frank@example.org").await;

    let resp = get_view(state, Some(&sid), "/api/views/set-aside").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    assert_eq!(json, serde_json::json!({ "items": [] }));
}

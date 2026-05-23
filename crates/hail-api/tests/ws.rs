use std::sync::Arc;

use chrono::{Duration, Utc};
use futures_util::TryStreamExt;
use hail_api::events::AppEvent;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::StatusCode;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_ws_test_{uniq}?mode=memory&cache=shared");
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

    (
        AppState {
            db,
            config,
            server_key: Arc::new(key),
            login_limiter: Arc::new(IpRateLimiter::default()),
            events: hail_api::events::AppEventBus::default(),
        },
        key,
    )
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{pid}_{n}")
}

async fn seed_session(state: &AppState, key: &[u8; KEY_LEN], email: &str) -> String {
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

async fn spawn_server(state: AppState) -> String {
    let app = hail_api::build_router(state, true);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .expect("serve");
    });
    format!("ws://{addr}/api/ws")
}

#[tokio::test]
async fn websocket_requires_auth() {
    let (state, _key) = fixture_state().await;
    let url = spawn_server(state).await;

    let err = tokio_tungstenite::connect_async(&url)
        .await
        .expect_err("unauthenticated handshake should fail");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        other => panic!("expected HTTP 401 handshake error, got {other:?}"),
    }
}

#[tokio::test]
async fn websocket_handshake_succeeds_with_valid_session() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
    let url = spawn_server(state).await;

    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "cookie",
        format!("hail_session={sid}")
            .parse()
            .expect("cookie header"),
    );
    req.headers_mut()
        .insert("origin", "http://localhost".parse().expect("origin header"));
    let (_socket, response) = tokio_tungstenite::connect_async(req)
        .await
        .expect("authenticated websocket connects");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
}

#[tokio::test]
async fn websocket_rejects_authenticated_request_without_origin() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "missing-origin@example.org").await;
    let url = spawn_server(state).await;

    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "cookie",
        format!("hail_session={sid}")
            .parse()
            .expect("cookie header"),
    );

    let err = tokio_tungstenite::connect_async(req)
        .await
        .expect_err("missing Origin should fail");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        other => panic!("expected HTTP 403 handshake error, got {other:?}"),
    }
}

#[tokio::test]
async fn websocket_rejects_authenticated_request_with_foreign_origin() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "foreign-origin@example.org").await;
    let url = spawn_server(state).await;

    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "cookie",
        format!("hail_session={sid}")
            .parse()
            .expect("cookie header"),
    );
    req.headers_mut().insert(
        "origin",
        "https://evil.example".parse().expect("origin header"),
    );

    let err = tokio_tungstenite::connect_async(req)
        .await
        .expect_err("foreign Origin should fail");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        other => panic!("expected HTTP 403 handshake error, got {other:?}"),
    }
}

#[tokio::test]
async fn websocket_receives_broadcast_event() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "bob@example.org").await;
    let sender = state.events.clone();
    let url = spawn_server(state).await;

    let mut req = url.into_client_request().expect("ws request");
    req.headers_mut().insert(
        "cookie",
        format!("hail_session={sid}")
            .parse()
            .expect("cookie header"),
    );
    req.headers_mut()
        .insert("origin", "http://localhost".parse().expect("origin header"));
    let (mut socket, _response) = tokio_tungstenite::connect_async(req)
        .await
        .expect("authenticated websocket connects");

    sender.publish(AppEvent::ImboxNew).expect("publish event");

    let json: serde_json::Value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let message = socket.try_next().await.expect("websocket read");
            if let Some(tokio_tungstenite::tungstenite::Message::Text(text)) = message {
                let json: serde_json::Value = serde_json::from_str(&text).expect("event JSON");
                if json["type"] == "imbox.new" {
                    break json;
                }
            }
        }
    })
    .await
    .expect("event within timeout");

    assert_eq!(json, serde_json::json!({ "type": "imbox.new" }));
}

#[tokio::test]
async fn websocket_receives_db_bridged_worker_event_for_same_user_only() {
    let (state, key) = fixture_state().await;
    let alice_sid = seed_session(&state, &key, "alice-bridge@example.org").await;
    let bob_sid = seed_session(&state, &key, "bob-bridge@example.org").await;
    let alice_user_id: i64 = sqlx::query_scalar("SELECT user_id FROM sessions WHERE id = ?")
        .bind(&alice_sid)
        .fetch_one(&state.db)
        .await
        .expect("alice user id");

    let cancel = tokio_util::sync::CancellationToken::new();
    let bridge = hail_api::events::spawn_db_event_bridge(
        state.db.clone(),
        state.events.clone(),
        cancel.clone(),
    );
    let url = spawn_server(state.clone()).await;

    let mut alice_req = url.clone().into_client_request().expect("alice ws request");
    alice_req.headers_mut().insert(
        "cookie",
        format!("hail_session={alice_sid}")
            .parse()
            .expect("cookie header"),
    );
    alice_req
        .headers_mut()
        .insert("origin", "http://localhost".parse().expect("origin header"));
    let (mut alice_socket, _response) = tokio_tungstenite::connect_async(alice_req)
        .await
        .expect("alice websocket connects");

    let mut bob_req = url.into_client_request().expect("bob ws request");
    bob_req.headers_mut().insert(
        "cookie",
        format!("hail_session={bob_sid}")
            .parse()
            .expect("cookie header"),
    );
    bob_req
        .headers_mut()
        .insert("origin", "http://localhost".parse().expect("origin header"));
    let (mut bob_socket, _response) = tokio_tungstenite::connect_async(bob_req)
        .await
        .expect("bob websocket connects");

    hail_db::app_events::insert_app_event(&state.db, Some(alice_user_id), "screener.pending", "{}")
        .await
        .expect("insert app event");

    let json: serde_json::Value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let message = alice_socket.try_next().await.expect("websocket read");
            if let Some(tokio_tungstenite::tungstenite::Message::Text(text)) = message {
                let json: serde_json::Value = serde_json::from_str(&text).expect("event JSON");
                if json["type"] == "screener.pending" {
                    break json;
                }
            }
        }
    })
    .await
    .expect("db event within timeout");
    assert_eq!(json, serde_json::json!({ "type": "screener.pending" }));

    let bob_message =
        tokio::time::timeout(std::time::Duration::from_millis(250), bob_socket.try_next()).await;
    assert!(
        bob_message.is_err(),
        "bob must not receive alice-scoped app event"
    );

    cancel.cancel();
    bridge.await.expect("bridge task joins");
}

#[tokio::test]
async fn app_event_json_shape_is_stable() {
    assert_eq!(
        serde_json::to_value(AppEvent::ImboxNew).unwrap(),
        serde_json::json!({ "type": "imbox.new" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::FeedNew).unwrap(),
        serde_json::json!({ "type": "feed.new" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::PapertrailNew).unwrap(),
        serde_json::json!({ "type": "papertrail.new" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::ScreenerPending).unwrap(),
        serde_json::json!({ "type": "screener.pending" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::ThreadUpdated).unwrap(),
        serde_json::json!({ "type": "thread.updated" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::ThreadRemoved).unwrap(),
        serde_json::json!({ "type": "thread.removed" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::BubbleFired).unwrap(),
        serde_json::json!({ "type": "bubble.fired" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::SendCompleted).unwrap(),
        serde_json::json!({ "type": "send.completed" })
    );
    assert_eq!(
        serde_json::to_value(AppEvent::SendFailed).unwrap(),
        serde_json::json!({ "type": "send.failed" })
    );
}

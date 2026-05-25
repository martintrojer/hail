use chrono::{Duration, TimeZone, Utc};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::require_auth;
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use tower::ServiceExt;

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
    let json = json_body(resp).await;
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
    let json = json_body(resp).await;
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
    let json = json_body(resp).await;
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
    assert_eq!(json["items"][0]["thread_id"], "bob-visible");
}

#[tokio::test]
async fn empty_list_returns_empty_items() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "frank@example.org").await;

    let resp = get_view(state, Some(&sid), "/api/views/set-aside").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json, serde_json::json!({ "items": [] }));
}

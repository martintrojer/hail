use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::CSRF_HEADER;
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use tower::ServiceExt;

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
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, Body::from))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn valid_body(name: &str) -> String {
    serde_json::json!({
        "name": name,
        "conditions": [
            { "field": "from", "op": "contains", "value": "news@example.org" }
        ],
        "action": { "classify_as": "feed" }
    })
    .to_string()
}

#[tokio::test]
async fn create_list_get_update_delete_rule_round_trips() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/workflows",
        Some(&sid),
        true,
        Some(valid_body("Newsletters")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = json_body(resp).await;
    let id = created["rule"]["id"].as_i64().unwrap();
    assert_eq!(created["rule"]["name"], "Newsletters");
    assert_eq!(created["rule"]["enabled"], true);
    assert_eq!(created["rule"]["conditions"][0]["field"], "from");
    assert_eq!(created["rule"]["action"]["classify_as"], "feed");

    let resp = request(
        state.clone(),
        Method::GET,
        "/api/workflows",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = json_body(resp).await;
    assert_eq!(listed["rules"].as_array().unwrap().len(), 1);
    assert_eq!(listed["rules"][0]["id"], id);

    let resp = request(
        state.clone(),
        Method::GET,
        &format!("/api/workflows/{id}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["rule"]["name"], "Newsletters");

    let replacement = serde_json::json!({
        "name": "Receipts",
        "enabled": false,
        "conditions": [{ "field": "subject", "op": "contains", "value": "receipt" }],
        "action": { "classify_as": "papertrail", "add_label": "Receipts" }
    })
    .to_string();
    let resp = request(
        state.clone(),
        Method::PUT,
        &format!("/api/workflows/{id}"),
        Some(&sid),
        true,
        Some(replacement),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = json_body(resp).await;
    assert_eq!(updated["rule"]["name"], "Receipts");
    assert_eq!(updated["rule"]["enabled"], false);
    assert_eq!(updated["rule"]["conditions"][0]["field"], "subject");
    assert_eq!(updated["rule"]["action"]["classify_as"], "papertrail");
    assert_eq!(updated["rule"]["action"]["add_label"], "Receipts");

    let resp = request(
        state.clone(),
        Method::DELETE,
        &format!("/api/workflows/{id}"),
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = request(
        state,
        Method::GET,
        &format!("/api/workflows/{id}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rules_are_scoped_to_current_user() {
    let (state, key) = fixture_state().await;
    let (_alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (_bob_id, bob_sid) = seed_session(&state, &key, "bob@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/workflows",
        Some(&alice_sid),
        true,
        Some(valid_body("Alice only")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = json_body(resp).await["rule"]["id"].as_i64().unwrap();

    let resp = request(
        state.clone(),
        Method::GET,
        "/api/workflows",
        Some(&bob_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        json_body(resp).await["rules"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let resp = request(
        state,
        Method::DELETE,
        &format!("/api/workflows/{id}"),
        Some(&bob_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn validation_rejects_empty_conditions_and_actions() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let no_conditions = serde_json::json!({
        "name": "Bad",
        "conditions": [],
        "action": { "classify_as": "feed" }
    })
    .to_string();
    let resp = request(
        state.clone(),
        Method::POST,
        "/api/workflows",
        Some(&sid),
        true,
        Some(no_conditions),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "no_conditions");

    let no_action = serde_json::json!({
        "name": "Bad",
        "conditions": [{ "field": "from", "op": "equals", "value": "a@example.org" }],
        "action": {}
    })
    .to_string();
    let resp = request(
        state,
        Method::POST,
        "/api/workflows",
        Some(&sid),
        true,
        Some(no_action),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "no_action");
}

#[tokio::test]
async fn mutating_routes_require_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let resp = request(
        state,
        Method::POST,
        "/api/workflows",
        Some(&sid),
        false,
        Some(valid_body("No csrf")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn no_auth_returns_401() {
    let (state, _key) = fixture_state().await;

    let resp = request(state, Method::GET, "/api/workflows", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

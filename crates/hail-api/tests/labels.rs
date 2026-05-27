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

fn create_body(name: &str) -> String {
    serde_json::json!({ "name": name }).to_string()
}

#[tokio::test]
async fn create_list_rename_delete_round_trips_label() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "name": " Work / Receipts ", "color": "blue" }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = json_body(resp).await;
    let id = created["label"]["id"].as_i64().unwrap();
    assert_eq!(created["label"]["name"], "Work/Receipts");
    assert_eq!(created["label"]["leaf_name"], "Receipts");
    assert_eq!(
        created["label"]["path_segments"],
        serde_json::json!(["Work", "Receipts"])
    );
    assert_eq!(created["label"]["source"], "manual");
    assert_eq!(created["label"]["color"], "blue");
    assert_eq!(created["label"]["thread_count"], 0);

    let resp = request(
        state.clone(),
        Method::GET,
        "/api/labels",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = json_body(resp).await;
    assert_eq!(listed["labels"].as_array().unwrap().len(), 1);
    assert_eq!(listed["labels"][0]["id"], id);

    let resp = request(
        state.clone(),
        Method::PATCH,
        &format!("/api/labels/{id}"),
        Some(&sid),
        true,
        Some(create_body("Finance/Receipts")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let renamed = json_body(resp).await;
    assert_eq!(renamed["label"]["id"], id);
    assert_eq!(renamed["label"]["name"], "Finance/Receipts");
    assert_eq!(
        renamed["label"]["path_segments"],
        serde_json::json!(["Finance", "Receipts"])
    );

    let resp = request(
        state.clone(),
        Method::DELETE,
        &format!("/api/labels/{id}"),
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = request(state, Method::GET, "/api/labels", Some(&sid), false, None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        json_body(resp).await["labels"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn validation_and_duplicate_names_return_stable_errors() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        true,
        Some(create_body("Work//Receipts")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_label_name");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        true,
        Some("not json".to_owned()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_json");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        true,
        Some(create_body("Work/Receipts")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        true,
        Some(create_body(" work / receipts ")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "duplicate_label_name");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        true,
        Some(create_body("Travel")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let travel_id = json_body(resp).await["label"]["id"].as_i64().unwrap();

    let resp = request(
        state,
        Method::PATCH,
        &format!("/api/labels/{travel_id}"),
        Some(&sid),
        true,
        Some(create_body("WORK/RECEIPTS")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "duplicate_label_name");
}

#[tokio::test]
async fn labels_are_scoped_to_current_user() {
    let (state, key) = fixture_state().await;
    let (_alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (_bob_id, bob_sid) = seed_session(&state, &key, "bob@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&alice_sid),
        true,
        Some(create_body("Alice only")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = json_body(resp).await["label"]["id"].as_i64().unwrap();

    let resp = request(
        state.clone(),
        Method::GET,
        "/api/labels",
        Some(&bob_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        json_body(resp).await["labels"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let resp = request(
        state,
        Method::DELETE,
        &format!("/api/labels/{id}"),
        Some(&bob_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_cascades_thread_assignments() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "cascade@example.org").await;
    let label = hail_db::labels::create_label(&state.db, user_id, "Cascade", None)
        .await
        .expect("create label");
    assert!(
        hail_db::labels::assign_label_to_thread(&state.db, user_id, "thread-a", label.id)
            .await
            .expect("assign label")
    );

    let resp = request(
        state.clone(),
        Method::DELETE,
        &format!("/api/labels/{}", label.id),
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let assignments: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_labels WHERE label_id = ?1")
            .bind(label.id)
            .fetch_one(&state.db)
            .await
            .expect("count assignments");
    assert_eq!(assignments, 0);
}

#[tokio::test]
async fn not_found_and_invalid_ids_are_404s() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "notfound@example.org").await;

    let resp = request(
        state.clone(),
        Method::PATCH,
        "/api/labels/999",
        Some(&sid),
        true,
        Some(create_body("Missing")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(resp).await["error"], "label");

    let resp = request(
        state,
        Method::DELETE,
        "/api/labels/0",
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn no_auth_returns_401() {
    let (state, _key) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/labels", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mutating_routes_require_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "csrf@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/labels",
        Some(&sid),
        false,
        Some(create_body("No csrf")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let label = hail_db::labels::create_label(&state.db, _user_id, "Existing", None)
        .await
        .expect("create label");
    let resp = request(
        state,
        Method::PATCH,
        &format!("/api/labels/{}", label.id),
        Some(&sid),
        false,
        Some(create_body("Still no csrf")),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn openapi_contains_label_paths() {
    let (state, _key) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/openapi.json", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["paths"].get("/api/labels").is_some());
    assert!(json["paths"].get("/api/labels/{id}").is_some());
}

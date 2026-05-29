use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::IntoResponse;
use hail_api::middleware::auth::CSRF_HEADER;
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
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

struct JmapEmailFixture {
    id: &'static str,
    thread_id: &'static str,
    from_name: &'static str,
    from_email: &'static str,
    subject: &'static str,
    preview: &'static str,
}

async fn start_fake_jmap(emails: Vec<JmapEmailFixture>) -> (String, tokio::task::JoinHandle<()>) {
    let emails = Arc::new(
        emails
            .into_iter()
            .map(|email| (email.thread_id.to_owned(), email))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route("/.well-known/jmap", axum::routing::get(fake_jmap_session))
        .route("/jmap/", axum::routing::post(fake_jmap_api))
        .with_state(emails);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake jmap");
    let addr = listener.local_addr().expect("fake jmap addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .expect("fake jmap server");
    });
    (format!("http://{addr}"), handle)
}

async fn fake_jmap_session(
    State(_emails): State<Arc<HashMap<String, JmapEmailFixture>>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let base_url = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| format!("http://{host}"))
        .unwrap_or_else(|| "http://127.0.0.1:0".to_owned());
    axum::Json(serde_json::json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": {
                "maxSizeUpload": 50_000_000,
                "maxConcurrentUpload": 4,
                "maxSizeRequest": 10_000_000,
                "maxConcurrentRequests": 4,
                "maxCallsInRequest": 16,
                "maxObjectsInGet": 500,
                "maxObjectsInSet": 500,
                "collationAlgorithms": ["i;unicode-casemap"]
            },
            "urn:ietf:params:jmap:mail": {
                "maxMailboxesPerEmail": 16,
                "maxMailboxDepth": 10,
                "maxSizeMailboxName": 255,
                "maxSizeAttachmentsPerEmail": 50_000_000,
                "emailQuerySortOptions": ["receivedAt"],
                "mayCreateTopLevelMailbox": true
            }
        },
        "accounts": {
            "account-test": {
                "name": "Test Account",
                "isPersonal": true,
                "isReadOnly": false,
                "accountCapabilities": {
                    "urn:ietf:params:jmap:mail": {}
                }
            }
        },
        "primaryAccounts": {
            "urn:ietf:params:jmap:mail": "account-test"
        },
        "username": "labels@example.org",
        "apiUrl": format!("{base_url}/jmap/"),
        "downloadUrl": format!("{base_url}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
        "uploadUrl": format!("{base_url}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base_url}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": "fake-state"
    }))
}

async fn fake_jmap_api(
    State(emails): State<Arc<HashMap<String, JmapEmailFixture>>>,
    body: Bytes,
) -> impl IntoResponse {
    let request: Value = serde_json::from_slice(&body).expect("jmap request json");
    let call = request["methodCalls"]
        .as_array()
        .and_then(|calls| calls.first())
        .expect("single jmap method call");
    let method = call[0].as_str().expect("jmap method name");
    let tag = call[2].as_str().unwrap_or("s0");
    let response = match method {
        "Email/query" => {
            let requested_threads = collect_requested_threads(&call[1]);
            let ids = requested_threads
                .iter()
                .filter_map(|thread_id| emails.get(thread_id).map(|email| email.id))
                .collect::<Vec<_>>();
            serde_json::json!({
                "accountId": "account-test",
                "queryState": "fake-query-state",
                "canCalculateChanges": false,
                "position": 0,
                "total": ids.len(),
                "ids": ids
            })
        }
        "Email/get" => {
            let ids = call[1]["ids"]
                .as_array()
                .expect("Email/get ids")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let list = ids
                .iter()
                .filter_map(|id| emails.values().find(|email| email.id == *id))
                .map(|email| {
                    serde_json::json!({
                        "id": email.id,
                        "threadId": email.thread_id,
                        "from": [{
                            "name": email.from_name,
                            "email": email.from_email
                        }],
                        "subject": email.subject,
                        "preview": email.preview
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "accountId": "account-test",
                "state": "fake-get-state",
                "list": list,
                "notFound": []
            })
        }
        other => panic!("unexpected JMAP method {other}"),
    };
    axum::Json(serde_json::json!({
        "sessionState": "fake-state",
        "methodResponses": [[method, response, tag]]
    }))
}

fn collect_requested_threads(arguments: &Value) -> Vec<String> {
    fn walk(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(thread_id) = map.get("inThread").and_then(Value::as_str) {
                    out.push(thread_id.to_owned());
                }
                for child in map.values() {
                    walk(child, out);
                }
            }
            Value::Array(values) => {
                for child in values {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }

    let mut thread_ids = Vec::new();
    walk(
        arguments.get("filter").unwrap_or(arguments),
        &mut thread_ids,
    );
    thread_ids
}

fn create_body(name: &str) -> String {
    serde_json::json!({ "name": name }).to_string()
}

fn item_for_thread<'a>(items: &'a [Value], thread_id: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["thread_id"] == thread_id)
        .unwrap_or_else(|| panic!("{thread_id} in response"))
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
async fn label_threads_returns_current_user_assigned_threads_with_labels() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice-label-view@example.org").await;
    let (bob_id, _bob_sid) = seed_session(&state, &key, "bob-label-view@example.org").await;
    let label = hail_db::labels::create_label(&state.db, alice_id, "Work/Receipts", None)
        .await
        .expect("create alice label");
    let other_label = hail_db::labels::create_label(&state.db, alice_id, "Important", Some("red"))
        .await
        .expect("create other label");
    let bob_label = hail_db::labels::create_label(&state.db, bob_id, "Work/Receipts", None)
        .await
        .expect("create bob label");

    hail_db::labels::assign_label_to_thread(&state.db, alice_id, "thread-a", label.id)
        .await
        .expect("assign target label to thread-a");
    hail_db::labels::assign_label_to_thread(&state.db, alice_id, "thread-a", other_label.id)
        .await
        .expect("assign second label to thread-a");
    hail_db::labels::assign_label_to_thread(&state.db, alice_id, "thread-b", label.id)
        .await
        .expect("assign target label to thread-b");
    hail_db::labels::assign_label_to_thread(&state.db, bob_id, "thread-bob", bob_label.id)
        .await
        .expect("assign bob label");

    let resp = request(
        state,
        Method::GET,
        &format!("/api/labels/{}/threads?limit=10", label.id),
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["label"]["id"], label.id);
    assert_eq!(json["label"]["name"], "Work/Receipts");
    assert_eq!(json["label"]["thread_count"], 2);
    assert_eq!(json["next_cursor"], serde_json::Value::Null);

    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let returned_threads = items
        .iter()
        .map(|item| item["thread_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        returned_threads,
        std::collections::BTreeSet::from(["thread-a", "thread-b"])
    );
    assert!(!returned_threads.contains("thread-bob"));

    let thread_a = items
        .iter()
        .find(|item| item["thread_id"] == "thread-a")
        .expect("thread-a in response");
    let label_names = thread_a["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(label_names, vec!["Important", "Work/Receipts"]);
}

#[tokio::test]
async fn label_threads_hydrates_previews_from_jmap_and_keeps_missing_threads_empty() {
    let (mut state, key) = fixture_state().await;
    let (jmap_url, fake_jmap) = start_fake_jmap(vec![
        JmapEmailFixture {
            id: "email-alpha",
            thread_id: "thread-alpha",
            from_name: "Alice Sender",
            from_email: "alice.sender@example.org",
            subject: "Alpha fixture subject",
            preview: "Alpha fixture preview text",
        },
        JmapEmailFixture {
            id: "email-beta",
            thread_id: "thread-beta",
            from_name: "Bob Sender",
            from_email: "bob.sender@example.org",
            subject: "Beta fixture subject",
            preview: "Beta fixture preview text",
        },
    ])
    .await;
    state.config.stalwart.jmap_url = jmap_url;
    let (user_id, sid) = seed_session(&state, &key, "label-preview@example.org").await;
    let label = hail_db::labels::create_label(&state.db, user_id, "Hydrated", None)
        .await
        .expect("create label");
    let other_label = hail_db::labels::create_label(&state.db, user_id, "Pinned", Some("gold"))
        .await
        .expect("create other label");

    for thread_id in ["thread-missing", "thread-beta", "thread-alpha"] {
        hail_db::labels::assign_label_to_thread(&state.db, user_id, thread_id, label.id)
            .await
            .unwrap_or_else(|err| panic!("assign {thread_id}: {err}"));
    }
    hail_db::labels::assign_label_to_thread(&state.db, user_id, "thread-alpha", other_label.id)
        .await
        .expect("assign second label");

    let resp = request(
        state,
        Method::GET,
        &format!("/api/labels/{}/threads?limit=10", label.id),
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["label"]["id"], label.id);
    assert_eq!(json["label"]["thread_count"], 3);
    let items = json["items"].as_array().expect("label thread items");
    assert_eq!(items.len(), 3);

    let alpha = item_for_thread(items, "thread-alpha");
    assert_eq!(alpha["from"], "Alice Sender <alice.sender@example.org>");
    assert_eq!(alpha["subject"], "Alpha fixture subject");
    assert_eq!(alpha["preview"], "Alpha fixture preview text");
    assert!(!alpha["from"].as_str().unwrap().is_empty());
    assert!(!alpha["subject"].as_str().unwrap().is_empty());
    assert!(!alpha["preview"].as_str().unwrap().is_empty());
    let alpha_labels = alpha["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(alpha_labels, vec!["Hydrated", "Pinned"]);

    let beta = item_for_thread(items, "thread-beta");
    assert_eq!(beta["from"], "Bob Sender <bob.sender@example.org>");
    assert_eq!(beta["subject"], "Beta fixture subject");
    assert_eq!(beta["preview"], "Beta fixture preview text");
    assert!(!beta["from"].as_str().unwrap().is_empty());
    assert!(!beta["subject"].as_str().unwrap().is_empty());
    assert!(!beta["preview"].as_str().unwrap().is_empty());

    let missing = item_for_thread(items, "thread-missing");
    assert_eq!(missing["from"], "");
    assert_eq!(missing["subject"], "");
    assert_eq!(missing["preview"], "");
    assert_eq!(missing["labels"][0]["name"], "Hydrated");

    fake_jmap.abort();
}

#[tokio::test]
async fn label_threads_paginates_and_validates_label_ids() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "label-pages@example.org").await;
    let label = hail_db::labels::create_label(&state.db, user_id, "Pages", None)
        .await
        .expect("create label");
    hail_db::labels::assign_label_to_thread(&state.db, user_id, "thread-1", label.id)
        .await
        .expect("assign thread 1");
    hail_db::labels::assign_label_to_thread(&state.db, user_id, "thread-2", label.id)
        .await
        .expect("assign thread 2");

    let resp = request(
        state.clone(),
        Method::GET,
        &format!("/api/labels/{}/threads?limit=1", label.id),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let first_page = json_body(resp).await;
    assert_eq!(first_page["items"].as_array().unwrap().len(), 1);
    assert_eq!(first_page["next_cursor"], "1");

    let resp = request(
        state.clone(),
        Method::GET,
        &format!("/api/labels/{}/threads?cursor=1&limit=1", label.id),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let second_page = json_body(resp).await;
    assert_eq!(second_page["items"].as_array().unwrap().len(), 1);
    assert_eq!(second_page["next_cursor"], serde_json::Value::Null);

    let resp = request(
        state.clone(),
        Method::GET,
        &format!("/api/labels/{}/threads?cursor=oops", label.id),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_cursor");

    let resp = request(
        state.clone(),
        Method::GET,
        "/api/labels/0/threads",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = request(
        state,
        Method::GET,
        "/api/labels/999/threads",
        Some(&sid),
        false,
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
async fn assign_and_remove_thread_label_are_idempotent_and_user_scoped() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice-assign@example.org").await;
    let (bob_id, bob_sid) = seed_session(&state, &key, "bob-assign@example.org").await;
    let label = hail_db::labels::create_label(&state.db, alice_id, "Project", None)
        .await
        .expect("create alice label");
    let bob_label = hail_db::labels::create_label(&state.db, bob_id, "Project", None)
        .await
        .expect("create bob label");

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let assigned = json_body(resp).await;
    assert_eq!(assigned["label"]["id"], label.id);
    assert_eq!(assigned["label"]["name"], "Project");

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let alice_assignments: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1 AND thread_id = 'thread-1' AND label_id = ?2",
    )
    .bind(alice_id)
    .bind(label.id)
    .fetch_one(&state.db)
    .await
    .expect("count alice assignments");
    assert_eq!(alice_assignments, 1);

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        Some(&bob_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/thread-1/labels/{}", bob_label.id),
        Some(&bob_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = request(
        state.clone(),
        Method::DELETE,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = request(
        state.clone(),
        Method::DELETE,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1 AND thread_id = 'thread-1' AND label_id = ?2",
    )
    .bind(alice_id)
    .bind(label.id)
    .fetch_one(&state.db)
    .await
    .expect("count remaining alice assignments");
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn inline_create_upserts_existing_normalized_label_and_assigns() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "inline-label@example.org").await;
    let existing = hail_db::labels::create_label(&state.db, user_id, "Work/Receipts", Some("blue"))
        .await
        .expect("create existing label");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/thread-2/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "label_name": " work / receipts " }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let assigned = json_body(resp).await;
    assert_eq!(assigned["label"]["id"], existing.id);
    assert_eq!(assigned["label"]["name"], "Work/Receipts");
    assert_eq!(assigned["label"]["color"], "blue");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/thread-3/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "label_name": " Personal / Follow Up " }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let created = json_body(resp).await;
    let created_id = created["label"]["id"].as_i64().unwrap();
    assert_ne!(created_id, existing.id);
    assert_eq!(created["label"]["name"], "Personal/Follow Up");
    assert_eq!(created["label"]["source"], "manual");

    let labels = hail_db::labels::list_thread_labels(&state.db, user_id, "thread-3")
        .await
        .expect("list thread labels");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].id, created_id);
}

#[tokio::test]
async fn thread_label_assignment_validates_payload_auth_and_csrf() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "thread-label-guard@example.org").await;
    let label = hail_db::labels::create_label(&state.db, user_id, "Guarded", None)
        .await
        .expect("create label");

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/thread-1/labels/{}", label.id),
        None,
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = request(
        state.clone(),
        Method::POST,
        &format!("/api/threads/bad%20id/labels/{}", label.id),
        Some(&sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_thread_id");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/thread-1/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "label_name": "Bad//Name" }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_label_name");

    let resp = request(
        state,
        Method::POST,
        "/api/threads/thread-1/labels",
        Some(&sid),
        true,
        Some("not json".to_owned()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_json");
}

#[tokio::test]
async fn batch_assigns_existing_label_id_idempotently_and_user_scoped() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice-batch@example.org").await;
    let (bob_id, bob_sid) = seed_session(&state, &key, "bob-batch@example.org").await;
    let label = hail_db::labels::create_label(&state.db, alice_id, "Batch", None)
        .await
        .expect("create alice label");
    let bob_label = hail_db::labels::create_label(&state.db, bob_id, "Batch", None)
        .await
        .expect("create bob label");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&alice_sid),
        true,
        Some(
            serde_json::json!({
                "label_id": label.id,
                "thread_ids": ["thread-a", "thread-b", "thread-a"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let assigned = json_body(resp).await;
    assert_eq!(assigned["label"]["id"], label.id);
    assert_eq!(assigned["label"]["name"], "Batch");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&alice_sid),
        true,
        Some(
            serde_json::json!({
                "label_id": label.id,
                "thread_ids": ["thread-a", "thread-b"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let alice_assignments: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1 AND label_id = ?2",
    )
    .bind(alice_id)
    .bind(label.id)
    .fetch_one(&state.db)
    .await
    .expect("count alice batch assignments");
    assert_eq!(alice_assignments, 2);

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&bob_sid),
        true,
        Some(
            serde_json::json!({
                "label_id": label.id,
                "thread_ids": ["thread-a"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = request(
        state,
        Method::POST,
        "/api/threads/labels",
        Some(&bob_sid),
        true,
        Some(
            serde_json::json!({
                "label_id": bob_label.id,
                "thread_ids": ["thread-a"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn batch_inline_create_upserts_label_name_and_assigns_all_threads() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "batch-inline@example.org").await;
    let existing = hail_db::labels::create_label(&state.db, user_id, "Work/Receipts", Some("blue"))
        .await
        .expect("create existing label");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(
            serde_json::json!({
                "label_name": " work / receipts ",
                "thread_ids": ["thread-1", "thread-2"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let assigned = json_body(resp).await;
    assert_eq!(assigned["label"]["id"], existing.id);
    assert_eq!(assigned["label"]["name"], "Work/Receipts");
    assert_eq!(assigned["label"]["thread_count"], 2);

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(
            serde_json::json!({
                "label_name": "Personal / Follow Up",
                "thread_ids": ["thread-3", "thread-4", "thread-3"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let created = json_body(resp).await;
    let created_id = created["label"]["id"].as_i64().unwrap();
    assert_ne!(created_id, existing.id);
    assert_eq!(created["label"]["name"], "Personal/Follow Up");
    assert_eq!(created["label"]["source"], "manual");
    assert_eq!(created["label"]["thread_count"], 2);

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM thread_labels WHERE user_id = ?1 AND label_id = ?2",
    )
    .bind(user_id)
    .bind(created_id)
    .fetch_one(&state.db)
    .await
    .expect("count created batch assignments");
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn batch_assignment_validates_payload_auth_and_csrf() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "batch-guard@example.org").await;
    let label = hail_db::labels::create_label(&state.db, user_id, "Guarded Batch", None)
        .await
        .expect("create label");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        false,
        Some(serde_json::json!({ "label_id": label.id, "thread_ids": ["thread-1"] }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        None,
        true,
        Some(serde_json::json!({ "label_id": label.id, "thread_ids": ["thread-1"] }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
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
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "label_id": label.id, "thread_ids": [] }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "empty_thread_ids");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "label_id": label.id, "thread_ids": ["bad id"] }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_thread_id");

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(serde_json::json!({ "thread_ids": ["thread-1"] }).to_string()),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(resp).await["error"],
        "exactly_one_label_selector_required"
    );

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(
            serde_json::json!({
                "label_id": label.id,
                "label_name": "Guarded Batch",
                "thread_ids": ["thread-1"]
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(resp).await["error"],
        "exactly_one_label_selector_required"
    );

    let resp = request(
        state,
        Method::POST,
        "/api/threads/labels",
        Some(&sid),
        true,
        Some(
            serde_json::json!({ "label_name": "Bad//Name", "thread_ids": ["thread-1"] })
                .to_string(),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_label_name");
}

#[tokio::test]
async fn openapi_contains_label_paths() {
    let (state, _key) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/openapi.json", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["paths"].get("/api/labels").is_some());
    assert!(json["paths"].get("/api/labels/{id}").is_some());
    assert!(json["paths"].get("/api/labels/{id}/threads").is_some());
    assert!(
        json["paths"]
            .get("/api/threads/{thread_id}/labels/{label_id}")
            .is_some()
    );
    assert!(
        json["paths"]
            .get("/api/threads/{thread_id}/labels")
            .is_some()
    );
    assert!(json["paths"].get("/api/threads/labels").is_some());
}

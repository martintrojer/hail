use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
    response::IntoResponse,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use hail_api::{
    middleware::auth::{CSRF_HEADER, require_auth},
    routes::screener::{Classification, ScreenerBackfill, ScreenerBackfillError, ScreenerDecision},
    state::AppState,
};
use hail_test::{fixture_state, json_body, seed_session};
use serde_json::Value;
use tower::ServiceExt;

async fn seed_rule(
    state: &AppState,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
    first_seen_at: chrono::DateTime<Utc>,
) {
    seed_rule_with_decided_at(
        state,
        user_id,
        sender,
        decision,
        classify_as,
        first_seen_at,
        None,
    )
    .await;
}

async fn seed_rule_with_decided_at(
    state: &AppState,
    user_id: i64,
    sender: &str,
    decision: &str,
    classify_as: Option<&str>,
    first_seen_at: chrono::DateTime<Utc>,
    decided_at: Option<chrono::DateTime<Utc>>,
) {
    sqlx::query("INSERT INTO screener_rules (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
        .bind(user_id)
        .bind(sender)
        .bind(decision)
        .bind(classify_as)
        .bind(decided_at)
        .bind(first_seen_at)
        .execute(&state.db)
        .await
        .unwrap();
}

async fn set_latest_pending_received_at(
    state: &AppState,
    user_id: i64,
    sender: &str,
    latest_pending_received_at: chrono::DateTime<Utc>,
) {
    sqlx::query(
        "UPDATE screener_rules SET latest_pending_received_at = ?1 \
         WHERE user_id = ?2 AND sender_address = ?3",
    )
    .bind(latest_pending_received_at)
    .bind(user_id)
    .bind(sender)
    .execute(&state.db)
    .await
    .unwrap();
}

async fn seed_mail_account(state: &AppState, user_id: i64, email: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?1, ?2, 'gmail', 'gmail', ?2, ?3, ?4, 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z') RETURNING id",
    )
    .bind(user_id)
    .bind(format!("provider-{user_id}"))
    .bind(email)
    .bind(vec![7_u8; 32])
    .fetch_one(&state.db)
    .await
    .expect("insert mail account")
}

async fn seed_cached_message(
    state: &AppState,
    account_id: i64,
    backend_id: &str,
    from: &str,
    pinned: bool,
) {
    sqlx::query(
        "INSERT INTO messages \
         (account_id, backend_msg_id, thread_id, internal_date, from_addr, subject, preview, size_bytes, body_blob_id, body_text, inserted_at, accessed_at, pinned) \
         VALUES (?1, ?2, ?3, 1, ?4, 'subject', 'preview', 10, 'blob-id', 'body', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?5)",
    )
    .bind(account_id)
    .bind(backend_id)
    .bind(format!("thread-{backend_id}"))
    .bind(from)
    .bind(i64::from(pinned))
    .execute(&state.db)
    .await
    .expect("insert cached message");
}

async fn cached_pin(state: &AppState, backend_id: &str) -> i64 {
    sqlx::query_scalar("SELECT pinned FROM messages WHERE backend_msg_id = ?1")
        .bind(backend_id)
        .fetch_one(&state.db)
        .await
        .expect("cached pin")
}

fn app(state: AppState, backfill: Arc<FakeBackfill>) -> Router {
    let protected = hail_api::routes::screener::router_with_backfill(backfill).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

async fn request(
    state: AppState,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    request_with_backfill(
        state,
        Arc::new(FakeBackfill::default()),
        method,
        uri,
        sid,
        csrf,
        body,
    )
    .await
}

async fn request_with_backfill(
    state: AppState,
    backfill: Arc<FakeBackfill>,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
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
    app(state, backfill)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[derive(Clone)]
struct JmapEmailFixture {
    id: &'static str,
    sender: &'static str,
    subject: &'static str,
    preview: &'static str,
    text_body: Option<&'static str>,
    html_body: Option<&'static str>,
}

#[derive(Clone)]
struct OwnedJmapEmailFixture {
    id: String,
    sender: String,
    subject: String,
    preview: String,
    text_body: Option<String>,
    html_body: Option<String>,
}

impl From<JmapEmailFixture> for OwnedJmapEmailFixture {
    fn from(email: JmapEmailFixture) -> Self {
        Self {
            id: email.id.to_owned(),
            sender: email.sender.to_owned(),
            subject: email.subject.to_owned(),
            preview: email.preview.to_owned(),
            text_body: email.text_body.map(ToOwned::to_owned),
            html_body: email.html_body.map(ToOwned::to_owned),
        }
    }
}

async fn start_fake_screener_jmap(
    emails: Vec<JmapEmailFixture>,
) -> (String, tokio::task::JoinHandle<()>) {
    start_fake_screener_jmap_with_recorder(emails, Arc::new(Mutex::new(Vec::new()))).await
}

async fn start_fake_screener_jmap_with_recorder(
    emails: Vec<JmapEmailFixture>,
    requests: Arc<Mutex<Vec<Value>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    start_fake_screener_jmap_owned_with_recorder(
        emails.into_iter().map(Into::into).collect(),
        requests,
    )
    .await
}

async fn start_fake_screener_jmap_owned_with_recorder(
    emails: Vec<OwnedJmapEmailFixture>,
    requests: Arc<Mutex<Vec<Value>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = Arc::new(FakeJmapState { emails, requests });
    let app = Router::new()
        .route("/.well-known/jmap", axum::routing::get(fake_jmap_session))
        .route("/jmap/", axum::routing::post(fake_jmap_api))
        .with_state(state);
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

#[derive(Default)]
struct FakeJmapState {
    emails: Vec<OwnedJmapEmailFixture>,
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn fake_jmap_session(headers: axum::http::HeaderMap) -> impl IntoResponse {
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
        "username": "screener@example.org",
        "apiUrl": format!("{base_url}/jmap/"),
        "downloadUrl": format!("{base_url}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
        "uploadUrl": format!("{base_url}/upload/{{accountId}}/"),
        "eventSourceUrl": format!("{base_url}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": "fake-state"
    }))
}

async fn fake_jmap_api(
    axum::extract::State(state): axum::extract::State<Arc<FakeJmapState>>,
    body: Bytes,
) -> impl IntoResponse {
    let request: Value = serde_json::from_slice(&body).expect("jmap request json");
    state.requests.lock().unwrap().push(request.clone());
    let call = request["methodCalls"]
        .as_array()
        .and_then(|calls| calls.first())
        .expect("single jmap method call");
    let method = call[0].as_str().expect("jmap method name");
    let tag = call[2].as_str().unwrap_or("s0");
    let response = match method {
        "Mailbox/query" => serde_json::json!({
            "accountId": "account-test",
            "queryState": "fake-mailbox-query-state",
            "canCalculateChanges": false,
            "position": 0,
            "total": 1,
            "ids": ["mailbox-screener"]
        }),
        "Email/query" => {
            let sender = requested_sender(&call[1]);
            let ids = state
                .emails
                .iter()
                .filter(|email| Some(email.sender.as_str()) == sender.as_deref())
                .map(|email| email.id.as_str())
                .collect::<Vec<_>>();
            serde_json::json!({
                "accountId": "account-test",
                "queryState": "fake-email-query-state",
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
                .filter_map(|id| state.emails.iter().find(|email| email.id == *id))
                .map(email_json)
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

fn requested_sender(arguments: &Value) -> Option<String> {
    fn walk(value: &Value) -> Option<String> {
        match value {
            Value::Object(map) => {
                if let Some(sender) = map.get("from").and_then(Value::as_str) {
                    return Some(sender.to_owned());
                }
                map.values().find_map(walk)
            }
            Value::Array(values) => values.iter().find_map(walk),
            _ => None,
        }
    }

    walk(arguments.get("filter").unwrap_or(arguments))
}

fn email_json(email: &OwnedJmapEmailFixture) -> Value {
    let mut body_values = serde_json::Map::new();
    let mut text_body = Vec::new();
    let mut html_body = Vec::new();

    if let Some(text) = email.text_body.as_deref() {
        body_values.insert(
            "text-1".to_owned(),
            serde_json::json!({ "value": text, "isTruncated": false }),
        );
        text_body.push(serde_json::json!({
            "partId": "text-1",
            "type": "text/plain"
        }));
    }

    if let Some(html) = email.html_body.as_deref() {
        body_values.insert(
            "html-1".to_owned(),
            serde_json::json!({ "value": html, "isTruncated": false }),
        );
        html_body.push(serde_json::json!({
            "partId": "html-1",
            "type": "text/html"
        }));
    }

    serde_json::json!({
        "id": email.id,
        "threadId": format!("thread-{}", email.id),
        "from": [{
            "name": "Pending Sender",
            "email": email.sender
        }],
        "subject": email.subject,
        "preview": email.preview,
        "receivedAt": "2026-05-23T12:15:00Z",
        "textBody": text_body,
        "htmlBody": html_body,
        "bodyValues": body_values
    })
}

fn sender<'a>(senders: &'a [Value], address: &str) -> &'a Value {
    senders
        .iter()
        .find(|sender| sender["sender"] == address)
        .unwrap_or_else(|| panic!("sender {address} present"))
}

fn collapse_for_expected_preview(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BackfillCall {
    user_id: i64,
    sender: String,
    decision: ScreenerDecision,
    classify_as: Option<Classification>,
}

#[derive(Default)]
struct FakeBackfill {
    calls: Mutex<Vec<BackfillCall>>,
    fail: bool,
}

impl FakeBackfill {
    fn failing() -> Self {
        Self {
            calls: Mutex::default(),
            fail: true,
        }
    }
}

#[async_trait]
impl ScreenerBackfill for FakeBackfill {
    async fn apply(
        &self,
        _state: &AppState,
        user: &hail_api::middleware::auth::AuthUser,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Classification>,
    ) -> Result<(), ScreenerBackfillError> {
        self.calls.lock().unwrap().push(BackfillCall {
            user_id: user.id,
            sender: sender.to_string(),
            decision,
            classify_as,
        });
        if self.fail {
            return Err(ScreenerBackfillError("forced backfill failure".to_string()));
        }
        Ok(())
    }

    async fn apply_undo_deny(
        &self,
        _state: &AppState,
        user: &hail_api::middleware::auth::AuthUser,
        sender: &str,
        classify_as: Classification,
    ) -> Result<(), ScreenerBackfillError> {
        self.calls.lock().unwrap().push(BackfillCall {
            user_id: user.id,
            sender: sender.to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(classify_as),
        });
        if self.fail {
            return Err(ScreenerBackfillError("forced backfill failure".to_string()));
        }
        Ok(())
    }
}

#[tokio::test]
async fn screener_view_requires_auth() {
    let (state, _) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/views/screener", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "legacy JMAP screener enrichment superseded by cache-backed production enrichment"]
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
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "alice-pending@example.org");
    assert_eq!(senders[0]["message_count"], 1);
    assert!(senders[0]["latest_preview"].is_null());
    assert!(senders[0]["emails"].as_array().unwrap().is_empty());
    assert!(json["next_cursor"].is_null());
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
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
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
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
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
    assert!(json["next_cursor"].is_null());
}

#[tokio::test]
async fn screener_view_paginates_pending_senders_with_cursor() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    let fixtures = [
        ("newest@example.org", now),
        ("same-a@example.org", now - Duration::minutes(1)),
        ("same-b@example.org", now - Duration::minutes(1)),
        ("same-c@example.org", now - Duration::minutes(1)),
        ("oldest@example.org", now - Duration::minutes(2)),
    ];
    for (sender, first_seen_at) in fixtures {
        seed_rule(&state, user_id, sender, "pending", None, first_seen_at).await;
    }

    let first = request(
        state.clone(),
        Method::GET,
        "/api/views/screener?limit=2",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    let first_senders: Vec<&str> = first["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        first_senders,
        vec!["newest@example.org", "same-a@example.org"]
    );
    let first_cursor = first["next_cursor"]
        .as_str()
        .expect("first page cursor")
        .to_owned();

    let second = request(
        state.clone(),
        Method::GET,
        &format!("/api/views/screener?limit=2&cursor={first_cursor}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = json_body(second).await;
    let second_senders: Vec<&str> = second["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(
        second_senders,
        vec!["same-b@example.org", "same-c@example.org"]
    );
    let second_cursor = second["next_cursor"]
        .as_str()
        .expect("second page cursor")
        .to_owned();
    assert_ne!(second_cursor, first_cursor);

    let third = request(
        state,
        Method::GET,
        &format!("/api/views/screener?limit=2&cursor={second_cursor}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(third.status(), StatusCode::OK);
    let third = json_body(third).await;
    let third_senders: Vec<&str> = third["senders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sender"].as_str().unwrap())
        .collect();
    assert_eq!(third_senders, vec!["oldest@example.org"]);
    assert!(third["next_cursor"].is_null());
}

#[tokio::test]
async fn screener_view_sorts_by_latest_pending_received_at_with_first_seen_fallback() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    let imported_old = chrono::DateTime::parse_from_rfc3339("2014-02-03T04:05:06Z")
        .unwrap()
        .with_timezone(&Utc);

    seed_rule(
        &state,
        user_id,
        "recent@example.org",
        "pending",
        None,
        now - Duration::days(10),
    )
    .await;
    set_latest_pending_received_at(&state, user_id, "recent@example.org", now).await;

    seed_rule(
        &state,
        user_id,
        "old-import@example.org",
        "pending",
        None,
        now,
    )
    .await;
    set_latest_pending_received_at(&state, user_id, "old-import@example.org", imported_old).await;

    seed_rule(
        &state,
        user_id,
        "fallback@example.org",
        "pending",
        None,
        now - Duration::hours(1),
    )
    .await;

    let tie_time = now - Duration::hours(2);
    for sender in ["tie-b@example.org", "tie-a@example.org"] {
        seed_rule(
            &state,
            user_id,
            sender,
            "pending",
            None,
            now - Duration::days(1),
        )
        .await;
        set_latest_pending_received_at(&state, user_id, sender, tie_time).await;
    }

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;
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
        vec![
            "recent@example.org",
            "fallback@example.org",
            "tie-a@example.org",
            "tie-b@example.org",
            "old-import@example.org",
        ]
    );
}

#[tokio::test]
async fn screener_view_accepts_v1_cursor_and_emits_v2_cursor() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    for (sender, first_seen_at) in [
        ("new@example.org", now),
        ("middle@example.org", now - Duration::minutes(1)),
        ("old@example.org", now - Duration::minutes(2)),
    ] {
        seed_rule(&state, user_id, sender, "pending", None, first_seen_at).await;
    }

    let first = request(
        state.clone(),
        Method::GET,
        "/api/views/screener?limit=1",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = json_body(first).await;
    let v2_cursor = first["next_cursor"].as_str().expect("v2 cursor");
    let decoded_v2 = String::from_utf8(URL_SAFE_NO_PAD.decode(v2_cursor).unwrap()).unwrap();
    assert!(decoded_v2.starts_with("2\n"), "cursor was {decoded_v2:?}");

    let v1_cursor = URL_SAFE_NO_PAD.encode(format!(
        "{}\n{}",
        (now - Duration::minutes(1)).to_rfc3339(),
        "middle@example.org"
    ));
    let after_v1 = request(
        state,
        Method::GET,
        &format!("/api/views/screener?limit=1&cursor={v1_cursor}"),
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(after_v1.status(), StatusCode::OK);
    let after_v1 = json_body(after_v1).await;
    let senders = after_v1["senders"].as_array().unwrap();
    assert_eq!(senders.len(), 1);
    assert_eq!(senders[0]["sender"], "old@example.org");
}

#[tokio::test]
async fn screener_view_rejects_invalid_cursor() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener?cursor=not-a-cursor",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "legacy JMAP screener enrichment superseded by cache-backed production enrichment"]
async fn screener_view_derives_previews_from_body_when_jmap_preview_is_empty() {
    const LONG_TEXT_BODY: &str = "   First line from text body.\n\nSecond line with extra spacing before a deliberately long suffix: 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789";

    let (mut state, key) = fixture_state().await;
    let (jmap_url, fake_jmap) = start_fake_screener_jmap(vec![
        JmapEmailFixture {
            id: "email-text",
            sender: "text@example.org",
            subject: "Text body fallback",
            preview: "",
            text_body: Some(LONG_TEXT_BODY),
            html_body: None,
        },
        JmapEmailFixture {
            id: "email-html",
            sender: "html@example.org",
            subject: "HTML body fallback",
            preview: "",
            text_body: None,
            html_body: Some(
                r#"<div><p>Order <strong>Ready</strong></p><table><tr><td>Total</td><td>$55.08</td></tr></table><img src="https://tracker.example/open.gif" width="1" height="1"><p>Thanks for reading.</p></div>"#,
            ),
        },
        JmapEmailFixture {
            id: "email-empty",
            sender: "empty@example.org",
            subject: "Empty body stays empty",
            preview: "",
            text_body: Some(" \n\t "),
            html_body: Some("<div> \n </div>"),
        },
    ])
    .await;
    state.config.stalwart.jmap_url = jmap_url;
    let (user_id, sid) = seed_session(&state, &key, "screener-preview@example.org").await;
    let now = Utc::now();
    for sender in ["text@example.org", "html@example.org", "empty@example.org"] {
        seed_rule(&state, user_id, sender, "pending", None, now).await;
    }

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().expect("screener senders");

    let text_sender = sender(senders, "text@example.org");
    let collapsed_text = collapse_for_expected_preview(LONG_TEXT_BODY);
    let expected_text = collapsed_text.chars().take(200).collect::<String>();
    assert_eq!(text_sender["latest_preview"]["preview"], expected_text);
    assert_eq!(text_sender["emails"][0]["preview"], expected_text);
    assert_eq!(
        text_sender["latest_preview"]["preview"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        200
    );
    assert!(
        !text_sender["latest_preview"]["preview"]
            .as_str()
            .unwrap()
            .starts_with(char::is_whitespace)
    );

    let html_sender = sender(senders, "html@example.org");
    let expected_html = "Order Ready Total $55.08 Thanks for reading.";
    assert_eq!(html_sender["latest_preview"]["preview"], expected_html);
    assert_eq!(html_sender["emails"][0]["preview"], expected_html);

    let empty_sender = sender(senders, "empty@example.org");
    assert_eq!(empty_sender["latest_preview"]["preview"], "");
    assert_eq!(empty_sender["emails"][0]["preview"], "");

    fake_jmap.abort();
}

#[tokio::test]
#[ignore = "legacy JMAP screener enrichment superseded by cache-backed production enrichment"]
async fn screener_view_fetches_body_values_only_for_newest_email_per_sender() {
    let mut emails = Vec::new();
    for index in 0..12 {
        emails.push(OwnedJmapEmailFixture {
            id: format!("bulk-email-{index:02}"),
            sender: "bulk@example.org".to_owned(),
            subject: format!("Bulk message {index:02}"),
            preview: String::new(),
            text_body: Some(format!(
                "Newest body preview {index:02} with enough words to prove body fallback hydration is used."
            )),
            html_body: None,
        });
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let (mut state, key) = fixture_state().await;
    let (jmap_url, fake_jmap) =
        start_fake_screener_jmap_owned_with_recorder(emails, requests.clone()).await;
    state.config.stalwart.jmap_url = jmap_url;
    let (user_id, sid) = seed_session(&state, &key, "screener-bulk@example.org").await;
    seed_rule(
        &state,
        user_id,
        "bulk@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().expect("screener senders");
    let bulk_sender = sender(senders, "bulk@example.org");
    assert_eq!(bulk_sender["message_count"], 12);
    assert_eq!(bulk_sender["emails"].as_array().unwrap().len(), 12);
    assert_eq!(bulk_sender["emails"][0]["email_id"], "bulk-email-00");
    assert_eq!(
        bulk_sender["latest_preview"]["preview"],
        "Newest body preview 00 with enough words to prove body fallback hydration is used."
    );
    assert_eq!(
        bulk_sender["emails"][0]["preview"],
        "Newest body preview 00 with enough words to prove body fallback hydration is used."
    );
    assert_eq!(bulk_sender["emails"][1]["email_id"], "bulk-email-01");
    assert_eq!(bulk_sender["emails"][1]["preview"], "");

    let requests = requests.lock().unwrap();
    let email_gets = requests
        .iter()
        .filter_map(|request| {
            let call = request["methodCalls"].as_array()?.first()?;
            (call[0].as_str()? == "Email/get").then_some(&call[1])
        })
        .collect::<Vec<_>>();
    assert_eq!(
        email_gets.len(),
        2,
        "expected rich and light Email/get calls"
    );

    let rich_get = email_gets
        .iter()
        .find(|arguments| arguments["fetchTextBodyValues"] == true)
        .expect("rich Email/get fetches body values");
    assert_eq!(rich_get["fetchHTMLBodyValues"], true);
    assert_eq!(rich_get["ids"], serde_json::json!(["bulk-email-00"]));

    let light_get = email_gets
        .iter()
        .find(|arguments| arguments["fetchTextBodyValues"].is_null())
        .expect("light Email/get omits body value flags");
    let light_ids = light_get["ids"].as_array().expect("light ids");
    assert_eq!(light_ids.len(), 11);
    assert!(!light_ids.iter().any(|id| id == "bulk-email-00"));

    fake_jmap.abort();
}

#[tokio::test]
#[ignore = "legacy JMAP screener enrichment superseded by cache-backed production enrichment"]
async fn screener_view_prefers_existing_jmap_preview_over_body_fallback() {
    let (mut state, key) = fixture_state().await;
    let (jmap_url, fake_jmap) = start_fake_screener_jmap(vec![JmapEmailFixture {
        id: "email-jmap-preview",
        sender: "jmap@example.org",
        subject: "Existing preview",
        preview: "  Provider preview wins.  ",
        text_body: Some("Body fallback should not replace the JMAP preview."),
        html_body: None,
    }])
    .await;
    state.config.stalwart.jmap_url = jmap_url;
    let (user_id, sid) = seed_session(&state, &key, "screener-jmap@example.org").await;
    seed_rule(
        &state,
        user_id,
        "jmap@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let senders = json["senders"].as_array().expect("screener senders");
    let jmap_sender = sender(senders, "jmap@example.org");
    assert_eq!(
        jmap_sender["latest_preview"]["preview"],
        "Provider preview wins."
    );
    assert_eq!(
        jmap_sender["emails"][0]["preview"],
        "Provider preview wins."
    );

    fake_jmap.abort();
}

#[tokio::test]
async fn allowed_senders_view_returns_current_user_allowed_rows_with_timestamps() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule_with_decided_at(
        &state,
        alice_id,
        "old-allowed@example.org",
        "allow",
        Some("feed"),
        now - Duration::days(3),
        Some(now - Duration::hours(2)),
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "pending@example.org",
        "pending",
        None,
        now,
    )
    .await;
    seed_rule(&state, alice_id, "denied@example.org", "deny", None, now).await;
    seed_rule_with_decided_at(
        &state,
        alice_id,
        "NEW-ALLOWED@EXAMPLE.ORG",
        "allow",
        Some("papertrail"),
        now - Duration::days(1),
        Some(now),
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "legacy-allowed@example.org",
        "allow",
        Some("imbox"),
        now - Duration::hours(1),
    )
    .await;
    seed_rule(
        &state,
        bob_id,
        "bob-allowed@example.org",
        "allow",
        Some("imbox"),
        now,
    )
    .await;

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener/allowed",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let allowed = json["allowed"].as_array().unwrap();
    assert_eq!(allowed.len(), 3);
    assert_eq!(allowed[0]["sender_address"], "new-allowed@example.org");
    assert_eq!(allowed[0]["classify_as"], "papertrail");
    assert!(allowed[0]["first_seen_at"].is_string());
    assert!(allowed[0]["decided_at"].is_string());
    assert_eq!(allowed[1]["sender_address"], "legacy-allowed@example.org");
    assert_eq!(allowed[1]["classify_as"], "imbox");
    assert!(allowed[1]["decided_at"].is_null());
    assert_eq!(allowed[2]["sender_address"], "old-allowed@example.org");
    assert_eq!(allowed[2]["classify_as"], "feed");
}

#[tokio::test]
async fn allowed_senders_view_requires_auth() {
    let (state, _) = fixture_state().await;
    let resp = request(
        state,
        Method::GET,
        "/api/views/screener/allowed",
        None,
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn denied_senders_view_returns_current_user_denied_rows() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule(
        &state,
        alice_id,
        "old-denied@example.org",
        "deny",
        None,
        now - Duration::hours(2),
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "allowed@example.org",
        "allow",
        Some("imbox"),
        now,
    )
    .await;
    seed_rule(
        &state,
        alice_id,
        "new-denied@example.org",
        "deny",
        None,
        now,
    )
    .await;
    seed_rule(&state, bob_id, "bob-denied@example.org", "deny", None, now).await;
    sqlx::query(
        "UPDATE screener_rules SET decided_at = first_seen_at WHERE user_id = ?1 AND decision = 'deny'",
    )
    .bind(alice_id)
    .execute(&state.db)
    .await
    .unwrap();

    let resp = request(
        state,
        Method::GET,
        "/api/views/screener/denied",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let denied = json["denied"].as_array().unwrap();
    assert_eq!(denied.len(), 2);
    assert_eq!(denied[0]["sender_address"], "new-denied@example.org");
    assert!(denied[0]["denied_at"].is_string());
    assert_eq!(denied[1]["sender_address"], "old-denied@example.org");
}

#[tokio::test]
async fn undo_deny_approves_current_user_denied_rule_with_default_classification() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _) = seed_session(&state, &key, "bob@example.org").await;
    let now = Utc::now();
    seed_rule(&state, alice_id, "spam@example.org", "deny", None, now).await;
    seed_rule(&state, bob_id, "spam@example.org", "deny", None, now).await;
    seed_rule(
        &state,
        alice_id,
        "allowed@example.org",
        "allow",
        Some("feed"),
        now,
    )
    .await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&alice_sid),
        true,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "approved");
    assert_eq!(json["classify_as"], "imbox");

    let alice_spam: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(alice_spam.0, "allow");
    assert_eq!(alice_spam.1.as_deref(), Some("imbox"));

    let bob_spam: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(bob_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(bob_spam.0, "deny");
    assert!(bob_spam.1.is_none());

    let allowed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ?1 AND sender_address = 'allowed@example.org'",
    )
    .bind(alice_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(allowed_count, 1);
}

#[tokio::test]
async fn undo_deny_accepts_classify_as_and_backfills_history() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(&state, user_id, "spam@example.org", "deny", None, now).await;
    let backfill = Arc::new(FakeBackfill::default());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&sid),
        true,
        Some(r#"{"classify_as":"papertrail"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["status"], "approved");
    assert_eq!(json["classify_as"], "papertrail");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("papertrail"));

    let calls = backfill.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        BackfillCall {
            user_id,
            sender: "spam@example.org".to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(Classification::Papertrail),
        }
    );
}

#[tokio::test]
async fn undo_deny_rejects_invalid_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let now = Utc::now();
    seed_rule(&state, user_id, "spam@example.org", "deny", None, now).await;
    let backfill = Arc::new(FakeBackfill::default());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&sid),
        true,
        Some(r#"{"classify_as":"other"}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_classify_as");
    assert!(backfill.calls.lock().unwrap().is_empty());

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "deny");
    assert!(row.1.is_none());
}

#[tokio::test]
async fn undo_deny_missing_csrf_returns_403() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/spam%40example.org/undo-deny",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn approve_creates_or_updates_row_and_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "news@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    let resp = request(state.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":" News@Example.ORG ","decision":"approve","classify_as":"feed","apply_to_history":false}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["sender"], "news@example.org");
    assert_eq!(json["decision"], "approve");
    assert_eq!(json["classify_as"], "feed");
    let row: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT decision, classify_as, decided_at FROM screener_rules WHERE user_id = ?1 AND sender_address = 'news@example.org'").bind(user_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("feed"));
    assert!(row.2.is_some());

    let audit: (i64, String, String) =
        sqlx::query_as("SELECT user_id, action, payload_json FROM audit_log")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(audit.0, user_id);
    assert_eq!(audit.1, "screener.decision");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&audit.2).unwrap(),
        serde_json::json!({
            "sender": "news@example.org",
            "decision": "approve",
            "classify_as": "feed",
            "apply_to_history": false,
        })
    );
}

#[tokio::test]
async fn approve_clears_pin_for_sender_without_other_pin_sources() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let account_id = seed_mail_account(&state, user_id, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "news@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    seed_cached_message(&state, account_id, "news-1", "news@example.org", true).await;

    let resp = request(state.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":"news@example.org","decision":"approve","classify_as":"feed","apply_to_history":false}"#)).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(cached_pin(&state, "news-1").await, 0);
}

#[tokio::test]
async fn deny_clears_pin_for_sender_without_other_pin_sources() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let account_id = seed_mail_account(&state, user_id, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "pending",
        None,
        Utc::now(),
    )
    .await;
    seed_cached_message(&state, account_id, "spam-1", "spam@example.org", true).await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(cached_pin(&state, "spam-1").await, 0);
}

#[tokio::test]
async fn approve_without_classify_as_defaults_to_imbox() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"person@example.org","decision":"approve","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["classify_as"], "imbox");
}

#[tokio::test]
async fn deny_creates_or_updates_row() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "allow",
        Some("imbox"),
        Utc::now(),
    )
    .await;
    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["sender"], "spam@example.org");
    assert_eq!(json["decision"], "deny");
    assert!(json["classify_as"].is_null());
    let row: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT decision, classify_as, decided_at FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'").bind(user_id).fetch_one(&state.db).await.unwrap();
    assert_eq!(row.0, "deny");
    assert!(row.1.is_none());
    assert!(row.2.is_some());
}

#[tokio::test]
async fn deny_rejects_classify_as() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    seed_rule(
        &state,
        user_id,
        "spam@example.org",
        "allow",
        Some("feed"),
        Utc::now(),
    )
    .await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"spam@example.org","decision":"deny","classify_as":"imbox","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_classify_as");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT decision, classify_as FROM screener_rules WHERE user_id = ?1 AND sender_address = 'spam@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "allow");
    assert_eq!(row.1.as_deref(), Some("feed"));
}

#[tokio::test]
async fn decision_missing_csrf_returns_403() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        false,
        Some(r#"{"sender":"x@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn decision_without_auth_returns_401() {
    let (state, _) = fixture_state().await;
    let resp = request(
        state,
        Method::POST,
        "/api/screener/decisions",
        None,
        true,
        Some(r#"{"sender":"x@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_decision_or_classify_as_returns_400() {
    let (state, key) = fixture_state().await;
    let (_, sid) = seed_session(&state, &key, "alice@example.org").await;
    let bad_decision = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"x@example.org","decision":"maybe","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(bad_decision.status(), StatusCode::BAD_REQUEST);
    let bad_classify = request(state, Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":"x@example.org","decision":"approve","classify_as":"other","apply_to_history":false}"#)).await;
    assert_eq!(bad_classify.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn apply_to_history_calls_fake_backfill_once() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let backfill = Arc::new(FakeBackfill::default());
    let resp = request_with_backfill(state, backfill.clone(), Method::POST, "/api/screener/decisions", Some(&sid), true, Some(r#"{"sender":" Sender@Example.ORG ","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let calls = backfill.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        BackfillCall {
            user_id,
            sender: "sender@example.org".to_string(),
            decision: ScreenerDecision::Approve,
            classify_as: Some(Classification::Papertrail)
        }
    );
}

#[tokio::test]
async fn decision_response_undo_payload_snapshots_previous_rule() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "undo-existing@example.org").await;
    let first_seen_at = Utc::now() - Duration::days(3);
    seed_rule(
        &state,
        user_id,
        "sender@example.org",
        "allow",
        Some("feed"),
        first_seen_at,
    )
    .await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"deny","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["undo"]["action"], "screener.decision");
    let undo_id = json["undo"]["id"].as_str().unwrap();

    let payload_json: String =
        sqlx::query_scalar("SELECT payload_json FROM undo_actions WHERE id = ?1")
            .bind(undo_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
    assert_eq!(payload["sender"], "sender@example.org");
    assert_eq!(payload["previous_rule"]["decision"], "allow");
    assert_eq!(payload["previous_rule"]["classify_as"], "feed");
    assert!(payload["previous_rule"]["decided_at"].is_null());
    assert_eq!(
        payload["previous_rule"]["first_seen_at"],
        serde_json::json!(first_seen_at)
    );
}

#[tokio::test]
async fn decision_response_undo_payload_marks_new_sender_for_delete() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "undo-new@example.org").await;

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"new@example.org","decision":"approve","apply_to_history":false}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["undo"]["action"], "screener.decision");
    let undo_id = json["undo"]["id"].as_str().unwrap();

    let payload_json: String =
        sqlx::query_scalar("SELECT payload_json FROM undo_actions WHERE id = ?1")
            .bind(undo_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
    assert_eq!(payload["sender"], "new@example.org");
    assert!(payload["previous_rule"].is_null());
}

#[tokio::test]
async fn backfill_failure_returns_500_without_persisting_decision_audit_or_undo() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "backfill-fail@example.org").await;
    let backfill = Arc::new(FakeBackfill::failing());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM screener_rules WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row_count, 0);
    assert_eq!(backfill.calls.lock().unwrap().len(), 1);

    let audit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);
    let undo_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM undo_actions")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(undo_count, 0);
}

#[tokio::test]
async fn backfill_failure_preserves_previous_screener_rule() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "backfill-existing@example.org").await;
    let first_seen_at = Utc::now() - Duration::days(2);
    seed_rule(
        &state,
        user_id,
        "sender@example.org",
        "pending",
        None,
        first_seen_at,
    )
    .await;
    let backfill = Arc::new(FakeBackfill::failing());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"approve","classify_as":"papertrail","apply_to_history":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let row: (
        String,
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        chrono::DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT decision, classify_as, decided_at, first_seen_at FROM screener_rules \
             WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, None);
    assert_eq!(row.2, None);
    assert_eq!(row.3, first_seen_at);
    assert_eq!(backfill.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn backfill_failure_rolls_back_decision_audit_and_undo_together() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "backfill-atomic@example.org").await;
    let first_seen_at = Utc::now() - Duration::days(2);
    seed_rule(
        &state,
        user_id,
        "sender@example.org",
        "pending",
        None,
        first_seen_at,
    )
    .await;
    let backfill = Arc::new(FakeBackfill::failing());

    let resp = request_with_backfill(
        state.clone(),
        backfill.clone(),
        Method::POST,
        "/api/screener/decisions",
        Some(&sid),
        true,
        Some(r#"{"sender":"sender@example.org","decision":"deny","apply_to_history":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(backfill.calls.lock().unwrap().len(), 1);

    let row: (
        String,
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        chrono::DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT decision, classify_as, decided_at, first_seen_at FROM screener_rules \
             WHERE user_id = ?1 AND sender_address = 'sender@example.org'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "pending");
    assert_eq!(row.1, None);
    assert_eq!(row.2, None);
    assert_eq!(row.3, first_seen_at);

    let audit_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);
    let undo_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM undo_actions")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(undo_count, 0);
}

#[tokio::test]
async fn invalid_sender_returns_400_without_persisting_or_backfill() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "invalid-sender@example.org").await;

    for body in [
        r#"{"sender":"","decision":"deny","apply_to_history":true}"#,
        r#"{"sender":"not-an-email","decision":"deny","apply_to_history":true}"#,
        r#"{"sender":"bad domain@example.org","decision":"deny","apply_to_history":true}"#,
        r#"{"sender":"bad@example","decision":"deny","apply_to_history":true}"#,
    ] {
        let backfill = Arc::new(FakeBackfill::default());
        let resp = request_with_backfill(
            state.clone(),
            backfill.clone(),
            Method::POST,
            "/api/screener/decisions",
            Some(&sid),
            true,
            Some(body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "invalid_sender", "{body}");
        assert!(backfill.calls.lock().unwrap().is_empty(), "{body}");
    }

    let rule_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screener_rules")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(rule_count, 0);
}

#[tokio::test]
async fn malformed_decision_bodies_return_400_without_persisting_or_backfill() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bad-body@example.org").await;

    for body in [
        "not-json",
        r#"{"sender":"sender@example.org","decision":"approve"}"#,
        r#"{"sender":"sender@example.org","decision":"approve","apply_to_history":"yes"}"#,
        r#"{"decision":"approve","apply_to_history":false}"#,
    ] {
        let backfill = Arc::new(FakeBackfill::default());
        let resp = request_with_backfill(
            state.clone(),
            backfill.clone(),
            Method::POST,
            "/api/screener/decisions",
            Some(&sid),
            true,
            Some(body),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
        let json = json_body(resp).await;
        assert_eq!(json["error"], "invalid_decision_body", "{body}");
        assert!(backfill.calls.lock().unwrap().is_empty(), "{body}");
    }

    let rule_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screener_rules")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(rule_count, 0);
}

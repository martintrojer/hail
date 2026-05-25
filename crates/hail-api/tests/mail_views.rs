use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, TimeZone, Utc};
use hail_api::middleware::auth::require_auth;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::views::{
    MailClassification, MailView, MailViewError, MailViewItem, MailViewProvider,
};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use serde_json::Value;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_mail_views_test_{uniq}?mode=memory&cache=shared");
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
        events: hail_api::events::AppEventBus::default(),
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

fn app(state: AppState, provider: Arc<FakeProvider>) -> Router {
    let protected = hail_api::routes::views::router_with_provider(provider).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Default)]
struct FakeProvider {
    items: Vec<MailViewItem>,
    error: Option<String>,
    calls: Mutex<Vec<(MailView, usize)>>,
}

impl FakeProvider {
    fn new(items: Vec<MailViewItem>) -> Self {
        Self {
            items,
            error: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(message: impl Into<String>) -> Self {
        Self {
            items: Vec::new(),
            error: Some(message.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(MailView, usize)> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl MailViewProvider for FakeProvider {
    fn list<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        view: MailView,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailViewItem>, MailViewError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().expect("calls lock").push((view, limit));
            if let Some(message) = &self.error {
                return Err(MailViewError::provider(message.clone()));
            }
            Ok(self.items.iter().take(limit).cloned().collect())
        })
    }
}

async fn get_view(
    state: AppState,
    provider: Arc<FakeProvider>,
    sid: Option<&str>,
    path: &str,
) -> axum::response::Response {
    let mut builder = Request::builder().method(Method::GET).uri(path);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    let req = builder.body(Body::empty()).unwrap();
    app(state, provider).oneshot(req).await.unwrap()
}

async fn response_json(resp: axum::response::Response) -> Value {
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

fn item(n: i64, classification: MailClassification) -> MailViewItem {
    MailViewItem {
        thread_id: format!("thread-{n}"),
        email_id: format!("email-{n}"),
        from: format!("Sender {n} <sender{n}@example.org>"),
        to: vec![format!("Recipient {n} <recipient{n}@example.org>")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: format!("Subject {n}"),
        preview: format!("Preview {n}"),
        received_at: Some(
            Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap() - Duration::minutes(n),
        ),
        unread: n % 2 == 0,
        classification,
        has_notes: false,
    }
}

#[tokio::test]
async fn auth_required_returns_401() {
    let (state, _key) = fixture_state().await;
    let provider = Arc::new(FakeProvider::default());

    let resp = get_view(state, provider, None, "/api/views/imbox").await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn imbox_feed_papertrail_map_to_correct_view_and_classification() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let cases = [
        (
            "/api/views/imbox",
            MailView::Imbox,
            MailClassification::Imbox,
            "imbox",
        ),
        (
            "/api/views/feed",
            MailView::Feed,
            MailClassification::Feed,
            "feed",
        ),
        (
            "/api/views/papertrail",
            MailView::Papertrail,
            MailClassification::Papertrail,
            "papertrail",
        ),
        (
            "/api/views/drafts",
            MailView::Drafts,
            MailClassification::Drafts,
            "drafts",
        ),
        (
            "/api/views/trash",
            MailView::Trash,
            MailClassification::Trash,
            "trash",
        ),
    ];

    for (path, expected_view, classification, expected_json) in cases {
        let provider = Arc::new(FakeProvider::new(vec![item(1, classification)]));
        let resp = get_view(state.clone(), provider.clone(), Some(&sid), path).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(provider.calls(), vec![(expected_view, 50)]);
        let json = response_json(resp).await;
        assert_eq!(json["items"][0]["classification"], expected_json);
        assert_eq!(json["next_cursor"], Value::Null);
    }

    assert_eq!(MailView::Imbox.keyword(), "$hail_imbox");
    assert_eq!(MailView::Feed.keyword(), "$hail_feed");
    assert_eq!(MailView::Papertrail.keyword(), "$hail_papertrail");
    assert_eq!(MailView::Drafts.keyword(), "$draft");
    assert_eq!(MailView::Trash.keyword(), "$deleted");
}

#[tokio::test]
async fn limit_defaults_to_50_and_caps_at_100() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bob@example.org").await;
    let provider = Arc::new(FakeProvider::default());

    let resp = get_view(
        state.clone(),
        provider.clone(),
        Some(&sid),
        "/api/views/feed?cursor=ignored",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = get_view(
        state,
        provider.clone(),
        Some(&sid),
        "/api/views/feed?limit=999",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    assert_eq!(
        provider.calls(),
        vec![(MailView::Feed, 50), (MailView::Feed, 100)]
    );
}

#[tokio::test]
async fn response_preserves_provider_order() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "carol@example.org").await;
    let provider = Arc::new(FakeProvider::new(vec![
        item(30, MailClassification::Imbox),
        item(10, MailClassification::Imbox),
        item(20, MailClassification::Imbox),
    ]));

    let resp = get_view(state, provider, Some(&sid), "/api/views/imbox?limit=3").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    let ids: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["email_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["email-30", "email-10", "email-20"]);
}

#[tokio::test]
async fn response_sets_has_notes_for_current_users_thread_notes() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "notes-owner@example.org").await;
    let (other_user_id, _other_sid) = seed_session(&state, &key, "other-notes@example.org").await;
    sqlx::query(
        "INSERT INTO thread_notes (user_id, thread_id, email_id, body) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id)
    .bind("thread-20")
    .bind("email-20")
    .bind("owned note")
    .execute(&state.db)
    .await
    .expect("insert owned note");
    sqlx::query(
        "INSERT INTO thread_notes (user_id, thread_id, email_id, body) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(other_user_id)
    .bind("thread-30")
    .bind("email-30")
    .bind("other note")
    .execute(&state.db)
    .await
    .expect("insert other note");
    let provider = Arc::new(FakeProvider::new(vec![
        item(30, MailClassification::Imbox),
        item(20, MailClassification::Imbox),
        item(10, MailClassification::Imbox),
    ]));

    let resp = get_view(state, provider, Some(&sid), "/api/views/imbox?limit=3").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    let flags: Vec<bool> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["has_notes"].as_bool().unwrap())
        .collect();
    assert_eq!(flags, vec![false, true, false]);
}

#[tokio::test]
async fn bubble_up_view_returns_current_users_future_pending_rows_ordered_by_surface_time() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice-bubbles@example.org").await;
    let (bob_id, _bob_sid) = seed_session(&state, &key, "bob-bubbles@example.org").await;
    let provider = Arc::new(FakeProvider::default());
    let now = Utc::now();
    let first_at = now + Duration::minutes(5);
    let second_at = now + Duration::minutes(10);

    for (user_id, thread_id, surface_at, fired_at) in [
        (alice_id, "thread-second", second_at, None),
        (alice_id, "thread-first", first_at, None),
        (alice_id, "thread-past", now - Duration::minutes(5), None),
        (alice_id, "thread-fired", now + Duration::minutes(15), Some(now)),
        (bob_id, "thread-other-user", now + Duration::minutes(1), None),
    ] {
        sqlx::query(
            "INSERT INTO bubble_ups (user_id, thread_id, surface_at, fired_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(user_id)
        .bind(thread_id)
        .bind(surface_at)
        .bind(fired_at)
        .bind(now)
        .execute(&state.db)
        .await
        .unwrap();
    }

    let resp = get_view(
        state,
        provider.clone(),
        Some(&alice_sid),
        "/api/views/bubble-up",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(provider.calls(), Vec::<(MailView, usize)>::new());
    let json = response_json(resp).await;
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    assert_eq!(json["items"][0]["thread_id"], "thread-first");
    assert_eq!(json["items"][1]["thread_id"], "thread-second");
    assert!(json["items"][0]["bubble_id"].as_i64().is_some());
    assert_eq!(json["items"][0]["surface_at"], first_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
    assert_eq!(json["items"][0]["created_at"], now.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
}

#[tokio::test]
async fn provider_error_returns_stable_internal_json() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "erin@example.org").await;
    let provider = Arc::new(FakeProvider::failing("upstream query failed"));

    let resp = get_view(state, provider.clone(), Some(&sid), "/api/views/imbox").await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(provider.calls(), vec![(MailView::Imbox, 50)]);
    assert_eq!(
        response_json(resp).await,
        serde_json::json!({"error": "internal"})
    );
}

#[tokio::test]
async fn required_identifier_provider_error_returns_stable_internal_json() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "frank@example.org").await;
    let provider = Arc::new(FakeProvider::failing("JMAP Email missing required id"));

    let resp = get_view(
        state,
        provider.clone(),
        Some(&sid),
        "/api/views/feed?limit=2",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(provider.calls(), vec![(MailView::Feed, 2)]);
    assert_eq!(
        response_json(resp).await,
        serde_json::json!({"error": "internal"})
    );
}

#[tokio::test]
async fn empty_list_returns_empty_items_and_null_cursor() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "dana@example.org").await;
    let provider = Arc::new(FakeProvider::default());

    let resp = get_view(state, provider, Some(&sid), "/api/views/papertrail").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = response_json(resp).await;
    assert_eq!(
        json,
        serde_json::json!({ "items": [], "next_cursor": null })
    );
}

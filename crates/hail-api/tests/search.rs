use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, Duration, TimeZone, Utc};
use hail_api::middleware::auth::require_auth;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::views::{
    MailSearchResult, MailView, MailViewError, MailViewItem, MailViewProvider, SearchError,
    SearchProvider,
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
    let url = format!("sqlite:file:hail_search_test_{uniq}?mode=memory&cache=shared");
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

async fn insert_note(state: &AppState, user_id: i64, address: &str, markdown: &str) {
    insert_note_at(state, user_id, address, markdown, Utc::now()).await;
}

async fn insert_note_at(
    state: &AppState,
    user_id: i64,
    address: &str,
    markdown: &str,
    updated_at: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO contact_notes (user_id, address, markdown, updated_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id)
    .bind(address)
    .bind(markdown)
    .bind(updated_at)
    .execute(&state.db)
    .await
    .expect("insert note");
}

fn app(state: AppState, search: Arc<FakeSearchProvider>) -> Router {
    let protected =
        hail_api::routes::views::router_with_providers(Arc::new(EmptyMailViewProvider), search)
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            ));
    Router::new().merge(protected).with_state(state)
}

struct EmptyMailViewProvider;

impl MailViewProvider for EmptyMailViewProvider {
    fn list<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        _view: MailView,
        _limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailViewItem>, MailViewError>> + Send + 'a>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[derive(Default)]
struct FakeSearchProvider {
    items: Vec<MailSearchResult>,
    error: Option<String>,
    calls: Mutex<Vec<(String, usize)>>,
}

impl FakeSearchProvider {
    fn new(items: Vec<MailSearchResult>) -> Self {
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

    fn calls(&self) -> Vec<(String, usize)> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl SearchProvider for FakeSearchProvider {
    fn search<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        q: &'a str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MailSearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls lock")
                .push((q.to_string(), limit));
            if let Some(message) = &self.error {
                return Err(SearchError::provider(message.clone()));
            }
            Ok(self.items.iter().take(limit).cloned().collect())
        })
    }
}

async fn request_search(
    state: AppState,
    search: Arc<FakeSearchProvider>,
    sid: Option<&str>,
    path: &str,
) -> axum::response::Response {
    let mut builder = Request::builder().method(Method::GET).uri(path);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    app(state, search)
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json_body(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn mail_item() -> MailSearchResult {
    MailSearchResult {
        thread_id: "thread-1".to_string(),
        email_id: "email-1".to_string(),
        from: "Ada <ada@example.org>".to_string(),
        subject: "Project update".to_string(),
        preview: "Needle in mail".to_string(),
        received_at: Some(Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap()),
    }
}

fn result_addresses(json: &Value) -> Vec<String> {
    json["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|item| item["address"].as_str().expect("address").to_string())
        .collect()
}

#[tokio::test]
async fn auth_required_returns_401() {
    let (state, _key) = fixture_state().await;
    let search = Arc::new(FakeSearchProvider::default());

    let resp = request_search(state, search, None, "/api/views/search?q=needle").await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn short_q_returns_400() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let search = Arc::new(FakeSearchProvider::default());

    let resp = request_search(state, search, Some(&sid), "/api/views/search?q=x").await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn trims_query_before_validation_and_mail_provider_search() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let search = Arc::new(FakeSearchProvider::new(vec![mail_item()]));

    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        "/api/views/search?q=%20%20needle%20%20&scope=mail",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(search.calls(), vec![("needle".to_string(), 50)]);
}

#[tokio::test]
async fn mail_provider_error_returns_stable_internal_json() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bob@example.org").await;
    let search = Arc::new(FakeSearchProvider::failing("upstream search failed"));

    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=mail",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(search.calls(), vec![("needle".to_string(), 50)]);
    assert_eq!(
        json_body(resp).await,
        serde_json::json!({"error": "internal"})
    );
}

#[tokio::test]
async fn required_identifier_provider_error_returns_stable_internal_json() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "carol@example.org").await;
    let search = Arc::new(FakeSearchProvider::failing(
        "JMAP Email missing required threadId",
    ));

    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=all",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(search.calls(), vec![("needle".to_string(), 50)]);
    assert_eq!(
        json_body(resp).await,
        serde_json::json!({"error": "internal"})
    );
}

#[tokio::test]
async fn notes_search_finds_current_user_only() {
    let (state, key) = fixture_state().await;
    let (alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (bob_id, _bob_sid) = seed_session(&state, &key, "bob@example.org").await;
    insert_note(&state, alice_id, "ada@example.org", "needle for alice").await;
    insert_note(&state, bob_id, "mallory@example.org", "needle for bob").await;
    let search = Arc::new(FakeSearchProvider::default());

    let resp = request_search(
        state,
        search,
        Some(&alice_sid),
        "/api/views/search?q=needle&scope=notes",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["results"].as_array().unwrap().len(), 1);
    assert_eq!(json["results"][0]["type"], "contact_note");
    assert_eq!(json["results"][0]["address"], "ada@example.org");
    assert_eq!(json["results"][0]["markdown"], "needle for alice");
}

#[tokio::test]
async fn notes_search_escapes_like_wildcards() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    insert_note(
        &state,
        user_id,
        "percent@example.org",
        "literal 100% complete",
    )
    .await;
    insert_note(
        &state,
        user_id,
        "percent-false@example.org",
        "100x should not match",
    )
    .await;
    insert_note(
        &state,
        user_id,
        "underscore@example.org",
        "literal a_b marker",
    )
    .await;
    insert_note(
        &state,
        user_id,
        "underscore-false@example.org",
        "axb should not match",
    )
    .await;
    insert_note(
        &state,
        user_id,
        "backslash@example.org",
        r"literal path\to marker",
    )
    .await;
    insert_note(
        &state,
        user_id,
        "backslash-false@example.org",
        "path/to should not match",
    )
    .await;
    let search = Arc::new(FakeSearchProvider::default());

    let resp = request_search(
        state.clone(),
        search.clone(),
        Some(&sid),
        "/api/views/search?q=100%25&scope=notes",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        result_addresses(&json_body(resp).await),
        vec!["percent@example.org"]
    );

    let resp = request_search(
        state.clone(),
        search.clone(),
        Some(&sid),
        "/api/views/search?q=a_b&scope=notes",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        result_addresses(&json_body(resp).await),
        vec!["underscore@example.org"]
    );

    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        r"/api/views/search?q=path%5Cto&scope=notes",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        result_addresses(&json_body(resp).await),
        vec!["backslash@example.org"]
    );
    assert!(search.calls().is_empty());
}

#[tokio::test]
async fn notes_search_orders_deterministically_and_limits_results() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let base = Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap();

    insert_note_at(
        &state,
        user_id,
        "z-latest@example.org",
        "needle newest",
        base + Duration::seconds(2),
    )
    .await;
    insert_note_at(
        &state,
        user_id,
        "tie-b@example.org",
        "needle tie b",
        base + Duration::seconds(1),
    )
    .await;
    insert_note_at(
        &state,
        user_id,
        "tie-a@example.org",
        "needle tie a",
        base + Duration::seconds(1),
    )
    .await;
    for index in 0..49 {
        insert_note_at(
            &state,
            user_id,
            &format!("old-{index:02}@example.org"),
            &format!("needle old {index}"),
            base,
        )
        .await;
    }
    let search = Arc::new(FakeSearchProvider::default());

    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=notes",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let addresses = result_addresses(&json);
    assert_eq!(addresses.len(), 50);
    assert_eq!(
        &addresses[..5],
        &[
            "z-latest@example.org".to_string(),
            "tie-a@example.org".to_string(),
            "tie-b@example.org".to_string(),
            "old-00@example.org".to_string(),
            "old-01@example.org".to_string(),
        ]
    );
    assert!(addresses.contains(&"old-46@example.org".to_string()));
    assert!(!addresses.contains(&"old-47@example.org".to_string()));
    assert!(!addresses.contains(&"old-48@example.org".to_string()));
    assert!(search.calls().is_empty());
}

#[tokio::test]
async fn scope_filters_notes_mail_and_all() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    insert_note(&state, user_id, "ada@example.org", "needle note").await;

    let mail_only = Arc::new(FakeSearchProvider::new(vec![mail_item()]));
    let resp = request_search(
        state.clone(),
        mail_only.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=mail",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["results"].as_array().unwrap().len(), 1);
    assert_eq!(json["results"][0]["type"], "mail");
    assert_eq!(mail_only.calls(), vec![("needle".to_string(), 50)]);

    let notes_only = Arc::new(FakeSearchProvider::new(vec![mail_item()]));
    let resp = request_search(
        state.clone(),
        notes_only.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=notes",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["results"].as_array().unwrap().len(), 1);
    assert_eq!(json["results"][0]["type"], "contact_note");
    assert!(notes_only.calls().is_empty());

    let all = Arc::new(FakeSearchProvider::new(vec![mail_item()]));
    let resp = request_search(
        state.clone(),
        all.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=all",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["results"].as_array().unwrap().len(), 2);
    assert_eq!(json["results"][0]["type"], "mail");
    assert_eq!(json["results"][1]["type"], "contact_note");
    assert_eq!(all.calls(), vec![("needle".to_string(), 50)]);
}

#[tokio::test]
async fn clips_scope_returns_400_unsupported_without_searching_mail() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let clips = Arc::new(FakeSearchProvider::new(vec![mail_item()]));
    let resp = request_search(
        state,
        clips.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=clips",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "clips_unsupported");
    assert!(clips.calls().is_empty());
}

#[tokio::test]
async fn unknown_scope_returns_400_invalid_scope() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let search = Arc::new(FakeSearchProvider::new(vec![mail_item()]));
    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        "/api/views/search?q=needle&scope=bogus",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = json_body(resp).await;
    assert_eq!(json["error"], "invalid_scope");
    assert!(search.calls().is_empty());
}

#[tokio::test]
async fn default_all_scope_calls_fake_mail_provider() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let search = Arc::new(FakeSearchProvider::new(vec![mail_item()]));

    let resp = request_search(
        state,
        search.clone(),
        Some(&sid),
        "/api/views/search?q=needle",
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["results"][0]["thread_id"], "thread-1");
    assert_eq!(search.calls(), vec![("needle".to_string(), 50)]);
}

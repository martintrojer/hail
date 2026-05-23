use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::admin_domains::{ManagementError, StalwartManagement};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_admin_domains_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", hex::encode(key));
        std::env::remove_var("HAIL_STALWART__MANAGEMENT_URL");
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

async fn seed_session(
    state: &AppState,
    key: &[u8; KEY_LEN],
    email: &str,
    is_admin: bool,
) -> String {
    let now = Utc::now();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
    )
    .bind(email)
    .bind(format!("account-{email}"))
    .bind(if is_admin { 1_i64 } else { 0_i64 })
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

fn app(state: AppState, management: Arc<FakeManagement>) -> Router {
    let protected = hail_api::routes::admin_domains::router_with_management(management).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone, Default)]
struct FakeManagement {
    domains: Arc<Mutex<Vec<String>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeManagement {
    fn with_domains(domains: &[&str]) -> Self {
        Self {
            domains: Arc::new(Mutex::new(
                domains.iter().map(ToString::to_string).collect(),
            )),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl StalwartManagement for FakeManagement {
    fn list_domains<'a>(
        &'a self,
        _state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("list".to_string());
            Ok(self.domains.lock().unwrap().clone())
        })
    }

    fn add_domain<'a>(
        &'a self,
        _state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("add:{domain}"));
            self.domains.lock().unwrap().push(domain.to_string());
            Ok(())
        })
    }

    fn delete_domain<'a>(
        &'a self,
        _state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("delete:{domain}"));
            self.domains.lock().unwrap().retain(|d| d != domain);
            Ok(())
        })
    }
}

async fn request(
    state: AppState,
    management: Arc<FakeManagement>,
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
    app(state, management)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn auth_required_for_list() {
    let (state, _key) = fixture_state().await;
    let management = Arc::new(FakeManagement::with_domains(&["example.org"]));

    let resp = request(
        state,
        management,
        Method::GET,
        "/api/admin/domains",
        None,
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_admin_returns_403() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org", false).await;
    let management = Arc::new(FakeManagement::with_domains(&["example.org"]));

    let resp = request(
        state,
        management,
        Method::GET,
        "/api/admin/domains",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(resp).await["error"], "admin_required");
}

#[tokio::test]
async fn list_uses_fake_management() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::with_domains(&["z.example", "a.example"]));

    let resp = request(
        state,
        management.clone(),
        Method::GET,
        "/api/admin/domains",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(
        json["domains"],
        serde_json::json!(["a.example", "z.example"])
    );
    assert_eq!(management.calls(), vec!["list"]);
}

#[tokio::test]
async fn add_uses_fake_management_and_normalizes_domain() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::default());

    let resp = request(
        state,
        management.clone(),
        Method::POST,
        "/api/admin/domains",
        Some(&sid),
        true,
        Some(r#"{"domain":" Example.ORG. "}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = json_body(resp).await;
    assert_eq!(json["domain"], "example.org");
    assert_eq!(management.calls(), vec!["add:example.org"]);
}

#[tokio::test]
async fn delete_uses_fake_management_and_normalizes_domain() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::with_domains(&["example.org"]));

    let resp = request(
        state,
        management.clone(),
        Method::DELETE,
        "/api/admin/domains/Example.ORG.",
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(management.calls(), vec!["delete:example.org"]);
}

#[tokio::test]
async fn invalid_domain_returns_400_and_does_not_call_management() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeManagement::default());

    let resp = request(
        state,
        management.clone(),
        Method::POST,
        "/api/admin/domains",
        Some(&sid),
        true,
        Some(r#"{"domain":"-bad.example"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "invalid_domain");
    assert!(management.calls().is_empty());
}

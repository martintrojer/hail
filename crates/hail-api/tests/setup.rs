use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration as ChronoDuration, Utc};
use hail_api::middleware::session::SESSION_COOKIE;
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::setup::{ProvisionError, ProvisionedUser, UserProvisioner};
use hail_api::state::AppState;
use hail_core::{AdminConfig, Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use tokio::sync::Barrier;
use tower::ServiceExt;

async fn fixture_state(admin: Option<AdminConfig>) -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_setup_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", hex::encode(key));
        std::env::remove_var("HAIL_ADMIN__EMAIL");
        std::env::remove_var("HAIL_ADMIN__PASSWORD_HASH");
        std::env::remove_var("HAIL_ADMIN__DISPLAY_NAME");
    }
    let mut config = Config::load_from(None).expect("load config");
    config.admin = admin;

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

fn app(state: AppState) -> Router {
    app_with_provisioner(state, Arc::new(FakeProvisioner::default()))
}

fn app_with_provisioner(state: AppState, provisioner: Arc<FakeProvisioner>) -> Router {
    hail_api::routes::setup::router_with_provisioner(provisioner).with_state(state)
}

fn app_with_protected_auth_and_provisioner(
    state: AppState,
    provisioner: Arc<FakeProvisioner>,
) -> Router {
    let protected =
        hail_api::routes::auth::protected_router().layer(axum::middleware::from_fn_with_state(
            state.clone(),
            hail_api::middleware::auth::require_auth,
        ));

    Router::new()
        .merge(hail_api::routes::setup::router_with_provisioner(
            provisioner,
        ))
        .merge(protected)
        .with_state(state)
}

fn session_cookie_value(set_cookie: &str) -> String {
    let value = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .strip_prefix(&format!("{SESSION_COOKIE}="))
        .expect("hail_session cookie");
    value.to_string()
}

#[derive(Default)]
struct FakeProvisioner {
    calls: AtomicUsize,
    delay: Option<Duration>,
}

impl FakeProvisioner {
    fn with_delay(delay: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            delay: Some(delay),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl UserProvisioner for FakeProvisioner {
    fn provision<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        _password: SecretString,
        _display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ProvisionedUser, ProvisionError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            Ok(ProvisionedUser {
                jmap_account_id: format!("acct-{email}"),
                bearer_token: SecretString::from("fake-bearer-token"),
            })
        })
    }
}

async fn get_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn get_json_with_cookie(
    app: Router,
    uri: &str,
    session_id: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn post_admin(app: Router, body: &str) -> axum::response::Response {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/setup/admin")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "setup-test")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

#[tokio::test]
async fn wizard_state_active_when_empty_and_no_config_admin() {
    let (state, _key) = fixture_state(None).await;
    let (status, json) = get_json(app(state), "/api/setup/state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["wizard_active"], true);
    assert!(json.get("reason").is_none());
}

#[tokio::test]
async fn wizard_state_inactive_when_admin_user_exists() {
    let (state, _key) = fixture_state(None).await;
    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (?1, ?2, NULL, 1, ?3)",
    )
    .bind("admin@example.org")
    .bind("acct")
    .bind(Utc::now())
    .execute(&state.db)
    .await
    .unwrap();

    let (status, json) = get_json(app(state), "/api/setup/state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["wizard_active"], false);
    assert_eq!(json["reason"], "admin_user_exists");
}

#[tokio::test]
async fn wizard_state_inactive_when_config_admin_set() {
    let (state, _key) = fixture_state(Some(AdminConfig {
        email: "operator@example.org".to_string(),
        password_hash: None,
        display_name: Some("Operator".to_string()),
    }))
    .await;

    let (status, json) = get_json(app(state), "/api/setup/state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["wizard_active"], false);
    assert_eq!(json["reason"], "config_admin_set");
}

#[tokio::test]
async fn post_setup_admin_succeeds_and_sets_session_cookie() {
    let (state, key) = fixture_state(None).await;
    let db = state.db.clone();
    let api = app_with_protected_auth_and_provisioner(state, Arc::new(FakeProvisioner::default()));
    let resp = post_admin(
        api.clone(),
        r#"{"email":"Alice@Example.org","password":"correct horse battery","display_name":"Alice","domain":"example.org"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("set-cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("hail_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    let session_id = session_cookie_value(&cookie);
    assert_eq!(session_id.len(), 64);
    assert!(session_id.bytes().all(|b| b.is_ascii_hexdigit()));

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["user"]["email"], "alice@example.org");
    assert_eq!(json["user"]["display_name"], "Alice");
    assert_eq!(json["user"]["is_admin"], true);

    let (me_status, me_json) = get_json_with_cookie(api, "/api/auth/me", &session_id).await;
    assert_eq!(me_status, StatusCode::OK);
    assert_eq!(me_json["user"]["email"], "alice@example.org");
    assert_eq!(me_json["user"]["display_name"], "Alice");
    assert_eq!(me_json["user"]["is_admin"], true);

    let (row_session_id, user_id, token_enc, user_agent, expires_at, created_at, last_used_at): (
        String,
        i64,
        Vec<u8>,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at \
         FROM sessions WHERE id = ?1",
    )
    .bind(&session_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(row_session_id, session_id);
    assert!(user_id > 0);
    let token_plain = hail_core::open(&token_enc, &key).expect("decrypt setup token");
    assert_eq!(token_plain, b"fake-bearer-token");
    assert_eq!(user_agent.as_deref(), Some("setup-test"));
    assert!(expires_at > Utc::now() + ChronoDuration::days(29));
    assert!(expires_at <= created_at + ChronoDuration::days(30) + ChronoDuration::seconds(1));
    assert!(last_used_at >= created_at);
    assert!(last_used_at <= Utc::now() + ChronoDuration::seconds(1));
}

#[tokio::test]
async fn post_setup_admin_twice_returns_409_second_time_without_reprovisioning() {
    let (state, _key) = fixture_state(None).await;
    let db = state.db.clone();
    let provisioner = Arc::new(FakeProvisioner::default());
    let first = post_admin(
        app_with_provisioner(state.clone(), provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org"}"#,
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(provisioner.call_count(), 1);

    let second = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"bob@example.org","password":"correct horse battery","domain":"example.org"}"#,
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(provisioner.call_count(), 1);
    let bytes = second.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_disabled");

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(user_count, 1);
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(session_count, 1);
}

#[tokio::test]
async fn concurrent_setup_admin_posts_only_provision_once() {
    let (state, _key) = fixture_state(None).await;
    let db = state.db.clone();
    let provisioner = Arc::new(FakeProvisioner::with_delay(Duration::from_millis(50)));
    let barrier = Arc::new(Barrier::new(3));

    let first_state = state.clone();
    let first_provisioner = provisioner.clone();
    let first_barrier = barrier.clone();
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        post_admin(
            app_with_provisioner(first_state, first_provisioner),
            r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org"}"#,
        )
        .await
    });

    let second_state = state;
    let second_provisioner = provisioner.clone();
    let second_barrier = barrier.clone();
    let second = tokio::spawn(async move {
        second_barrier.wait().await;
        post_admin(
            app_with_provisioner(second_state, second_provisioner),
            r#"{"email":"bob@example.org","password":"correct horse battery","domain":"example.org"}"#,
        )
        .await
    });

    barrier.wait().await;
    let first = first.await.unwrap();
    let second = second.await.unwrap();
    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "expected one created response, got {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1,
        "expected one conflict response, got {statuses:?}"
    );
    assert_eq!(provisioner.call_count(), 1);

    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(user_count, 1);
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(session_count, 1);
}

#[tokio::test]
async fn post_setup_admin_returns_409_and_does_not_provision_when_config_admin_set() {
    let (state, _key) = fixture_state(Some(AdminConfig {
        email: "operator@example.org".to_string(),
        password_hash: None,
        display_name: Some("Operator".to_string()),
    }))
    .await;
    let provisioner = Arc::new(FakeProvisioner::default());

    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(provisioner.call_count(), 0);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_disabled");
}

#[tokio::test]
async fn post_setup_admin_returns_409_and_does_not_provision_when_admin_user_exists() {
    let (state, _key) = fixture_state(None).await;
    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, display_name, is_admin, created_at) \
         VALUES (?1, ?2, NULL, 1, ?3)",
    )
    .bind("admin@example.org")
    .bind("acct")
    .bind(Utc::now())
    .execute(&state.db)
    .await
    .unwrap();
    let provisioner = Arc::new(FakeProvisioner::default());

    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(provisioner.call_count(), 0);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_disabled");
}

#[tokio::test]
async fn post_setup_admin_rejects_invalid_email_or_short_password() {
    let (state, _key) = fixture_state(None).await;
    let bad_email = post_admin(
        app(state.clone()),
        r#"{"email":"not-an-email","password":"correct horse battery","domain":"example.org"}"#,
    )
    .await;
    assert_eq!(bad_email.status(), StatusCode::BAD_REQUEST);
    let bytes = bad_email.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_input");
    assert_eq!(json["field"], "email");

    let short_password = post_admin(
        app(state),
        r#"{"email":"alice@example.org","password":"too-short","domain":"example.org"}"#,
    )
    .await;
    assert_eq!(short_password.status(), StatusCode::BAD_REQUEST);
    let bytes = short_password
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_input");
    assert_eq!(json["field"], "password");
}

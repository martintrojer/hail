use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header};
use axum::response::IntoResponse;
use chrono::{Duration as ChronoDuration, Utc};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::middleware::session::SESSION_COOKIE;
use hail_api::routes::provider_accounts::{
    GmailAuthorizationRequest, GmailOAuthClient, GmailOAuthError, GmailProfile, GmailTokenExchange,
};
use hail_api::routes::setup::{ProvisionError, ProvisionedUser, UserProvisioner};
use hail_api::state::AppState;
use hail_core::{AdminConfig, KEY_LEN, MailBackend, SetupConfig};
use hail_db::connect;
use hail_test::fixture_config;
use http_body_util::BodyExt;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Barrier;
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum FakeManagementMode {
    Success,
    Auth401,
    DomainExists,
    PrincipalExists,
}

async fn fixture_state(admin: Option<AdminConfig>) -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_setup_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    let mut config = fixture_config(&url, &key);
    config.admin = admin;
    config.setup = SetupConfig {
        bootstrap_enabled: true,
        bootstrap_token: Some(SecretString::from("setup-test-bootstrap-token")),
    };
    config.stalwart.management_url = None;

    let state = AppState {
        db: db.clone(),
        config,
        server_key: Arc::new(key),
        auth_rate_limiter: Arc::new(IpRateLimiter::default()),
        mail: hail_api::test_support::fake_cached_mail(db.clone()),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProvisionCall {
    email: String,
    display_name: Option<String>,
    domain: String,
    stalwart_admin_username: String,
    stalwart_admin_password: String,
}

#[derive(Default)]
struct FakeProvisioner {
    calls: AtomicUsize,
    delay: Option<Duration>,
    fail: bool,
    observed: Mutex<Vec<ProvisionCall>>,
}

impl FakeProvisioner {
    fn with_delay(delay: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            delay: Some(delay),
            fail: false,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            delay: None,
            fail: true,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn observed_calls(&self) -> Vec<ProvisionCall> {
        self.observed.lock().expect("observed calls").clone()
    }
}

#[derive(Default)]
struct FakeGmailOAuthClient {
    auth_requests: Mutex<Vec<GmailAuthorizationRequest>>,
    exchange_codes: Mutex<Vec<String>>,
}

impl GmailOAuthClient for FakeGmailOAuthClient {
    fn authorization_url(&self, req: GmailAuthorizationRequest) -> Result<String, GmailOAuthError> {
        self.auth_requests
            .lock()
            .expect("auth requests")
            .push(req.clone());
        Ok(format!(
            "https://accounts.example.test/oauth?state={}",
            req.state
        ))
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        _redirect_uri: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<GmailTokenExchange, GmailOAuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.exchange_codes
                .lock()
                .expect("exchange codes")
                .push(code.to_owned());
            Ok(GmailTokenExchange {
                access_token: SecretString::from("setup-access-token"),
                refresh_token: Some(SecretString::from("setup-refresh-token")),
                expires_at: None,
                granted_scopes: vec![
                    "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
                    "https://www.googleapis.com/auth/gmail.send".to_owned(),
                ],
                profile: GmailProfile {
                    email: "alice@example.org".to_owned(),
                    history_id: Some("h-1".to_owned()),
                },
            })
        })
    }

    fn revoke_refresh_token<'a>(
        &'a self,
        _refresh_token: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<(), GmailOAuthError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

impl UserProvisioner for FakeProvisioner {
    fn provision<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        _password: SecretString,
        display_name: Option<&'a str>,
        domain: &'a str,
        stalwart_admin_username: &'a str,
        stalwart_admin_password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ProvisionedUser, ProvisionError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.observed
            .lock()
            .expect("observed calls")
            .push(ProvisionCall {
                email: email.to_string(),
                display_name: display_name.map(str::to_string),
                domain: domain.to_string(),
                stalwart_admin_username: stalwart_admin_username.to_string(),
                stalwart_admin_password: stalwart_admin_password.expose_secret().to_string(),
            });
        Box::pin(async move {
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if self.fail {
                return Err(ProvisionError::Management(
                    "fake provision failure".to_string(),
                ));
            }
            Ok(ProvisionedUser {
                jmap_account_id: format!("acct-{email}"),
                bearer_token: SecretString::from("fake-bearer-token"),
            })
        })
    }
}

#[derive(Debug, Clone)]
struct ManagementCall {
    path: String,
    body: serde_json::Value,
}

async fn start_fake_management(
    mode: FakeManagementMode,
) -> (
    String,
    Arc<Mutex<Vec<ManagementCall>>>,
    tokio::task::JoinHandle<()>,
) {
    async fn handler(
        State(state): State<(FakeManagementMode, Arc<Mutex<Vec<ManagementCall>>>)>,
        headers: axum::http::HeaderMap,
        uri: axum::http::Uri,
        body: String,
    ) -> impl IntoResponse {
        let (mode, calls) = state;
        let path = uri.path().to_string();
        let body_json = if body.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&body).unwrap_or_else(|_| serde_json::json!({"raw": body}))
        };
        calls.lock().expect("calls").push(ManagementCall {
            path: path.clone(),
            body: body_json.clone(),
        });

        if path == "/api/auth" {
            assert_eq!(body_json["type"], "authCode");
            assert_eq!(body_json["accountName"], "admin");
            assert_eq!(body_json["accountSecret"], "admin1234");
            return match mode {
                FakeManagementMode::Auth401 => (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({
                        "type": "about:blank",
                        "status": 401,
                        "title": "Unauthorized",
                        "detail": "invalid Stalwart admin credentials"
                    })),
                ),
                _ => (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({ "client_code": "client-code" })),
                ),
            };
        }

        if path == "/auth/token" {
            let raw = body_json
                .get("raw")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(raw.contains("code=client-code"), "token body: {raw}");
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!({ "access_token": "management-token" })),
            );
        }

        if path == "/.well-known/jmap" {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer management-token")
            );
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "primaryAccounts": { "urn:stalwart:jmap": "mgmt-account" }
                })),
            );
        }

        if path == "/jmap/" {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer management-token")
            );
            assert_eq!(
                body_json["using"],
                serde_json::json!(["urn:ietf:params:jmap:core", "urn:stalwart:jmap"])
            );
            let calls = body_json["methodCalls"]
                .as_array()
                .expect("methodCalls array");
            let method = calls[0][0].as_str().expect("method name");
            return match method {
                "x:Domain/set" => {
                    assert_eq!(calls[0][1]["accountId"], "mgmt-account");
                    assert_eq!(calls[0][1]["create"]["new-0"]["name"], "example.org");
                    match mode {
                        FakeManagementMode::DomainExists => (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "methodResponses": [["x:Domain/set", {"accountId": "mgmt-account", "notCreated": {"new-0": {"type": "primaryKeyViolation", "properties": ["name"], "objectId": {"object": "Domain", "id": "domain-id"}}}}, "set-0"]]
                            })),
                        ),
                        _ => (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "methodResponses": [["x:Domain/set", {"accountId": "mgmt-account", "created": {"new-0": {"id": "domain-id"}}}, "set-0"]]
                            })),
                        ),
                    }
                }
                "x:Domain/query" => (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "methodResponses": [
                            ["x:Domain/query", {"accountId": "mgmt-account", "ids": ["domain-id"]}, "0"],
                            ["x:Domain/get", {"accountId": "mgmt-account", "list": [{"id": "domain-id", "name": "example.org"}], "notFound": []}, "1"]
                        ]
                    })),
                ),
                "x:Account/set" => {
                    assert_eq!(calls[0][1]["accountId"], "mgmt-account");
                    let account = &calls[0][1]["create"]["new-0"];
                    assert_eq!(account["@type"], "User");
                    assert_eq!(account["name"], "alice");
                    assert_eq!(account["domainId"], "domain-id");
                    assert_eq!(account["description"], "Alice");
                    assert_eq!(account["credentials"]["0"]["@type"], "Password");
                    assert_eq!(
                        account["credentials"]["0"]["secret"],
                        "correct horse battery"
                    );
                    assert_eq!(account["roles"]["@type"], "User");
                    match mode {
                        FakeManagementMode::PrincipalExists => (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "methodResponses": [["x:Account/set", {"accountId": "mgmt-account", "notCreated": {"new-0": {"type": "primaryKeyViolation", "properties": ["name", "domainId"]}}}, "set-0"]]
                            })),
                        ),
                        _ => (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "methodResponses": [["x:Account/set", {"accountId": "mgmt-account", "created": {"new-0": {"id": "user-id"}}}, "set-0"]]
                            })),
                        ),
                    }
                }
                other => panic!("unexpected JMAP method {other}"),
            };
        }

        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "title": "Not Found" })),
        )
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .fallback(axum::routing::any(handler))
        .with_state((mode, calls.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake management");
    let addr = listener.local_addr().expect("fake management addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .expect("fake management server");
    });
    (format!("http://{addr}"), calls, handle)
}

async fn start_fake_jmap() -> (String, tokio::task::JoinHandle<()>) {
    async fn session(headers: axum::http::HeaderMap) -> impl IntoResponse {
        assert!(
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("Basic "))
        );
        let base_url = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(|host| format!("http://{host}"))
            .unwrap_or_else(|| "http://127.0.0.1:0".to_owned());
        axum::Json(serde_json::json!({
            "capabilities": {
                "urn:ietf:params:jmap:core": {
                    "maxSizeUpload": 50000000,
                    "maxConcurrentUpload": 4,
                    "maxSizeRequest": 10000000,
                    "maxConcurrentRequests": 4,
                    "maxCallsInRequest": 16,
                    "maxObjectsInGet": 500,
                    "maxObjectsInSet": 500,
                    "collationAlgorithms": ["i;unicode-casemap"]
                },
                "urn:ietf:params:jmap:mail": {}
            },
            "accounts": {
                "account-test": {
                    "name": "Alice",
                    "isPersonal": true,
                    "isReadOnly": false,
                    "accountCapabilities": { "urn:ietf:params:jmap:mail": {} }
                }
            },
            "primaryAccounts": { "urn:ietf:params:jmap:mail": "account-test" },
            "username": "alice@example.org",
            "apiUrl": format!("{base_url}/jmap/"),
            "downloadUrl": format!("{base_url}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
            "uploadUrl": format!("{base_url}/upload/{{accountId}}/"),
            "eventSourceUrl": format!("{base_url}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
            "state": "fake-state"
        }))
    }

    let app = axum::Router::new().route("/.well-known/jmap", axum::routing::get(session));
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
    post_admin_raw(
        app,
        &body_with_bootstrap_token(body, "setup-test-bootstrap-token"),
    )
    .await
}

async fn post_admin_raw(app: Router, body: &str) -> axum::response::Response {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/setup/admin")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "setup-test")
        .extension(axum::extract::ConnectInfo(
            "127.0.0.1:10000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

fn body_with_bootstrap_token(body: &str, token: &str) -> String {
    let mut json: serde_json::Value = serde_json::from_str(body).expect("setup admin json body");
    json["bootstrap_token"] = serde_json::Value::String(token.to_string());
    json.to_string()
}

#[tokio::test]
async fn wizard_state_active_when_empty_and_no_config_admin() {
    let (state, _key) = fixture_state(None).await;
    let (status, json) = get_json(app(state), "/api/setup/state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["wizard_active"], true);
    assert_eq!(json["backend"], "jmap");
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
    assert_eq!(json["backend"], "jmap");
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
    assert_eq!(json["backend"], "jmap");
    assert_eq!(json["reason"], "config_admin_set");
}

#[tokio::test]
async fn post_setup_admin_requires_enabled_bootstrap_token() {
    let (mut disabled_state, _key) = fixture_state(None).await;
    disabled_state.config.setup.bootstrap_enabled = false;
    let disabled_provisioner = Arc::new(FakeProvisioner::default());
    let disabled = post_admin(
        app_with_provisioner(disabled_state, disabled_provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::FORBIDDEN);
    assert_eq!(disabled_provisioner.call_count(), 0);
    let bytes = disabled.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_bootstrap_required");

    let (mut missing_token_state, _key) = fixture_state(None).await;
    missing_token_state.config.setup.bootstrap_token = None;
    let missing_token_provisioner = Arc::new(FakeProvisioner::default());
    let missing_token = post_admin(
        app_with_provisioner(missing_token_state, missing_token_provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(missing_token.status(), StatusCode::FORBIDDEN);
    assert_eq!(missing_token_provisioner.call_count(), 0);
}

#[tokio::test]
async fn post_setup_admin_rejects_missing_or_wrong_bootstrap_token() {
    let (state, _key) = fixture_state(None).await;
    let provisioner = Arc::new(FakeProvisioner::default());

    let missing = post_admin_raw(
        app_with_provisioner(state.clone(), provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    assert_eq!(provisioner.call_count(), 0);
    let bytes = missing.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_bootstrap_required");

    let wrong = post_admin_raw(
        app_with_provisioner(state, provisioner.clone()),
        &body_with_bootstrap_token(
            r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
            "wrong-token",
        ),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    assert_eq!(provisioner.call_count(), 0);
    let bytes = wrong.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_bootstrap_required");
}

#[tokio::test]
async fn post_setup_admin_rate_limited_by_forwarded_ip_without_provisioning() {
    let (mut state, _key) = fixture_state(None).await;
    state.auth_rate_limiter = Arc::new(IpRateLimiter::new(2, Duration::from_secs(60)));
    let provisioner = Arc::new(FakeProvisioner::default());
    let body = body_with_bootstrap_token(
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
        "setup-test-bootstrap-token",
    );

    for _ in 0..2 {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/setup/admin")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::USER_AGENT, "setup-test")
            .header("x-forwarded-for", "203.0.113.77")
            .extension(axum::extract::ConnectInfo(
                "127.0.0.1:10000".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::from(body.clone()))
            .unwrap();
        let resp = app_with_provisioner(state.clone(), provisioner.clone())
            .oneshot(req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        sqlx::query("DELETE FROM sessions")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM users")
            .execute(&state.db)
            .await
            .unwrap();
    }

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/setup/admin")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "setup-test")
        .header("x-forwarded-for", "203.0.113.77")
        .extension(axum::extract::ConnectInfo(
            "127.0.0.1:10000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::from(body))
        .unwrap();
    let resp = app_with_provisioner(state, provisioner.clone())
        .oneshot(req)
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "rate_limited");
    assert_eq!(provisioner.call_count(), 2);
}

#[tokio::test]
async fn post_setup_admin_succeeds_and_sets_session_cookie() {
    let (state, key) = fixture_state(None).await;
    let db = state.db.clone();
    let api = app_with_protected_auth_and_provisioner(state, Arc::new(FakeProvisioner::default()));
    let resp = post_admin(
        api.clone(),
        r#"{"email":"Alice@Example.org","password":"correct horse battery","display_name":"Alice","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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

    type SessionRow = (
        String,
        i64,
        Vec<u8>,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
    );
    let (row_session_id, user_id, token_enc, user_agent, expires_at, created_at, last_used_at): SessionRow = sqlx::query_as(
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
async fn openapi_includes_flavour_aware_setup_schema() {
    let (state, _key) = fixture_state(None).await;
    let (status, json) = get_json(hail_api::build_router(state, false), "/api/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.pointer("/components/schemas/SetupStateResponse/required"),
        Some(&serde_json::json!(["wizard_active", "backend"])),
    );
    assert_eq!(
        json.pointer("/components/schemas/SetupBackend/enum"),
        Some(&serde_json::json!(["gmail", "jmap"])),
    );
    assert!(
        json.pointer("/paths/~1api~1setup~1gmail~1connect/post")
            .is_some()
    );
    assert!(
        json.pointer("/paths/~1api~1setup~1gmail~1callback/get")
            .is_some()
    );
}

#[tokio::test]
async fn post_setup_admin_twice_returns_409_second_time_without_reprovisioning() {
    let (state, _key) = fixture_state(None).await;
    let db = state.db.clone();
    let provisioner = Arc::new(FakeProvisioner::default());
    let first = post_admin(
        app_with_provisioner(state.clone(), provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(provisioner.call_count(), 1);

    let second = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"bob@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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
            r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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
            r#"{"email":"bob@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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
        r#"{"email":"not-an-email","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(bad_email.status(), StatusCode::BAD_REQUEST);
    let bytes = bad_email.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_input");
    assert_eq!(json["detail"], "email");

    let short_password = post_admin(
        app(state),
        r#"{"email":"alice@example.org","password":"too-short","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
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
    assert_eq!(json["detail"], "password");
}

#[tokio::test]
async fn post_setup_admin_rejects_invalid_domain() {
    let invalid_domains = [
        "",
        "localhost",
        ".example.org",
        "exa mple.org",
        "example..org",
        "-bad.example",
        "bad-.example",
        "123.456",
    ];

    for domain in invalid_domains {
        let (state, _key) = fixture_state(None).await;
        let provisioner = Arc::new(FakeProvisioner::default());
        let body = serde_json::json!({
            "email": "alice@example.org",
            "password": "correct horse battery",
            "domain": domain,
            "stalwart_admin_username": "admin",
            "stalwart_admin_password": "admin1234",
        })
        .to_string();

        let resp = post_admin(app_with_provisioner(state, provisioner.clone()), &body).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "domain={domain:?}");
        assert_eq!(provisioner.call_count(), 0, "domain={domain:?}");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"], "invalid_input", "domain={domain:?}");
        assert_eq!(json["detail"], "domain", "domain={domain:?}");
    }
}

#[tokio::test]
async fn post_setup_admin_normalizes_trailing_dot_domain() {
    let (state, _key) = fixture_state(None).await;
    let provisioner = Arc::new(FakeProvisioner::default());

    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"Example.ORG.","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(
        provisioner.observed_calls(),
        vec![ProvisionCall {
            email: "alice@example.org".to_string(),
            display_name: None,
            domain: "example.org".to_string(),
            stalwart_admin_username: "admin".to_string(),
            stalwart_admin_password: "admin1234".to_string(),
        }]
    );
}

#[tokio::test]
async fn post_setup_admin_rejects_email_domain_mismatch() {
    let (state, _key) = fixture_state(None).await;
    let provisioner = Arc::new(FakeProvisioner::default());

    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.net","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(provisioner.call_count(), 0);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_input");
    assert_eq!(json["detail"], "email");
}

#[tokio::test]
async fn post_setup_admin_trims_display_name_and_omits_empty_display_name() {
    let (trim_state, _key) = fixture_state(None).await;
    let trim_db = trim_state.db.clone();
    let trim_provisioner = Arc::new(FakeProvisioner::default());
    let trim_resp = post_admin(
        app_with_provisioner(trim_state, trim_provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"  Alice Example  ","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(trim_resp.status(), StatusCode::CREATED);
    assert_eq!(
        trim_provisioner.observed_calls(),
        vec![ProvisionCall {
            email: "alice@example.org".to_string(),
            display_name: Some("Alice Example".to_string()),
            domain: "example.org".to_string(),
            stalwart_admin_username: "admin".to_string(),
            stalwart_admin_password: "admin1234".to_string(),
        }]
    );
    let stored_display_name: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM users WHERE email = ?1")
            .bind("alice@example.org")
            .fetch_one(&trim_db)
            .await
            .unwrap();
    assert_eq!(stored_display_name.as_deref(), Some("Alice Example"));

    let (empty_state, _key) = fixture_state(None).await;
    let empty_db = empty_state.db.clone();
    let empty_provisioner = Arc::new(FakeProvisioner::default());
    let empty_resp = post_admin(
        app_with_provisioner(empty_state, empty_provisioner.clone()),
        r#"{"email":"bob@example.org","password":"correct horse battery","display_name":"   ","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(empty_resp.status(), StatusCode::CREATED);
    assert_eq!(
        empty_provisioner.observed_calls(),
        vec![ProvisionCall {
            email: "bob@example.org".to_string(),
            display_name: None,
            domain: "example.org".to_string(),
            stalwart_admin_username: "admin".to_string(),
            stalwart_admin_password: "admin1234".to_string(),
        }]
    );
    let stored_display_name: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM users WHERE email = ?1")
            .bind("bob@example.org")
            .fetch_one(&empty_db)
            .await
            .unwrap();
    assert_eq!(stored_display_name, None);
}

#[tokio::test]
async fn post_setup_admin_passes_lowercased_email_to_provisioner() {
    let (state, _key) = fixture_state(None).await;
    let provisioner = Arc::new(FakeProvisioner::default());

    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"  Alice@Example.ORG  ","password":"correct horse battery","domain":" EXAMPLE.org ","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(
        provisioner.observed_calls(),
        vec![ProvisionCall {
            email: "alice@example.org".to_string(),
            display_name: None,
            domain: "example.org".to_string(),
            stalwart_admin_username: "admin".to_string(),
            stalwart_admin_password: "admin1234".to_string(),
        }]
    );
}

#[tokio::test]
async fn post_setup_admin_failing_provisioner_leaves_no_users_or_sessions() {
    let (state, _key) = fixture_state(None).await;
    let db = state.db.clone();
    let provisioner = Arc::new(FakeProvisioner::failing());

    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_provision_failed");
    assert_eq!(json["detail"], "fake provision failure");
    assert_eq!(provisioner.call_count(), 1);
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(user_count, 0);
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(session_count, 0);
}

#[tokio::test]
async fn stalwart_provisioner_uses_auth_token_domain_principal_sequence() {
    let (management_url, calls, _management) =
        start_fake_management(FakeManagementMode::Success).await;
    let (jmap_url, _jmap) = start_fake_jmap().await;
    let (mut state, _key) = fixture_state(None).await;
    state.config.stalwart.management_url = Some(management_url);
    state.config.stalwart.jmap_url = jmap_url;

    let resp = post_admin(
        hail_api::routes::setup::router().with_state(state),
        r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"Alice","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let observed = calls.lock().expect("calls").clone();
    let paths: Vec<String> = observed.iter().map(|call| call.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            "/api/auth",
            "/auth/token",
            "/.well-known/jmap",
            "/jmap/",
            "/.well-known/jmap",
            "/jmap/",
            "/jmap/",
        ]
    );
    let domain_set = observed
        .iter()
        .find(|call| {
            call.body.pointer("/methodCalls/0/0") == Some(&serde_json::json!("x:Domain/set"))
        })
        .expect("domain set call");
    assert_eq!(
        domain_set
            .body
            .pointer("/methodCalls/0/1/create/new-0/name"),
        Some(&serde_json::json!("example.org"))
    );
    let account_set = observed
        .iter()
        .find(|call| {
            call.body.pointer("/methodCalls/0/0") == Some(&serde_json::json!("x:Account/set"))
        })
        .expect("account set call");
    assert_eq!(
        account_set
            .body
            .pointer("/methodCalls/0/1/create/new-0/credentials/0/@type"),
        Some(&serde_json::json!("Password"))
    );
}

#[tokio::test]
async fn stalwart_provisioner_surfaces_auth_401_detail_as_setup_error() {
    let (management_url, _calls, _management) =
        start_fake_management(FakeManagementMode::Auth401).await;
    let (jmap_url, _jmap) = start_fake_jmap().await;
    let (mut state, _key) = fixture_state(None).await;
    state.config.stalwart.management_url = Some(management_url);
    state.config.stalwart.jmap_url = jmap_url;

    let resp = post_admin(
        hail_api::routes::setup::router().with_state(state),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "setup_provision_failed");
    assert_eq!(json["detail"], "invalid Stalwart admin credentials");
}

#[tokio::test]
async fn stalwart_provisioner_treats_existing_domain_as_success() {
    let (management_url, calls, _management) =
        start_fake_management(FakeManagementMode::DomainExists).await;
    let (jmap_url, _jmap) = start_fake_jmap().await;
    let (mut state, _key) = fixture_state(None).await;
    state.config.stalwart.management_url = Some(management_url);
    state.config.stalwart.jmap_url = jmap_url;

    let resp = post_admin(
        hail_api::routes::setup::router().with_state(state),
        r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"Alice","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(
        calls
            .lock()
            .expect("calls")
            .iter()
            .any(|call| call.body.pointer("/methodCalls/0/0")
                == Some(&serde_json::json!("x:Domain/set")))
    );
}

#[tokio::test]
async fn stalwart_provisioner_treats_existing_principal_as_success() {
    let (management_url, calls, _management) =
        start_fake_management(FakeManagementMode::PrincipalExists).await;
    let (jmap_url, _jmap) = start_fake_jmap().await;
    let (mut state, _key) = fixture_state(None).await;
    state.config.stalwart.management_url = Some(management_url);
    state.config.stalwart.jmap_url = jmap_url;

    let resp = post_admin(
        hail_api::routes::setup::router().with_state(state),
        r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"Alice","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(
        calls
            .lock()
            .expect("calls")
            .iter()
            .any(|call| call.body.pointer("/methodCalls/0/0")
                == Some(&serde_json::json!("x:Account/set")))
    );
}

#[tokio::test]
async fn wizard_state_reports_gmail_backend() {
    let (mut state, _key) = fixture_state(None).await;
    state.config.mail.backend = MailBackend::Gmail;
    let (status, json) = get_json(app(state), "/api/setup/state").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["wizard_active"], true);
    assert_eq!(json["backend"], "gmail");
}

#[tokio::test]
async fn jmap_setup_admin_is_hidden_for_gmail_backend() {
    let (mut state, _key) = fixture_state(None).await;
    state.config.mail.backend = MailBackend::Gmail;
    let provisioner = Arc::new(FakeProvisioner::default());
    let resp = post_admin(
        app_with_provisioner(state, provisioner.clone()),
        r#"{"email":"alice@example.org","password":"correct horse battery","domain":"example.org","stalwart_admin_username":"admin","stalwart_admin_password":"admin1234"}"#,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(provisioner.call_count(), 0);
}

#[tokio::test]
async fn gmail_setup_connect_requires_csrf_and_returns_authorization_url() {
    let (mut state, _key) = fixture_state(None).await;
    state.config.mail.backend = MailBackend::Gmail;
    state.config.mail.gmail.oauth_client_id = Some("gmail-client-id".to_owned());
    let client = Arc::new(FakeGmailOAuthClient::default());
    let app = hail_api::routes::setup::router_with_deps(
        Arc::new(FakeProvisioner::default()),
        client.clone(),
    )
    .with_state(state);

    let missing_csrf = Request::builder()
        .method(Method::POST)
        .uri("/api/setup/gmail/connect")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"Alice"}"#,
        ))
        .unwrap();
    let missing_csrf = app.clone().oneshot(missing_csrf).await.unwrap();
    assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/setup/gmail/connect")
        .header(header::CONTENT_TYPE, "application/json")
        .header(hail_api::middleware::auth::CSRF_HEADER, "1")
        .body(Body::from(
            r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"Alice"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["authorization_url"]
            .as_str()
            .unwrap()
            .contains("state=")
    );
    let auth_requests = client.auth_requests.lock().expect("auth requests");
    assert_eq!(auth_requests.len(), 1);
    assert_eq!(auth_requests[0].client_id, "gmail-client-id");
}

#[tokio::test]
async fn gmail_setup_callback_creates_local_user_account_policy_and_session() {
    let (mut state, key) = fixture_state(None).await;
    state.config.mail.backend = MailBackend::Gmail;
    state.config.mail.gmail.oauth_client_id = Some("gmail-client-id".to_owned());
    let client = Arc::new(FakeGmailOAuthClient::default());
    let app = hail_api::routes::setup::router_with_deps(
        Arc::new(FakeProvisioner::default()),
        client.clone(),
    )
    .with_state(state.clone());

    let connect = Request::builder()
        .method(Method::POST)
        .uri("/api/setup/gmail/connect")
        .header(header::CONTENT_TYPE, "application/json")
        .header(hail_api::middleware::auth::CSRF_HEADER, "1")
        .body(Body::from(
            r#"{"email":"alice@example.org","password":"correct horse battery","display_name":"Alice"}"#,
        ))
        .unwrap();
    let connect = app.clone().oneshot(connect).await.unwrap();
    assert_eq!(connect.status(), StatusCode::OK);
    let body = connect.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let auth_url = url::Url::parse(json["authorization_url"].as_str().unwrap()).unwrap();
    let state_token = auth_url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .unwrap();

    let callback = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/setup/gmail/callback?state={state_token}&code=ok"
        ))
        .header(header::USER_AGENT, "setup-test")
        .body(Body::empty())
        .unwrap();
    let callback = app.oneshot(callback).await.unwrap();
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    let cookie = callback
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let session_id = session_cookie_value(&cookie);

    let account: (String, String, String) =
        sqlx::query_as("SELECT backend_kind, provider_email, sync_status FROM mail_accounts")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(
        account,
        (
            "gmail".to_owned(),
            "alice@example.org".to_owned(),
            "active".to_owned()
        )
    );

    let policy: (String, String) = sqlx::query_as("SELECT mode, backfill FROM cache_policy")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(policy, ("bounded".to_owned(), "incremental".to_owned()));

    let token_enc: Vec<u8> =
        sqlx::query_scalar("SELECT jmap_token_enc FROM sessions WHERE id = ?1")
            .bind(&session_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    let token_plain = hail_core::open(&token_enc, &key).expect("decrypt setup token");
    assert_eq!(token_plain, b"correct horse battery");
    assert_eq!(
        client
            .exchange_codes
            .lock()
            .expect("exchange codes")
            .as_slice(),
        ["ok"]
    );
}

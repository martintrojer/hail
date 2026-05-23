use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::admin_users::{ManagedUser, StalwartUserManagement, UserManagementError};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_admin_users_test_{uniq}?mode=memory&cache=shared");
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

async fn seed_session(
    state: &AppState,
    key: &[u8; KEY_LEN],
    email: &str,
    is_admin: bool,
) -> (i64, String) {
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
    (user_id, session_id)
}

async fn seed_user(state: &AppState, email: &str, is_admin: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
    )
    .bind(email)
    .bind(format!("account-{email}"))
    .bind(if is_admin { 1_i64 } else { 0_i64 })
    .bind(Utc::now())
    .fetch_one(&state.db)
    .await
    .expect("insert user")
}

fn app(state: AppState, management: Arc<FakeUserManagement>) -> Router {
    let protected = hail_api::routes::admin_users::router_with_management(management).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone, Default)]
struct FakeUserManagement {
    users: Arc<Mutex<Vec<ManagedUser>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeUserManagement {
    fn with_users(users: &[(&str, &str, Option<&str>)]) -> Self {
        Self {
            users: Arc::new(Mutex::new(
                users
                    .iter()
                    .map(|(email, account, display)| ManagedUser {
                        email: (*email).to_string(),
                        jmap_account_id: (*account).to_string(),
                        display_name: display.map(str::to_owned),
                    })
                    .collect(),
            )),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl StalwartUserManagement for FakeUserManagement {
    fn list_users<'a>(
        &'a self,
        _state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ManagedUser>, UserManagementError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push("list".to_string());
            Ok(self.users.lock().unwrap().clone())
        })
    }

    fn create_user<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        _password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("create:{email}"));
            let user = ManagedUser {
                email: email.to_string(),
                jmap_account_id: format!("account-{email}"),
                display_name: display_name.map(str::to_owned),
            };
            self.users.lock().unwrap().push(user.clone());
            Ok(user)
        })
    }

    fn delete_user<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("delete:{email}"));
            self.users.lock().unwrap().retain(|u| u.email != email);
            Ok(())
        })
    }

    fn reset_password<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        _password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("reset:{email}"));
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: format!("account-{email}"),
                display_name: Some("Reset User".to_string()),
            })
        })
    }
}

async fn request(
    state: AppState,
    management: Arc<FakeUserManagement>,
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
    let management = Arc::new(FakeUserManagement::default());

    let resp = request(
        state,
        management,
        Method::GET,
        "/api/admin/users",
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
    let (_id, sid) = seed_session(&state, &key, "alice@example.org", false).await;
    let management = Arc::new(FakeUserManagement::default());

    let resp = request(
        state,
        management,
        Method::GET,
        "/api/admin/users",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(resp).await["error"], "admin_required");
}

#[tokio::test]
async fn list_uses_fake_management_and_mirrors_users() {
    let (state, key) = fixture_state().await;
    let (_admin_id, sid) = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeUserManagement::with_users(&[
        ("bob@example.org", "bob-account", Some("Bob")),
        ("alice@example.org", "alice-account", None),
    ]));

    let resp = request(
        state.clone(),
        management.clone(),
        Method::GET,
        "/api/admin/users",
        Some(&sid),
        false,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["users"][0]["email"], "admin@example.org");
    assert_eq!(json["users"][1]["email"], "alice@example.org");
    assert_eq!(json["users"][2]["display_name"], "Bob");
    assert_eq!(management.calls(), vec!["list"]);

    let mirrored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(mirrored, 3);
}

#[tokio::test]
async fn create_uses_fake_management_and_mirrors_local_user() {
    let (state, key) = fixture_state().await;
    let (_admin_id, sid) = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeUserManagement::default());

    let resp = request(
        state.clone(),
        management.clone(),
        Method::POST,
        "/api/admin/users",
        Some(&sid),
        true,
        Some(r#"{"email":" Bob@Example.ORG ","password":"correct horse battery","display_name":" Bob "}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = json_body(resp).await;
    assert_eq!(json["user"]["email"], "bob@example.org");
    assert_eq!(json["user"]["display_name"], "Bob");
    assert_eq!(management.calls(), vec!["create:bob@example.org"]);

    let db_email: String =
        sqlx::query_scalar("SELECT email FROM users WHERE email = 'bob@example.org'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(db_email, "bob@example.org");
}

#[tokio::test]
async fn delete_uses_fake_management_and_deletes_local_user() {
    let (state, key) = fixture_state().await;
    let (_admin_id, sid) = seed_session(&state, &key, "admin@example.org", true).await;
    let target_id = seed_user(&state, "bob@example.org", false).await;
    let management = Arc::new(FakeUserManagement::default());

    let resp = request(
        state.clone(),
        management.clone(),
        Method::DELETE,
        &format!("/api/admin/users/{target_id}"),
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(management.calls(), vec!["delete:bob@example.org"]);
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?1)")
        .bind(target_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(exists, 0);
}

#[tokio::test]
async fn delete_rejects_current_admin_self() {
    let (state, key) = fixture_state().await;
    let (admin_id, sid) = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeUserManagement::default());

    let resp = request(
        state,
        management.clone(),
        Method::DELETE,
        &format!("/api/admin/users/{admin_id}"),
        Some(&sid),
        true,
        None,
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(resp).await["error"], "cannot_delete_self");
    assert!(management.calls().is_empty());
}

#[tokio::test]
async fn reset_password_uses_fake_management() {
    let (state, key) = fixture_state().await;
    let (_admin_id, sid) = seed_session(&state, &key, "admin@example.org", true).await;
    let target_id = seed_user(&state, "bob@example.org", false).await;
    let management = Arc::new(FakeUserManagement::default());

    let resp = request(
        state,
        management.clone(),
        Method::POST,
        &format!("/api/admin/users/{target_id}/reset-password"),
        Some(&sid),
        true,
        Some(r#"{"password":"new correct horse"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["user"]["email"], "bob@example.org");
    assert_eq!(management.calls(), vec!["reset:bob@example.org"]);
}

#[tokio::test]
async fn invalid_email_or_short_password_return_400_without_management() {
    let (state, key) = fixture_state().await;
    let (_admin_id, sid) = seed_session(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(FakeUserManagement::default());

    let bad_email = request(
        state.clone(),
        management.clone(),
        Method::POST,
        "/api/admin/users",
        Some(&sid),
        true,
        Some(r#"{"email":"bad","password":"correct horse battery"}"#),
    )
    .await;
    assert_eq!(bad_email.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(bad_email).await["field"], "email");

    let short_password = request(
        state,
        management.clone(),
        Method::POST,
        "/api/admin/users",
        Some(&sid),
        true,
        Some(r#"{"email":"bob@example.org","password":"short"}"#),
    )
    .await;
    assert_eq!(short_password.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(short_password).await["field"], "password");
    assert!(management.calls().is_empty());
}

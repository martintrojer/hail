use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::routes::admin_users::{ManagedUser, StalwartUserManagement, UserManagementError};
use hail_api::routes::invites::{
    InviteProvisionError, InviteProvisionedUser, InviteProvisioner, invite_token_hash,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session_with_admin};
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

#[derive(Default)]
struct DummyUserManagement;

impl StalwartUserManagement for DummyUserManagement {
    fn list_users<'a>(
        &'a self,
        _state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ManagedUser>, UserManagementError>> + Send + 'a>>
    {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn create_user<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        _password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: format!("account-{email}"),
                display_name: display_name.map(str::to_owned),
            })
        })
    }

    fn delete_user<'a>(
        &'a self,
        _state: &'a AppState,
        _email: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn reset_password<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        _password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: format!("account-{email}"),
                display_name: None,
            })
        })
    }
}

#[derive(Default)]
struct FakeInviteProvisioner {
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeInviteProvisioner {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl InviteProvisioner for FakeInviteProvisioner {
    fn provision<'a>(
        &'a self,
        _state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<InviteProvisionedUser, InviteProvisionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert!(password.expose_secret().len() >= 12);
            self.calls
                .lock()
                .unwrap()
                .push(format!("provision:{email}"));
            Ok(InviteProvisionedUser {
                email: email.to_string(),
                jmap_account_id: format!("account-{email}"),
                display_name: display_name.map(str::to_owned),
                bearer_token: SecretString::from(format!("bearer-for-{email}")),
            })
        })
    }
}

fn app(state: AppState, provisioner: Arc<FakeInviteProvisioner>) -> Router {
    hail_api::routes::invites::router_with_provisioner(provisioner).with_state(state)
}

async fn public_request(
    state: AppState,
    provisioner: Arc<FakeInviteProvisioner>,
    method: Method,
    uri: &str,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if csrf {
        builder = builder.header(hail_api::middleware::auth::CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    app(state, provisioner)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn admin_create_invite_returns_link_and_stores_only_hash() {
    let (state, key) = fixture_state().await;
    let (_admin_id, sid) = seed_session_with_admin(&state, &key, "admin@example.org", true).await;
    let management = Arc::new(DummyUserManagement);

    let protected = hail_api::routes::admin_users::router_with_management(management).layer(
        axum::middleware::from_fn_with_state(
            state.clone(),
            hail_api::middleware::auth::require_auth,
        ),
    );
    let resp = Router::new()
        .merge(protected)
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/admin/invites")
                .header(header::COOKIE, format!("hail_session={sid}"))
                .header(hail_api::middleware::auth::CSRF_HEADER, "1")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"email":" Invited@Example.ORG ","display_name":" Invited User "}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = json_body(resp).await;
    assert_eq!(json["invite"]["email"], "invited@example.org");
    assert_eq!(json["invite"]["display_name"], "Invited User");
    let invite_url = json["invite"]["invite_url"].as_str().unwrap();
    assert!(invite_url.starts_with("http://localhost/invite/"));
    let token = invite_url.rsplit('/').next().unwrap();
    assert_eq!(token.len(), 64);

    let (token_hash, stored_email): (String, String) = sqlx::query_as(
        "SELECT token_hash, email FROM user_invites WHERE email = 'invited@example.org'",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(stored_email, "invited@example.org");
    assert_eq!(token_hash, invite_token_hash(token));
    assert_ne!(token_hash, token);
}

#[tokio::test]
async fn accept_invite_creates_user_sets_session_and_marks_used() {
    let (state, _key) = fixture_state().await;
    let token = "feedface";
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) VALUES ('admin@example.org', 'admin-account', 1, ?1)",
    )
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_invites (email, display_name, token_hash, created_by_user_id, expires_at, created_at) VALUES (?1, ?2, ?3, 1, ?4, ?5)",
    )
    .bind("new@example.org")
    .bind("New User")
    .bind(invite_token_hash(token))
    .bind(now + Duration::hours(1))
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    let provisioner = Arc::new(FakeInviteProvisioner::default());

    let resp = public_request(
        state.clone(),
        provisioner.clone(),
        Method::POST,
        "/api/invite/feedface/accept",
        true,
        Some(r#"{"password":"correct horse battery"}"#),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let set_cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.starts_with("hail_session="));
    let json = json_body(resp).await;
    assert_eq!(json["user"]["email"], "new@example.org");
    assert_eq!(json["user"]["is_admin"], false);
    assert_eq!(provisioner.calls(), vec!["provision:new@example.org"]);

    let accepted_user_id: i64 =
        sqlx::query_scalar("SELECT accepted_user_id FROM user_invites WHERE token_hash = ?1")
            .bind(invite_token_hash(token))
            .fetch_one(&state.db)
            .await
            .unwrap();
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = ?1")
        .bind(accepted_user_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(session_count, 1);
}

#[tokio::test]
async fn invite_accept_requires_csrf_and_is_single_use() {
    let (state, _key) = fixture_state().await;
    let token = "deadbeef";
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) VALUES ('admin@example.org', 'admin-account', 1, ?1)",
    )
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_invites (email, token_hash, created_by_user_id, expires_at, created_at) VALUES (?1, ?2, 1, ?3, ?4)",
    )
    .bind("once@example.org")
    .bind(invite_token_hash(token))
    .bind(now + Duration::hours(1))
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    let provisioner = Arc::new(FakeInviteProvisioner::default());

    let missing_csrf = public_request(
        state.clone(),
        provisioner.clone(),
        Method::POST,
        "/api/invite/deadbeef/accept",
        false,
        Some(r#"{"password":"correct horse battery"}"#),
    )
    .await;
    assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);

    let first = public_request(
        state.clone(),
        provisioner.clone(),
        Method::POST,
        "/api/invite/deadbeef/accept",
        true,
        Some(r#"{"password":"correct horse battery"}"#),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = public_request(
        state,
        provisioner,
        Method::POST,
        "/api/invite/deadbeef/accept",
        true,
        Some(r#"{"password":"correct horse battery"}"#),
    )
    .await;
    assert_eq!(second.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn expired_invite_is_not_previewed_or_accepted() {
    let (state, _key) = fixture_state().await;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) VALUES ('admin@example.org', 'admin-account', 1, ?1)",
    )
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_invites (email, token_hash, created_by_user_id, expires_at, created_at) VALUES (?1, ?2, 1, ?3, ?4)",
    )
    .bind("old@example.org")
    .bind(invite_token_hash("oldtoken"))
    .bind(now - Duration::seconds(1))
    .bind(now)
    .execute(&state.db)
    .await
    .unwrap();
    let provisioner = Arc::new(FakeInviteProvisioner::default());

    let preview = public_request(
        state.clone(),
        provisioner.clone(),
        Method::GET,
        "/api/invite/oldtoken",
        false,
        None,
    )
    .await;
    assert_eq!(preview.status(), StatusCode::NOT_FOUND);

    let accept = public_request(
        state,
        provisioner,
        Method::POST,
        "/api/invite/oldtoken/accept",
        true,
        Some(r#"{"password":"correct horse battery"}"#),
    )
    .await;
    assert_eq!(accept.status(), StatusCode::NOT_FOUND);
}

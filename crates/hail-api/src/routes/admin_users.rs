//! Admin user management endpoints.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::auth::UserView;
use crate::routes::invites;
use crate::routes::response::{ApiError, bad_request, error_response, internal, not_found};
use crate::routes::validation::valid_email;
use crate::state::AppState;

pub trait StalwartUserManagement: Send + Sync + 'static {
    fn list_users<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ManagedUser>, UserManagementError>> + Send + 'a>>;

    fn ensure_domain<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>> {
        let _ = (state, bearer, domain);
        Box::pin(async { Ok(()) })
    }

    fn create_user<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>>;

    fn delete_user<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        email: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>>;

    fn reset_password<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        email: &'a str,
        password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedUser {
    pub email: String,
    pub jmap_account_id: String,
    pub display_name: Option<String>,
}

pub struct HttpStalwartUserManagement;

impl StalwartUserManagement for HttpStalwartUserManagement {
    fn list_users<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ManagedUser>, UserManagementError>> + Send + 'a>>
    {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            Ok(session
                .list_individuals()
                .await?
                .into_iter()
                .map(managed_user_from_principal)
                .collect())
        })
    }

    fn create_user<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            let id = match session
                .create_individual(email, &password, display_name)
                .await?
            {
                Some(id) => id,
                None => session
                    .list_individuals()
                    .await?
                    .into_iter()
                    .find(|principal| principal.name.eq_ignore_ascii_case(email))
                    .map(|principal| principal.id)
                    .unwrap_or_else(|| email.to_string()),
            };
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: id,
                display_name: display_name.map(str::to_owned),
            })
        })
    }

    fn ensure_domain<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            session.create_domain(domain).await?;
            Ok(())
        })
    }

    fn delete_user<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        email: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            session.destroy_individual(email).await?;
            Ok(())
        })
    }

    fn reset_password<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        email: &'a str,
        password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            let id = session
                .reset_individual_secret(email, &password)
                .await?
                .unwrap_or_else(|| email.to_string());
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: id,
                display_name: None,
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserManagementError {
    #[error("stalwart.management_url is not configured")]
    NotConfigured,
    #[error("stalwart management API returned HTTP {status}: {detail}")]
    Api { status: StatusCode, detail: String },
    #[error("stalwart user management request failed: {0}")]
    Upstream(String),
}

pub fn router() -> Router<AppState> {
    router_with_management(Arc::new(HttpStalwartUserManagement))
}

pub fn router_with_management<M>(management: Arc<M>) -> Router<AppState>
where
    M: StalwartUserManagement,
{
    Router::new()
        .route("/api/admin/users", axum::routing::get(list_users::<M>))
        .route("/api/admin/users", axum::routing::post(create_user::<M>))
        .route("/api/admin/invites", axum::routing::post(create_invite))
        .route(
            "/api/admin/users/{id}",
            axum::routing::delete(delete_user::<M>),
        )
        .route(
            "/api/admin/users/{id}/reset-password",
            axum::routing::post(reset_password::<M>),
        )
        .layer(Extension(management))
}

#[derive(Debug, Serialize)]
struct UserListResponse {
    users: Vec<UserView>,
}

#[derive(Debug, Serialize)]
struct UserEnvelope {
    user: UserView,
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    email: String,
    password: SecretString,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResetPasswordRequest {
    password: SecretString,
}

#[derive(Debug, Deserialize)]
struct CreateInviteRequest {
    email: String,
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct InviteEnvelope {
    invite: invites::CreatedInviteResponse,
}

async fn list_users<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
) -> Response
where
    M: StalwartUserManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }

    let managed = match management.list_users(&state, user.jmap_token.clone()).await {
        Ok(users) => users,
        Err(err) => return management_error(err),
    };
    for managed_user in managed {
        if let Err(err) = upsert_local_user(&state, managed_user, false).await {
            tracing::error!(error = %err, "admin users: mirror list user failed");
            return internal();
        }
    }

    match load_users(&state).await {
        Ok(users) => Json(UserListResponse { users }).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "admin users: list failed");
            internal()
        }
    }
}

async fn create_user<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
    Json(body): Json<CreateUserRequest>,
) -> Response
where
    M: StalwartUserManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }

    let email = body.email.trim().to_lowercase();
    let display_name = normalize_display_name(body.display_name.as_deref());
    if !valid_email(&email) {
        return invalid_input("email");
    }
    if !valid_password(&body.password) {
        return invalid_input("password");
    }
    let Some(domain) = email_domain(&email) else {
        return invalid_input("email");
    };

    if let Err(err) = management
        .ensure_domain(&state, user.jmap_token.clone(), domain)
        .await
    {
        return management_error(err);
    }

    let managed = match management
        .create_user(
            &state,
            user.jmap_token.clone(),
            &email,
            body.password,
            display_name.as_deref(),
        )
        .await
    {
        Ok(user) => user,
        Err(err) => return management_error(err),
    };

    match upsert_local_user(&state, managed, false).await {
        Ok(created_user) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "admin.user.create",
                &serde_json::json!({ "target_user_id": created_user.id, "email": created_user.email }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            (
                StatusCode::CREATED,
                Json(UserEnvelope { user: created_user }),
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "admin users: local create failed");
            internal()
        }
    }
}

async fn create_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<CreateInviteRequest>,
) -> Response {
    if !user.is_admin {
        return forbidden_admin();
    }

    let email = body.email.trim().to_lowercase();
    let display_name = normalize_display_name(body.display_name.as_deref());
    if !valid_email(&email) {
        return invalid_input("email");
    }

    match invites::insert_invite(&state, user.id, &email, display_name.as_deref()).await {
        Ok(invite) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "admin.user.invite",
                &serde_json::json!({ "email": email }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            (StatusCode::CREATED, Json(InviteEnvelope { invite })).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "admin users: invite create failed");
            internal()
        }
    }
}

async fn delete_user<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
    Path(id): Path<i64>,
) -> Response
where
    M: StalwartUserManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }
    if id == user.id {
        return bad_request("cannot_delete_self");
    }

    let Some(email) = local_user_email(&state, id).await else {
        return not_found("not_found");
    };

    if let Err(err) = management
        .delete_user(&state, user.jmap_token.clone(), &email)
        .await
    {
        return management_error(err);
    }

    match sqlx::query("DELETE FROM users WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(_) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "admin.user.delete",
                &serde_json::json!({ "target_user_id": id, "email": email }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(err) => {
            tracing::error!(id, error = %err, "admin users: local delete failed");
            internal()
        }
    }
}

async fn reset_password<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
    Path(id): Path<i64>,
    Json(body): Json<ResetPasswordRequest>,
) -> Response
where
    M: StalwartUserManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }
    if !valid_password(&body.password) {
        return invalid_input("password");
    }

    let Some(email) = local_user_email(&state, id).await else {
        return not_found("not_found");
    };

    let managed = match management
        .reset_password(&state, user.jmap_token.clone(), &email, body.password)
        .await
    {
        Ok(user) => user,
        Err(err) => return management_error(err),
    };

    let user_view = match upsert_local_user(&state, managed, false).await {
        Ok(user) => user,
        Err(err) => {
            tracing::error!(id, error = %err, "admin users: local reset mirror failed");
            return internal();
        }
    };

    if let Err(err) = sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
        .bind(id)
        .execute(&state.db)
        .await
    {
        tracing::warn!(id, error = %err, "admin users: reset session invalidation failed");
    }

    if let Err(err) = audit::record(
        &state.db,
        user.id,
        "admin.user.reset_password",
        &serde_json::json!({ "target_user_id": id, "email": email }),
    )
    .await
    {
        tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
    }

    Json(UserEnvelope { user: user_view }).into_response()
}

async fn load_users(state: &AppState) -> Result<Vec<UserView>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, String, Option<String>, i64)>(
        "SELECT id, email, display_name, is_admin FROM users ORDER BY email",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, email, display_name, is_admin)| UserView {
            id,
            email,
            display_name,
            is_admin: is_admin != 0,
        })
        .collect())
}

async fn local_user_email(state: &AppState, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT email FROM users WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
}

async fn upsert_local_user(
    state: &AppState,
    managed: ManagedUser,
    is_admin: bool,
) -> Result<UserView, sqlx::Error> {
    let email = managed.email.trim().to_lowercase();
    let display_name = normalize_display_name(managed.display_name.as_deref());
    let now = Utc::now();
    let (id, email, display_name, is_admin_int): (i64, String, Option<String>, i64) =
        sqlx::query_as(
            "INSERT INTO users (email, jmap_account_id, display_name, is_admin, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(email) DO UPDATE SET \
                jmap_account_id = excluded.jmap_account_id, \
                display_name = COALESCE(excluded.display_name, users.display_name) \
             RETURNING id, email, display_name, is_admin",
        )
        .bind(&email)
        .bind(&managed.jmap_account_id)
        .bind(display_name.as_deref())
        .bind(if is_admin { 1_i64 } else { 0_i64 })
        .bind(now)
        .fetch_one(&state.db)
        .await?;
    Ok(UserView {
        id,
        email,
        display_name,
        is_admin: is_admin_int != 0,
    })
}

async fn management_session(
    state: &AppState,
    bearer: SecretString,
) -> Result<hail_jmap::management::ManagementSession, UserManagementError> {
    let base = management_base(state)?;
    Ok(hail_jmap::management::ManagementSession::connect(&base, bearer).await?)
}

fn management_base(state: &AppState) -> Result<String, UserManagementError> {
    state
        .config
        .stalwart
        .management_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| url.trim_end_matches('/').to_string())
        .ok_or(UserManagementError::NotConfigured)
}

fn managed_user_from_principal(
    principal: hail_jmap::management::ManagementPrincipal,
) -> ManagedUser {
    let email = principal
        .emails
        .iter()
        .find(|email| email.contains('@'))
        .cloned()
        .unwrap_or_else(|| principal.name.clone());
    ManagedUser {
        email,
        jmap_account_id: principal.id,
        display_name: principal.description,
    }
}

impl From<hail_jmap::management::ManagementError> for UserManagementError {
    fn from(err: hail_jmap::management::ManagementError) -> Self {
        match err {
            hail_jmap::management::ManagementError::Api { status, detail } => Self::Api {
                status: StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                detail,
            },
            other => Self::Upstream(other.to_string()),
        }
    }
}

fn email_domain(email: &str) -> Option<&str> {
    email.rsplit_once('@').map(|(_, domain)| domain)
}

fn normalize_display_name(display_name: Option<&str>) -> Option<String> {
    display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn valid_password(password: &SecretString) -> bool {
    password.expose_secret().len() >= 12
}

fn invalid_input(field: &'static str) -> Response {
    crate::routes::response::ApiError::new("invalid_input")
        .with_detail(field)
        .into_response(StatusCode::BAD_REQUEST)
}

fn forbidden_admin() -> Response {
    error_response(StatusCode::FORBIDDEN, "admin_required")
}

fn management_error(err: UserManagementError) -> Response {
    match err {
        UserManagementError::NotConfigured => error_response(
            StatusCode::NOT_IMPLEMENTED,
            "stalwart_management_unconfigured",
        ),
        UserManagementError::Api { status, detail } if status.is_client_error() => {
            ApiError::new("stalwart_management_failed")
                .with_detail(detail)
                .into_response(status)
        }
        UserManagementError::Api { status, detail } => {
            tracing::warn!(%status, error = %detail, "stalwart user management failed");
            error_response(StatusCode::BAD_GATEWAY, "stalwart_management_failed")
        }
        UserManagementError::Upstream(message) => {
            tracing::warn!(error = %message, "stalwart user management failed");
            error_response(StatusCode::BAD_GATEWAY, "stalwart_management_failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_domain_from_valid_email() {
        assert_eq!(email_domain("user+tag@example.org"), Some("example.org"));
    }

    #[test]
    fn maps_principal_email_and_id_to_managed_user() {
        let managed = managed_user_from_principal(hail_jmap::management::ManagementPrincipal {
            id: "principal-id".to_string(),
            name: "alice".to_string(),
            description: Some("Alice".to_string()),
            emails: vec!["alice@example.org".to_string()],
        });
        assert_eq!(managed.email, "alice@example.org");
        assert_eq!(managed.jmap_account_id, "principal-id");
        assert_eq!(managed.display_name.as_deref(), Some("Alice"));
    }
}

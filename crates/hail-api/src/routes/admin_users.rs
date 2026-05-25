//! Admin user management endpoints.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::auth::UserView;
use crate::routes::management_http;
use crate::routes::response::{bad_request, internal, not_found};
use crate::routes::validation::valid_email;
use crate::state::AppState;

pub trait StalwartUserManagement: Send + Sync + 'static {
    fn list_users<'a>(
        &'a self,
        state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ManagedUser>, UserManagementError>> + Send + 'a>>;

    fn create_user<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>>;

    fn delete_user<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>>;

    fn reset_password<'a>(
        &'a self,
        state: &'a AppState,
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ManagedUser>, UserManagementError>> + Send + 'a>>
    {
        Box::pin(async move {
            let base = management_base(state)?;
            let response = management_http::client()
                .get(format!("{}/api/principal", base))
                .send()
                .await
                .map_err(|err| UserManagementError::Upstream(err.to_string()))?;
            if !response.status().is_success() {
                return Err(UserManagementError::Upstream(format!(
                    "GET /api/principal returned HTTP {}",
                    response.status()
                )));
            }
            decode_user_list(response).await
        })
    }

    fn create_user<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let base = management_base(state)?;
            create_or_update_principal(&base, email, &password, display_name).await?;
            let session = hail_jmap::login_basic(&state.config.stalwart.jmap_url, email, password)
                .await
                .map_err(|err| UserManagementError::Upstream(err.to_string()))?;
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: session.account_id().to_string(),
                display_name: display_name.map(str::to_owned),
            })
        })
    }

    fn delete_user<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let base = management_base(state)?;
            let response = management_http::client()
                .delete(management_path(&base, &["api", "principal", email]))
                .send()
                .await
                .map_err(|err| UserManagementError::Upstream(err.to_string()))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(UserManagementError::Upstream(format!(
                    "DELETE /api/principal/{email} returned HTTP {}",
                    response.status()
                )))
            }
        })
    }

    fn reset_password<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ManagedUser, UserManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let base = management_base(state)?;
            create_or_update_principal(&base, email, &password, None).await?;
            let session = hail_jmap::login_basic(&state.config.stalwart.jmap_url, email, password)
                .await
                .map_err(|err| UserManagementError::Upstream(err.to_string()))?;
            Ok(ManagedUser {
                email: email.to_string(),
                jmap_account_id: session.account_id().to_string(),
                display_name: None,
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserManagementError {
    #[error("stalwart.management_url is not configured")]
    NotConfigured,
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

    let managed = match management.list_users(&state).await {
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

    let managed = match management
        .create_user(&state, &email, body.password, display_name.as_deref())
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
        return not_found();
    };

    if let Err(err) = management.delete_user(&state, &email).await {
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
        return not_found();
    };

    let managed = match management
        .reset_password(&state, &email, body.password)
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

fn management_path(base: &str, segments: &[&str]) -> String {
    let mut url = base.trim_end_matches('/').to_string();
    for segment in segments {
        url.push('/');
        url.push_str(&url::form_urlencoded::byte_serialize(segment.as_bytes()).collect::<String>());
    }
    url
}

async fn create_or_update_principal(
    base: &str,
    email: &str,
    password: &SecretString,
    display_name: Option<&str>,
) -> Result<(), UserManagementError> {
    #[derive(Serialize)]
    struct Principal<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        name: &'a str,
        secrets: [&'a str; 1],
        emails: [&'a str; 1],
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<&'a str>,
    }

    let payload = Principal {
        kind: "individual",
        name: email,
        secrets: [password.expose_secret()],
        emails: [email],
        description: display_name,
    };
    let response = management_http::client()
        .post(format!("{}/api/principal", base))
        .json(&payload)
        .send()
        .await
        .map_err(|err| UserManagementError::Upstream(err.to_string()))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(UserManagementError::Upstream(format!(
            "POST /api/principal returned HTTP {}",
            response.status()
        )))
    }
}

async fn decode_user_list(
    response: reqwest::Response,
) -> Result<Vec<ManagedUser>, UserManagementError> {
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| UserManagementError::Upstream(err.to_string()))?;

    let array = value
        .as_array()
        .or_else(|| value.get("users").and_then(serde_json::Value::as_array))
        .or_else(|| value.get("data").and_then(serde_json::Value::as_array))
        .ok_or_else(|| {
            UserManagementError::Upstream(
                "user list response was not an array or object with users/data".to_string(),
            )
        })?;

    array.iter().map(managed_user_from_value).collect()
}

fn managed_user_from_value(value: &serde_json::Value) -> Result<ManagedUser, UserManagementError> {
    if let Some(email) = value.as_str() {
        return Ok(ManagedUser {
            email: email.to_string(),
            jmap_account_id: email.to_string(),
            display_name: None,
        });
    }

    let email = value
        .get("email")
        .or_else(|| value.get("name"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| UserManagementError::Upstream("user list entry missing email".to_string()))?
        .to_string();
    let jmap_account_id = value
        .get("jmap_account_id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&email)
        .to_string();
    let display_name = value
        .get("display_name")
        .or_else(|| value.get("description"))
        .and_then(serde_json::Value::as_str)
        .and_then(|s| normalize_display_name(Some(s)));
    Ok(ManagedUser {
        email,
        jmap_account_id,
        display_name,
    })
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
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"invalid_input","field":"{field}"}}"#),
    )
        .into_response()
}

fn forbidden_admin() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"admin_required"}"#,
    )
        .into_response()
}

fn management_error(err: UserManagementError) -> Response {
    match err {
        UserManagementError::NotConfigured => (
            StatusCode::NOT_IMPLEMENTED,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"stalwart_management_unconfigured"}"#,
        )
            .into_response(),
        UserManagementError::Upstream(message) => {
            tracing::warn!(error = %message, "stalwart user management failed");
            (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                r#"{"error":"stalwart_management_failed"}"#,
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_path_percent_encodes_email_path_segment() {
        let url = management_path(
            "http://stalwart.local/",
            &["api", "principal", "User+tag@example.org"],
        );
        assert_eq!(
            url,
            "http://stalwart.local/api/principal/User%2Btag%40example.org"
        );
    }
}

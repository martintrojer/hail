//! Public invite acceptance endpoints.

use std::{future::Future, pin::Pin, sync::Arc, sync::OnceLock};

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, Utc};
use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use utoipa_axum::{
    router::{OpenApiRouter, UtoipaMethodRouterExt},
    routes,
};

use crate::{
    middleware::session::{SESSION_TTL_DAYS, basic_bearer, build_session_cookie, new_session_id},
    routes::{
        auth::UserView,
        response::{ApiError, error_response, internal},
    },
    state::AppState,
};

pub const TAG: &str = "invites";
const INVITE_TTL_DAYS: i64 = 7;
static INVITE_ACCEPT_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub trait InviteProvisioner: Send + Sync + 'static {
    fn provision<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<InviteProvisionedUser, InviteProvisionError>> + Send + 'a>,
    >;
}

pub struct InviteProvisionedUser {
    pub email: String,
    pub jmap_account_id: String,
    pub display_name: Option<String>,
    pub bearer_token: SecretString,
}

#[derive(Debug, thiserror::Error)]
pub enum InviteProvisionError {
    #[error("stalwart user management request failed: {0}")]
    Management(String),
}

pub struct StalwartInviteProvisioner;

async fn latest_inviter_management_bearer(
    state: &AppState,
    email: &str,
) -> Result<SecretString, InviteProvisionError> {
    let row = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT s.jmap_token_enc \
         FROM user_invites i \
         JOIN sessions s ON s.user_id = i.created_by_user_id \
         WHERE i.email = ?1 AND s.expires_at > ?2 \
         ORDER BY i.created_at DESC, s.last_used_at DESC \
         LIMIT 1",
    )
    .bind(email)
    .bind(Utc::now())
    .fetch_optional(&state.db)
    .await
    .map_err(|err| InviteProvisionError::Management(err.to_string()))?
    .ok_or_else(|| {
        InviteProvisionError::Management("inviter admin session is no longer active".to_string())
    })?;
    let token_bytes = hail_core::open(&row, &state.server_key)
        .map_err(|err| InviteProvisionError::Management(err.to_string()))?;
    String::from_utf8(token_bytes)
        .map(SecretString::from)
        .map_err(|err| InviteProvisionError::Management(err.to_string()))
}

impl InviteProvisioner for StalwartInviteProvisioner {
    fn provision<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<InviteProvisionedUser, InviteProvisionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let management = crate::routes::admin_users::HttpStalwartUserManagement;
            let Some(domain) = email.rsplit_once('@').map(|(_, domain)| domain) else {
                return Err(InviteProvisionError::Management(
                    "invite email missing domain".to_string(),
                ));
            };
            let management_bearer = latest_inviter_management_bearer(state, email).await?;
            crate::routes::admin_users::StalwartUserManagement::ensure_domain(
                &management,
                state,
                management_bearer.clone(),
                domain,
            )
            .await
            .map_err(|err| InviteProvisionError::Management(err.to_string()))?;
            let managed = crate::routes::admin_users::StalwartUserManagement::create_user(
                &management,
                state,
                management_bearer,
                email,
                password.clone(),
                display_name,
            )
            .await
            .map_err(|err| InviteProvisionError::Management(err.to_string()))?;
            Ok(InviteProvisionedUser {
                email: managed.email,
                jmap_account_id: managed.jmap_account_id,
                display_name: managed.display_name,
                bearer_token: SecretString::from(basic_bearer(email, &password)),
            })
        })
    }
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_provisioner(Arc::new(StalwartInviteProvisioner))
}

pub fn router_with_provisioner<P>(provisioner: Arc<P>) -> Router<AppState>
where
    P: InviteProvisioner,
{
    let provisioner: Arc<dyn InviteProvisioner> = provisioner;
    Router::new()
        .route("/api/invite/{token}", axum::routing::get(get_invite))
        .route(
            "/api/invite/{token}/accept",
            axum::routing::post(accept_invite),
        )
        .layer(Extension(provisioner))
}

pub fn openapi_router_with_provisioner<P>(provisioner: Arc<P>) -> OpenApiRouter<AppState>
where
    P: InviteProvisioner,
{
    let provisioner: Arc<dyn InviteProvisioner> = provisioner;
    OpenApiRouter::new()
        .routes(routes!(get_invite))
        .routes(routes!(accept_invite).layer(Extension(provisioner)))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvitePreviewResponse {
    pub email: String,
    pub display_name: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AcceptInviteRequest {
    #[schema(value_type = String)]
    pub password: SecretString,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteAcceptResponse {
    pub user: UserView,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreatedInviteResponse {
    pub email: String,
    pub display_name: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: chrono::DateTime<Utc>,
    pub invite_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum InviteCreateError {
    #[error("failed to draw invite token from OS RNG")]
    Token,
    #[error("invite insert failed: {0}")]
    Db(sqlx::Error),
}

pub fn new_invite_token() -> Result<String, InviteCreateError> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| InviteCreateError::Token)?;
    Ok(hex::encode(bytes))
}

#[must_use]
pub fn invite_token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn insert_invite(
    state: &AppState,
    created_by_user_id: i64,
    email: &str,
    display_name: Option<&str>,
) -> Result<CreatedInviteResponse, InviteCreateError> {
    let token = new_invite_token()?;
    let now = Utc::now();
    let expires_at = now + Duration::days(INVITE_TTL_DAYS);
    sqlx::query(
        "INSERT INTO user_invites (email, display_name, token_hash, created_by_user_id, expires_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(email)
    .bind(display_name)
    .bind(invite_token_hash(&token))
    .bind(created_by_user_id)
    .bind(expires_at)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(InviteCreateError::Db)?;

    Ok(CreatedInviteResponse {
        email: email.to_string(),
        display_name: display_name.map(str::to_owned),
        expires_at,
        invite_url: format!(
            "{}/invite/{token}",
            state.config.server.public_url.trim_end_matches('/')
        ),
    })
}

#[utoipa::path(
    get,
    path = "/api/invite/{token}",
    tag = TAG,
    params(("token" = String, Path, description = "Opaque invite token.")),
    responses(
        (status = 200, description = "Invite can be accepted.", body = InvitePreviewResponse),
        (status = 404, description = "Invite is missing, expired, or already used.")
    )
)]
async fn get_invite(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let now = Utc::now();
    match sqlx::query_as::<_, (String, Option<String>, chrono::DateTime<Utc>)>(
        "SELECT email, display_name, expires_at FROM user_invites WHERE token_hash = ?1 AND accepted_at IS NULL AND expires_at > ?2",
    )
    .bind(invite_token_hash(&token))
    .bind(now)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some((email, display_name, expires_at))) => Json(InvitePreviewResponse { email, display_name, expires_at }).into_response(),
        Ok(None) => not_found_invite(),
        Err(err) => {
            tracing::error!(error = %err, "invite: preview lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/invite/{token}/accept",
    tag = TAG,
    request_body = AcceptInviteRequest,
    params(("token" = String, Path, description = "Opaque invite token.")),
    responses(
        (status = 201, description = "Invite accepted; session cookie has been set.", body = InviteAcceptResponse),
        (status = 400, description = "Password failed validation."),
        (status = 403, description = "Missing CSRF header."),
        (status = 404, description = "Invite is missing, expired, or already used."),
        (status = 502, description = "Upstream user provisioning failed.")
    )
)]
async fn accept_invite(
    State(state): State<AppState>,
    Extension(provisioner): Extension<Arc<dyn InviteProvisioner>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    Json(body): Json<AcceptInviteRequest>,
) -> Response {
    if headers
        .get(crate::middleware::auth::CSRF_HEADER)
        .map(|v| v.as_bytes())
        != Some(b"1")
    {
        return error_response(StatusCode::FORBIDDEN, "csrf_required");
    }
    if body.password.expose_secret().len() < 12 {
        return invalid_input("password");
    }

    let _accept_guard = INVITE_ACCEPT_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;

    let now = Utc::now();
    let row = match sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT id, email, display_name FROM user_invites WHERE token_hash = ?1 AND accepted_at IS NULL AND expires_at > ?2",
    )
    .bind(invite_token_hash(&token))
    .bind(now)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(error = %err, "invite: accept lookup failed");
            return internal();
        }
    };
    let Some((invite_id, email, invite_display_name)) = row else {
        return not_found_invite();
    };

    let provisioned = match provisioner
        .provision(
            &state,
            &email,
            body.password,
            invite_display_name.as_deref(),
        )
        .await
    {
        Ok(user) => user,
        Err(err) => {
            tracing::warn!(email, error = %err, "invite: provisioning failed");
            return error_response(StatusCode::BAD_GATEWAY, "invite_provision_failed");
        }
    };

    let now = Utc::now();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "invite: tx begin failed");
            return internal();
        }
    };
    let claim = match sqlx::query("UPDATE user_invites SET accepted_at = ?1 WHERE id = ?2 AND accepted_at IS NULL AND expires_at > ?1")
        .bind(now)
        .bind(invite_id)
        .execute(&mut *tx)
        .await
    {
        Ok(done) => done,
        Err(err) => {
            tracing::error!(invite_id, error = %err, "invite: claim failed");
            return internal();
        }
    };
    if claim.rows_affected() != 1 {
        return not_found_invite();
    }

    let display_name = provisioned
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let (user_id, db_email, db_display_name, is_admin_int): (i64, String, Option<String>, i64) = match sqlx::query_as(
        "INSERT INTO users (email, jmap_account_id, display_name, is_admin, created_at) VALUES (?1, ?2, ?3, 0, ?4) ON CONFLICT(email) DO UPDATE SET jmap_account_id = excluded.jmap_account_id, display_name = COALESCE(excluded.display_name, users.display_name) RETURNING id, email, display_name, is_admin",
    )
    .bind(provisioned.email.trim().to_lowercase())
    .bind(&provisioned.jmap_account_id)
    .bind(display_name.as_deref())
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(invite_id, error = %err, "invite: user upsert failed");
            return internal();
        }
    };

    let token_enc = match hail_core::seal(
        provisioned.bearer_token.expose_secret().as_bytes(),
        &state.server_key,
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(error = %err, "invite: token seal failed");
            return internal();
        }
    };
    let session_id = match new_session_id() {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(error = %err, "invite: failed to draw session id from OS RNG");
            return internal();
        }
    };
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let expires_at = now + Duration::days(SESSION_TTL_DAYS);
    if let Err(err) = sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&token_enc)
    .bind(user_agent.as_deref())
    .bind(expires_at)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(user_id, error = %err, "invite: session insert failed");
        return internal();
    }
    if let Err(err) = sqlx::query("UPDATE user_invites SET accepted_user_id = ?1 WHERE id = ?2")
        .bind(user_id)
        .bind(invite_id)
        .execute(&mut *tx)
        .await
    {
        tracing::error!(user_id, invite_id, error = %err, "invite: accepted user update failed");
        return internal();
    }
    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "invite: tx commit failed");
        return internal();
    }

    (
        StatusCode::CREATED,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::SET_COOKIE, build_session_cookie(&session_id)),
        ],
        serde_json::to_string(&InviteAcceptResponse {
            user: UserView {
                id: user_id,
                email: db_email,
                display_name: db_display_name,
                is_admin: is_admin_int != 0,
            },
        })
        .unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

fn invalid_input(field: &'static str) -> Response {
    ApiError::new("invalid_input")
        .with_detail(field)
        .into_response(StatusCode::BAD_REQUEST)
}

fn not_found_invite() -> Response {
    error_response(StatusCode::NOT_FOUND, "invite_not_found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_tokens_are_256_bit_lowercase_hex() {
        let token = new_invite_token().expect("token");
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(token, token.to_ascii_lowercase());
    }

    #[test]
    fn hashes_do_not_equal_raw_token() {
        let token = "abc123";
        let hash = invite_token_hash(token);
        assert_ne!(hash, token);
        assert_eq!(hash.len(), 64);
    }
}

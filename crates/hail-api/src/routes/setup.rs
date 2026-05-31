//! First-run setup wizard endpoints.
//!
//! Public by design: `/setup` is reachable before any user/session exists.
//! Every mutating path re-checks the wizard gate inside the DB transaction so
//! two concurrent first-run submissions cannot both create admins.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use axum::extract::{ConnectInfo, Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::session::{
    SESSION_TTL_DAYS, basic_bearer, build_session_cookie, new_session_id,
};
use crate::routes::auth::UserView;
use crate::routes::response::{error_response, internal};
use crate::routes::validation::{valid_domain, valid_email};
use crate::state::AppState;

static SETUP_PROVISION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Serialize, ToSchema)]
struct SetupStateResponse {
    wizard_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<SetupDisabledReason>,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SetupDisabledReason {
    ConfigAdminSet,
    AdminUserExists,
}

#[derive(Debug, Deserialize, ToSchema)]
struct SetupAdminRequest {
    email: String,
    #[schema(value_type = String)]
    password: SecretString,
    display_name: Option<String>,
    domain: String,
    #[schema(value_type = Option<String>)]
    bootstrap_token: Option<SecretString>,
    stalwart_admin_username: String,
    #[schema(value_type = String)]
    stalwart_admin_password: SecretString,
}

#[derive(Debug, Serialize, ToSchema)]
struct UserEnvelope {
    user: UserView,
}

/// Provisioning seam for `/api/setup/admin`.
///
/// Production uses [`StalwartProvisioner`]. Tests inject a fake via
/// [`router_with_provisioner`] so CI never needs a live Stalwart.
pub trait UserProvisioner: Send + Sync + 'static {
    fn provision<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
        domain: &'a str,
        stalwart_admin_username: &'a str,
        stalwart_admin_password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ProvisionedUser, ProvisionError>> + Send + 'a>>;
}

pub struct ProvisionedUser {
    pub jmap_account_id: String,
    pub bearer_token: SecretString,
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error(
        "stalwart management API returned 404 for {path}; Stalwart versions differ here, check management_url/path"
    )]
    ManagementPathNotFound { path: String },
    #[error("stalwart management request failed: {0}")]
    Management(String),
    #[error("stalwart management API returned HTTP {status}: {detail}")]
    ManagementApi { status: StatusCode, detail: String },
    #[error("failed to draw management auth nonce from OS RNG")]
    Nonce,
    #[error("jmap login failed: {0}")]
    Jmap(String),
}

/// Production first-run provisioner.
///
/// If `config.stalwart.management_url` is set, we create/update the Stalwart
/// principal through the management API and then login through JMAP to discover
/// the JMAP account id. If it is absent, we deliberately fall back to direct
/// `hail_jmap::login_basic`: this supports operators who pre-created the
/// account/domain with Stalwart CLI/config but still want the hail first-run
/// wizard to create the hail-side admin row and session.
pub struct StalwartProvisioner;

impl UserProvisioner for StalwartProvisioner {
    fn provision<'a>(
        &'a self,
        state: &'a AppState,
        email: &'a str,
        password: SecretString,
        display_name: Option<&'a str>,
        domain: &'a str,
        stalwart_admin_username: &'a str,
        stalwart_admin_password: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<ProvisionedUser, ProvisionError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(management_url) = state.config.stalwart.management_url.as_deref() {
                let token = hail_jmap::management::login_authcode_to_bearer(
                    management_url,
                    stalwart_admin_username,
                    stalwart_admin_password,
                )
                .await
                .map_err(management_error)?;
                hail_jmap::management::principal_set_domain(management_url, &token, domain)
                    .await
                    .map_err(management_error)?;
                hail_jmap::management::principal_set_individual(
                    management_url,
                    &token,
                    email,
                    &password,
                    display_name,
                )
                .await
                .map_err(management_error)?;
            } else {
                tracing::info!(
                    email,
                    "setup: stalwart.management_url unset; falling back to JMAP login for pre-created account"
                );
            }

            let session =
                hail_jmap::login_basic(&state.config.stalwart.jmap_url, email, password.clone())
                    .await
                    .map_err(|err| ProvisionError::Jmap(err.to_string()))?;
            let bearer = basic_bearer(email, &password);
            Ok(ProvisionedUser {
                jmap_account_id: session.account_id().to_string(),
                bearer_token: SecretString::from(bearer),
            })
        })
    }
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_provisioner(Arc::new(StalwartProvisioner))
}

pub fn router_with_provisioner<P>(provisioner: Arc<P>) -> Router<AppState>
where
    P: UserProvisioner,
{
    Router::from(openapi_router_with_provisioner(provisioner))
}

pub fn openapi_router_with_provisioner<P>(provisioner: Arc<P>) -> OpenApiRouter<AppState>
where
    P: UserProvisioner,
{
    let provisioner: Arc<dyn UserProvisioner> = provisioner;
    OpenApiRouter::new()
        .routes(routes!(setup_state))
        .routes(routes!(setup_admin).layer(Extension(provisioner)))
}

#[utoipa::path(
    get,
    path = "/api/setup/state",
    tag = "setup",
    responses(
        (status = 200, description = "First-run setup wizard state.", body = SetupStateResponse),
        (status = 500, description = "Failed to read setup state.", body = crate::routes::response::ApiError)
    )
)]
async fn setup_state(State(state): State<AppState>) -> Response {
    match setup_status(&state).await {
        Ok(status) => Json(status.into_response()).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "setup: state check failed");
            internal()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/setup/admin",
    tag = "setup",
    request_body = SetupAdminRequest,
    responses(
        (status = 201, description = "Admin user created; session cookie has been set.", body = UserEnvelope),
        (status = 400, description = "Setup input or Stalwart provisioning failed.", body = crate::routes::response::ApiError),
        (status = 403, description = "Setup bootstrap token is missing or invalid.", body = crate::routes::response::ApiError),
        (status = 409, description = "Setup wizard is no longer active.", body = crate::routes::response::ApiError),
        (status = 500, description = "Internal setup failure.", body = crate::routes::response::ApiError)
    )
)]
async fn setup_admin(
    State(state): State<AppState>,
    Extension(provisioner): Extension<Arc<dyn UserProvisioner>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetupAdminRequest>,
) -> Response {
    let ip =
        crate::middleware::rate_limit::client_ip(&headers, Some(addr.ip())).unwrap_or(addr.ip());
    if !state.auth_rate_limiter.check(ip) {
        tracing::warn!(%ip, "setup: rate-limited");
        return crate::middleware::rate_limit::too_many_requests();
    }

    let email = body.email.trim().to_lowercase();
    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let domain = normalize_domain(&body.domain);
    let stalwart_admin_username = body.stalwart_admin_username.trim().to_owned();

    if !valid_email(&email) {
        return invalid_input("email");
    }
    if body.password.expose_secret().len() < 12 {
        return invalid_input("password");
    }
    if !valid_domain(&domain) {
        return invalid_input("domain");
    }
    if stalwart_admin_username.is_empty() {
        return invalid_input("stalwart_admin_username");
    }
    if body
        .stalwart_admin_password
        .expose_secret()
        .trim()
        .is_empty()
    {
        return invalid_input("stalwart_admin_password");
    }
    if !email.ends_with(&format!("@{domain}")) {
        return invalid_input("email");
    }
    if !setup_bootstrap_authorized(&state, body.bootstrap_token.as_ref()) {
        return forbidden_setup_bootstrap_required();
    }

    let now = Utc::now();
    // External setup provisioning is not transactional with the hail sidecar DB.
    // Serialize the first-run gate through provisioning so a stale/concurrent
    // second POST waits, observes the newly-created admin, and returns 409
    // before touching Stalwart.
    let _provision_guard = SETUP_PROVISION_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let mut gate_tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "setup: gate tx begin failed");
            return internal();
        }
    };
    let status = match setup_status_in_tx(&state, &mut gate_tx).await {
        Ok(status) => status,
        Err(err) => {
            tracing::error!(error = %err, "setup: active precheck failed");
            return internal();
        }
    };
    if !status.wizard_active {
        return conflict_setup_disabled();
    }
    if let Err(err) = gate_tx.commit().await {
        tracing::error!(error = %err, "setup: gate tx commit failed");
        return internal();
    }

    let provisioned = match provisioner
        .provision(
            &state,
            &email,
            body.password,
            display_name.as_deref(),
            &domain,
            &stalwart_admin_username,
            body.stalwart_admin_password,
        )
        .await
    {
        Ok(user) => user,
        Err(err) => {
            tracing::error!(email, error = %err, "setup: provision failed");
            return provision_error_response(err);
        }
    };

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "setup: tx begin failed");
            return internal();
        }
    };

    let status = match setup_status_in_tx(&state, &mut tx).await {
        Ok(status) => status,
        Err(err) => {
            tracing::error!(error = %err, "setup: active recheck failed");
            return internal();
        }
    };
    if !status.wizard_active {
        return conflict_setup_disabled();
    }

    let (user_id, db_email, db_display_name, is_admin_int): (i64, String, Option<String>, i64) =
        match sqlx::query_as(
            "INSERT INTO users (email, jmap_account_id, display_name, is_admin, created_at) \
             VALUES (?1, ?2, ?3, 1, ?4) \
             RETURNING id, email, display_name, is_admin",
        )
        .bind(&email)
        .bind(&provisioned.jmap_account_id)
        .bind(display_name.as_deref())
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(row) => row,
            Err(err) => {
                tracing::error!(email, error = %err, "setup: admin insert failed");
                return internal();
            }
        };

    let token_enc = match hail_core::seal(
        provisioned.bearer_token.expose_secret().as_bytes(),
        &state.server_key,
    ) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(error = %err, "setup: token seal failed");
            return internal();
        }
    };

    let session_id = match new_session_id() {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(error = %err, "setup: failed to draw session id from OS RNG");
            return internal();
        }
    };
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let expires_at = now + Duration::days(SESSION_TTL_DAYS);

    if let Err(err) = sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
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
        tracing::error!(user_id, error = %err, "setup: session insert failed");
        return internal();
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "setup: tx commit failed");
        return internal();
    }

    let view = UserView {
        id: user_id,
        email: db_email,
        display_name: db_display_name,
        is_admin: is_admin_int != 0,
    };
    let cookie = build_session_cookie(&session_id);
    (
        StatusCode::CREATED,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::SET_COOKIE, cookie),
        ],
        serde_json::to_string(&UserEnvelope { user: view }).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

#[derive(Debug)]
struct SetupStatus {
    wizard_active: bool,
    reason: Option<SetupDisabledReason>,
}

impl SetupStatus {
    fn into_response(self) -> SetupStateResponse {
        SetupStateResponse {
            wizard_active: self.wizard_active,
            reason: self.reason,
        }
    }
}

async fn setup_status(state: &AppState) -> Result<SetupStatus, sqlx::Error> {
    let mut conn = state.db.acquire().await?;
    setup_status_with_executor(state, &mut *conn).await
}

async fn setup_status_in_tx(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<SetupStatus, sqlx::Error> {
    setup_status_with_executor(state, &mut **tx).await
}

async fn setup_status_with_executor<'e, E>(
    state: &AppState,
    executor: E,
) -> Result<SetupStatus, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    if state.config.admin.is_some() {
        return Ok(SetupStatus {
            wizard_active: false,
            reason: Some(SetupDisabledReason::ConfigAdminSet),
        });
    }

    let any_admin: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE is_admin = 1)")
            .fetch_one(executor)
            .await?;
    if any_admin != 0 {
        Ok(SetupStatus {
            wizard_active: false,
            reason: Some(SetupDisabledReason::AdminUserExists),
        })
    } else {
        Ok(SetupStatus {
            wizard_active: true,
            reason: None,
        })
    }
}

fn management_error(err: hail_jmap::management::ManagementError) -> ProvisionError {
    match err {
        hail_jmap::management::ManagementError::Nonce => ProvisionError::Nonce,
        hail_jmap::management::ManagementError::Api { status, detail } => {
            ProvisionError::ManagementApi { status, detail }
        }
        other => ProvisionError::Management(other.to_string()),
    }
}

fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn setup_bootstrap_authorized(state: &AppState, provided: Option<&SecretString>) -> bool {
    if !state.config.setup.bootstrap_enabled {
        return false;
    }
    let Some(expected) = state.config.setup.bootstrap_token.as_ref() else {
        return false;
    };
    let Some(provided) = provided else {
        return false;
    };

    let expected = expected.expose_secret().as_bytes();
    let provided = provided.expose_secret().as_bytes();
    !expected.is_empty() && expected.ct_eq(provided).into()
}

fn invalid_input(field: &'static str) -> Response {
    crate::routes::response::ApiError::new("invalid_input")
        .with_detail(field)
        .into_response(StatusCode::BAD_REQUEST)
}

fn provision_error_response(err: ProvisionError) -> Response {
    match err {
        ProvisionError::ManagementApi { detail, .. } => {
            crate::routes::response::ApiError::new("setup_provision_failed")
                .with_detail(detail)
                .into_response(StatusCode::BAD_REQUEST)
        }
        ProvisionError::ManagementPathNotFound { path } => {
            crate::routes::response::ApiError::new("setup_provision_failed")
                .with_detail(format!("Stalwart management API path not found: {path}"))
                .into_response(StatusCode::BAD_REQUEST)
        }
        ProvisionError::Management(detail) | ProvisionError::Jmap(detail) => {
            crate::routes::response::ApiError::new("setup_provision_failed")
                .with_detail(detail)
                .into_response(StatusCode::BAD_REQUEST)
        }
        ProvisionError::Nonce => internal(),
    }
}

fn forbidden_setup_bootstrap_required() -> Response {
    crate::routes::response::ApiError::new("setup_bootstrap_required")
        .with_detail("Setup bootstrap token is required or invalid.")
        .into_response(StatusCode::FORBIDDEN)
}

fn conflict_setup_disabled() -> Response {
    error_response(StatusCode::CONFLICT, "setup_disabled")
}

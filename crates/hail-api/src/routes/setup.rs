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
use crate::routes::provider_accounts::{
    GMAIL_PROVIDER_KIND, GMAIL_READONLY_SCOPE, GMAIL_SEND_SCOPE, GmailAuthorizationRequest,
    GmailOAuthClient, GmailTokenExchange, LiveGmailOAuthClient, PROVIDER_REFRESH_TOKEN_KEY_ID,
};
use crate::routes::response::{error_response, internal};
use crate::routes::validation::{valid_domain, valid_email};
use crate::state::AppState;

static SETUP_PROVISION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static SETUP_GMAIL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

const SETUP_GMAIL_OAUTH_STATE_TTL_MINUTES: i64 = 10;
const SETUP_GMAIL_REDIRECT_URI: &str = "/api/setup/gmail/callback";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SetupBackend {
    Gmail,
    Jmap,
}

#[derive(Debug, Serialize, ToSchema)]
struct SetupStateResponse {
    wizard_active: bool,
    backend: SetupBackend,
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

#[derive(Debug, Deserialize, ToSchema)]
struct SetupGmailConnectRequest {
    email: String,
    #[schema(value_type = String)]
    password: SecretString,
    display_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct SetupGmailConnectResponse {
    authorization_url: String,
    scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SetupGmailCallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
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

pub fn router_with_deps<P, G>(provisioner: Arc<P>, gmail_client: Arc<G>) -> Router<AppState>
where
    P: UserProvisioner,
    G: GmailOAuthClient,
{
    Router::from(openapi_router_with_deps(provisioner, gmail_client))
}

pub fn openapi_router_with_provisioner<P>(provisioner: Arc<P>) -> OpenApiRouter<AppState>
where
    P: UserProvisioner,
{
    openapi_router_with_deps(
        provisioner,
        Arc::new(LiveGmailOAuthClient::from_config_default()),
    )
}

pub fn openapi_router_with_deps<P, G>(
    provisioner: Arc<P>,
    gmail_client: Arc<G>,
) -> OpenApiRouter<AppState>
where
    P: UserProvisioner,
    G: GmailOAuthClient,
{
    let provisioner: Arc<dyn UserProvisioner> = provisioner;
    let gmail_client: Arc<dyn GmailOAuthClient> = gmail_client;
    OpenApiRouter::new()
        .routes(routes!(setup_state))
        .routes(routes!(setup_admin).layer(Extension(provisioner)))
        .routes(routes!(setup_gmail_connect).layer(Extension(gmail_client.clone())))
        .routes(routes!(setup_gmail_callback).layer(Extension(gmail_client)))
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
    if state.config.mail.backend != hail_core::MailBackend::Jmap {
        return error_response(StatusCode::NOT_FOUND, "setup_backend_mismatch");
    }

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
    if status.backend != SetupBackend::Jmap {
        return error_response(StatusCode::NOT_FOUND, "setup_backend_mismatch");
    }
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
    if status.backend != SetupBackend::Jmap {
        return error_response(StatusCode::NOT_FOUND, "setup_backend_mismatch");
    }
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
    user_created_response(view, &cookie)
}

#[utoipa::path(
    post,
    path = "/api/setup/gmail/connect",
    tag = "setup",
    request_body = SetupGmailConnectRequest,
    responses(
        (status = 200, description = "Gmail setup OAuth authorization URL.", body = SetupGmailConnectResponse),
        (status = 400, description = "Setup input is invalid.", body = crate::routes::response::ApiError),
        (status = 403, description = "Missing CSRF header.", body = crate::routes::response::ApiError),
        (status = 409, description = "Setup wizard is no longer active.", body = crate::routes::response::ApiError),
        (status = 503, description = "Gmail OAuth is not configured.", body = crate::routes::response::ApiError)
    )
)]
async fn setup_gmail_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(client): Extension<Arc<dyn GmailOAuthClient>>,
    Json(body): Json<SetupGmailConnectRequest>,
) -> Response {
    if headers
        .get(crate::middleware::auth::CSRF_HEADER)
        .map(|v| v.as_bytes())
        != Some(b"1")
    {
        return error_response(StatusCode::FORBIDDEN, "csrf_required");
    }
    if state.config.mail.backend != hail_core::MailBackend::Gmail {
        return error_response(StatusCode::NOT_FOUND, "setup_backend_mismatch");
    }

    let email = body.email.trim().to_lowercase();
    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    if !valid_email(&email) {
        return invalid_input("email");
    }
    if body.password.expose_secret().len() < 12 {
        return invalid_input("password");
    }

    let Some(client_id) = state
        .config
        .mail
        .gmail
        .oauth_client_id
        .clone()
        .or_else(|| state.config.provider_import.gmail.oauth_client_id.clone())
    else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "gmail_oauth_not_configured",
        );
    };

    if let Err(response) = ensure_setup_active_for_backend(&state, SetupBackend::Gmail).await {
        return response;
    }

    let redirect_uri = setup_gmail_redirect_uri(&state);
    let scopes = setup_gmail_scopes();
    let state_token = match create_setup_gmail_oauth_state(
        &state,
        &email,
        body.password,
        display_name.as_deref(),
        &redirect_uri,
        &scopes,
    )
    .await
    {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(error = %err, "setup gmail: state creation failed");
            return internal();
        }
    };

    let authorization_url = match client.authorization_url(GmailAuthorizationRequest {
        client_id,
        redirect_uri,
        state: state_token,
        scopes: scopes.clone(),
    }) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(error = %err, "setup gmail: authorization URL failed");
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "gmail_oauth_not_configured",
            );
        }
    };

    Json(SetupGmailConnectResponse {
        authorization_url,
        scopes,
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/api/setup/gmail/callback",
    tag = "setup",
    params(("state" = Option<String>, Query), ("code" = Option<String>, Query), ("error" = Option<String>, Query)),
    responses(
        (status = 303, description = "Gmail admin user created; session cookie has been set and browser is redirected to the app."),
        (status = 400, description = "Gmail OAuth callback failed.", body = crate::routes::response::ApiError),
        (status = 409, description = "Setup wizard is no longer active.", body = crate::routes::response::ApiError),
        (status = 500, description = "Internal setup failure.", body = crate::routes::response::ApiError)
    )
)]
async fn setup_gmail_callback(
    State(state): State<AppState>,
    Extension(client): Extension<Arc<dyn GmailOAuthClient>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SetupGmailCallbackQuery>,
) -> Response {
    if query.error.is_some() {
        return error_response(StatusCode::BAD_REQUEST, "oauth_denied");
    }
    let Some(state_token) = query.state.as_deref() else {
        return error_response(StatusCode::BAD_REQUEST, "missing_state");
    };
    let Some(code) = query.code.as_deref() else {
        return error_response(StatusCode::BAD_REQUEST, "missing_code");
    };

    let _setup_guard = SETUP_GMAIL_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    if let Err(response) = ensure_setup_active_for_backend(&state, SetupBackend::Gmail).await {
        return response;
    }

    let oauth_state = match consume_setup_gmail_oauth_state(&state, state_token).await {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(StatusCode::BAD_REQUEST, "invalid_oauth_state"),
        Err(err) => {
            tracing::error!(error = %err, "setup gmail: state lookup failed");
            return internal();
        }
    };

    let exchange = match client.exchange_code(code, &oauth_state.redirect_uri).await {
        Ok(exchange) => exchange,
        Err(err) => {
            tracing::warn!(error = %err, "setup gmail: exchange failed");
            return error_response(StatusCode::BAD_REQUEST, "oauth_exchange_failed");
        }
    };

    match create_gmail_setup_user(&state, &headers, oauth_state, exchange).await {
        Ok((_view, cookie)) => user_created_redirect_response(&cookie),
        Err(SetupGmailCreateError::Conflict) => conflict_setup_disabled(),
        Err(SetupGmailCreateError::InvalidGrant(reason)) => {
            crate::routes::response::ApiError::new("gmail_oauth_invalid_grant")
                .with_detail(reason)
                .into_response(StatusCode::BAD_REQUEST)
        }
        Err(SetupGmailCreateError::Sql(err)) => {
            tracing::error!(error = %err, "setup gmail: create user failed");
            internal()
        }
        Err(SetupGmailCreateError::Crypto(err)) => {
            tracing::error!(error = %err, "setup gmail: token seal failed");
            internal()
        }
        Err(SetupGmailCreateError::ProviderTokenCrypto(err)) => {
            tracing::error!(error = %err, "setup gmail: refresh token seal failed");
            internal()
        }
        Err(SetupGmailCreateError::SessionId) => {
            tracing::error!("setup gmail: failed to draw session id from OS RNG");
            internal()
        }
        Err(SetupGmailCreateError::Json(err)) => {
            tracing::error!(error = %err, "setup gmail: scopes json failed");
            internal()
        }
    }
}

fn user_created_response(view: UserView, cookie: &str) -> Response {
    (
        StatusCode::CREATED,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::SET_COOKIE, cookie.to_string()),
        ],
        serde_json::to_string(&UserEnvelope { user: view }).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

fn user_created_redirect_response(cookie: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie.to_string()),
            (header::LOCATION, "/imbox".to_string()),
        ],
    )
        .into_response()
}

#[derive(Debug)]
struct SetupStatus {
    wizard_active: bool,
    backend: SetupBackend,
    reason: Option<SetupDisabledReason>,
}

impl SetupStatus {
    fn into_response(self) -> SetupStateResponse {
        SetupStateResponse {
            wizard_active: self.wizard_active,
            backend: self.backend,
            reason: self.reason,
        }
    }
}

#[derive(Debug)]
struct SetupGmailOAuthState {
    email: String,
    display_name: Option<String>,
    password: SecretString,
    redirect_uri: String,
}

#[derive(Debug, thiserror::Error)]
enum SetupGmailCreateError {
    #[error("setup wizard is no longer active")]
    Conflict,
    #[error("{0}")]
    InvalidGrant(String),
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Crypto(#[from] hail_core::CryptoError),
    #[error(transparent)]
    ProviderTokenCrypto(#[from] hail_core::ProviderTokenCryptoError),
    #[error("failed to draw session id")]
    SessionId,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
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
            backend: setup_backend(state),
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
            backend: setup_backend(state),
            reason: Some(SetupDisabledReason::AdminUserExists),
        })
    } else {
        Ok(SetupStatus {
            wizard_active: true,
            backend: setup_backend(state),
            reason: None,
        })
    }
}

async fn ensure_setup_active_for_backend(
    state: &AppState,
    expected: SetupBackend,
) -> Result<(), Response> {
    let status = setup_status(state).await.map_err(|err| {
        tracing::error!(error = %err, "setup: active check failed");
        internal()
    })?;
    if status.backend != expected {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "setup_backend_mismatch",
        ));
    }
    if !status.wizard_active {
        return Err(conflict_setup_disabled());
    }
    Ok(())
}

fn setup_backend(state: &AppState) -> SetupBackend {
    match state.config.mail.backend {
        hail_core::MailBackend::Gmail => SetupBackend::Gmail,
        hail_core::MailBackend::Jmap => SetupBackend::Jmap,
    }
}

fn setup_gmail_redirect_uri(state: &AppState) -> String {
    format!(
        "{}{}",
        state.config.server.public_url.trim_end_matches('/'),
        SETUP_GMAIL_REDIRECT_URI
    )
}

fn setup_gmail_scopes() -> Vec<String> {
    vec![GMAIL_READONLY_SCOPE.to_owned(), GMAIL_SEND_SCOPE.to_owned()]
}

async fn create_setup_gmail_oauth_state(
    state: &AppState,
    email: &str,
    password: SecretString,
    display_name: Option<&str>,
    redirect_uri: &str,
    scopes: &[String],
) -> Result<String, sqlx::Error> {
    let now = Utc::now();
    let expires_at = now + Duration::minutes(SETUP_GMAIL_OAUTH_STATE_TTL_MINUTES);
    let mut payload = serde_json::Map::new();
    payload.insert(
        "email".to_owned(),
        serde_json::Value::String(email.to_owned()),
    );
    let password_enc = hail_core::seal(password.expose_secret().as_bytes(), &state.server_key)
        .map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    payload.insert(
        "password_enc".to_owned(),
        serde_json::Value::String(hex::encode(password_enc)),
    );
    if let Some(display_name) = display_name {
        payload.insert(
            "display_name".to_owned(),
            serde_json::Value::String(display_name.to_owned()),
        );
    }
    payload.insert(
        "redirect_uri".to_owned(),
        serde_json::Value::String(redirect_uri.to_owned()),
    );
    payload.insert(
        "expires_at".to_owned(),
        serde_json::Value::String(expires_at.to_rfc3339()),
    );
    payload.insert(
        "scopes".to_owned(),
        serde_json::Value::Array(
            scopes
                .iter()
                .map(|scope| serde_json::Value::String(scope.clone()))
                .collect(),
        ),
    );
    let payload = serde_json::Value::Object(payload).to_string();
    let ciphertext = hail_core::seal(payload.as_bytes(), &state.server_key)
        .map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    Ok(hex::encode(ciphertext))
}

async fn consume_setup_gmail_oauth_state(
    state: &AppState,
    token: &str,
) -> Result<Option<SetupGmailOAuthState>, sqlx::Error> {
    let ciphertext = match hex::decode(token) {
        Ok(ciphertext) => ciphertext,
        Err(_) => return Ok(None),
    };
    let payload = match hail_core::open(&ciphertext, &state.server_key) {
        Ok(payload) => payload,
        Err(_) => return Ok(None),
    };
    let payload: serde_json::Value =
        serde_json::from_slice(&payload).map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    let expires_at = payload
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| sqlx::Error::Protocol("missing setup gmail expires_at".to_string()))?
        .parse::<chrono::DateTime<Utc>>()
        .map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    if expires_at <= Utc::now() {
        return Ok(None);
    }
    let redirect_uri = payload
        .get("redirect_uri")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| sqlx::Error::Protocol("missing setup gmail redirect_uri".to_string()))?
        .to_owned();
    let email = payload
        .get("email")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| sqlx::Error::Protocol("missing setup gmail email".to_string()))?
        .to_owned();
    let password_enc_hex = payload
        .get("password_enc")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| sqlx::Error::Protocol("missing setup gmail password_enc".to_string()))?;
    let password_enc =
        hex::decode(password_enc_hex).map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    let password = hail_core::open(&password_enc, &state.server_key)
        .map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    let password =
        String::from_utf8(password).map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    let display_name = payload
        .get("display_name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    Ok(Some(SetupGmailOAuthState {
        email,
        display_name,
        password: SecretString::from(password),
        redirect_uri,
    }))
}

async fn create_gmail_setup_user(
    state: &AppState,
    headers: &HeaderMap,
    oauth_state: SetupGmailOAuthState,
    exchange: GmailTokenExchange,
) -> Result<(UserView, String), SetupGmailCreateError> {
    let profile_email = exchange.profile.email.trim().to_lowercase();
    if profile_email != oauth_state.email {
        return Err(SetupGmailCreateError::InvalidGrant(
            "Google account email did not match setup email.".to_string(),
        ));
    }

    let scopes = normalize_gmail_scopes(exchange.granted_scopes);
    if !scopes.iter().any(|scope| scope == GMAIL_READONLY_SCOPE) {
        return Err(SetupGmailCreateError::InvalidGrant(
            "gmail.readonly scope missing".to_string(),
        ));
    }
    if !scopes.iter().any(|scope| scope == GMAIL_SEND_SCOPE) {
        return Err(SetupGmailCreateError::InvalidGrant(
            "gmail.send scope missing".to_string(),
        ));
    }
    let Some(refresh_token) = exchange.refresh_token else {
        return Err(SetupGmailCreateError::InvalidGrant(
            "gmail refresh token missing".to_string(),
        ));
    };

    let now = Utc::now();
    let mut tx = state.db.begin().await?;
    let status = setup_status_in_tx(state, &mut tx).await?;
    if status.backend != SetupBackend::Gmail || !status.wizard_active {
        return Err(SetupGmailCreateError::Conflict);
    }

    let jmap_account_id = format!("gmail:{profile_email}");
    let (user_id, db_email, db_display_name, is_admin_int): (i64, String, Option<String>, i64) =
        sqlx::query_as(
            "INSERT INTO users (email, jmap_account_id, display_name, is_admin, created_at) \
             VALUES (?1, ?2, ?3, 1, ?4) \
             RETURNING id, email, display_name, is_admin",
        )
        .bind(&profile_email)
        .bind(&jmap_account_id)
        .bind(oauth_state.display_name.as_deref())
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;

    let session_token_enc = hail_core::seal(
        oauth_state.password.expose_secret().as_bytes(),
        &state.server_key,
    )?;
    let session_id = new_session_id().map_err(|_| SetupGmailCreateError::SessionId)?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let expires_at = now + Duration::days(SESSION_TTL_DAYS);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&session_token_enc)
    .bind(user_agent.as_deref())
    .bind(expires_at)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let scopes_json = serde_json::to_string(&scopes)?;
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, display_email, granted_scopes_json, consented_at, \
          cached_access_token_expires_at, access_token_refreshed_at, last_profile_history_id, profile_synced_at, sync_status, created_at, updated_at) \
         VALUES (?1, ?2, 'gmail', 'gmail', ?3, ?4, ?5, ?6, ?7, ?8, ?7, ?9, ?7, 'disabled', ?7, ?7) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(&jmap_account_id)
    .bind(&profile_email)
    .bind(&profile_email)
    .bind(&profile_email)
    .bind(&scopes_json)
    .bind(now)
    .bind(exchange.expires_at)
    .bind(exchange.profile.history_id.as_deref())
    .fetch_one(&mut *tx)
    .await?;

    let context = hail_core::ProviderTokenContext::new(
        user_id,
        account_id,
        GMAIL_PROVIDER_KIND,
        &profile_email,
        hail_core::ProviderOAuthTokenKind::Refresh,
    );
    let encrypted = hail_core::seal_provider_oauth_token(
        &hail_core::ProviderOAuthToken::from(refresh_token),
        &state.server_key,
        &context,
    )?
    .into_bytes();

    sqlx::query(
        "UPDATE mail_accounts \
         SET refresh_token_enc = ?1, refresh_token_ref = NULL, refresh_token_key_id = ?2, sync_status = 'active', updated_at = ?3 \
         WHERE id = ?4",
    )
    .bind(encrypted)
    .bind(PROVIDER_REFRESH_TOKEN_KEY_ID)
    .bind(now)
    .bind(account_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO cache_policy (account_id, mode, keep_days, keep_max_msgs, keep_max_bytes, backfill, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(account_id) DO UPDATE SET \
           mode = excluded.mode, keep_days = excluded.keep_days, keep_max_msgs = excluded.keep_max_msgs, \
           keep_max_bytes = excluded.keep_max_bytes, backfill = excluded.backfill, updated_at = excluded.updated_at",
    )
    .bind(account_id)
    .bind("bounded")
    .bind(i64::from(state.config.mail.cache.keep_days))
    .bind(i64::try_from(state.config.mail.cache.keep_max_msgs).unwrap_or(i64::MAX))
    .bind(i64::try_from(state.config.mail.cache.keep_max_bytes).unwrap_or(i64::MAX))
    .bind("incremental")
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let view = UserView {
        id: user_id,
        email: db_email,
        display_name: db_display_name,
        is_admin: is_admin_int != 0,
    };
    Ok((view, build_session_cookie(&session_id)))
}

fn normalize_gmail_scopes(scopes: Vec<String>) -> Vec<String> {
    let mut scopes = scopes;
    scopes.sort();
    scopes.dedup();
    scopes
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

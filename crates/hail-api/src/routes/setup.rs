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
use rand::TryRngCore;
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
use crate::routes::management_http;
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
                let token = authenticate_stalwart_management(
                    management_url,
                    stalwart_admin_username,
                    stalwart_admin_password,
                )
                .await?;
                create_stalwart_domain(management_url, &token, domain).await?;
                create_stalwart_principal(management_url, &token, email, &password, display_name)
                    .await?;
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

async fn authenticate_stalwart_management(
    management_url: &str,
    username: &str,
    password: SecretString,
) -> Result<SecretString, ProvisionError> {
    #[derive(Serialize)]
    struct AuthCodeRequest<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        #[serde(rename = "accountName")]
        account_name: &'a str,
        #[serde(rename = "accountSecret")]
        account_secret: &'a str,
        #[serde(rename = "clientId")]
        client_id: &'static str,
        #[serde(rename = "redirectUri")]
        redirect_uri: &'static str,
        nonce: &'a str,
    }

    let nonce = random_nonce()?;
    let auth_body = AuthCodeRequest {
        kind: "authCode",
        account_name: username,
        account_secret: password.expose_secret(),
        client_id: "webadmin",
        redirect_uri: "https://localhost/setup",
        nonce: &nonce,
    };
    let auth_json = post_management_json(
        management_url,
        "/api/auth",
        None,
        &auth_body,
        "invalid Stalwart admin credentials",
    )
    .await?;

    let Some(code) = auth_json
        .pointer("/data/code")
        .and_then(serde_json::Value::as_str)
    else {
        if let Some(token) = extract_access_token(&auth_json) {
            return Ok(SecretString::from(token));
        }
        return Err(ProvisionError::Management(
            "Stalwart auth response did not include a client code or access token".to_string(),
        ));
    };

    exchange_stalwart_client_code(management_url, code, &nonce).await
}

async fn exchange_stalwart_client_code(
    management_url: &str,
    code: &str,
    nonce: &str,
) -> Result<SecretString, ProvisionError> {
    #[derive(Serialize)]
    struct AuthTokenRequest<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        code: &'a str,
        #[serde(rename = "clientId")]
        client_id: &'static str,
        #[serde(rename = "redirectUri")]
        redirect_uri: &'static str,
        nonce: &'a str,
    }

    let token_body = AuthTokenRequest {
        kind: "code",
        code,
        client_id: "webadmin",
        redirect_uri: "https://localhost/setup",
        nonce,
    };
    match post_management_json(
        management_url,
        "/api/auth/token",
        None,
        &token_body,
        "Stalwart management token exchange failed",
    )
    .await
    {
        Ok(json) => extract_exchange_token(&json)
            .map(SecretString::from)
            .ok_or_else(|| {
                ProvisionError::Management(
                    "Stalwart token response did not include an access token".to_string(),
                )
            }),
        Err(ProvisionError::ManagementPathNotFound { .. }) => {
            #[derive(Serialize)]
            struct OAuthRequest<'a> {
                #[serde(rename = "type")]
                kind: &'static str,
                #[serde(rename = "client_id")]
                client_id: &'static str,
                #[serde(rename = "redirect_uri")]
                redirect_uri: &'static str,
                nonce: &'a str,
            }

            let oauth_body = OAuthRequest {
                kind: "code",
                client_id: "webadmin",
                redirect_uri: "stalwart://auth",
                nonce,
            };
            let json = post_management_json(
                management_url,
                "/api/oauth",
                Some(code),
                &oauth_body,
                "Stalwart management token exchange failed",
            )
            .await?;
            extract_exchange_token(&json)
                .map(SecretString::from)
                .ok_or_else(|| {
                    ProvisionError::Management(
                        "Stalwart OAuth response did not include an access token".to_string(),
                    )
                })
        }
        Err(err) => Err(err),
    }
}

async fn create_stalwart_domain(
    management_url: &str,
    token: &SecretString,
    domain: &str,
) -> Result<(), ProvisionError> {
    let body = serde_json::json!({
        "type": "domain",
        "name": domain,
    });
    post_management_json(
        management_url,
        "/api/principal",
        Some(token.expose_secret()),
        &body,
        "Stalwart domain provisioning failed",
    )
    .await
    .map(|_| ())
}

async fn create_stalwart_principal(
    management_url: &str,
    token: &SecretString,
    email: &str,
    password: &SecretString,
    display_name: Option<&str>,
) -> Result<(), ProvisionError> {
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
    post_management_json(
        management_url,
        "/api/principal",
        Some(token.expose_secret()),
        &payload,
        "Stalwart mailbox provisioning failed",
    )
    .await
    .map(|_| ())
}

async fn post_management_json<T: Serialize + ?Sized>(
    management_url: &str,
    path: &str,
    bearer: Option<&str>,
    body: &T,
    failure_context: &str,
) -> Result<serde_json::Value, ProvisionError> {
    let url = format!("{}{}", management_url.trim_end_matches('/'), path);
    let mut request = management_http::client().post(&url).json(body);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|err| ProvisionError::Management(err.to_string()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| ProvisionError::Management(err.to_string()))?;

    if status.is_success() || is_idempotent_already_exists(status, &text) {
        if text.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
        return serde_json::from_str(&text)
            .map_err(|err| ProvisionError::Management(format!("invalid Stalwart JSON: {err}")));
    }

    let sanitized = sanitized_management_body(&text);
    let err = management_api_error(status, failure_context, &sanitized);
    tracing::error!(%url, error = %err, status = %status, body = %sanitized, "setup: Stalwart management request failed");
    if status.as_u16() == 404 {
        return Err(ProvisionError::ManagementPathNotFound {
            path: path.to_string(),
        });
    }
    Err(err)
}

fn is_idempotent_already_exists(status: StatusCode, body: &str) -> bool {
    if status != StatusCode::CONFLICT {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("already exists") || lower.contains("already exist") || lower.contains("exists")
}

fn management_api_error(status: StatusCode, context: &str, body: &str) -> ProvisionError {
    let detail = problem_detail(body).unwrap_or_else(|| {
        if status == StatusCode::UNAUTHORIZED {
            "invalid Stalwart admin credentials".to_string()
        } else if body.trim().is_empty() {
            context.to_string()
        } else {
            format!("{context}: {body}")
        }
    });
    ProvisionError::ManagementApi { status, detail }
}

fn problem_detail(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let title = json.get("title").and_then(serde_json::Value::as_str);
    let detail = json
        .get("detail")
        .or_else(|| json.get("details"))
        .or_else(|| json.get("reason"))
        .and_then(serde_json::Value::as_str);
    match (title, detail) {
        (Some(title), Some(detail)) if !detail.is_empty() => Some(format!("{title}: {detail}")),
        (Some(title), _) => Some(title.to_string()),
        (_, Some(detail)) => Some(detail.to_string()),
        _ => None,
    }
}

fn extract_access_token(json: &serde_json::Value) -> Option<String> {
    [
        "/data/access_token",
        "/data/accessToken",
        "/data/token",
        "/access_token",
        "/accessToken",
        "/token",
    ]
    .into_iter()
    .find_map(|pointer| json.pointer(pointer)?.as_str().map(str::to_string))
}

fn extract_exchange_token(json: &serde_json::Value) -> Option<String> {
    extract_access_token(json).or_else(|| {
        json.pointer("/data/code")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    })
}

fn random_nonce() -> Result<String, ProvisionError> {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ProvisionError::Nonce)?;
    Ok(hex::encode(bytes))
}

fn sanitized_management_body(body: &str) -> String {
    const MAX: usize = 2048;
    let trimmed = body.trim();
    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        redact_secret_fields(&mut json);
        return truncate(&json.to_string(), MAX);
    }
    truncate(trimmed, MAX)
}

fn redact_secret_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "accountSecret"
                        | "account_secret"
                        | "secret"
                        | "secrets"
                        | "password"
                        | "access_token"
                        | "accessToken"
                        | "token"
                        | "code"
                ) {
                    *value = serde_json::Value::String("<redacted>".to_string());
                } else {
                    redact_secret_fields(value);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_secret_fields),
        _ => {}
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_string()
    } else {
        format!("{}…", &value[..max])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_management_body_redacts_secret_fields() {
        let body = serde_json::json!({
            "accountSecret": "admin1234",
            "access_token": "bearer-token",
            "nested": { "secrets": ["mailbox-password"], "code": "client-code" },
            "title": "Unauthorized"
        })
        .to_string();

        let redacted = sanitized_management_body(&body);
        assert!(!redacted.contains("admin1234"));
        assert!(!redacted.contains("bearer-token"));
        assert!(!redacted.contains("mailbox-password"));
        assert!(!redacted.contains("client-code"));
        assert!(redacted.contains("<redacted>"));
    }
}

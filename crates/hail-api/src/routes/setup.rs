//! First-run setup wizard endpoints.
//!
//! Public by design: `/setup` is reachable before any user/session exists.
//! Every mutating path re-checks the wizard gate inside the DB transaction so
//! two concurrent first-run submissions cannot both create admins.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use chrono::{Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::middleware::session::{
    SESSION_TTL_DAYS, basic_bearer, build_session_cookie, new_session_id,
};
use crate::routes::auth::UserView;
use crate::state::AppState;

static SETUP_PROVISION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Serialize)]
struct SetupStateResponse {
    wizard_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<SetupDisabledReason>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SetupDisabledReason {
    ConfigAdminSet,
    AdminUserExists,
}

#[derive(Debug, Deserialize)]
struct SetupAdminRequest {
    email: String,
    password: SecretString,
    display_name: Option<String>,
    domain: String,
    bootstrap_token: Option<SecretString>,
}

#[derive(Debug, Serialize)]
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
    ) -> Pin<Box<dyn Future<Output = Result<ProvisionedUser, ProvisionError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(management_url) = state.config.stalwart.management_url.as_deref() {
                create_stalwart_principal(management_url, email, &password, display_name).await?;
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
    router_with_provisioner(Arc::new(StalwartProvisioner))
}

pub fn router_with_provisioner<P>(provisioner: Arc<P>) -> Router<AppState>
where
    P: UserProvisioner,
{
    Router::new()
        .route("/api/setup/state", axum::routing::get(setup_state))
        .route("/api/setup/admin", post(setup_admin::<P>))
        .layer(Extension(provisioner))
}

async fn setup_state(State(state): State<AppState>) -> Response {
    match setup_status(&state).await {
        Ok(status) => Json(status.into_response()).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "setup: state check failed");
            internal()
        }
    }
}

async fn setup_admin<P>(
    State(state): State<AppState>,
    Extension(provisioner): Extension<Arc<P>>,
    headers: HeaderMap,
    Json(body): Json<SetupAdminRequest>,
) -> Response
where
    P: UserProvisioner,
{
    let email = body.email.trim().to_lowercase();
    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let domain = body.domain.trim().to_lowercase();

    if !valid_email(&email) {
        return invalid_input("email");
    }
    if body.password.expose_secret().len() < 12 {
        return invalid_input("password");
    }
    if !valid_domain(&domain) {
        return invalid_input("domain");
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
        .provision(&state, &email, body.password, display_name.as_deref())
        .await
    {
        Ok(user) => user,
        Err(err) => {
            tracing::error!(email, error = %err, "setup: provision failed");
            return internal();
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

async fn create_stalwart_principal(
    management_url: &str,
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

    let path = "/api/principal";
    let url = format!("{}{}", management_url.trim_end_matches('/'), path);
    let payload = Principal {
        kind: "individual",
        name: email,
        secrets: [password.expose_secret()],
        emails: [email],
        description: display_name,
    };
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|err| ProvisionError::Management(err.to_string()))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 404 {
        tracing::error!(%url, "setup: Stalwart management principal path returned 404; API path may differ across Stalwart versions");
        return Err(ProvisionError::ManagementPathNotFound {
            path: path.to_string(),
        });
    }
    Err(ProvisionError::Management(format!(
        "POST {path} returned HTTP {status}"
    )))
}

fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && valid_domain(domain)
        && !email.contains(char::is_whitespace)
        && email.matches('@').count() == 1
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
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
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"invalid_input","field":"{field}"}}"#),
    )
        .into_response()
}

fn forbidden_setup_bootstrap_required() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"setup_bootstrap_required"}"#,
    )
        .into_response()
}

fn conflict_setup_disabled() -> Response {
    (
        StatusCode::CONFLICT,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"setup_disabled"}"#,
    )
        .into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}

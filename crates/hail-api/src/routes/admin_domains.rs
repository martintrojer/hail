//! Admin domain management endpoints.
//!
//! Routes are mounted behind auth/CSRF middleware. Production uses Stalwart's
//! JMAP management capability (`urn:stalwart:jmap`) via Principal/*; tests can
//! still inject a fake through [`StalwartManagement`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::response::{ApiError, bad_request, error_response};
use crate::routes::validation::valid_domain;
use crate::state::AppState;

pub trait StalwartManagement: Send + Sync + 'static {
    fn list_domains<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>>;

    fn add_domain<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>>;

    fn delete_domain<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>>;
}

pub struct HttpStalwartManagement;

impl StalwartManagement for HttpStalwartManagement {
    fn list_domains<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            Ok(session
                .list_domains()
                .await?
                .into_iter()
                .map(|principal| principal.name)
                .collect())
        })
    }

    fn add_domain<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            session.create_domain(domain).await?;
            Ok(())
        })
    }

    fn delete_domain<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let session = management_session(state, bearer).await?;
            session.destroy_domain(domain).await?;
            Ok(())
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManagementError {
    #[error("stalwart.management_url is not configured")]
    NotConfigured,
    #[error("stalwart management API returned HTTP {status}: {detail}")]
    Api { status: StatusCode, detail: String },
    #[error("stalwart management request failed: {0}")]
    Upstream(String),
}

impl From<hail_jmap::management::ManagementError> for ManagementError {
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

pub fn router() -> Router<AppState> {
    router_with_management(Arc::new(HttpStalwartManagement))
}

pub fn router_with_management<M>(management: Arc<M>) -> Router<AppState>
where
    M: StalwartManagement,
{
    Router::new()
        .route("/api/admin/domains", axum::routing::get(list_domains::<M>))
        .route("/api/admin/domains", axum::routing::post(add_domain::<M>))
        .route(
            "/api/admin/domains/{domain}",
            axum::routing::delete(delete_domain::<M>),
        )
        .layer(Extension(management))
}

#[derive(Debug, Serialize)]
struct DomainListResponse {
    domains: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DomainResponse {
    domain: String,
}

#[derive(Debug, Deserialize)]
struct AddDomainRequest {
    domain: String,
}

async fn list_domains<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
) -> Response
where
    M: StalwartManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }

    match management
        .list_domains(&state, user.jmap_token.clone())
        .await
    {
        Ok(mut domains) => {
            domains.sort_unstable();
            domains.dedup();
            Json(DomainListResponse { domains }).into_response()
        }
        Err(err) => management_error(err),
    }
}

async fn add_domain<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
    Json(body): Json<AddDomainRequest>,
) -> Response
where
    M: StalwartManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }

    let domain = normalize_domain(&body.domain);
    if !valid_domain(&domain) {
        return invalid_domain();
    }

    match management
        .add_domain(&state, user.jmap_token.clone(), &domain)
        .await
    {
        Ok(()) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "admin.domain.add",
                &serde_json::json!({ "domain": domain }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            (StatusCode::CREATED, Json(DomainResponse { domain })).into_response()
        }
        Err(err) => management_error(err),
    }
}

async fn delete_domain<M>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(management): Extension<Arc<M>>,
    Path(domain): Path<String>,
) -> Response
where
    M: StalwartManagement,
{
    if !user.is_admin {
        return forbidden_admin();
    }

    let domain = normalize_domain(&domain);
    if !valid_domain(&domain) {
        return invalid_domain();
    }

    match management
        .delete_domain(&state, user.jmap_token.clone(), &domain)
        .await
    {
        Ok(()) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "admin.domain.delete",
                &serde_json::json!({ "domain": domain }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(err) => management_error(err),
    }
}

async fn management_session(
    state: &AppState,
    bearer: SecretString,
) -> Result<hail_jmap::management::ManagementSession, ManagementError> {
    let base = management_base(state)?;
    Ok(hail_jmap::management::ManagementSession::connect(&base, bearer).await?)
}

fn management_base(state: &AppState) -> Result<String, ManagementError> {
    state
        .config
        .stalwart
        .management_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| url.trim_end_matches('/').to_string())
        .ok_or(ManagementError::NotConfigured)
}

fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn invalid_domain() -> Response {
    bad_request("invalid_domain")
}

fn forbidden_admin() -> Response {
    error_response(StatusCode::FORBIDDEN, "admin_required")
}

fn management_error(err: ManagementError) -> Response {
    match err {
        ManagementError::NotConfigured => error_response(
            StatusCode::NOT_IMPLEMENTED,
            "stalwart_management_unconfigured",
        ),
        ManagementError::Api { status, detail } if status.is_client_error() => {
            ApiError::new("stalwart_management_failed")
                .with_detail(detail)
                .into_response(status)
        }
        ManagementError::Api { status, detail } => {
            tracing::warn!(%status, error = %detail, "stalwart domain management failed");
            error_response(StatusCode::BAD_GATEWAY, "stalwart_management_failed")
        }
        ManagementError::Upstream(message) => {
            tracing::warn!(error = %message, "stalwart domain management failed");
            error_response(StatusCode::BAD_GATEWAY, "stalwart_management_failed")
        }
    }
}

//! Admin domain management endpoints.
//!
//! These routes are mounted behind the global auth/CSRF middleware. Handlers
//! additionally require `AuthUser::is_admin` before touching Stalwart. The
//! Stalwart management surface is isolated behind [`StalwartManagement`] so
//! tests can use a fake and production can grow with Stalwart API details.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::middleware::auth::AuthUser;
use crate::routes::management_http;
use crate::routes::response::{bad_request, error_response};
use crate::routes::validation::valid_domain;
use crate::state::AppState;

/// Dependency-injection seam for Stalwart domain administration.
pub trait StalwartManagement: Send + Sync + 'static {
    fn list_domains<'a>(
        &'a self,
        state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>>;

    fn add_domain<'a>(
        &'a self,
        state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>>;

    fn delete_domain<'a>(
        &'a self,
        state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>>;
}

/// Production Stalwart management implementation.
///
/// If `stalwart.management_url` is absent, handlers return a clear 501 so
/// operators know the admin surface is unavailable rather than silently doing
/// nothing. If it is present, we call the currently expected Stalwart domain
/// management paths; non-success responses are surfaced as upstream errors.
pub struct HttpStalwartManagement;

impl StalwartManagement for HttpStalwartManagement {
    fn list_domains<'a>(
        &'a self,
        state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let base = management_base(state)?;
            let url = format!("{}/api/domain", base);
            let response = management_http::client()
                .get(&url)
                .send()
                .await
                .map_err(|err| ManagementError::Upstream(err.to_string()))?;
            if !response.status().is_success() {
                return Err(ManagementError::Upstream(format!(
                    "GET /api/domain returned HTTP {}",
                    response.status()
                )));
            }
            decode_domain_list(response).await
        })
    }

    fn add_domain<'a>(
        &'a self,
        state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let base = management_base(state)?;
            let url = management_path(&base, &["api", "domain", domain]);
            let response = management_http::client()
                .post(&url)
                .json(&serde_json::json!({ "domain": domain }))
                .send()
                .await
                .map_err(|err| ManagementError::Upstream(err.to_string()))?;
            if response.status().is_success() || response.status() == StatusCode::CONFLICT {
                Ok(())
            } else {
                Err(ManagementError::Upstream(format!(
                    "POST /api/domain/{domain} returned HTTP {}",
                    response.status()
                )))
            }
        })
    }

    fn delete_domain<'a>(
        &'a self,
        state: &'a AppState,
        domain: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ManagementError>> + Send + 'a>> {
        Box::pin(async move {
            let base = management_base(state)?;
            let url = management_path(&base, &["api", "domain", domain]);
            let response = management_http::client()
                .delete(&url)
                .send()
                .await
                .map_err(|err| ManagementError::Upstream(err.to_string()))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(ManagementError::Upstream(format!(
                    "DELETE /api/domain/{domain} returned HTTP {}",
                    response.status()
                )))
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManagementError {
    #[error("stalwart.management_url is not configured")]
    NotConfigured,
    #[error("stalwart management request failed: {0}")]
    Upstream(String),
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

    match management.list_domains(&state).await {
        Ok(mut domains) => {
            domains.sort_unstable();
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

    match management.add_domain(&state, &domain).await {
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

    match management.delete_domain(&state, &domain).await {
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

fn management_path(base: &str, segments: &[&str]) -> String {
    let mut url = base.trim_end_matches('/').to_string();
    for segment in segments {
        url.push('/');
        url.push_str(&url::form_urlencoded::byte_serialize(segment.as_bytes()).collect::<String>());
    }
    url
}

async fn decode_domain_list(response: reqwest::Response) -> Result<Vec<String>, ManagementError> {
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| ManagementError::Upstream(err.to_string()))?;

    if let Some(domains) = value.as_array() {
        return domains
            .iter()
            .map(|v| {
                v.as_str().map(str::to_owned).ok_or_else(|| {
                    ManagementError::Upstream("domain list contained a non-string".to_string())
                })
            })
            .collect();
    }

    if let Some(domains) = value.get("domains").and_then(serde_json::Value::as_array) {
        return domains
            .iter()
            .map(|v| {
                v.as_str().map(str::to_owned).ok_or_else(|| {
                    ManagementError::Upstream("domain list contained a non-string".to_string())
                })
            })
            .collect();
    }

    Err(ManagementError::Upstream(
        "domain list response was not an array or { domains: [...] }".to_string(),
    ))
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
        ManagementError::Upstream(message) => {
            tracing::warn!(error = %message, "stalwart domain management failed");
            error_response(StatusCode::BAD_GATEWAY, "stalwart_management_failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_path_percent_encodes_each_path_segment() {
        let url = management_path(
            "http://stalwart.local/",
            &["api", "domain", "Example+Domain/xn--bcher-kva.example"],
        );
        assert_eq!(
            url,
            "http://stalwart.local/api/domain/Example%2BDomain%2Fxn--bcher-kva.example"
        );
    }
}

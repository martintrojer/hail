//! Runtime capability flags surfaced to the SPA.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use hail_backend::Capabilities;
use hail_core::{MailBackend, MailCacheMode};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::internal;
use crate::state::AppState;

pub const TAG: &str = "capabilities";

const GMAIL_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: true,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: true,
    max_attachment_size: 25 * 1024 * 1024,
    label_path_separator: '/',
};

const JMAP_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: true,
    supports_principals_admin: true,
    supports_send: true,
    native_threading: false,
    max_attachment_size: 50 * 1024 * 1024,
    label_path_separator: '/',
};

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct CapabilitiesResponse {
    pub backend: String,
    pub cache_mode: String,
    pub supports_initial_import: bool,
    pub supports_principals_admin: bool,
    pub supports_bulk_archive: bool,
    pub supports_eventsource: bool,
    pub label_path_separator: String,
    pub accounts: Vec<CapabilityAccount>,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct CapabilityAccount {
    pub id: i64,
    pub email: String,
    pub backend: String,
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_capabilities))
}

#[utoipa::path(
    get,
    path = "/api/capabilities",
    tag = TAG,
    responses(
        (status = 200, description = "Runtime mail backend and cache capabilities for SPA feature gates.", body = CapabilitiesResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Capability lookup failed."),
    ),
)]
async fn get_capabilities(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<AuthUser>,
) -> Response {
    match build_capabilities(&state, &user).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "capabilities lookup failed");
            internal()
        }
    }
}

async fn build_capabilities(
    state: &AppState,
    user: &AuthUser,
) -> Result<CapabilitiesResponse, sqlx::Error> {
    let backend = state.config.mail.backend;
    let capabilities = capabilities_for_backend(backend);
    let accounts = accounts_for_user(state, user, backend).await?;

    Ok(CapabilitiesResponse {
        backend: backend_name(backend).to_owned(),
        cache_mode: cache_mode_name(state.config.mail.cache.mode).to_owned(),
        supports_initial_import: capabilities.supports_initial_import,
        supports_principals_admin: capabilities.supports_principals_admin,
        supports_bulk_archive: true,
        supports_eventsource: capabilities.supports_eventsource,
        label_path_separator: capabilities.label_path_separator.to_string(),
        accounts,
    })
}

fn capabilities_for_backend(backend: MailBackend) -> &'static Capabilities {
    match backend {
        MailBackend::Gmail => &GMAIL_CAPABILITIES,
        MailBackend::Jmap => &JMAP_CAPABILITIES,
    }
}

async fn accounts_for_user(
    state: &AppState,
    user: &AuthUser,
    backend: MailBackend,
) -> Result<Vec<CapabilityAccount>, sqlx::Error> {
    let mut accounts = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT id, COALESCE(display_email, provider_email), backend_kind \
         FROM mail_accounts \
         WHERE user_id = ?1 AND sync_status != 'disconnected' \
         ORDER BY id",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(id, email, backend)| CapabilityAccount { id, email, backend })
    .collect::<Vec<_>>();

    if accounts.is_empty() && backend == MailBackend::Jmap {
        accounts.push(CapabilityAccount {
            id: user.id,
            email: user.email.clone(),
            backend: backend_name(backend).to_owned(),
        });
    }

    Ok(accounts)
}

fn backend_name(backend: MailBackend) -> &'static str {
    match backend {
        MailBackend::Gmail => "gmail",
        MailBackend::Jmap => "jmap",
    }
}

fn cache_mode_name(mode: MailCacheMode) -> &'static str {
    match mode {
        MailCacheMode::Off => "off",
        MailCacheMode::Bounded => "bounded",
        MailCacheMode::Full => "full",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_serialized_as_config_values() {
        assert_eq!(backend_name(MailBackend::Gmail), "gmail");
        assert_eq!(backend_name(MailBackend::Jmap), "jmap");
        assert_eq!(cache_mode_name(MailCacheMode::Off), "off");
        assert_eq!(cache_mode_name(MailCacheMode::Bounded), "bounded");
        assert_eq!(cache_mode_name(MailCacheMode::Full), "full");
    }
}

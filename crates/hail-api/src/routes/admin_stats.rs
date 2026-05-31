//! Admin mailbox statistics and Stalwart health endpoints.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use secrecy::SecretString;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::jmap_session;
use crate::routes::response::{error_response, internal};
use crate::state::AppState;

pub const TAG: &str = "admin";

pub trait StalwartStatsProvider: Send + Sync + 'static {
    fn stats<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<AdminStatsResponse, StatsError>> + Send + 'a>>;
}

pub struct HttpStalwartStatsProvider;

impl StalwartStatsProvider for HttpStalwartStatsProvider {
    fn stats<'a>(
        &'a self,
        state: &'a AppState,
        bearer: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<AdminStatsResponse, StatsError>> + Send + 'a>> {
        Box::pin(async move {
            let status = stalwart_status(state, &bearer).await;
            let users = load_stats_users(state).await?;
            Ok(AdminStatsResponse {
                users,
                stalwart_status: status,
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StatsError {
    #[error("database query failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminStatsResponse {
    pub users: Vec<AdminUserStats>,
    pub stalwart_status: StalwartStatus,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminUserStats {
    pub email: String,
    pub total_emails: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_size_bytes: Option<u64>,
    pub mailbox_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StalwartStatus {
    Connected,
    Unreachable,
}

pub fn router() -> OpenApiRouter<AppState> {
    router_with_provider(Arc::new(HttpStalwartStatsProvider))
}

pub fn router_with_provider<P>(provider: Arc<P>) -> OpenApiRouter<AppState>
where
    P: StalwartStatsProvider,
{
    let provider: Arc<dyn StalwartStatsProvider> = provider;
    OpenApiRouter::new().routes(routes!(get_admin_stats).layer(Extension(provider)))
}

#[utoipa::path(
    get,
    path = "/api/admin/stats",
    tag = TAG,
    responses(
        (status = 200, description = "Admin mailbox statistics and Stalwart health status.", body = AdminStatsResponse),
        (status = 403, description = "Authenticated user is not an administrator."),
        (status = 500, description = "Failed to load local user or session state.")
    )
)]
async fn get_admin_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn StalwartStatsProvider>>,
) -> Response {
    if !user.is_admin {
        return forbidden_admin();
    }

    match provider.stats(&state, user.jmap_token.clone()).await {
        Ok(stats) => Json(stats).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "admin stats: load failed");
            internal()
        }
    }
}

#[derive(Debug)]
struct StatsUserRow {
    email: String,
    jmap_token_enc: Option<Vec<u8>>,
}

async fn load_stats_users(state: &AppState) -> Result<Vec<AdminUserStats>, StatsError> {
    let now = Utc::now();
    let rows = sqlx::query_as::<_, (String, Option<Vec<u8>>)>(
        "SELECT u.email, s.jmap_token_enc \
         FROM users u \
         LEFT JOIN sessions s ON s.id = ( \
             SELECT id FROM sessions \
             WHERE user_id = u.id AND expires_at > ?1 \
             ORDER BY last_used_at DESC LIMIT 1 \
         ) \
         ORDER BY u.email",
    )
    .bind(now)
    .fetch_all(&state.db)
    .await?;

    let mut users = Vec::with_capacity(rows.len());
    for (email, jmap_token_enc) in rows {
        let row = StatsUserRow {
            email,
            jmap_token_enc,
        };
        let token = match row.jmap_token_enc.as_deref() {
            Some(token_enc) => match jmap_token(state, token_enc) {
                Ok(token) => Some(token),
                Err(err) => {
                    tracing::warn!(email = %row.email, error = %err, "admin stats: JMAP token decrypt failed");
                    None
                }
            },
            None => None,
        };

        let (total_emails, mailbox_count) = match token.clone() {
            Some(token) => match jmap_mailbox_counts(state, token).await {
                Ok(counts) => counts,
                Err(err) => {
                    tracing::warn!(email = %row.email, error = %err, "admin stats: JMAP mailbox count failed");
                    (0, 0)
                }
            },
            None => (0, 0),
        };

        let total_size_bytes = match token {
            Some(token) => match jmap_quota_size(state, token, &row.email).await {
                Ok(size) => size,
                Err(err) => {
                    tracing::debug!(email = %row.email, error = %err, "admin stats: quota size unavailable");
                    None
                }
            },
            None => None,
        };

        users.push(AdminUserStats {
            email: row.email,
            total_emails,
            total_size_bytes,
            mailbox_count,
        });
    }

    Ok(users)
}

fn jmap_token(
    state: &AppState,
    token_enc: &[u8],
) -> Result<SecretString, Box<dyn std::error::Error + Send + Sync>> {
    let token_bytes = hail_core::open(token_enc, &state.server_key)?;
    Ok(SecretString::from(String::from_utf8(token_bytes)?))
}

async fn jmap_mailbox_counts(
    state: &AppState,
    token: SecretString,
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let session = jmap_session(state, token).await?;
    let mut request = session.client().build();
    request.get_mailbox().properties([
        hail_jmap::jmap_client::mailbox::Property::Id,
        hail_jmap::jmap_client::mailbox::Property::TotalEmails,
    ]);
    let mut response = request.send_get_mailbox().await?;
    let mailboxes = response.take_list();
    let mailbox_count = mailboxes.len() as u64;
    let total_emails = mailboxes
        .iter()
        .map(|mailbox| mailbox.total_emails() as u64)
        .sum();
    Ok((total_emails, mailbox_count))
}

async fn stalwart_status(state: &AppState, bearer: &SecretString) -> StalwartStatus {
    match state.config.stalwart.management_url.as_deref() {
        Some(base) => {
            match hail_jmap::management::ManagementSession::connect(base, bearer.clone()).await {
                Ok(_) => StalwartStatus::Connected,
                Err(err) => {
                    tracing::debug!(error = %err, "admin stats: JMAP management status check failed");
                    StalwartStatus::Unreachable
                }
            }
        }
        None => StalwartStatus::Unreachable,
    }
}

async fn jmap_quota_size(
    state: &AppState,
    token: SecretString,
    email: &str,
) -> Result<Option<u64>, Box<dyn std::error::Error + Send + Sync>> {
    let session = jmap_session(state, token.clone()).await?;
    match hail_jmap::management::quota_used_bytes(
        &state.config.stalwart.jmap_url,
        &token,
        session.account_id(),
    )
    .await
    {
        Ok(size) => Ok(size),
        Err(err) => {
            tracing::debug!(email, error = %err, "admin stats: Quota/get failed; leaving size unknown");
            Ok(None)
        }
    }
}

fn forbidden_admin() -> Response {
    error_response(StatusCode::FORBIDDEN, "admin_required")
}

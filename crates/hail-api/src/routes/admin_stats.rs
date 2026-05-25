//! Admin mailbox statistics and Stalwart health endpoints.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use secrecy::SecretString;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::jmap_session;
use crate::routes::response::internal;
use crate::state::AppState;

pub const TAG: &str = "admin";

pub trait StalwartStatsProvider: Send + Sync + 'static {
    fn stats<'a>(
        &'a self,
        state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<AdminStatsResponse, StatsError>> + Send + 'a>>;
}

pub struct HttpStalwartStatsProvider;

impl StalwartStatsProvider for HttpStalwartStatsProvider {
    fn stats<'a>(
        &'a self,
        state: &'a AppState,
    ) -> Pin<Box<dyn Future<Output = Result<AdminStatsResponse, StatsError>> + Send + 'a>> {
        Box::pin(async move {
            let status = stalwart_status(state).await;
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

    match provider.stats(&state).await {
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
        let (total_emails, mailbox_count) = match row.jmap_token_enc.as_deref() {
            Some(token_enc) => match jmap_mailbox_counts(state, token_enc).await {
                Ok(counts) => counts,
                Err(err) => {
                    tracing::warn!(email = %row.email, error = %err, "admin stats: JMAP mailbox count failed");
                    (0, 0)
                }
            },
            None => (0, 0),
        };

        let total_size_bytes = match stalwart_quota_size(state, &row.email).await {
            Ok(size) => size,
            Err(err) => {
                tracing::debug!(email = %row.email, error = %err, "admin stats: quota size unavailable");
                None
            }
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

async fn jmap_mailbox_counts(
    state: &AppState,
    token_enc: &[u8],
) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let token_bytes = hail_core::open(token_enc, &state.server_key)?;
    let token = SecretString::from(String::from_utf8(token_bytes)?);
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

async fn stalwart_status(state: &AppState) -> StalwartStatus {
    match management_base(state) {
        Some(base) if management_health_connected(&base).await => StalwartStatus::Connected,
        _ => StalwartStatus::Unreachable,
    }
}

async fn management_health_connected(base: &str) -> bool {
    let client = reqwest::Client::new();
    for path in ["/api/healthz", "/healthz", "/healthz/live"] {
        let Ok(response) = client.get(format!("{base}{path}")).send().await else {
            continue;
        };
        if response.status().is_success() || response.status() == StatusCode::NO_CONTENT {
            return true;
        }
    }
    false
}

async fn stalwart_quota_size(
    state: &AppState,
    email: &str,
) -> Result<Option<u64>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(base) = management_base(state) else {
        return Ok(None);
    };
    let response = reqwest::Client::new()
        .get(management_path(&base, &["api", "store", "quota", email]))
        .send()
        .await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let value = response.json::<serde_json::Value>().await?;
    Ok(size_bytes_from_value(&value))
}

fn management_base(state: &AppState) -> Option<String> {
    state
        .config
        .stalwart
        .management_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| url.trim_end_matches('/').to_string())
}

fn management_path(base: &str, segments: &[&str]) -> String {
    let mut url = base.trim_end_matches('/').to_string();
    for segment in segments {
        url.push('/');
        url.push_str(
            &url::form_urlencoded::byte_serialize(segment.as_bytes()).collect::<String>(),
        );
    }
    url
}

fn size_bytes_from_value(value: &serde_json::Value) -> Option<u64> {
    value
        .get("total_size_bytes")
        .or_else(|| value.get("totalSizeBytes"))
        .or_else(|| value.get("used_bytes"))
        .or_else(|| value.get("usedBytes"))
        .or_else(|| value.get("size"))
        .or_else(|| value.get("used"))
        .or_else(|| value.get("quota").and_then(|quota| quota.get("used")))
        .and_then(serde_json::Value::as_u64)
}

fn forbidden_admin() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"admin_required"}"#,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_size_bytes_from_common_quota_shapes() {
        assert_eq!(
            size_bytes_from_value(&serde_json::json!({ "totalSizeBytes": 42 })),
            Some(42)
        );
        assert_eq!(
            size_bytes_from_value(&serde_json::json!({ "quota": { "used": 99 } })),
            Some(99)
        );
        assert_eq!(
            size_bytes_from_value(&serde_json::json!({ "quota": {} })),
            None
        );
    }

    #[test]
    fn management_path_percent_encodes_email_path_segment() {
        let url = management_path(
            "http://stalwart.local/",
            &["api", "store", "quota", "User+tag@example.org"],
        );
        assert_eq!(
            url,
            "http://stalwart.local/api/store/quota/User%2Btag%40example.org"
        );
    }
}

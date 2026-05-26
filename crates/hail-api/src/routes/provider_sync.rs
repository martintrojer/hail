//! Provider import sync status endpoints.

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{error_response, internal};
use crate::state::AppState;

pub const TAG: &str = "provider-sync";

pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_provider_sync_statuses))
        .routes(routes!(trigger_provider_sync))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProviderSyncStatusListResponse {
    pub accounts: Vec<ProviderSyncStatusResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProviderSyncTriggerResponse {
    pub account: ProviderSyncStatusResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProviderSyncStatusResponse {
    pub id: i64,
    pub provider_kind: String,
    pub provider_account_id: String,
    pub provider_email: String,
    pub display_email: Option<String>,
    pub sync_status: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_sync_attempted_at: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_sync_succeeded_at: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub next_sync_after: Option<DateTime<Utc>>,
    pub sync_backoff_secs: Option<i64>,
    pub last_error_class: Option<String>,
    pub last_error_message: Option<String>,
    pub last_profile_history_id: Option<String>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub profile_synced_at: Option<DateTime<Utc>>,
    pub last_sync_event: Option<ProviderSyncEventSummary>,
    pub last_error_event: Option<ProviderSyncEventSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProviderSyncEventSummary {
    pub event_type: String,
    pub result_status: String,
    pub safe_error_class: Option<String>,
    pub safe_error_message: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTime<Utc>,
}

#[utoipa::path(get, path = "/api/provider-accounts/sync-status", tag = TAG,
    responses((status = 200, description = "Connected Gmail provider account sync statuses.", body = ProviderSyncStatusListResponse)))]
async fn list_provider_sync_statuses(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match list_statuses(&state.db, user.id).await {
        Ok(accounts) => Json(ProviderSyncStatusListResponse { accounts }).into_response(),
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, "provider sync status list failed");
            internal()
        }
    }
}

#[utoipa::path(post, path = "/api/provider-accounts/{id}/sync", tag = TAG,
    params(("id" = i64, Path)),
    responses((status = 200, description = "Provider account marked due for safe background sync.", body = ProviderSyncTriggerResponse)))]
async fn trigger_provider_sync(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match mark_provider_account_due(&state.db, user.id, id).await {
        Ok(Some(account)) => Json(ProviderSyncTriggerResponse { account }).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "provider_account_not_found"),
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, provider_account_id = id, "provider sync trigger failed");
            internal()
        }
    }
}

async fn list_statuses(
    db: &SqlitePool,
    user_id: i64,
) -> Result<Vec<ProviderSyncStatusResponse>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, provider_kind, provider_account_id, provider_email, display_email, \
                sync_status, last_sync_attempted_at, last_sync_succeeded_at, next_sync_after, \
                sync_backoff_secs, last_error_class, last_error_message, last_profile_history_id, \
                profile_synced_at \
         FROM provider_accounts \
         WHERE user_id = ?1 AND provider_kind = 'gmail' AND sync_status != 'disconnected' \
         ORDER BY provider_email COLLATE NOCASE, id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;

    let mut accounts = Vec::with_capacity(rows.len());
    for row in rows {
        accounts.push(row_to_status(db, user_id, row).await?);
    }
    Ok(accounts)
}

async fn mark_provider_account_due(
    db: &SqlitePool,
    user_id: i64,
    provider_account_id: i64,
) -> Result<Option<ProviderSyncStatusResponse>, sqlx::Error> {
    let mut tx = db.begin().await?;
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM provider_accounts \
         WHERE id = ?1 AND user_id = ?2 AND provider_kind = 'gmail' AND sync_status != 'disconnected'",
    )
    .bind(provider_account_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    if found.is_none() {
        tx.commit().await?;
        return Ok(None);
    }

    let now = Utc::now();
    sqlx::query(
        "UPDATE provider_accounts \
         SET next_sync_after = NULL, sync_backoff_secs = NULL, updated_at = ?1 \
         WHERE id = ?2 AND user_id = ?3",
    )
    .bind(now)
    .bind(provider_account_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    load_status(db, user_id, provider_account_id)
        .await
        .map(Some)
}

async fn load_status(
    db: &SqlitePool,
    user_id: i64,
    provider_account_id: i64,
) -> Result<ProviderSyncStatusResponse, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, provider_kind, provider_account_id, provider_email, display_email, \
                sync_status, last_sync_attempted_at, last_sync_succeeded_at, next_sync_after, \
                sync_backoff_secs, last_error_class, last_error_message, last_profile_history_id, \
                profile_synced_at \
         FROM provider_accounts \
         WHERE id = ?1 AND user_id = ?2 AND provider_kind = 'gmail'",
    )
    .bind(provider_account_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    row_to_status(db, user_id, row).await
}

async fn row_to_status(
    db: &SqlitePool,
    user_id: i64,
    row: sqlx::sqlite::SqliteRow,
) -> Result<ProviderSyncStatusResponse, sqlx::Error> {
    let id: i64 = row.get("id");
    Ok(ProviderSyncStatusResponse {
        id,
        provider_kind: row.get("provider_kind"),
        provider_account_id: row.get("provider_account_id"),
        provider_email: row.get("provider_email"),
        display_email: row.get("display_email"),
        sync_status: row.get("sync_status"),
        last_sync_attempted_at: row.get("last_sync_attempted_at"),
        last_sync_succeeded_at: row.get("last_sync_succeeded_at"),
        next_sync_after: row.get("next_sync_after"),
        sync_backoff_secs: row.get("sync_backoff_secs"),
        last_error_class: row.get("last_error_class"),
        last_error_message: row.get("last_error_message"),
        last_profile_history_id: row.get("last_profile_history_id"),
        profile_synced_at: row.get("profile_synced_at"),
        last_sync_event: load_event_summary(db, user_id, id, None).await?,
        last_error_event: load_event_summary(db, user_id, id, Some("failed")).await?,
    })
}

async fn load_event_summary(
    db: &SqlitePool,
    user_id: i64,
    provider_account_id: i64,
    result_status: Option<&str>,
) -> Result<Option<ProviderSyncEventSummary>, sqlx::Error> {
    let row = if let Some(result_status) = result_status {
        sqlx::query(
            "SELECT event_type, result_status, safe_error_class, safe_error_message, created_at \
             FROM provider_sync_events \
             WHERE user_id = ?1 AND provider_account_id = ?2 \
               AND operation_kind IN ('sync', 'retry', 'failure') AND result_status = ?3 \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(provider_account_id)
        .bind(result_status)
        .fetch_optional(db)
        .await?
    } else {
        sqlx::query(
            "SELECT event_type, result_status, safe_error_class, safe_error_message, created_at \
             FROM provider_sync_events \
             WHERE user_id = ?1 AND provider_account_id = ?2 \
               AND operation_kind IN ('sync', 'retry', 'failure') \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(provider_account_id)
        .fetch_optional(db)
        .await?
    };

    Ok(row.map(|row| ProviderSyncEventSummary {
        event_type: row.get("event_type"),
        result_status: row.get("result_status"),
        safe_error_class: row.get("safe_error_class"),
        safe_error_message: row.get("safe_error_message"),
        created_at: row.get("created_at"),
    }))
}

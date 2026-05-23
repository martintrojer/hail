//! Short-lived server-side undo actions.
//!
//! Destructive handlers can persist an opaque undo token with enough JSON
//! payload for a later compensating action. `POST /api/undo/:id` is protected
//! by the normal auth + CSRF middleware, consumes the token exactly once, and
//! delegates the actual compensating mutation through [`UndoExecutor`] so tests
//! can fake execution without a live JMAP server.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use chrono::{DateTime, Duration, Utc};
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const UNDO_TTL_SECONDS: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UndoActionPayload {
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UndoToken {
    pub id: String,
    pub action: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct UndoError(pub String);

#[async_trait]
pub trait UndoExecutor: Send + Sync + 'static {
    async fn execute(
        &self,
        state: &AppState,
        user: &AuthUser,
        undo: UndoActionPayload,
    ) -> Result<(), UndoError>;
}

/// Production executor stub. Tokens are persisted now so clients can expose an
/// undo affordance; action-specific compensating JMAP/sidecar work should be
/// implemented alongside each destructive mutation and matched on `undo.action`.
pub struct NoopUndoExecutor;

#[async_trait]
impl UndoExecutor for NoopUndoExecutor {
    async fn execute(
        &self,
        _state: &AppState,
        _user: &AuthUser,
        undo: UndoActionPayload,
    ) -> Result<(), UndoError> {
        tracing::debug!(action = %undo.action, "undo execution is currently a production no-op");
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct UndoResponse {
    id: String,
    action: String,
}

#[derive(Debug, sqlx::FromRow)]
struct UndoActionRow {
    action: String,
    payload_json: String,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
}

pub fn router() -> Router<AppState> {
    router_with_executor(Arc::new(NoopUndoExecutor))
}

pub fn router_with_executor<E>(executor: Arc<E>) -> Router<AppState>
where
    E: UndoExecutor,
{
    Router::new()
        .route("/api/undo/{id}", post(post_undo::<E>))
        .layer(Extension(executor))
}

pub async fn create_undo_action(
    state: &AppState,
    user_id: i64,
    action: &str,
    payload: Value,
) -> Result<UndoToken, sqlx::Error> {
    let now = Utc::now();
    let expires_at = now + Duration::seconds(UNDO_TTL_SECONDS);
    let id = new_undo_id().map_err(sqlx::Error::Protocol)?;
    let payload_json =
        serde_json::to_string(&payload).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;

    sqlx::query(
        "INSERT INTO undo_actions (id, user_id, action, payload_json, expires_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(action)
    .bind(&payload_json)
    .bind(expires_at)
    .bind(now)
    .execute(&state.db)
    .await?;

    Ok(UndoToken {
        id,
        action: action.to_string(),
        expires_at,
    })
}

fn new_undo_id() -> Result<String, String> {
    let mut id_bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut id_bytes)
        .map_err(|err| err.to_string())?;
    Ok(hex::encode(id_bytes))
}

async fn post_undo<E>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(executor): Extension<Arc<E>>,
    Path(id): Path<String>,
) -> Response
where
    E: UndoExecutor,
{
    if !looks_like_undo_id(&id) {
        return not_found();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "undo transaction begin failed");
            return internal();
        }
    };

    let row = match sqlx::query_as::<_, UndoActionRow>(
        "SELECT action, payload_json, expires_at, used_at \
         FROM undo_actions \
         WHERE id = ?1 AND user_id = ?2",
    )
    .bind(&id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(err) => {
            tracing::error!(user_id = user.id, undo_id = %id, error = %err, "undo lookup failed");
            return internal();
        }
    };

    let now = Utc::now();
    if row.expires_at <= now {
        return gone("undo_expired");
    }
    if row.used_at.is_some() {
        return gone("undo_used");
    }

    let payload = match serde_json::from_str::<Value>(&row.payload_json) {
        Ok(payload) => payload,
        Err(err) => {
            tracing::error!(user_id = user.id, undo_id = %id, error = %err, "undo payload decode failed");
            return internal();
        }
    };

    let update = match sqlx::query(
        "UPDATE undo_actions SET used_at = ?1 \
         WHERE id = ?2 AND user_id = ?3 AND used_at IS NULL AND expires_at > ?1",
    )
    .bind(now)
    .bind(&id)
    .bind(user.id)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result,
        Err(err) => {
            tracing::error!(user_id = user.id, undo_id = %id, error = %err, "undo consume failed");
            return internal();
        }
    };

    if update.rows_affected() != 1 {
        return gone("undo_unavailable");
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(user_id = user.id, undo_id = %id, error = %err, "undo consume commit failed");
        return internal();
    }

    let action = row.action;
    if let Err(err) = executor
        .execute(
            &state,
            &user,
            UndoActionPayload {
                action: action.clone(),
                payload,
            },
        )
        .await
    {
        tracing::error!(user_id = user.id, undo_id = %id, action = %action, error = %err.0, "undo executor failed after token consume");
        return internal();
    }

    Json(UndoResponse { id, action }).into_response()
}

fn looks_like_undo_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"not_found"}"#,
    )
        .into_response()
}

fn gone(error: &'static str) -> Response {
    (
        StatusCode::GONE,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
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

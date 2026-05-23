//! Pile view endpoints.
//!
//! `GET /api/views/set-aside` and `GET /api/views/reply-later` are backed
//! by the sidecar `stack_positions` table (design.md §6.2). They return the
//! authenticated user's saved thread ordering only. `preview` is intentionally
//! `null` for now; future JMAP enrichment can hydrate each row with a thread
//! card/excerpt without changing the ordering source of truth.

use axum::extract::{Extension, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

/// OpenAPI tag for saved thread piles.
pub const TAG: &str = "piles";

const STACK_SET_ASIDE: &str = "set_aside";
const STACK_REPLY_LATER: &str = "reply_later";

/// Build protected pile view routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected pile view routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_set_aside))
        .routes(routes!(get_reply_later))
}

#[derive(Debug, Serialize, ToSchema)]
struct PileViewResponse {
    items: Vec<PileItem>,
}

#[derive(Debug, Serialize, ToSchema)]
struct PileItem {
    thread_id: String,
    position: i64,
    #[schema(value_type = String, format = DateTime)]
    added_at: DateTime<Utc>,
    preview: Option<serde_json::Value>,
}

#[utoipa::path(
    get,
    path = "/api/views/set-aside",
    tag = TAG,
    responses(
        (status = 200, description = "Threads in the Set Aside pile.", body = PileViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Pile lookup failed."),
    ),
)]
async fn get_set_aside(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    get_stack(state, user, STACK_SET_ASIDE).await
}

#[utoipa::path(
    get,
    path = "/api/views/reply-later",
    tag = TAG,
    responses(
        (status = 200, description = "Threads in the Reply Later pile.", body = PileViewResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Pile lookup failed."),
    ),
)]
async fn get_reply_later(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    get_stack(state, user, STACK_REPLY_LATER).await
}

async fn get_stack(state: AppState, user: AuthUser, stack: &'static str) -> Response {
    let rows = match sqlx::query_as::<_, (String, i64, DateTime<Utc>)>(
        "SELECT thread_id, position, added_at \
         FROM stack_positions \
         WHERE user_id = ?1 AND stack = ?2 \
         ORDER BY position ASC",
    )
    .bind(user.id)
    .bind(stack)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(
                user_id = user.id,
                stack,
                error = %err,
                "pile view lookup failed"
            );
            return internal();
        }
    };

    let items = rows
        .into_iter()
        .map(|(thread_id, position, added_at)| PileItem {
            thread_id,
            position,
            added_at,
            // TODO(JMAP enrichment): hydrate this with a thread preview/card
            // once view builders can batch-fetch JMAP Thread/Email data.
            preview: None,
        })
        .collect();

    Json(PileViewResponse { items }).into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}

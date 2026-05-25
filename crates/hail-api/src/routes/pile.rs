//! Pile view endpoints.
//!
//! `GET /api/views/set-aside` and `GET /api/views/reply-later` are backed
//! by the sidecar `stack_positions` table (design.md §6.2). They return the
//! authenticated user's saved thread ordering and hydrate each row with a
//! best-effort JMAP thread preview. If Stalwart/JMAP is unavailable, the
//! endpoint still returns the pile ordering with `preview: null`.

use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::hydrate_thread_previews;
use crate::routes::response::internal;
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

    let mut items: Vec<_> = rows
        .into_iter()
        .map(|(thread_id, position, added_at)| PileItem {
            thread_id,
            position,
            added_at,
            preview: None,
        })
        .collect();

    let previews = hydrate_thread_previews(
        &state,
        user.id,
        user.jmap_token.clone(),
        stack,
        items.iter().map(|item| item.thread_id.clone()),
    )
    .await;
    for item in &mut items {
        item.preview = previews.get(&item.thread_id).map(|preview| {
            serde_json::json!({
                "from": preview.from,
                "subject": preview.subject,
                "snippet": preview.preview,
            })
        });
    }

    Json(PileViewResponse { items }).into_response()
}

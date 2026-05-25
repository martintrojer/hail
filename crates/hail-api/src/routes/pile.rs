//! Pile view endpoints.
//!
//! `GET /api/views/set-aside` and `GET /api/views/reply-later` are backed
//! by the sidecar `stack_positions` table (design.md §6.2). They return the
//! authenticated user's saved thread ordering and hydrate each row with a
//! best-effort JMAP thread preview. If Stalwart/JMAP is unavailable, the
//! endpoint still returns the pile ordering with `preview: null`.

use std::collections::HashMap;

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

    let mut items: Vec<_> = rows
        .into_iter()
        .map(|(thread_id, position, added_at)| PileItem {
            thread_id,
            position,
            added_at,
            preview: None,
        })
        .collect();

    hydrate_previews(&state, &user, stack, &mut items).await;

    Json(PileViewResponse { items }).into_response()
}

async fn hydrate_previews(
    state: &AppState,
    user: &AuthUser,
    stack: &'static str,
    items: &mut [PileItem],
) {
    if items.is_empty() {
        return;
    }

    let session = match hail_jmap::login_bearer(
        &state.config.stalwart.jmap_url,
        user.jmap_token.clone(),
    )
    .await
    {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(
                user_id = user.id,
                stack,
                error = %err,
                "pile preview JMAP login failed; returning null previews"
            );
            return;
        }
    };

    let mut previews = HashMap::with_capacity(items.len());
    for thread_id in items.iter().map(|item| item.thread_id.as_str()) {
        match latest_thread_preview(&session, thread_id).await {
            Ok(Some(preview)) => {
                previews.insert(thread_id.to_string(), preview);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    user_id = user.id,
                    stack,
                    thread_id = %thread_id,
                    error = %err,
                    "pile preview JMAP hydration failed; leaving preview null"
                );
            }
        }
    }

    for item in items {
        item.preview = previews.remove(&item.thread_id);
    }
}

async fn latest_thread_preview(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Option<serde_json::Value>, String> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::Property;
    use hail_jmap::jmap_client::email::query as email_query;

    let mut request = session.client().build();
    request
        .query_email()
        .filter(Filter::from(email_query::Filter::in_thread(thread_id)))
        .sort([email_query::Comparator::received_at().descending()])
        .limit(1);
    let mut query = request
        .send_query_email()
        .await
        .map_err(|err| err.to_string())?;
    let Some(email_id) = query.take_ids().into_iter().next() else {
        return Ok(None);
    };

    let mut request = session.client().build();
    request.get_email().ids([email_id]).properties([
        Property::From,
        Property::Subject,
        Property::Preview,
    ]);
    let mut response = request
        .send_get_email()
        .await
        .map_err(|err| err.to_string())?;
    let Some(email) = response.take_list().into_iter().next() else {
        return Ok(None);
    };

    Ok(Some(serde_json::json!({
        "from": format_from(email.from()),
        "subject": email.subject().unwrap_or_default(),
        "snippet": email.preview().unwrap_or_default(),
    })))
}

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(|address| match address.name() {
            Some(name) if !name.is_empty() => format!("{} <{}>", name, address.email()),
            _ => address.email().to_string(),
        })
        .unwrap_or_default()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}

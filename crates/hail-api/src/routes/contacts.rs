//! Contact endpoints.
//!
//! Contact history is intentionally thin for now: notes are backed by the
//! sidecar `contact_notes` table, while `threads` is returned as an empty
//! placeholder until the view-building tasks assemble contact thread history.

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal};
use crate::state::AppState;

/// OpenAPI tag for contact note endpoints.
pub const TAG: &str = "contacts";

const MAX_MARKDOWN_BYTES: usize = 64 * 1024;

/// Build protected contact routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected contact routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_contact))
        .routes(routes!(put_note))
        .routes(routes!(delete_note))
}

#[derive(Debug, Serialize, ToSchema)]
struct ContactResponse {
    address: String,
    note: Option<ContactNote>,
    /// Placeholder for future contact thread history. The view tasks will
    /// populate this with pre-shaped thread summaries; for now clients get
    /// a stable empty array rather than needing a nullable/missing field.
    threads: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ContactNote {
    markdown: String,
    #[schema(value_type = String, format = DateTime)]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct PutNoteRequest {
    markdown: String,
}

#[utoipa::path(
    get,
    path = "/api/contacts/{address}",
    tag = TAG,
    params(
        ("address" = String, Path, description = "Email address to inspect."),
    ),
    responses(
        (status = 200, description = "Contact detail with optional note.", body = ContactResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Contact lookup failed."),
    ),
)]
async fn get_contact(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(address): Path<String>,
) -> Response {
    let address = normalize_address(&address);
    let note = match sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "SELECT markdown, updated_at FROM contact_notes WHERE user_id = ?1 AND address = ?2",
    )
    .bind(user.id)
    .bind(&address)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some((markdown, updated_at))) => Some(ContactNote {
            markdown,
            updated_at,
        }),
        Ok(None) => None,
        Err(err) => {
            tracing::error!(user_id = user.id, address = %address, error = %err, "contact note lookup failed");
            return internal();
        }
    };

    Json(ContactResponse {
        address,
        note,
        threads: Vec::new(),
    })
    .into_response()
}

#[utoipa::path(
    put,
    path = "/api/contacts/{address}/note",
    tag = TAG,
    params(
        ("address" = String, Path, description = "Email address whose note should be saved."),
    ),
    request_body(content = PutNoteRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Contact note saved.", body = ContactNote),
        (status = 400, description = "Invalid contact note payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Contact note save failed."),
    ),
)]
async fn put_note(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(address): Path<String>,
    Json(body): Json<PutNoteRequest>,
) -> Response {
    if body.markdown.len() > MAX_MARKDOWN_BYTES {
        return bad_request("markdown_too_large");
    }

    let address = normalize_address(&address);
    let updated_at = Utc::now();
    let saved = match sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "INSERT INTO contact_notes (user_id, address, markdown, updated_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(user_id, address) DO UPDATE SET \
           markdown = excluded.markdown, updated_at = excluded.updated_at \
         RETURNING markdown, updated_at",
    )
    .bind(user.id)
    .bind(&address)
    .bind(&body.markdown)
    .bind(updated_at)
    .fetch_one(&state.db)
    .await
    {
        Ok((markdown, updated_at)) => ContactNote {
            markdown,
            updated_at,
        },
        Err(err) => {
            tracing::error!(user_id = user.id, address = %address, error = %err, "contact note upsert failed");
            return internal();
        }
    };

    Json(saved).into_response()
}

#[utoipa::path(
    delete,
    path = "/api/contacts/{address}/note",
    tag = TAG,
    params(
        ("address" = String, Path, description = "Email address whose note should be deleted."),
    ),
    responses(
        (status = 204, description = "Contact note deleted."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Contact note delete failed."),
    ),
)]
async fn delete_note(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(address): Path<String>,
) -> Response {
    let address = normalize_address(&address);
    match sqlx::query("DELETE FROM contact_notes WHERE user_id = ?1 AND address = ?2")
        .bind(user.id)
        .bind(&address)
        .execute(&state.db)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, address = %address, error = %err, "contact note delete failed");
            internal()
        }
    }
}

fn normalize_address(address: &str) -> String {
    address.trim().to_lowercase()
}

//! Per-thread note endpoints.
//!
//! Thread notes are hail-sidecar state keyed by the authenticated user plus
//! JMAP thread/email ids. The SPA renders these as inline sticky notes under
//! individual messages.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::threads_view::looks_like_jmap_id;
use crate::state::AppState;

/// OpenAPI tag for thread note endpoints.
pub const TAG: &str = "threads";

const MAX_NOTE_BODY_BYTES: usize = 64 * 1024;

/// Build protected thread note routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected thread note routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_thread_notes))
        .routes(routes!(create_thread_note))
        .routes(routes!(delete_thread_note))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ThreadNoteResponse {
    pub id: i64,
    pub email_id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct ThreadNotesResponse {
    notes: Vec<ThreadNoteResponse>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateThreadNoteRequest {
    email_id: String,
    body: String,
}

#[utoipa::path(
    get,
    path = "/api/threads/{thread_id}/notes",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id whose notes should be listed."),
    ),
    responses(
        (status = 200, description = "Thread notes for this user and thread.", body = ThreadNotesResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Thread note lookup failed."),
    ),
)]
async fn list_thread_notes(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(thread_id): Path<String>,
) -> Response {
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }

    let notes = match load_thread_notes(&state, user.id, &thread_id).await {
        Ok(notes) => notes,
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "thread note lookup failed");
            return internal();
        }
    };

    Json(ThreadNotesResponse { notes }).into_response()
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/notes",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id receiving a note."),
    ),
    request_body(content = CreateThreadNoteRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Thread note created.", body = ThreadNoteResponse),
        (status = 400, description = "Invalid thread note payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Thread note creation failed."),
    ),
)]
async fn create_thread_note(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(thread_id): Path<String>,
    body: Result<Json<CreateThreadNoteRequest>, JsonRejection>,
) -> Response {
    if !looks_like_jmap_id(&thread_id) {
        return bad_request("invalid_thread_id");
    }

    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };
    if !looks_like_jmap_id(&body.email_id) {
        return bad_request("invalid_email_id");
    }
    let trimmed_body = body.body.trim();
    if trimmed_body.is_empty() {
        return bad_request("empty_body");
    }
    if trimmed_body.len() > MAX_NOTE_BODY_BYTES {
        return bad_request("body_too_large");
    }

    let note = match sqlx::query_as::<_, (i64, String, String, String)>(
        "INSERT INTO thread_notes (user_id, thread_id, email_id, body) \
         VALUES (?1, ?2, ?3, ?4) \
         RETURNING id, email_id, body, created_at",
    )
    .bind(user.id)
    .bind(&thread_id)
    .bind(&body.email_id)
    .bind(trimmed_body)
    .fetch_one(&state.db)
    .await
    {
        Ok((id, email_id, body, created_at)) => ThreadNoteResponse {
            id,
            email_id,
            body,
            created_at,
        },
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, error = %err, "thread note insert failed");
            return internal();
        }
    };

    (StatusCode::CREATED, Json(note)).into_response()
}

#[utoipa::path(
    delete,
    path = "/api/threads/{thread_id}/notes/{note_id}",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id containing the note."),
        ("note_id" = i64, Path, description = "Thread note id to delete."),
    ),
    responses(
        (status = 204, description = "Thread note deleted."),
        (status = 400, description = "Invalid thread or note id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Thread note deletion failed."),
    ),
)]
async fn delete_thread_note(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((thread_id, note_id)): Path<(String, i64)>,
) -> Response {
    if !looks_like_jmap_id(&thread_id) || note_id <= 0 {
        return bad_request("invalid_note_id");
    }

    match sqlx::query("DELETE FROM thread_notes WHERE user_id = ?1 AND thread_id = ?2 AND id = ?3")
        .bind(user.id)
        .bind(&thread_id)
        .bind(note_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %thread_id, note_id, error = %err, "thread note delete failed");
            internal()
        }
    }
}

pub async fn load_thread_notes(
    state: &AppState,
    user_id: i64,
    thread_id: &str,
) -> Result<Vec<ThreadNoteResponse>, sqlx::Error> {
    sqlx::query_as::<_, (i64, String, String, String)>(
        "SELECT id, email_id, body, created_at \
         FROM thread_notes \
         WHERE user_id = ?1 AND thread_id = ?2 \
         ORDER BY id ASC",
    )
    .bind(user_id)
    .bind(thread_id)
    .fetch_all(&state.db)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(id, email_id, body, created_at)| ThreadNoteResponse {
                id,
                email_id,
                body,
                created_at,
            })
            .collect()
    })
}

fn bad_request(error: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
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

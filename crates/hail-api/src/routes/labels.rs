//! Local label CRUD endpoints.
//!
//! Labels are local, per-user, thread-level tags stored in the hail sidecar DB.
//! This module intentionally owns only label management; thread assignment
//! endpoints are tracked separately under `labels-api-thread-assignment`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use hail_db::labels::{self, Label, LabelDbError, LabelSource};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

/// OpenAPI tag for label management endpoints.
pub const TAG: &str = "labels";

/// Build protected label routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected label routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_labels, create_label))
        .routes(routes!(rename_label, delete_label))
}

#[derive(Debug, Serialize, ToSchema)]
struct LabelListResponse {
    labels: Vec<LabelResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
struct LabelItemResponse {
    label: LabelResponse,
}

#[derive(Debug, Serialize, ToSchema)]
struct LabelResponse {
    id: i64,
    name: String,
    leaf_name: String,
    path_segments: Vec<String>,
    source: LabelSourceResponse,
    color: Option<String>,
    thread_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum LabelSourceResponse {
    Manual,
    Gmail,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateLabelRequest {
    name: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct RenameLabelRequest {
    name: String,
}

#[utoipa::path(
    get,
    path = "/api/labels",
    tag = TAG,
    responses(
        (status = 200, description = "Labels for the current user.", body = LabelListResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Label lookup failed."),
    ),
)]
async fn list_labels(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match labels::list_labels(&state.db, user.id).await {
        Ok(labels) => Json(LabelListResponse {
            labels: labels.into_iter().map(LabelResponse::from).collect(),
        })
        .into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "label list failed");
            internal()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/labels",
    tag = TAG,
    request_body(content = CreateLabelRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Label created.", body = LabelItemResponse),
        (status = 400, description = "Invalid label payload or duplicate name."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 500, description = "Label create failed."),
    ),
)]
async fn create_label(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    body: Result<Json<CreateLabelRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };

    match labels::create_label(&state.db, user.id, &body.name, body.color.as_deref()).await {
        Ok(label) => (
            StatusCode::CREATED,
            Json(LabelItemResponse {
                label: LabelResponse::from(label),
            }),
        )
            .into_response(),
        Err(err) => label_db_error(err, user.id, "label create failed"),
    }
}

#[utoipa::path(
    patch,
    path = "/api/labels/{id}",
    tag = TAG,
    params(("id" = i64, Path, description = "Label id.")),
    request_body(content = RenameLabelRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Label renamed.", body = LabelItemResponse),
        (status = 400, description = "Invalid label payload or duplicate name."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 404, description = "Label not found."),
        (status = 500, description = "Label rename failed."),
    ),
)]
async fn rename_label(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    body: Result<Json<RenameLabelRequest>, JsonRejection>,
) -> Response {
    if id <= 0 {
        return not_found("label");
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };

    match labels::rename_label(&state.db, user.id, id, &body.name).await {
        Ok(label) => Json(LabelItemResponse {
            label: LabelResponse::from(label),
        })
        .into_response(),
        Err(err) => label_db_error(err, user.id, "label rename failed"),
    }
}

#[utoipa::path(
    delete,
    path = "/api/labels/{id}",
    tag = TAG,
    params(("id" = i64, Path, description = "Label id.")),
    responses(
        (status = 204, description = "Label deleted; thread label assignments cascade."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 404, description = "Label not found."),
        (status = 500, description = "Label delete failed."),
    ),
)]
async fn delete_label(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Response {
    if id <= 0 {
        return not_found("label");
    }

    match labels::delete_label(&state.db, user.id, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found("label"),
        Err(err) => label_db_error(err, user.id, "label delete failed"),
    }
}

impl From<Label> for LabelResponse {
    fn from(label: Label) -> Self {
        Self {
            id: label.id,
            leaf_name: label.leaf_name().to_owned(),
            path_segments: label.path_segments(),
            name: label.name,
            source: LabelSourceResponse::from(label.source),
            color: label.color,
            thread_count: label.thread_count,
        }
    }
}

impl From<LabelSource> for LabelSourceResponse {
    fn from(source: LabelSource) -> Self {
        match source {
            LabelSource::Manual => Self::Manual,
            LabelSource::Gmail => Self::Gmail,
        }
    }
}

fn label_db_error(err: LabelDbError, user_id: i64, message: &'static str) -> Response {
    match err {
        LabelDbError::InvalidName(_) => bad_request("invalid_label_name"),
        LabelDbError::Sqlx(sqlx::Error::RowNotFound) => not_found("label"),
        LabelDbError::Sqlx(err) if is_unique_constraint(&err) => {
            bad_request("duplicate_label_name")
        }
        other => {
            tracing::error!(user_id, error = %other, message);
            internal()
        }
    }
}

fn is_unique_constraint(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };

    db_err.code().as_deref() == Some("2067")
        || db_err
            .message()
            .contains("UNIQUE constraint failed: labels.user_id, labels.normalized_name")
}

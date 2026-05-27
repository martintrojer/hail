//! Local label endpoints.
//!
//! Labels are local, per-user, thread-level tags stored in the hail sidecar DB.
//! This module owns label management plus thread assignment/removal. Assignment
//! is scoped solely by the authenticated hail user and sidecar label ownership;
//! it does not mutate provider labels.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use hail_db::labels::{self, Label, LabelDbError, LabelSource};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{hydrate_thread_previews, validate_thread_id};
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

/// OpenAPI tag for label management endpoints.
pub const TAG: &str = "labels";

const DEFAULT_THREAD_LIMIT: usize = 50;
const MAX_THREAD_LIMIT: usize = 100;

/// Build protected label routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected label routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_labels, create_label))
        .routes(routes!(label_threads))
        .routes(routes!(rename_label, delete_label))
        .routes(routes!(assign_label_to_thread, remove_label_from_thread))
        .routes(routes!(assign_label_name_to_thread))
        .routes(routes!(assign_label_to_threads))
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
struct LabelThreadsResponse {
    label: LabelResponse,
    items: Vec<LabelThreadItem>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct LabelThreadItem {
    thread_id: String,
    from: String,
    subject: String,
    preview: String,
    labels: Vec<LabelResponse>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct LabelThreadsQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

impl LabelThreadsQuery {
    fn normalized_limit(&self) -> usize {
        self.limit
            .unwrap_or(DEFAULT_THREAD_LIMIT)
            .min(MAX_THREAD_LIMIT)
    }

    fn offset(&self) -> Result<usize, ()> {
        match self.cursor.as_deref().map(str::trim) {
            None | Some("") => Ok(0),
            Some(value) => value.parse::<usize>().map_err(|_| ()),
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LabelResponse {
    pub id: i64,
    pub name: String,
    pub leaf_name: String,
    pub path_segments: Vec<String>,
    pub source: LabelSourceResponse,
    pub color: Option<String>,
    pub thread_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LabelSourceResponse {
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

#[derive(Debug, Deserialize, ToSchema)]
struct AssignLabelNameRequest {
    label_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct BatchAssignLabelRequest {
    thread_ids: Vec<String>,
    #[serde(default)]
    label_id: Option<i64>,
    #[serde(default)]
    label_name: Option<String>,
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

#[utoipa::path(
    get,
    path = "/api/labels/{id}/threads",
    tag = TAG,
    params(
        ("id" = i64, Path, description = "Label id."),
        LabelThreadsQuery,
    ),
    responses(
        (status = 200, description = "Threads assigned to this label for the current user.", body = LabelThreadsResponse),
        (status = 400, description = "Invalid cursor."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Label not found."),
        (status = 500, description = "Label thread lookup failed."),
    ),
)]
async fn label_threads(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Query(query): Query<LabelThreadsQuery>,
) -> Response {
    if id <= 0 {
        return not_found("label");
    }
    let offset = match query.offset() {
        Ok(offset) => offset,
        Err(()) => return bad_request("invalid_cursor"),
    };
    let limit = query.normalized_limit();

    let label = match labels::get_label(&state.db, user.id, id).await {
        Ok(label) => label,
        Err(err) => return label_db_error(err, user.id, "label lookup failed"),
    };

    let mut thread_ids = match labels::list_label_thread_ids(
        &state.db,
        user.id,
        id,
        if limit == 0 { 0 } else { (limit + 1) as i64 },
        offset as i64,
    )
    .await
    {
        Ok(thread_ids) => thread_ids,
        Err(err) => return label_db_error(err, user.id, "label thread lookup failed"),
    };
    let has_more = thread_ids.len() > limit;
    if has_more {
        thread_ids.truncate(limit);
    }

    let previews = hydrate_thread_previews(
        &state,
        user.id,
        user.jmap_token.clone(),
        "label_threads",
        thread_ids.clone(),
    )
    .await;

    let labels_by_thread_id =
        match labels::list_labels_for_threads(&state.db, user.id, &thread_ids).await {
            Ok(labels_by_thread_id) => labels_by_thread_id,
            Err(err) => return label_db_error(err, user.id, "label thread label lookup failed"),
        };

    let items = thread_ids
        .iter()
        .map(|thread_id| {
            let preview = previews.get(thread_id);
            LabelThreadItem {
                thread_id: thread_id.clone(),
                from: preview
                    .map(|preview| preview.from.clone())
                    .unwrap_or_default(),
                subject: preview
                    .map(|preview| preview.subject.clone())
                    .unwrap_or_default(),
                preview: preview
                    .map(|preview| preview.preview.clone())
                    .unwrap_or_default(),
                labels: labels_by_thread_id
                    .get(thread_id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(LabelResponse::from)
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    let next_cursor = has_more.then(|| (offset + thread_ids.len()).to_string());

    Json(LabelThreadsResponse {
        label: LabelResponse::from(label),
        items,
        next_cursor,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/labels/{label_id}",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
        ("label_id" = i64, Path, description = "Label id."),
    ),
    responses(
        (status = 200, description = "Label assigned to the thread. Duplicate assignment is idempotent.", body = LabelItemResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 404, description = "Label not found for the current user."),
        (status = 500, description = "Label assignment failed."),
    ),
)]
async fn assign_label_to_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((thread_id, label_id)): Path<(String, i64)>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }
    if label_id <= 0 {
        return not_found("label");
    }

    let label = match labels::get_label(&state.db, user.id, label_id).await {
        Ok(label) => label,
        Err(err) => return label_db_error(err, user.id, "label assignment lookup failed"),
    };

    match labels::assign_label_to_thread(&state.db, user.id, &thread_id, label.id).await {
        Ok(_) => Json(LabelItemResponse {
            label: LabelResponse::from(label),
        })
        .into_response(),
        Err(err) => label_db_error(err, user.id, "label assignment failed"),
    }
}

#[utoipa::path(
    delete,
    path = "/api/threads/{thread_id}/labels/{label_id}",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id."),
        ("label_id" = i64, Path, description = "Label id."),
    ),
    responses(
        (status = 204, description = "Label assignment removed. Removing a non-assigned current-user label is idempotent."),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 404, description = "Label not found for the current user."),
        (status = 500, description = "Label assignment removal failed."),
    ),
)]
async fn remove_label_from_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((thread_id, label_id)): Path<(String, i64)>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }
    if label_id <= 0 {
        return not_found("label");
    }

    if let Err(err) = labels::get_label(&state.db, user.id, label_id).await {
        return label_db_error(err, user.id, "label removal lookup failed");
    }

    match labels::remove_label_from_thread(&state.db, user.id, &thread_id, label_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => label_db_error(err, user.id, "label assignment removal failed"),
    }
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/labels",
    tag = TAG,
    params(("thread_id" = String, Path, description = "JMAP thread id.")),
    request_body(content = AssignLabelNameRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Existing normalized label reused or a manual label created, then assigned idempotently.", body = LabelItemResponse),
        (status = 400, description = "Invalid thread id, JSON, or label name."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 500, description = "Inline label assignment failed."),
    ),
)]
async fn assign_label_name_to_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(thread_id): Path<String>,
    body: Result<Json<AssignLabelNameRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };

    match labels::assign_label_name_to_thread(&state.db, user.id, &thread_id, &body.label_name)
        .await
    {
        Ok(label) => Json(LabelItemResponse {
            label: LabelResponse::from(label),
        })
        .into_response(),
        Err(err) => label_db_error(err, user.id, "inline label assignment failed"),
    }
}

#[utoipa::path(
    post,
    path = "/api/threads/labels",
    tag = TAG,
    request_body(content = BatchAssignLabelRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Existing normalized label reused or a manual label created, then assigned idempotently to every selected thread.", body = LabelItemResponse),
        (status = 400, description = "Invalid JSON, payload shape, thread id, or label name."),
        (status = 401, description = "Missing or invalid session."),
        (status = 403, description = "Missing CSRF header."),
        (status = 404, description = "Label id not found for the current user."),
        (status = 500, description = "Batch label assignment failed."),
    ),
)]
async fn assign_label_to_threads(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    body: Result<Json<BatchAssignLabelRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };
    if body.thread_ids.is_empty() {
        return bad_request("empty_thread_ids");
    }
    for thread_id in &body.thread_ids {
        if let Err(response) = validate_thread_id(thread_id) {
            return response;
        }
    }

    let has_label_id = body.label_id.is_some();
    let has_label_name = body.label_name.is_some();
    if has_label_id == has_label_name {
        return bad_request("exactly_one_label_selector_required");
    }
    let thread_ids = body
        .thread_ids
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();

    if let Some(label_id) = body.label_id {
        if label_id <= 0 {
            return not_found("label");
        }
        let label = match labels::get_label(&state.db, user.id, label_id).await {
            Ok(label) => label,
            Err(err) => return label_db_error(err, user.id, "batch label lookup failed"),
        };
        match labels::assign_label_to_threads(&state.db, user.id, &thread_ids, label.id).await {
            Ok(_) => Json(LabelItemResponse {
                label: LabelResponse::from(label),
            })
            .into_response(),
            Err(err) => label_db_error(err, user.id, "batch label assignment failed"),
        }
    } else {
        let label_name = body
            .label_name
            .expect("exactly one selector validation guarantees label_name");
        match labels::assign_label_name_to_threads(&state.db, user.id, &thread_ids, &label_name)
            .await
        {
            Ok(label) => Json(LabelItemResponse {
                label: LabelResponse::from(label),
            })
            .into_response(),
            Err(err) => label_db_error(err, user.id, "batch inline label assignment failed"),
        }
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
        LabelDbError::InvalidThreadId(_) => bad_request("invalid_thread_id"),
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

//! Workflow/mail rule CRUD endpoints.
//!
//! These routes provide the small persisted foundation for HEY-style
//! Workflows. The evaluator is intentionally not here: API clients can list,
//! create, update, and delete the current user's rules while worker routing
//! work can later consume the same sidecar table.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use hail_core::MailClassification;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

/// OpenAPI tag for workflow/mail-rule endpoints.
pub const TAG: &str = "workflows";

const MAX_RULE_NAME_BYTES: usize = 120;
const MAX_CONDITION_VALUE_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 120;
const MAX_AUTO_REPLY_BYTES: usize = 64 * 1024;
const MAX_CONDITIONS: usize = 10;

/// Build protected workflow routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router())
}

/// Build the OpenAPI-tracked router for protected workflow routes.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_workflows, create_workflow))
        .routes(routes!(get_workflow, update_workflow, delete_workflow))
}

#[derive(Debug, Serialize, ToSchema)]
struct WorkflowRuleListResponse {
    rules: Vec<WorkflowRule>,
}

#[derive(Debug, Serialize, ToSchema)]
struct WorkflowRuleResponse {
    rule: WorkflowRule,
}

#[derive(Debug, Serialize, ToSchema)]
struct WorkflowRule {
    id: i64,
    name: String,
    enabled: bool,
    conditions: Vec<WorkflowCondition>,
    action: WorkflowAction,
    #[schema(value_type = String, format = DateTime)]
    created_at: DateTime<Utc>,
    #[schema(value_type = String, format = DateTime)]
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct WorkflowCondition {
    field: WorkflowConditionField,
    op: WorkflowConditionOp,
    value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum WorkflowConditionField {
    From,
    To,
    Cc,
    Subject,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum WorkflowConditionOp {
    Contains,
    Equals,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct WorkflowAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    classify_as: Option<MailClassification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    add_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_reply: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct WorkflowRulePayload {
    name: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    conditions: Vec<WorkflowCondition>,
    action: WorkflowAction,
}

#[derive(sqlx::FromRow)]
struct WorkflowRuleRow {
    id: i64,
    name: String,
    enabled: i64,
    conditions_json: String,
    action_json: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[utoipa::path(
    get,
    path = "/api/workflows",
    tag = TAG,
    responses(
        (status = 200, description = "Workflow rules for the current user.", body = WorkflowRuleListResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Workflow rule lookup failed."),
    ),
)]
async fn list_workflows(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    let rows = match sqlx::query_as::<_, WorkflowRuleRow>(
        "SELECT id, name, enabled, conditions_json, action_json, created_at, updated_at \
         FROM workflow_rules WHERE user_id = ?1 ORDER BY created_at DESC, id DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "workflow rule list failed");
            return internal();
        }
    };

    let rules = match rows.into_iter().map(WorkflowRule::try_from).collect() {
        Ok(rules) => rules,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "stored workflow rule is invalid");
            return internal();
        }
    };

    Json(WorkflowRuleListResponse { rules }).into_response()
}

#[utoipa::path(
    post,
    path = "/api/workflows",
    tag = TAG,
    request_body(content = WorkflowRulePayload, content_type = "application/json"),
    responses(
        (status = 201, description = "Workflow rule created.", body = WorkflowRuleResponse),
        (status = 400, description = "Invalid workflow rule payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Workflow rule create failed."),
    ),
)]
async fn create_workflow(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    body: Result<Json<WorkflowRulePayload>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };
    let rule = match ValidatedWorkflowRule::from_payload(body) {
        Ok(rule) => rule,
        Err(error) => return bad_request(error),
    };

    let now = Utc::now();
    let row = match sqlx::query_as::<_, WorkflowRuleRow>(
        "INSERT INTO workflow_rules \
         (user_id, name, enabled, conditions_json, action_json, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
         RETURNING id, name, enabled, conditions_json, action_json, created_at, updated_at",
    )
    .bind(user.id)
    .bind(&rule.name)
    .bind(enabled_i64(rule.enabled))
    .bind(&rule.conditions_json)
    .bind(&rule.action_json)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(user_id = user.id, error = %err, "workflow rule create failed");
            return internal();
        }
    };

    respond_with_rule(row, StatusCode::CREATED)
}

#[utoipa::path(
    get,
    path = "/api/workflows/{id}",
    tag = TAG,
    params(("id" = i64, Path, description = "Workflow rule id.")),
    responses(
        (status = 200, description = "Workflow rule detail.", body = WorkflowRuleResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Workflow rule not found."),
        (status = 500, description = "Workflow rule lookup failed."),
    ),
)]
async fn get_workflow(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Response {
    if id <= 0 {
        return not_found("workflow_rule");
    }

    match fetch_workflow_rule(&state, user.id, id).await {
        Ok(Some(row)) => respond_with_rule(row, StatusCode::OK),
        Ok(None) => not_found("workflow_rule"),
        Err(err) => {
            tracing::error!(user_id = user.id, id, error = %err, "workflow rule lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    put,
    path = "/api/workflows/{id}",
    tag = TAG,
    params(("id" = i64, Path, description = "Workflow rule id.")),
    request_body(content = WorkflowRulePayload, content_type = "application/json"),
    responses(
        (status = 200, description = "Workflow rule updated.", body = WorkflowRuleResponse),
        (status = 400, description = "Invalid workflow rule payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Workflow rule not found."),
        (status = 500, description = "Workflow rule update failed."),
    ),
)]
async fn update_workflow(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    body: Result<Json<WorkflowRulePayload>, JsonRejection>,
) -> Response {
    if id <= 0 {
        return not_found("workflow_rule");
    }
    let Ok(Json(body)) = body else {
        return bad_request("invalid_json");
    };

    let rule = match ValidatedWorkflowRule::from_payload(body) {
        Ok(rule) => rule,
        Err(error) => return bad_request(error),
    };

    let updated_at = Utc::now();
    let row = match sqlx::query_as::<_, WorkflowRuleRow>(
        "UPDATE workflow_rules SET \
           name = ?3, enabled = ?4, conditions_json = ?5, action_json = ?6, updated_at = ?7 \
         WHERE user_id = ?1 AND id = ?2 \
         RETURNING id, name, enabled, conditions_json, action_json, created_at, updated_at",
    )
    .bind(user.id)
    .bind(id)
    .bind(&rule.name)
    .bind(enabled_i64(rule.enabled))
    .bind(&rule.conditions_json)
    .bind(&rule.action_json)
    .bind(updated_at)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("workflow_rule"),
        Err(err) => {
            tracing::error!(user_id = user.id, id, error = %err, "workflow rule update failed");
            return internal();
        }
    };

    respond_with_rule(row, StatusCode::OK)
}

#[utoipa::path(
    delete,
    path = "/api/workflows/{id}",
    tag = TAG,
    params(("id" = i64, Path, description = "Workflow rule id.")),
    responses(
        (status = 204, description = "Workflow rule deleted."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Workflow rule not found."),
        (status = 500, description = "Workflow rule delete failed."),
    ),
)]
async fn delete_workflow(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Response {
    if id <= 0 {
        return not_found("workflow_rule");
    }

    match sqlx::query("DELETE FROM workflow_rules WHERE user_id = ?1 AND id = ?2")
        .bind(user.id)
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => not_found("workflow_rule"),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::error!(user_id = user.id, id, error = %err, "workflow rule delete failed");
            internal()
        }
    }
}

async fn fetch_workflow_rule(
    state: &AppState,
    user_id: i64,
    id: i64,
) -> Result<Option<WorkflowRuleRow>, sqlx::Error> {
    sqlx::query_as::<_, WorkflowRuleRow>(
        "SELECT id, name, enabled, conditions_json, action_json, created_at, updated_at \
         FROM workflow_rules WHERE user_id = ?1 AND id = ?2",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await
}

fn respond_with_rule(row: WorkflowRuleRow, status: StatusCode) -> Response {
    match WorkflowRule::try_from(row) {
        Ok(rule) => (status, Json(WorkflowRuleResponse { rule })).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "stored workflow rule is invalid");
            internal()
        }
    }
}

impl TryFrom<WorkflowRuleRow> for WorkflowRule {
    type Error = serde_json::Error;

    fn try_from(row: WorkflowRuleRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            name: row.name,
            enabled: row.enabled != 0,
            conditions: serde_json::from_str(&row.conditions_json)?,
            action: serde_json::from_str(&row.action_json)?,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

struct ValidatedWorkflowRule {
    name: String,
    enabled: bool,
    conditions_json: String,
    action_json: String,
}

impl ValidatedWorkflowRule {
    fn from_payload(payload: WorkflowRulePayload) -> Result<Self, &'static str> {
        Self::new(
            payload.name,
            payload.enabled,
            payload.conditions,
            payload.action,
        )
    }

    fn new(
        name: String,
        enabled: bool,
        conditions: Vec<WorkflowCondition>,
        action: WorkflowAction,
    ) -> Result<Self, &'static str> {
        let name = validate_name(name)?;
        validate_conditions(&conditions)?;
        validate_action(&action)?;
        let conditions_json = serde_json::to_string(&conditions).map_err(|_| "invalid_rule")?;
        let action_json = serde_json::to_string(&action).map_err(|_| "invalid_rule")?;
        Ok(Self {
            name,
            enabled,
            conditions_json,
            action_json,
        })
    }
}

fn validate_name(name: String) -> Result<String, &'static str> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("empty_name");
    }
    if name.len() > MAX_RULE_NAME_BYTES {
        return Err("name_too_large");
    }
    Ok(name)
}

fn validate_conditions(conditions: &[WorkflowCondition]) -> Result<(), &'static str> {
    if conditions.is_empty() {
        return Err("no_conditions");
    }
    if conditions.len() > MAX_CONDITIONS {
        return Err("too_many_conditions");
    }

    for condition in conditions {
        let value = condition.value.trim();
        if value.is_empty() {
            return Err("empty_condition_value");
        }
        if value.len() > MAX_CONDITION_VALUE_BYTES {
            return Err("condition_value_too_large");
        }
    }

    Ok(())
}

fn validate_action(action: &WorkflowAction) -> Result<(), &'static str> {
    if action.classify_as.is_none() && action.add_label.is_none() && action.auto_reply.is_none() {
        return Err("no_action");
    }

    if let Some(label) = &action.add_label {
        let label = label.trim();
        if label.is_empty() {
            return Err("empty_label");
        }
        if label.len() > MAX_LABEL_BYTES {
            return Err("label_too_large");
        }
    }

    if let Some(auto_reply) = &action.auto_reply {
        let auto_reply = auto_reply.trim();
        if auto_reply.is_empty() {
            return Err("empty_auto_reply");
        }
        if auto_reply.len() > MAX_AUTO_REPLY_BYTES {
            return Err("auto_reply_too_large");
        }
    }

    Ok(())
}

const fn default_enabled() -> bool {
    true
}

const fn enabled_i64(enabled: bool) -> i64 {
    if enabled { 1 } else { 0 }
}

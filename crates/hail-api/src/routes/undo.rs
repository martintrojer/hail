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
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use hail_core::MailClassification;
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{jmap_session, looks_like_jmap_id};
use crate::routes::response::{internal, not_found};
use crate::state::AppState;

const UNDO_TTL_SECONDS: i64 = 10;

/// OpenAPI tag for short-lived undo actions.
pub const TAG: &str = "undo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoActionKind {
    ThreadClassify,
    ThreadArchive,
    ThreadTrash,
    ThreadStack,
    ScreenerDecision,
}

impl UndoActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadClassify => "thread.classify",
            Self::ThreadArchive => "thread.archive",
            Self::ThreadTrash => "thread.trash",
            Self::ThreadStack => "thread.stack",
            Self::ScreenerDecision => "screener.decision",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "thread.classify" => Some(Self::ThreadClassify),
            "thread.archive" => Some(Self::ThreadArchive),
            "thread.trash" => Some(Self::ThreadTrash),
            "thread.stack" => Some(Self::ThreadStack),
            "screener.decision" => Some(Self::ScreenerDecision),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStackUndoTarget {
    SetAside,
    ReplyLater,
}

impl ThreadStackUndoTarget {
    pub const fn stack(self) -> &'static str {
        match self {
            Self::SetAside => "set_aside",
            Self::ReplyLater => "reply_later",
        }
    }

    pub const fn keyword(self) -> &'static str {
        match self {
            Self::SetAside => "$hail_setaside",
            Self::ReplyLater => "$hail_replylater",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUndoAction {
    kind: UndoActionKind,
    payload: Value,
}

impl NewUndoAction {
    pub fn thread_classify(
        thread_id: &str,
        previous_classification: &str,
        new_classification: &str,
    ) -> Self {
        Self {
            kind: UndoActionKind::ThreadClassify,
            payload: serde_json::json!({
                "thread_id": thread_id,
                "previous_classification": previous_classification,
                "new_classification": new_classification,
            }),
        }
    }

    pub fn thread_archive(email_mailbox_ids: Vec<EmailMailboxSnapshot>) -> Self {
        Self {
            kind: UndoActionKind::ThreadArchive,
            payload: serde_json::json!({ "email_mailbox_ids": email_mailbox_ids }),
        }
    }

    pub fn thread_trash(email_mailbox_ids: Vec<EmailMailboxSnapshot>) -> Self {
        Self {
            kind: UndoActionKind::ThreadTrash,
            payload: serde_json::json!({ "email_mailbox_ids": email_mailbox_ids }),
        }
    }

    pub fn thread_stack<P>(
        thread_id: &str,
        target: ThreadStackUndoTarget,
        previous_position: Option<P>,
    ) -> Self
    where
        P: Serialize,
    {
        Self {
            kind: UndoActionKind::ThreadStack,
            payload: serde_json::json!({
                "thread_id": thread_id,
                "stack": target.stack(),
                "keyword": target.keyword(),
                "previous_position": previous_position,
            }),
        }
    }

    pub fn screener_decision<P>(sender: &str, previous_rule: Option<&P>) -> Self
    where
        P: Serialize,
    {
        Self {
            kind: UndoActionKind::ScreenerDecision,
            payload: serde_json::json!({
                "sender": sender,
                "previous_rule": previous_rule,
            }),
        }
    }

    fn kind(&self) -> UndoActionKind {
        self.kind
    }

    fn payload(&self) -> &Value {
        &self.payload
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct UndoActionPayload {
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct UndoToken {
    pub id: String,
    pub action: String,
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct UndoError {
    pub message: String,
    status: UndoErrorStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UndoErrorStatus {
    BadRequest,
    NotImplemented,
    Internal,
}

impl UndoError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: UndoErrorStatus::Internal,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: UndoErrorStatus::BadRequest,
        }
    }

    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: UndoErrorStatus::NotImplemented,
        }
    }
}

#[async_trait]
pub trait UndoExecutor: Send + Sync + 'static {
    async fn execute(
        &self,
        state: &AppState,
        user: &AuthUser,
        undo: UndoActionPayload,
    ) -> Result<(), UndoError>;
}

/// Production executor for action-specific v1 undo payloads.
pub struct ActionUndoExecutor<R = JmapThreadUndoRestorer> {
    thread_restorer: Arc<R>,
}

impl Default for ActionUndoExecutor<JmapThreadUndoRestorer> {
    fn default() -> Self {
        Self::new(Arc::new(JmapThreadUndoRestorer))
    }
}

impl<R> ActionUndoExecutor<R> {
    pub fn new(thread_restorer: Arc<R>) -> Self {
        Self { thread_restorer }
    }
}

pub struct JmapThreadUndoRestorer;

#[async_trait]
pub trait ThreadUndoRestorer: Send + Sync + 'static {
    async fn restore_classification(
        &self,
        state: &AppState,
        user: &AuthUser,
        thread_id: &str,
        previous_classification: &str,
    ) -> Result<(), UndoError>;

    async fn restore_mailboxes(
        &self,
        state: &AppState,
        user: &AuthUser,
        snapshots: Vec<EmailMailboxSnapshot>,
    ) -> Result<(), UndoError>;

    async fn set_keyword(
        &self,
        state: &AppState,
        user: &AuthUser,
        thread_id: &str,
        keyword: &str,
        enabled: bool,
    ) -> Result<(), UndoError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailMailboxSnapshot {
    pub email_id: String,
    pub mailbox_ids: Vec<String>,
}

#[async_trait]
impl ThreadUndoRestorer for JmapThreadUndoRestorer {
    async fn restore_classification(
        &self,
        state: &AppState,
        user: &AuthUser,
        thread_id: &str,
        previous_classification: &str,
    ) -> Result<(), UndoError> {
        let previous = MailClassification::parse(previous_classification)
            .ok_or_else(|| UndoError::bad_request("invalid_previous_classification"))?;
        let session = jmap_session(state, user.jmap_token.clone())
            .await
            .map_err(UndoError::internal)?;
        for email_id in email_ids_in_thread(&session, thread_id).await? {
            for candidate in MailClassification::ALL {
                session
                    .client()
                    .email_set_keyword(&email_id, candidate.keyword(), candidate == previous)
                    .await
                    .map_err(|err| UndoError::internal(err.to_string()))?;
            }
        }
        Ok(())
    }

    async fn restore_mailboxes(
        &self,
        state: &AppState,
        user: &AuthUser,
        snapshots: Vec<EmailMailboxSnapshot>,
    ) -> Result<(), UndoError> {
        let session = jmap_session(state, user.jmap_token.clone())
            .await
            .map_err(UndoError::internal)?;
        for snapshot in snapshots {
            if snapshot.email_id.is_empty() || snapshot.mailbox_ids.is_empty() {
                return Err(UndoError::bad_request("invalid_mailbox_snapshot"));
            }
            session
                .client()
                .email_set_mailboxes(&snapshot.email_id, snapshot.mailbox_ids)
                .await
                .map_err(|err| UndoError::internal(err.to_string()))?;
        }
        Ok(())
    }

    async fn set_keyword(
        &self,
        state: &AppState,
        user: &AuthUser,
        thread_id: &str,
        keyword: &str,
        enabled: bool,
    ) -> Result<(), UndoError> {
        let session = jmap_session(state, user.jmap_token.clone())
            .await
            .map_err(UndoError::internal)?;
        for email_id in email_ids_in_thread(&session, thread_id).await? {
            session
                .client()
                .email_set_keyword(&email_id, keyword, enabled)
                .await
                .map_err(|err| UndoError::internal(err.to_string()))?;
        }
        Ok(())
    }
}

#[async_trait]
impl<R> UndoExecutor for ActionUndoExecutor<R>
where
    R: ThreadUndoRestorer,
{
    async fn execute(
        &self,
        state: &AppState,
        user: &AuthUser,
        undo: UndoActionPayload,
    ) -> Result<(), UndoError> {
        match UndoActionKind::parse(&undo.action) {
            Some(UndoActionKind::ThreadClassify) => {
                let payload: ThreadClassifyUndoPayload = serde_json::from_value(undo.payload)
                    .map_err(|_| UndoError::bad_request("invalid_undo_payload"))?;
                validate_thread_classify_payload(&payload)?;
                self.thread_restorer
                    .restore_classification(
                        state,
                        user,
                        &payload.thread_id,
                        &payload.previous_classification,
                    )
                    .await
            }
            Some(UndoActionKind::ThreadArchive | UndoActionKind::ThreadTrash) => {
                let payload: ThreadMoveUndoPayload = serde_json::from_value(undo.payload)
                    .map_err(|_| UndoError::not_implemented("thread_move_undo_missing_snapshot"))?;
                if payload.email_mailbox_ids.is_empty() {
                    return Err(UndoError::not_implemented(
                        "thread_move_undo_missing_snapshot",
                    ));
                }
                self.thread_restorer
                    .restore_mailboxes(state, user, payload.email_mailbox_ids)
                    .await
            }
            Some(UndoActionKind::ThreadStack) => {
                let payload: ThreadStackUndoPayload = serde_json::from_value(undo.payload)
                    .map_err(|_| UndoError::bad_request("invalid_undo_payload"))?;
                restore_thread_stack(state, user, self.thread_restorer.as_ref(), payload).await
            }
            Some(UndoActionKind::ScreenerDecision) => {
                restore_screener_decision(state, user, undo.payload).await
            }
            None => Err(UndoError::not_implemented("undo_action_not_supported")),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ThreadClassifyUndoPayload {
    thread_id: String,
    previous_classification: String,
    new_classification: String,
}

#[derive(Debug, Deserialize)]
struct ThreadMoveUndoPayload {
    email_mailbox_ids: Vec<EmailMailboxSnapshot>,
}

#[derive(Debug, Deserialize)]
struct ThreadStackUndoPayload {
    thread_id: String,
    stack: String,
    keyword: String,
    previous_position: Option<StackPositionSnapshot>,
}

#[derive(Debug, Deserialize)]
struct StackPositionSnapshot {
    position: i64,
    added_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct ScreenerDecisionUndoPayload {
    sender: String,
    previous_rule: Option<ScreenerRuleSnapshot>,
}

#[derive(Debug, Deserialize)]
struct ScreenerRuleSnapshot {
    decision: String,
    classify_as: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    first_seen_at: DateTime<Utc>,
}

async fn restore_screener_decision(
    state: &AppState,
    user: &AuthUser,
    payload: Value,
) -> Result<(), UndoError> {
    let payload: ScreenerDecisionUndoPayload = serde_json::from_value(payload)
        .map_err(|_| UndoError::bad_request("invalid_undo_payload"))?;
    if payload.sender.trim().is_empty() {
        return Err(UndoError::bad_request("invalid_sender"));
    }

    match payload.previous_rule {
        Some(rule) => {
            validate_screener_rule(&rule)?;
            sqlx::query(
                "INSERT INTO screener_rules \
                 (user_id, sender_address, decision, classify_as, decided_at, first_seen_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(user_id, sender_address) DO UPDATE SET \
                   decision = excluded.decision, \
                   classify_as = excluded.classify_as, \
                   decided_at = excluded.decided_at, \
                   first_seen_at = excluded.first_seen_at",
            )
            .bind(user.id)
            .bind(&payload.sender)
            .bind(&rule.decision)
            .bind(&rule.classify_as)
            .bind(rule.decided_at)
            .bind(rule.first_seen_at)
            .execute(&state.db)
            .await
            .map_err(|err| UndoError::internal(err.to_string()))?;
        }
        None => {
            sqlx::query("DELETE FROM screener_rules WHERE user_id = ?1 AND sender_address = ?2")
                .bind(user.id)
                .bind(&payload.sender)
                .execute(&state.db)
                .await
                .map_err(|err| UndoError::internal(err.to_string()))?;
        }
    }

    Ok(())
}

async fn restore_thread_stack<R>(
    state: &AppState,
    user: &AuthUser,
    thread_restorer: &R,
    payload: ThreadStackUndoPayload,
) -> Result<(), UndoError>
where
    R: ThreadUndoRestorer,
{
    validate_thread_stack_payload(&payload)?;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|err| UndoError::internal(err.to_string()))?;

    match &payload.previous_position {
        Some(snapshot) => {
            sqlx::query(
                "INSERT INTO stack_positions (user_id, stack, thread_id, position, added_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(user_id, stack, thread_id) DO UPDATE SET \
                   position = excluded.position, \
                   added_at = excluded.added_at",
            )
            .bind(user.id)
            .bind(&payload.stack)
            .bind(&payload.thread_id)
            .bind(snapshot.position)
            .bind(snapshot.added_at)
            .execute(&mut *tx)
            .await
            .map_err(|err| UndoError::internal(err.to_string()))?;
        }
        None => {
            sqlx::query(
                "DELETE FROM stack_positions WHERE user_id = ?1 AND stack = ?2 AND thread_id = ?3",
            )
            .bind(user.id)
            .bind(&payload.stack)
            .bind(&payload.thread_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| UndoError::internal(err.to_string()))?;
        }
    }

    tx.commit()
        .await
        .map_err(|err| UndoError::internal(err.to_string()))?;

    if payload.previous_position.is_none() {
        thread_restorer
            .set_keyword(state, user, &payload.thread_id, &payload.keyword, false)
            .await?;
    }

    Ok(())
}

fn validate_thread_classify_payload(payload: &ThreadClassifyUndoPayload) -> Result<(), UndoError> {
    if !looks_like_jmap_id(&payload.thread_id) {
        return Err(UndoError::bad_request("invalid_thread_id"));
    }
    let previous = MailClassification::parse(&payload.previous_classification)
        .ok_or_else(|| UndoError::bad_request("invalid_previous_classification"))?;
    let new = MailClassification::parse(&payload.new_classification)
        .ok_or_else(|| UndoError::bad_request("invalid_new_classification"))?;
    if previous == new {
        return Err(UndoError::bad_request("noop_classify_undo"));
    }
    Ok(())
}

fn validate_thread_stack_payload(payload: &ThreadStackUndoPayload) -> Result<(), UndoError> {
    if !looks_like_jmap_id(&payload.thread_id) {
        return Err(UndoError::bad_request("invalid_thread_id"));
    }
    match (payload.stack.as_str(), payload.keyword.as_str()) {
        ("set_aside", "$hail_setaside") | ("reply_later", "$hail_replylater") => {}
        _ => return Err(UndoError::bad_request("invalid_stack_undo")),
    }
    if payload
        .previous_position
        .as_ref()
        .is_some_and(|snapshot| snapshot.position < 1)
    {
        return Err(UndoError::bad_request("invalid_stack_undo"));
    }
    Ok(())
}

fn validate_screener_rule(rule: &ScreenerRuleSnapshot) -> Result<(), UndoError> {
    match rule.decision.as_str() {
        "pending" | "deny" => {
            if rule.classify_as.is_some() {
                return Err(UndoError::bad_request("invalid_previous_rule"));
            }
        }
        "allow" => match rule
            .classify_as
            .as_deref()
            .and_then(MailClassification::parse)
        {
            Some(_) => {}
            _ => return Err(UndoError::bad_request("invalid_previous_rule")),
        },
        _ => return Err(UndoError::bad_request("invalid_previous_rule")),
    }
    Ok(())
}

async fn email_ids_in_thread(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Vec<String>, UndoError> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;

    let mut query = session
        .client()
        .email_query(
            Some(Filter::from(email_query::Filter::in_thread(thread_id))),
            None::<Vec<hail_jmap::jmap_client::core::query::Comparator<email_query::Comparator>>>,
        )
        .await
        .map_err(|err| UndoError::internal(err.to_string()))?;
    let ids = query.take_ids();
    if ids.is_empty() {
        return Err(UndoError::bad_request("thread_not_found"));
    }
    Ok(ids)
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_executor(Arc::new(
        ActionUndoExecutor::default(),
    )))
}

/// Build the OpenAPI-tracked router for production undo execution.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_executor(Arc::new(ActionUndoExecutor::default()))
}

pub fn router_with_executor<E>(executor: Arc<E>) -> Router<AppState>
where
    E: UndoExecutor,
{
    Router::from(openapi_router_with_executor(executor))
}

fn openapi_router_with_executor<E>(executor: Arc<E>) -> OpenApiRouter<AppState>
where
    E: UndoExecutor,
{
    let executor: Arc<dyn UndoExecutor> = executor;
    OpenApiRouter::new().routes(routes!(post_undo).layer(Extension(executor)))
}

pub async fn create_undo_action(
    state: &AppState,
    user_id: i64,
    action: NewUndoAction,
) -> Result<UndoToken, sqlx::Error> {
    insert_undo_action(state, user_id, action.kind().as_str(), action.payload()).await
}

async fn insert_undo_action(
    state: &AppState,
    user_id: i64,
    action: &str,
    payload: &Value,
) -> Result<UndoToken, sqlx::Error> {
    let now = Utc::now();
    let expires_at = now + Duration::seconds(UNDO_TTL_SECONDS);
    let id = new_undo_id().map_err(sqlx::Error::Protocol)?;
    let payload_json =
        serde_json::to_string(payload).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;

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

#[derive(Debug, sqlx::FromRow)]
struct UndoActionRow {
    action: String,
    payload_json: String,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
}

#[utoipa::path(
    post,
    path = "/api/undo/{id}",
    tag = TAG,
    params(
        ("id" = String, Path, description = "Opaque 64-character undo token id."),
    ),
    responses(
        (status = 200, description = "Undo token consumed and action executed.", body = UndoResponse),
        (status = 400, description = "Undo payload is invalid."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Undo token not found."),
        (status = 410, description = "Undo token expired or was already used."),
        (status = 501, description = "Undo action is not implemented."),
        (status = 500, description = "Undo execution failed."),
    ),
)]
async fn post_undo(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(executor): Extension<Arc<dyn UndoExecutor>>,
    Path(id): Path<String>,
) -> Response {
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
        tracing::error!(
            user_id = user.id,
            undo_id = %id,
            error = %err,
            "undo consume commit failed"
        );
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
        tracing::error!(
            user_id = user.id,
            undo_id = %id,
            action = %action,
            error = %err.message,
            "undo executor failed after token consume"
        );
        return undo_error_response(err.status);
    }

    Json(UndoResponse { id, action }).into_response()
}

#[derive(Debug, Serialize, ToSchema)]
struct UndoResponse {
    id: String,
    action: String,
}

fn looks_like_undo_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn gone(error: &'static str) -> Response {
    (
        StatusCode::GONE,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
    )
        .into_response()
}

fn undo_error_response(status: UndoErrorStatus) -> Response {
    match status {
        UndoErrorStatus::BadRequest => (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"bad_request"}"#,
        )
            .into_response(),
        UndoErrorStatus::NotImplemented => (
            StatusCode::NOT_IMPLEMENTED,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"not_implemented"}"#,
        )
            .into_response(),
        UndoErrorStatus::Internal => internal(),
    }
}

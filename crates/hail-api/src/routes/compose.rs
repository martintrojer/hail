//! Compose and reply send pipeline.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::audit;
use crate::middleware::auth::{AuthSession, AuthUser};
use crate::routes::jmap_helpers::{jmap_session, provider_error};
use crate::routes::outbound::{
    OutboundHeaders, create_draft_email, looks_like_email, render_markdown, reply_subject,
    set_rendered_body, submit_email, validate_attachments, validate_body, validate_recipients,
    validate_subject,
};
pub use crate::routes::outbound::{OutboundMessage, ReplyHeaders};
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

/// OpenAPI tag for outbound compose/reply endpoints.
pub const TAG: &str = "compose";

pub trait Composer: Send + Sync + 'static {
    fn create_draft<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        message: OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<String, ComposeError>> + Send + 'a>>;

    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        email_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>>;

    fn thread_context<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ReplyContext>, ComposeError>> + Send + 'a>>;
}

pub struct JmapComposer;

impl Composer for JmapComposer {
    fn create_draft<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        message: OutboundMessage,
    ) -> Pin<Box<dyn Future<Output = Result<String, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            create_draft_email::<ComposeError, _>(
                &session,
                OutboundHeaders::new(
                    from,
                    &message.to,
                    &message.cc,
                    &message.bcc,
                    &message.subject,
                )
                .with_reply(message.reply.as_ref()),
                |email| set_rendered_body(email, &message.body),
            )
            .await
        })
    }

    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        email_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            submit_email::<ComposeError>(&session, from, email_id).await
        })
    }

    fn thread_context<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ReplyContext>, ComposeError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            use hail_jmap::jmap_client::core::query::Filter;
            use hail_jmap::jmap_client::email::query as email_query;

            let mut query = session
                .client()
                .email_query(
                    Some(Filter::from(email_query::Filter::in_thread(thread_id))),
                    Some([email_query::Comparator::received_at().ascending()]),
                )
                .await
                .map_err(provider_error)?;
            let email_ids = query.take_ids();
            if email_ids.is_empty() {
                return Ok(None);
            }
            let mut email_request = session.client().build();
            email_request
                .get_email()
                .ids(email_ids.clone())
                .properties([
                    hail_jmap::jmap_client::email::Property::Id,
                    hail_jmap::jmap_client::email::Property::Subject,
                    hail_jmap::jmap_client::email::Property::From,
                    hail_jmap::jmap_client::email::Property::MessageId,
                    hail_jmap::jmap_client::email::Property::References,
                ]);
            let mut email_response = email_request
                .send_get_email()
                .await
                .map_err(provider_error)?;
            let mut emails_by_id = email_response
                .take_list()
                .into_iter()
                .map(|email| (email.id().unwrap_or_default().to_string(), email))
                .collect::<std::collections::HashMap<_, _>>();
            let Some(last_email) = email_ids
                .into_iter()
                .rev()
                .find_map(|id| emails_by_id.remove(&id))
            else {
                return Ok(None);
            };
            let mut references = last_email
                .references()
                .map(<[String]>::to_vec)
                .unwrap_or_default();
            let in_reply_to = last_email
                .message_id()
                .map(<[String]>::to_vec)
                .unwrap_or_default();
            for id in &in_reply_to {
                if !references.iter().any(|existing| existing == id) {
                    references.push(id.clone());
                }
            }
            let subject = reply_subject(last_email.subject().unwrap_or_default());
            let to = last_email
                .from()
                .unwrap_or_default()
                .iter()
                .map(|addr| addr.email().to_string())
                .filter(|addr| looks_like_email(addr))
                .collect();
            Ok(Some(ReplyContext {
                to,
                subject,
                in_reply_to,
                references,
            }))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyContext {
    pub to: Vec<String>,
    pub subject: String,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
}
#[derive(Debug)]
pub enum ComposeError {
    Provider(String),
}

impl crate::routes::jmap_helpers::ProviderError for ComposeError {
    fn provider(message: String) -> Self {
        Self::Provider(message)
    }
}

pub fn router() -> Router<AppState> {
    router_with_composer(Arc::new(JmapComposer))
}

/// Build the OpenAPI-tracked router for the production composer.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_composer(Arc::new(JmapComposer))
}

pub fn router_with_composer<C>(composer: Arc<C>) -> Router<AppState>
where
    C: Composer,
{
    Router::from(openapi_router_with_composer(composer))
}

fn openapi_router_with_composer<C>(composer: Arc<C>) -> OpenApiRouter<AppState>
where
    C: Composer,
{
    let composer: Arc<dyn Composer> = composer;
    OpenApiRouter::new()
        .routes(routes!(compose).layer(Extension(composer.clone())))
        .routes(routes!(reply).layer(Extension(composer)))
        .routes(routes!(list_scheduled_sends))
        .routes(routes!(get_scheduled_send))
        .routes(routes!(cancel_scheduled_send))
}

#[derive(Debug, Deserialize, ToSchema)]
struct ComposePayload {
    to: Vec<String>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: String,
    body_markdown: String,
    attachments: Option<Vec<serde_json::Value>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    send_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Deserialize, ToSchema)]
struct ReplyPayload {
    body_markdown: String,
    attachments: Option<Vec<serde_json::Value>>,
    #[schema(value_type = Option<String>, format = DateTime)]
    send_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status")]
enum ComposeResponse {
    #[serde(rename = "pending")]
    Pending {
        scheduled_send_id: i64,
        draft_email_id: String,
    },
    #[serde(rename = "sent")]
    Sent {
        email_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        submission_id: Option<String>,
    },
}

#[derive(Debug, Serialize, ToSchema)]
struct ScheduledSendResponse {
    id: i64,
    draft_email_id: String,
    #[schema(value_type = String, format = DateTime)]
    send_at: DateTime<Utc>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    claimed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    sent_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    created_at: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/api/compose",
    tag = TAG,
    request_body(content = ComposePayload, content_type = "application/json"),
    responses(
        (status = 200, description = "Message sent immediately.", body = ComposeResponse),
        (status = 201, description = "Message draft scheduled for later delivery.", body = ComposeResponse),
        (status = 400, description = "Invalid compose payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP provider or scheduler failure."),
    ),
)]
async fn compose(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(auth_session): Extension<AuthSession>,
    Extension(composer): Extension<Arc<dyn Composer>>,
    body: Result<Json<ComposePayload>, JsonRejection>,
) -> Response {
    let Ok(Json(payload)) = body else {
        return bad_request("invalid_json");
    };
    let send_at = match validate_send_at(payload.send_at) {
        Ok(send_at) => send_at,
        Err(error) => return bad_request(error),
    };
    let message = match payload.into_message(None) {
        Ok(message) => message,
        Err(error) => return bad_request(error),
    };
    create_and_maybe_send(
        &state,
        &user,
        &auth_session,
        composer.as_ref(),
        message,
        send_at,
    )
    .await
}

#[utoipa::path(
    post,
    path = "/api/threads/{thread_id}/reply",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id to reply to."),
    ),
    request_body(content = ReplyPayload, content_type = "application/json"),
    responses(
        (status = 200, description = "Reply sent immediately.", body = ComposeResponse),
        (status = 201, description = "Reply draft scheduled for later delivery.", body = ComposeResponse),
        (status = 400, description = "Invalid thread id or reply payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "JMAP provider or scheduler failure."),
    ),
)]
async fn reply(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(auth_session): Extension<AuthSession>,
    Extension(composer): Extension<Arc<dyn Composer>>,
    Path(thread_id): Path<String>,
    body: Result<Json<ReplyPayload>, JsonRejection>,
) -> Response {
    if let Err(response) = crate::routes::jmap_helpers::validate_thread_id(&thread_id) {
        return response;
    }
    let Ok(Json(payload)) = body else {
        return bad_request("invalid_json");
    };
    let send_at = match validate_send_at(payload.send_at) {
        Ok(send_at) => send_at,
        Err(error) => return bad_request(error),
    };
    let context = match composer
        .thread_context(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(Some(context)) => context,
        Ok(None) => return not_found(),
        Err(ComposeError::Provider(err)) => return provider_failed(user.id, err),
    };
    let message = match payload.into_message(context) {
        Ok(message) => message,
        Err(error) => return bad_request(error),
    };
    create_and_maybe_send(
        &state,
        &user,
        &auth_session,
        composer.as_ref(),
        message,
        send_at,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/scheduled-sends",
    tag = TAG,
    responses(
        (status = 200, description = "Scheduled sends for the current user.", body = [ScheduledSendResponse]),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Scheduled send list failed."),
    ),
)]
async fn list_scheduled_sends(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match sqlx::query_as::<
        _,
        (
            i64,
            String,
            DateTime<Utc>,
            String,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<String>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, draft_email_id, send_at, status, claimed_at, sent_at, error, created_at \
         FROM scheduled_sends \
         WHERE user_id = ? \
         ORDER BY send_at ASC, id ASC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(
            rows.into_iter()
                .map(scheduled_send_response_from_row)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(err) => {
            tracing::warn!(user_id = user.id, error = %err, "scheduled send list failed");
            internal()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/scheduled-sends/{scheduled_send_id}",
    tag = TAG,
    params(
        ("scheduled_send_id" = i64, Path, description = "Scheduled send id."),
    ),
    responses(
        (status = 200, description = "Scheduled send detail for the current user.", body = ScheduledSendResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Scheduled send not found."),
        (status = 500, description = "Scheduled send lookup failed."),
    ),
)]
async fn get_scheduled_send(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(scheduled_send_id): Path<i64>,
) -> Response {
    match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found(),
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup failed");
            internal()
        }
    }
}

#[utoipa::path(
    delete,
    path = "/api/scheduled-sends/{scheduled_send_id}",
    tag = TAG,
    params(
        ("scheduled_send_id" = i64, Path, description = "Scheduled send id."),
    ),
    responses(
        (status = 200, description = "Scheduled send cancelled or already cancelled.", body = ScheduledSendResponse),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Scheduled send not found."),
        (status = 409, description = "Scheduled send is not cancellable."),
        (status = 500, description = "Scheduled send cancel failed."),
    ),
)]
async fn cancel_scheduled_send(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(scheduled_send_id): Path<i64>,
) -> Response {
    let row = match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup failed");
            return internal();
        }
    };

    match row.status.as_str() {
        "pending" => {}
        "cancelled" => return Json(row).into_response(),
        _ => return conflict("scheduled_send_not_cancellable"),
    }

    let now = Utc::now();
    let result = match sqlx::query(
        "UPDATE scheduled_sends \
         SET status = 'cancelled', error = NULL \
         WHERE id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(scheduled_send_id)
    .bind(user.id)
    .execute(&state.db)
    .await
    {
        Ok(result) => result,
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send cancel failed");
            return internal();
        }
    };

    if result.rows_affected() == 0 {
        return match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
            Ok(Some(row)) if row.status == "cancelled" => Json(row).into_response(),
            Ok(Some(_)) => conflict("scheduled_send_not_cancellable"),
            Ok(None) => not_found(),
            Err(err) => {
                tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup after cancel race failed");
                internal()
            }
        };
    }

    if let Err(err) = audit::record(
        &state.db,
        user.id,
        "compose.schedule_cancel",
        &serde_json::json!({
            "scheduled_send_id": scheduled_send_id,
            "cancelled_at": now,
        }),
    )
    .await
    {
        tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "audit log write failed");
    }

    match scheduled_send_for_user(&state, user.id, scheduled_send_id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found(),
        Err(err) => {
            tracing::warn!(user_id = user.id, scheduled_send_id, error = %err, "scheduled send lookup after cancel failed");
            internal()
        }
    }
}

async fn create_and_maybe_send(
    state: &AppState,
    user: &AuthUser,
    auth_session: &AuthSession,
    composer: &dyn Composer,
    message: OutboundMessage,
    send_at: Option<DateTime<Utc>>,
) -> Response {
    let draft_email_id = match composer
        .create_draft(state, user.jmap_token.clone(), &user.email, message)
        .await
    {
        Ok(id) => id,
        Err(ComposeError::Provider(err)) => return provider_failed(user.id, err),
    };
    if let Some(send_at) = send_at {
        let scheduled_send_id =
            match insert_scheduled_send(state, user.id, auth_session, &draft_email_id, send_at)
                .await
            {
                Ok(id) => id,
                Err(err) => {
                    tracing::warn!(user_id = user.id, error = %err, "scheduled send insert failed");
                    return internal();
                }
            };
        if let Err(err) = audit::record(
            &state.db,
            user.id,
            "compose.schedule",
            &serde_json::json!({
                "draft_email_id": draft_email_id,
                "scheduled_send_id": scheduled_send_id,
                "send_at": send_at,
            }),
        )
        .await
        {
            tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
        }
        // TODO(hail-worker): scheduled-send execution/failure is outside
        // hail-api; add matching audit rows in the worker send-later path.
        return (
            StatusCode::CREATED,
            Json(ComposeResponse::Pending {
                scheduled_send_id,
                draft_email_id,
            }),
        )
            .into_response();
    }
    match composer
        .submit(state, user.jmap_token.clone(), &user.email, &draft_email_id)
        .await
    {
        Ok(submission_id) => {
            if let Err(err) = audit::record(
                &state.db,
                user.id,
                "compose.send",
                &serde_json::json!({
                    "email_id": draft_email_id,
                    "submission_id": submission_id,
                }),
            )
            .await
            {
                tracing::warn!(user_id = user.id, error = %err, "audit log write failed");
            }
            Json(ComposeResponse::Sent {
                email_id: draft_email_id,
                submission_id,
            })
            .into_response()
        }
        Err(ComposeError::Provider(err)) => provider_failed(user.id, err),
    }
}

impl ComposePayload {
    fn into_message(self, reply: Option<ReplyHeaders>) -> Result<OutboundMessage, &'static str> {
        validate_attachments(&self.attachments)?;
        validate_recipients("to", &self.to, true)?;
        let cc = self.cc.unwrap_or_default();
        if !cc.is_empty() {
            validate_recipients("cc", &cc, true)?;
        }
        let bcc = self.bcc.unwrap_or_default();
        if !bcc.is_empty() {
            validate_recipients("bcc", &bcc, true)?;
        }
        validate_subject(&self.subject)?;
        validate_body(&self.body_markdown)?;
        Ok(OutboundMessage {
            to: self.to,
            cc,
            bcc,
            subject: self.subject,
            body: render_markdown(&self.body_markdown),
            reply,
        })
    }
}

impl ReplyPayload {
    fn into_message(self, context: ReplyContext) -> Result<OutboundMessage, &'static str> {
        validate_attachments(&self.attachments)?;
        validate_body(&self.body_markdown)?;
        if context.to.is_empty() {
            return Err("reply_recipient_not_found");
        }
        validate_recipients("to", &context.to, true)?;
        validate_subject(&context.subject)?;
        Ok(OutboundMessage {
            to: context.to,
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: context.subject,
            body: render_markdown(&self.body_markdown),
            reply: Some(ReplyHeaders {
                in_reply_to: context.in_reply_to,
                references: context.references,
            }),
        })
    }
}

fn validate_send_at(send_at: Option<DateTime<Utc>>) -> Result<Option<DateTime<Utc>>, &'static str> {
    match send_at {
        Some(send_at) if send_at <= Utc::now() => Err("invalid_send_at"),
        other => Ok(other),
    }
}

async fn insert_scheduled_send(
    state: &AppState,
    user_id: i64,
    auth_session: &AuthSession,
    draft_email_id: &str,
    send_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let now = Utc::now();
    sqlx::query_scalar(
        "INSERT INTO scheduled_sends \
         (user_id, draft_email_id, send_at, status, auth_session_id, auth_session_expires_at, created_at) \
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6) RETURNING id",
    )
    .bind(user_id)
    .bind(draft_email_id)
    .bind(send_at)
    .bind(auth_session.id.as_str())
    .bind(auth_session.expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await
}

async fn scheduled_send_for_user(
    state: &AppState,
    user_id: i64,
    scheduled_send_id: i64,
) -> Result<Option<ScheduledSendResponse>, sqlx::Error> {
    sqlx::query_as::<
        _,
        (
            i64,
            String,
            DateTime<Utc>,
            String,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<String>,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, draft_email_id, send_at, status, claimed_at, sent_at, error, created_at \
         FROM scheduled_sends \
         WHERE id = ? AND user_id = ?",
    )
    .bind(scheduled_send_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map(|row| row.map(scheduled_send_response_from_row))
}

fn scheduled_send_response_from_row(
    row: (
        i64,
        String,
        DateTime<Utc>,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<String>,
        DateTime<Utc>,
    ),
) -> ScheduledSendResponse {
    let (id, draft_email_id, send_at, status, claimed_at, sent_at, error, created_at) = row;
    ScheduledSendResponse {
        id,
        draft_email_id,
        send_at,
        status,
        claimed_at,
        sent_at,
        error,
        created_at,
    }
}

fn provider_failed(user_id: i64, err: String) -> Response {
    tracing::warn!(user_id, error = %err, "compose provider failed");
    internal()
}

fn conflict(error: &'static str) -> Response {
    (
        StatusCode::CONFLICT,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
    )
        .into_response()
}

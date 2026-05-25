//! Compose and reply send pipeline.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use chrono::{DateTime, Utc};
use hail_core::mail_render::sanitize_and_strip_trackers;
use hail_jmap::jmap_client::core::set::SetObject;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::audit;
use crate::middleware::auth::{AuthSession, AuthUser};
use crate::routes::jmap_helpers::{jmap_session, provider_error, required_drafts_mailbox_id};
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

const MAX_SUBJECT_CHARS: usize = 998;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_RECIPIENTS_PER_FIELD: usize = 200;

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
            let drafts_mailbox_id = required_drafts_mailbox_id(&session)
                .await
                .map_err(provider_error)?;
            let mut request = session.client().build();
            let create_id = {
                let email = request.set_email().create();
                email
                    .mailbox_ids([drafts_mailbox_id])
                    .keywords(["$draft"])
                    .from([from])
                    .to(message.to.iter().map(String::as_str))
                    .subject(message.subject.clone());
                if !message.cc.is_empty() {
                    email.cc(message.cc.iter().map(String::as_str));
                }
                if !message.bcc.is_empty() {
                    email.bcc(message.bcc.iter().map(String::as_str));
                }
                if let Some(reply) = &message.reply {
                    if !reply.in_reply_to.is_empty() {
                        email.in_reply_to(reply.in_reply_to.iter().map(String::as_str));
                    }
                    if !reply.references.is_empty() {
                        email.references(reply.references.iter().map(String::as_str));
                    }
                }
                set_body_values(email, &message.body);
                email.create_id().ok_or_else(|| {
                    ComposeError::Provider("Email/set create id missing".to_string())
                })?
            };
            let mut response = request.send_set_email().await.map_err(provider_error)?;
            let mut created = response.created(&create_id).map_err(provider_error)?;
            let id = created.take_id();
            if id.is_empty() {
                Err(ComposeError::Provider(
                    "Email/set created draft without id".to_string(),
                ))
            } else {
                Ok(id)
            }
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
            let identity_id = identity_id_for(&session, from).await?;
            let mut request = session.client().build();
            let create_id = request
                .set_email_submission()
                .create()
                .email_id(email_id)
                .identity_id(identity_id)
                .create_id()
                .ok_or_else(|| {
                    ComposeError::Provider("EmailSubmission/set create id missing".to_string())
                })?;
            let mut response = request
                .send_set_email_submission()
                .await
                .map_err(provider_error)?;
            let mut created = response.created(&create_id).map_err(provider_error)?;
            let submission_id = created.take_id();
            Ok((!submission_id.is_empty()).then_some(submission_id))
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

#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: RenderedBody,
    pub reply: Option<ReplyHeaders>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBody {
    pub plain_text: String,
    pub html: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyHeaders {
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
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
        .route("/api/scheduled-sends", get(list_scheduled_sends))
        .route(
            "/api/scheduled-sends/{scheduled_send_id}",
            get(get_scheduled_send).delete(cancel_scheduled_send),
        )
}

fn openapi_router_with_composer<C>(composer: Arc<C>) -> OpenApiRouter<AppState>
where
    C: Composer,
{
    let composer: Arc<dyn Composer> = composer;
    OpenApiRouter::new()
        .routes(routes!(compose).layer(Extension(composer.clone())))
        .routes(routes!(reply).layer(Extension(composer)))
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

#[derive(Debug, Serialize)]
struct ScheduledSendResponse {
    id: i64,
    draft_email_id: String,
    send_at: DateTime<Utc>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    claimed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sent_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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
        validate_recipients("to", &self.to)?;
        let cc = self.cc.unwrap_or_default();
        if !cc.is_empty() {
            validate_recipients("cc", &cc)?;
        }
        let bcc = self.bcc.unwrap_or_default();
        if !bcc.is_empty() {
            validate_recipients("bcc", &bcc)?;
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
        validate_recipients("to", &context.to)?;
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

fn render_markdown(markdown: &str) -> RenderedBody {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(markdown));
    RenderedBody {
        plain_text: markdown_to_plain_text(markdown),
        html: sanitize_and_strip_trackers(&html).html,
    }
}

fn markdown_to_plain_text(markdown: &str) -> String {
    let mut text = String::new();
    let mut need_space = false;
    for event in pulldown_cmark::Parser::new(markdown) {
        match event {
            pulldown_cmark::Event::Text(value) | pulldown_cmark::Event::Code(value) => {
                if need_space && !text.ends_with([' ', '\n']) {
                    text.push(' ');
                }
                text.push_str(&value);
                need_space = false;
            }
            pulldown_cmark::Event::SoftBreak | pulldown_cmark::Event::HardBreak => {
                text.push('\n');
                need_space = false;
            }
            pulldown_cmark::Event::End(
                pulldown_cmark::TagEnd::Paragraph | pulldown_cmark::TagEnd::Heading(_),
            ) => {
                text.push_str("\n\n");
                need_space = false;
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Item) => {
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str("- ");
                need_space = false;
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Item) => {
                text.push('\n');
                need_space = false;
            }
            pulldown_cmark::Event::End(_) => need_space = true,
            _ => {}
        }
    }
    text.trim().to_string()
}

fn validate_attachments(attachments: &Option<Vec<serde_json::Value>>) -> Result<(), &'static str> {
    match attachments {
        Some(attachments) if !attachments.is_empty() => Err("attachments_not_supported"),
        _ => Ok(()),
    }
}

fn validate_recipients(field: &'static str, recipients: &[String]) -> Result<(), &'static str> {
    if recipients.is_empty() {
        return Err(match field {
            "to" => "to_required",
            "cc" => "cc_required",
            "bcc" => "bcc_required",
            _ => "recipients_required",
        });
    }
    if recipients.len() > MAX_RECIPIENTS_PER_FIELD {
        return Err(match field {
            "to" => "too_many_to",
            "cc" => "too_many_cc",
            "bcc" => "too_many_bcc",
            _ => "too_many_recipients",
        });
    }
    if recipients.iter().any(|address| !looks_like_email(address)) {
        return Err(match field {
            "to" => "invalid_to",
            "cc" => "invalid_cc",
            "bcc" => "invalid_bcc",
            _ => "invalid_recipient",
        });
    }
    Ok(())
}

fn validate_subject(subject: &str) -> Result<(), &'static str> {
    if subject.chars().count() > MAX_SUBJECT_CHARS || subject.contains(['\r', '\n']) {
        Err("invalid_subject")
    } else {
        Ok(())
    }
}

fn validate_body(body: &str) -> Result<(), &'static str> {
    if body.len() > MAX_BODY_BYTES {
        Err("body_too_large")
    } else {
        Ok(())
    }
}

fn looks_like_email(address: &str) -> bool {
    let address = address.trim();
    if address.is_empty()
        || address.len() > 320
        || address.contains(char::is_whitespace)
        || address.contains(['\r', '\n'])
    {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

fn reply_subject(subject: &str) -> String {
    if subject
        .trim_start()
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:"))
    {
        subject.to_string()
    } else {
        format!("Re: {subject}")
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

async fn identity_id_for(session: &hail_jmap::Session, from: &str) -> Result<String, ComposeError> {
    let mut request = session.client().build();
    request.get_identity().properties([
        hail_jmap::jmap_client::identity::Property::Id,
        hail_jmap::jmap_client::identity::Property::Email,
    ]);
    let mut response = request.send_get_identity().await.map_err(provider_error)?;
    let mut identities = response.take_list();
    if let Some(index) = identities.iter().position(|identity| {
        identity
            .email()
            .is_some_and(|email| email.eq_ignore_ascii_case(from))
    }) {
        return Ok(identities[index].take_id());
    }
    identities
        .first_mut()
        .map(|identity| identity.take_id())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ComposeError::Provider("identity not found".to_string()))
}

fn set_body_values(
    email: &mut hail_jmap::jmap_client::email::Email<hail_jmap::jmap_client::Set>,
    body: &RenderedBody,
) {
    use hail_jmap::jmap_client::email::EmailBodyPart;

    email
        .text_body(
            EmailBodyPart::new()
                .part_id("text")
                .content_type("text/plain"),
        )
        .html_body(
            EmailBodyPart::new()
                .part_id("html")
                .content_type("text/html"),
        )
        .body_value("text".to_string(), body.plain_text.clone())
        .body_value("html".to_string(), body.html.clone());
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

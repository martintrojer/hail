//! Thread-as-document view endpoint.
//!
//! `GET /api/threads/:thread_id` assembles a JMAP thread into a UI-ready
//! document for the SPA. Rendering happens server-side: quoted history is
//! stripped before untrusted HTML is sanitized and likely tracking pixels are
//! removed/counted. Never log message bodies from this module.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use hail_core::mail_render::{
    plaintext_body_to_html, sanitize_and_strip_trackers, strip_quoted_history,
};
use secrecy::SecretString;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{jmap_session, validate_thread_id};
use crate::routes::notes::ThreadNoteResponse;
use crate::routes::response::{internal, not_found};
use crate::state::AppState;

/// Dependency-injection seam for assembling a thread. Production uses JMAP;
/// tests attach a fake assembler so route behavior does not need Stalwart.
pub trait ThreadAssembler: Send + Sync + 'static {
    fn assemble<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<AssembledThread>, ThreadAssembleError>> + Send + 'a>,
    >;
}

/// Opaque assembler failure. Detailed JMAP errors stay in server logs only.
#[derive(Debug)]
pub struct ThreadAssembleError(pub String);

/// Production assembler backed by JMAP `Email/query` + `Email/get`.
pub struct JmapThreadAssembler;

impl ThreadAssembler for JmapThreadAssembler {
    fn assemble<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        thread_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<AssembledThread>, ThreadAssembleError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let session = jmap_session(state, token)
                .await
                .map_err(ThreadAssembleError)?;

            let mut query_request = session.client().build();
            query_request
                .query_email()
                .filter(hail_jmap::jmap_client::email::query::Filter::in_thread(
                    thread_id,
                ))
                .sort([
                    hail_jmap::jmap_client::email::query::Comparator::received_at().ascending(),
                ]);
            let mut query_response = query_request
                .send_query_email()
                .await
                .map_err(|err| ThreadAssembleError(err.to_string()))?;
            let email_ids = query_response.take_ids();
            if email_ids.is_empty() {
                return Ok(None);
            }

            let mut email_request = session.client().build();
            let get_email = email_request.get_email();
            get_email.ids(email_ids.clone()).properties([
                hail_jmap::jmap_client::email::Property::Id,
                hail_jmap::jmap_client::email::Property::ThreadId,
                hail_jmap::jmap_client::email::Property::Subject,
                hail_jmap::jmap_client::email::Property::From,
                hail_jmap::jmap_client::email::Property::To,
                hail_jmap::jmap_client::email::Property::ReceivedAt,
                hail_jmap::jmap_client::email::Property::HtmlBody,
                hail_jmap::jmap_client::email::Property::TextBody,
                hail_jmap::jmap_client::email::Property::BodyValues,
                hail_jmap::jmap_client::email::Property::Preview,
            ]);
            get_email.arguments().fetch_html_body_values(true);
            get_email.arguments().fetch_text_body_values(true);
            let mut email_response = email_request
                .send_get_email()
                .await
                .map_err(|err| ThreadAssembleError(err.to_string()))?;

            let mut emails_by_id = email_response
                .take_list()
                .into_iter()
                .map(|email| (email.id().unwrap_or_default().to_string(), email))
                .collect::<std::collections::HashMap<_, _>>();

            let mut messages = Vec::with_capacity(email_ids.len());
            for email_id in email_ids {
                let Some(email) = emails_by_id.remove(&email_id) else {
                    continue;
                };
                if email.thread_id() != Some(thread_id) {
                    continue;
                }
                messages.push(AssembledMessage {
                    email_id,
                    from: addresses_from_jmap(email.from()),
                    to: addresses_from_jmap(email.to()),
                    received_at: email
                        .received_at()
                        .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0)),
                    subject: email.subject().unwrap_or_default().to_string(),
                    html: html_body_from_email(&email),
                    text: text_body_from_email(&email),
                    preview: email.preview().unwrap_or_default().to_string(),
                });
            }

            let subject = messages
                .iter()
                .find_map(|message| (!message.subject.is_empty()).then(|| message.subject.clone()))
                .unwrap_or_default();

            Ok(Some(AssembledThread {
                thread_id: thread_id.to_string(),
                subject,
                messages,
            }))
        })
    }
}

/// Raw assembled thread before render hygiene is applied by the handler.
#[derive(Debug, Clone)]
pub struct AssembledThread {
    pub thread_id: String,
    pub subject: String,
    pub messages: Vec<AssembledMessage>,
}

/// Raw assembled message before render hygiene is applied by the handler.
#[derive(Debug, Clone)]
pub struct AssembledMessage {
    pub email_id: String,
    pub from: Vec<Participant>,
    pub to: Vec<Participant>,
    pub received_at: Option<DateTime<Utc>>,
    pub subject: String,
    pub html: String,
    pub text: String,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Participant {
    pub name: Option<String>,
    pub email: String,
}

/// OpenAPI tag for thread document endpoints.
pub const TAG: &str = "threads";

/// Build protected thread-as-document routes.
pub fn router() -> OpenApiRouter<AppState> {
    router_with_assembler(Arc::new(JmapThreadAssembler))
}

/// Test/helper router that injects a fake thread assembler.
pub fn router_with_assembler<A>(assembler: Arc<A>) -> OpenApiRouter<AppState>
where
    A: ThreadAssembler,
{
    let assembler: Arc<dyn ThreadAssembler> = assembler;
    OpenApiRouter::new().routes(routes!(get_thread).layer(Extension(assembler)))
}

#[derive(Debug, Serialize, ToSchema)]
struct ThreadViewResponse {
    thread_id: String,
    subject: String,
    participants: Vec<Participant>,
    messages: Vec<ThreadMessageResponse>,
    notes: Vec<ThreadNoteResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ThreadMessageResponse {
    email_id: String,
    from: Vec<Participant>,
    to: Vec<Participant>,
    #[schema(value_type = Option<String>, format = DateTime)]
    received_at: Option<DateTime<Utc>>,
    html: String,
    preview: String,
    blocked_trackers: Vec<BlockedTrackerResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BlockedTrackerResponse {
    src: String,
    reason: String,
}

#[utoipa::path(
    get,
    path = "/api/threads/{thread_id}",
    tag = TAG,
    params(
        ("thread_id" = String, Path, description = "JMAP thread id to render as a sanitized document."),
    ),
    responses(
        (status = 200, description = "Thread rendered as sanitized HTML messages.", body = ThreadViewResponse),
        (status = 400, description = "Invalid thread id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Thread not found."),
        (status = 500, description = "Thread assembly failed."),
    ),
)]
async fn get_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(assembler): Extension<Arc<dyn ThreadAssembler>>,
    Path(thread_id): Path<String>,
) -> Response {
    if let Err(response) = validate_thread_id(&thread_id) {
        return response;
    }

    let assembled = match assembler
        .assemble(&state, user.jmap_token.clone(), &thread_id)
        .await
    {
        Ok(Some(thread)) => thread,
        Ok(None) => return not_found("not_found"),
        Err(err) => {
            tracing::warn!(user_id = user.id, thread_id = %thread_id, error = %err.0, "thread assembly failed");
            return internal();
        }
    };

    if let Err(err) = hail_db::mark_thread_seen(&state.db, user.id, &thread_id).await {
        tracing::warn!(?err, "failed to mark thread seen");
    }

    let participants = participants_for(&assembled.messages);
    let messages = assembled
        .messages
        .into_iter()
        .map(render_message)
        .collect::<Vec<_>>();

    let notes = match crate::routes::notes::load_thread_notes(&state, user.id, &assembled.thread_id)
        .await
    {
        Ok(notes) => notes,
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %assembled.thread_id, error = %err, "thread note lookup failed");
            return internal();
        }
    };

    Json(ThreadViewResponse {
        thread_id: assembled.thread_id,
        subject: assembled.subject,
        participants,
        messages,
        notes,
    })
    .into_response()
}

fn render_message(message: AssembledMessage) -> ThreadMessageResponse {
    let body_html = if message.html.is_empty() {
        plaintext_body_to_html(&message.text)
    } else {
        message.html
    };
    let stripped = strip_quoted_history(&body_html);
    let sanitized = sanitize_and_strip_trackers(&stripped.html);
    ThreadMessageResponse {
        email_id: message.email_id,
        from: message.from,
        to: message.to,
        received_at: message.received_at,
        html: sanitized.html,
        preview: message.preview,
        blocked_trackers: sanitized
            .blocked_trackers
            .into_iter()
            .map(|tracker| BlockedTrackerResponse {
                src: tracker.src,
                reason: tracker.reason,
            })
            .collect(),
    }
}

fn participants_for(messages: &[AssembledMessage]) -> Vec<Participant> {
    let mut participants = Vec::new();
    for participant in messages
        .iter()
        .flat_map(|message| message.from.iter().chain(message.to.iter()))
    {
        if !participants
            .iter()
            .any(|existing: &Participant| existing.email.eq_ignore_ascii_case(&participant.email))
        {
            participants.push(participant.clone());
        }
    }
    participants
}

fn addresses_from_jmap(
    addresses: Option<&[hail_jmap::jmap_client::email::EmailAddress]>,
) -> Vec<Participant> {
    addresses
        .unwrap_or_default()
        .iter()
        .map(|address| Participant {
            name: address
                .name()
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            email: address.email().to_string(),
        })
        .collect()
}

fn html_body_from_email(email: &hail_jmap::jmap_client::email::Email) -> String {
    body_from_parts(email, email.html_body())
}

fn text_body_from_email(email: &hail_jmap::jmap_client::email::Email) -> String {
    body_from_parts(email, email.text_body())
}

fn body_from_parts(
    email: &hail_jmap::jmap_client::email::Email,
    parts: Option<&[hail_jmap::jmap_client::email::EmailBodyPart]>,
) -> String {
    let Some(parts) = parts else {
        return String::new();
    };

    let mut body = String::new();
    for part in parts {
        let Some(part_id) = part.part_id() else {
            continue;
        };
        let Some(value) = email.body_value(part_id) else {
            continue;
        };
        body.push_str(value.value());
    }
    body
}

//! Thread-as-document view endpoint.
//!
//! `GET /api/threads/:thread_id` assembles a cached thread into a UI-ready
//! document for the SPA. Rendering happens server-side: quoted history is
//! stripped before untrusted HTML is sanitized and likely tracking pixels are
//! removed/counted. Never log message bodies from this module.

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use hail_core::mail_render::{
    build_reply_quote_html, plaintext_body_to_html, sanitize_and_strip_trackers,
    strip_quoted_history,
};
use secrecy::SecretString;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::validate_thread_id;
use crate::routes::labels::LabelResponse;
use crate::routes::notes::ThreadNoteResponse;
use crate::routes::response::{internal, not_found};
use crate::state::AppState;

/// Dependency-injection seam for assembling a thread. Production uses CachedMail;
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

/// Production assembler backed by CachedMail.
pub struct CacheThreadAssembler;

impl ThreadAssembler for CacheThreadAssembler {
    fn assemble<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        thread_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<AssembledThread>, ThreadAssembleError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let thread = state
                .mail
                .get_thread(thread_id)
                .await
                .map_err(|err| ThreadAssembleError(err.to_string()))?;
            if thread.messages.is_empty() {
                return Ok(None);
            }

            let mut messages = Vec::with_capacity(thread.messages.len());
            for message in thread.messages {
                let rfc822 = state
                    .mail
                    .get_message_body(&message.id)
                    .await
                    .map_err(|err| ThreadAssembleError(err.to_string()))?;
                let (html, text) = parsed_bodies_from_rfc822(&rfc822);
                let attachments = message
                    .blob_refs
                    .into_iter()
                    .map(|blob_ref| {
                        let blob_id = blob_ref.as_str().to_owned();
                        Attachment {
                            filename: blob_id.clone(),
                            size: 0,
                            mime_type: "application/octet-stream".to_string(),
                            download_url: format!(
                                "/api/attachments/{}/download",
                                urlencoding(&blob_id)
                            ),
                            blob_id,
                            inline: false,
                        }
                    })
                    .collect();
                messages.push(AssembledMessage {
                    email_id: message.id.as_str().to_owned(),
                    from: participants_from_strings(std::slice::from_ref(&message.from)),
                    to: participants_from_strings(&message.to),
                    received_at: message.received_at,
                    subject: message.subject,
                    html,
                    text,
                    preview: message.preview,
                    inline_images: Vec::new(),
                    attachments,
                });
            }

            let subject = messages
                .iter()
                .find_map(|message| (!message.subject.is_empty()).then(|| message.subject.clone()))
                .unwrap_or_default();

            Ok(Some(AssembledThread {
                thread_id: thread.thread_id,
                subject,
                messages,
            }))
        })
    }
}

fn parsed_bodies_from_rfc822(rfc822: &[u8]) -> (String, String) {
    let Some(message) = mail_parser::MessageParser::default().parse(rfc822) else {
        return (String::new(), String::from_utf8_lossy(rfc822).into_owned());
    };
    let mut html = Vec::new();
    for index in 0..message.html_body_count() {
        if let Some(body) = message.body_html(index) {
            html.push(body.into_owned());
        }
    }
    let mut text = Vec::new();
    for index in 0..message.text_body_count() {
        if let Some(body) = message.body_text(index) {
            text.push(body.into_owned());
        }
    }
    (html.join("\n\n"), text.join("\n\n"))
}

fn participants_from_strings(addresses: &[String]) -> Vec<Participant> {
    addresses
        .iter()
        .filter(|address| !address.trim().is_empty())
        .map(|address| Participant {
            name: None,
            email: address.clone(),
        })
        .collect()
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
    pub inline_images: Vec<InlineImage>,
    pub attachments: Vec<Attachment>,
}

/// User-downloadable JMAP attachment metadata for a message in the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Attachment {
    pub filename: String,
    pub size: u64,
    pub mime_type: String,
    pub blob_id: String,
    pub download_url: String,
    pub inline: bool,
}

/// Inline image reference resolved from a MIME Content-ID to a JMAP blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub cid: String,
    pub blob_id: String,
    pub type_: String,
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
    router_with_assembler(Arc::new(CacheThreadAssembler))
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
    labels: Vec<LabelResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ThreadMessageResponse {
    email_id: String,
    from: Vec<Participant>,
    to: Vec<Participant>,
    #[schema(value_type = Option<String>, format = DateTime)]
    received_at: Option<DateTime<Utc>>,
    html: String,
    html_with_remote_images: String,
    reply_quote_html: String,
    preview: String,
    blocked_trackers: Vec<BlockedTrackerResponse>,
    attachments: Vec<Attachment>,
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
    let labels = match hail_db::labels::list_thread_labels(&state.db, user.id, &assembled.thread_id)
        .await
    {
        Ok(labels) => labels.into_iter().map(LabelResponse::from).collect(),
        Err(err) => {
            tracing::error!(user_id = user.id, thread_id = %assembled.thread_id, error = %err, "thread label lookup failed");
            return internal();
        }
    };

    Json(ThreadViewResponse {
        thread_id: assembled.thread_id,
        subject: assembled.subject,
        participants,
        messages,
        notes,
        labels,
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
    let mut sanitized = sanitize_and_strip_trackers(&stripped.html);
    let mut html_with_remote_images = sanitized.html_with_remote_images.clone();
    sanitized.html = rewrite_inline_image_sources(&sanitized.html, &message.inline_images);
    html_with_remote_images =
        rewrite_inline_image_sources(&html_with_remote_images, &message.inline_images);
    let reply_quote_html = build_reply_quote_html(
        &reply_quote_date_label(message.received_at),
        &reply_quote_sender_label(&message.from),
        &sanitized.html,
    );
    ThreadMessageResponse {
        email_id: message.email_id,
        from: message.from,
        to: message.to,
        received_at: message.received_at,
        html: sanitized.html.clone(),
        html_with_remote_images,
        reply_quote_html,
        preview: message.preview,
        blocked_trackers: sanitized
            .blocked_trackers
            .into_iter()
            .map(|tracker| BlockedTrackerResponse {
                src: tracker.src,
                reason: tracker.reason,
            })
            .collect(),
        attachments: message.attachments,
    }
}

fn reply_quote_date_label(received_at: Option<DateTime<Utc>>) -> String {
    received_at
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| "an earlier message".to_string())
}

fn reply_quote_sender_label(participants: &[Participant]) -> String {
    let Some(sender) = participants.first() else {
        return "Unknown sender".to_string();
    };
    sender
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&sender.email)
        .to_string()
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

fn inline_image_url(image: &InlineImage) -> String {
    format!(
        "/api/attachments/{}/download?disposition=inline&type={}",
        urlencoding(&image.blob_id),
        urlencoding(&image.type_)
    )
}

fn rewrite_inline_image_sources(body_html: &str, inline_images: &[InlineImage]) -> String {
    if inline_images.is_empty() || !body_html.to_ascii_lowercase().contains("cid:") {
        return body_html.to_string();
    }
    let cid_to_url = inline_images
        .iter()
        .map(|image| (image.cid.as_str(), inline_image_url(image)))
        .collect::<HashMap<_, _>>();
    let mut rewritten = String::with_capacity(body_html.len());
    let mut rest = body_html;
    while let Some(img_start) = find_next_img_tag(rest) {
        rewritten.push_str(&rest[..img_start]);
        rest = &rest[img_start..];
        let Some(tag_end) = rest.find('>') else {
            rewritten.push_str(rest);
            return rewritten;
        };
        let tag = &rest[..=tag_end];
        rewritten.push_str(&rewrite_img_tag_src(tag, &cid_to_url));
        rest = &rest[tag_end + 1..];
    }
    rewritten.push_str(rest);
    rewritten
}

fn find_next_img_tag(input: &str) -> Option<usize> {
    let lower = input.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(index) = lower[offset..].find("<img") {
        let index = offset + index;
        let after = lower[index + 4..].chars().next();
        if after.is_none_or(|ch| ch == '>' || ch.is_ascii_whitespace() || ch == '/') {
            return Some(index);
        }
        offset = index + 4;
    }
    None
}

fn rewrite_img_tag_src<'a>(tag: &'a str, cid_to_url: &HashMap<&str, String>) -> Cow<'a, str> {
    let Some((value_start, value_end, cid)) = find_img_src_cid_value(tag) else {
        return Cow::Borrowed(tag);
    };
    let Some(url) = cid_to_url.get(cid.as_str()) else {
        return Cow::Borrowed(tag);
    };
    let mut rewritten = String::with_capacity(tag.len() + url.len());
    rewritten.push_str(&tag[..value_start]);
    rewritten.push_str(url);
    rewritten.push_str(&tag[value_end..]);
    Cow::Owned(rewritten)
}

fn find_img_src_cid_value(tag: &str) -> Option<(usize, usize, String)> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b':' | b'_'))
        {
            i += 1;
        }
        if name_start == i {
            i += 1;
            continue;
        }
        let name = &tag[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let (value_start, value_end) = if matches!(bytes[i], b'"' | b'\'') {
            let quote = bytes[i];
            i += 1;
            let value_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            (value_start, i)
        } else {
            let value_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            (value_start, i)
        };
        if name.eq_ignore_ascii_case("src")
            && let Some(cid) = cid_from_src(&tag[value_start..value_end])
        {
            return Some((value_start, value_end, cid));
        }
    }
    None
}

fn cid_from_src(src: &str) -> Option<String> {
    let trimmed = src.trim();
    let rest = trimmed
        .get(..4)
        .filter(|prefix| prefix.eq_ignore_ascii_case("cid:"))?;
    let cid = &trimmed[rest.len()..];
    let cid = cid.split(['?', '#']).next().unwrap_or(cid);
    percent_decode_cid(cid).and_then(|value| normalize_inline_cid(&value))
}

fn normalize_inline_cid(cid: &str) -> Option<String> {
    let trimmed = cid
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

fn percent_decode_cid(input: &str) -> Option<String> {
    let decoded = url::form_urlencoded::parse(input.as_bytes())
        .next()
        .map(|(value, _)| value.into_owned())
        .unwrap_or_else(|| input.to_string());
    Some(decoded)
}

fn urlencoding(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

//! Shared outbound mail helpers for compose/reply sends and draft autosave.

use hail_core::mail_render::{html_fragment_to_text, sanitize_outgoing_html};
use hail_jmap::jmap_client::core::set::SetObject;

use crate::routes::jmap_helpers::{ProviderError, provider_error, required_drafts_mailbox_id};

pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_SUBJECT_CHARS: usize = 998;
const MAX_RECIPIENTS_PER_FIELD: usize = 200;

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

pub struct OutboundHeaders<'a> {
    from: &'a str,
    to: &'a [String],
    cc: &'a [String],
    bcc: &'a [String],
    subject: &'a str,
    reply: Option<&'a ReplyHeaders>,
}

impl<'a> OutboundHeaders<'a> {
    pub fn new(
        from: &'a str,
        to: &'a [String],
        cc: &'a [String],
        bcc: &'a [String],
        subject: &'a str,
    ) -> Self {
        Self {
            from,
            to,
            cc,
            bcc,
            subject,
            reply: None,
        }
    }

    pub fn with_reply(mut self, reply: Option<&'a ReplyHeaders>) -> Self {
        self.reply = reply;
        self
    }
}

pub async fn create_draft_email<E, F>(
    session: &hail_jmap::Session,
    headers: OutboundHeaders<'_>,
    set_body: F,
) -> Result<String, E>
where
    E: ProviderError,
    F: FnOnce(&mut hail_jmap::jmap_client::email::Email<hail_jmap::jmap_client::Set>),
{
    ensure_sender_identity::<E>(session, headers.from).await?;
    let drafts_mailbox_id = required_drafts_mailbox_id(session)
        .await
        .map_err(provider_error::<E>)?;

    let mut request = session.client().build();
    let create_id = {
        let email = request.set_email().create();
        apply_headers(email, &headers);
        email.mailbox_ids([drafts_mailbox_id]).keywords(["$draft"]);
        set_body(email);
        email
            .create_id()
            .ok_or_else(|| E::provider("Email/set create id missing".to_string()))?
    };

    let mut response = request
        .send_set_email()
        .await
        .map_err(provider_error::<E>)?;
    let mut created = response.created(&create_id).map_err(provider_error::<E>)?;
    let id = created.take_id();
    if id.is_empty() {
        Err(E::provider(
            "Email/set created draft without id".to_string(),
        ))
    } else {
        Ok(id)
    }
}

pub async fn submit_email<E>(
    session: &hail_jmap::Session,
    from: &str,
    email_id: &str,
) -> Result<Option<String>, E>
where
    E: ProviderError,
{
    let identity_id = required_identity_id_for::<E>(session, from).await?;
    let mut request = session.client().build();
    let create_id = request
        .set_email_submission()
        .create()
        .email_id(email_id)
        .identity_id(identity_id)
        .create_id()
        .ok_or_else(|| E::provider("EmailSubmission/set create id missing".to_string()))?;
    let mut response = request
        .send_set_email_submission()
        .await
        .map_err(provider_error::<E>)?;
    let mut created = response.created(&create_id).map_err(provider_error::<E>)?;
    let submission_id = created.take_id();
    Ok((!submission_id.is_empty()).then_some(submission_id))
}

pub fn set_text_body(
    email: &mut hail_jmap::jmap_client::email::Email<hail_jmap::jmap_client::Set>,
    body: impl Into<String>,
) {
    use hail_jmap::jmap_client::email::EmailBodyPart;

    email
        .text_body(
            EmailBodyPart::new()
                .part_id("text")
                .content_type("text/plain"),
        )
        .body_value("text".to_string(), body.into());
}

pub fn set_rendered_body(
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

pub fn render_markdown(markdown: &str) -> RenderedBody {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(markdown));
    rendered_html(&html)
}

pub fn render_compose_body(
    body_html: Option<&str>,
    body_markdown: Option<&str>,
) -> Result<RenderedBody, &'static str> {
    let Some(body) = body_html
        .filter(|body| !body.trim().is_empty())
        .map(rendered_html)
        .or_else(|| {
            body_markdown
                .filter(|body| !body.trim().is_empty())
                .map(render_markdown)
        })
    else {
        return Err("body_required");
    };

    validate_body(&body.html)?;
    if body.html.trim().is_empty() {
        return Err("body_required");
    }
    Ok(body)
}

pub fn rendered_html(html: &str) -> RenderedBody {
    let html = sanitize_outgoing_html(html);
    let plain_text = html_to_plaintext(&html);
    RenderedBody { plain_text, html }
}

pub fn html_to_plaintext(html: &str) -> String {
    html_fragment_to_text(html)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn validate_attachments(
    attachments: &Option<Vec<serde_json::Value>>,
) -> Result<(), &'static str> {
    match attachments {
        Some(attachments) if !attachments.is_empty() => Err("attachments_not_supported"),
        _ => Ok(()),
    }
}

pub fn validate_recipients(
    field: &'static str,
    recipients: &[String],
    required: bool,
) -> Result<(), &'static str> {
    if recipients.is_empty() {
        return if required {
            Err(match field {
                "to" => "to_required",
                "cc" => "cc_required",
                "bcc" => "bcc_required",
                _ => "recipients_required",
            })
        } else {
            Ok(())
        };
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

pub fn validate_subject(subject: &str) -> Result<(), &'static str> {
    if subject.chars().count() > MAX_SUBJECT_CHARS || subject.contains(['\r', '\n']) {
        Err("invalid_subject")
    } else {
        Ok(())
    }
}

pub fn validate_body(body: &str) -> Result<(), &'static str> {
    if body.len() > MAX_BODY_BYTES {
        Err("body_too_large")
    } else {
        Ok(())
    }
}

pub fn looks_like_email(address: &str) -> bool {
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

pub fn reply_subject(subject: &str) -> String {
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

fn apply_headers(
    email: &mut hail_jmap::jmap_client::email::Email<hail_jmap::jmap_client::Set>,
    headers: &OutboundHeaders<'_>,
) {
    email
        .from([headers.from])
        .to(headers.to.iter().map(String::as_str))
        .subject(headers.subject.to_string());
    if !headers.cc.is_empty() {
        email.cc(headers.cc.iter().map(String::as_str));
    }
    if !headers.bcc.is_empty() {
        email.bcc(headers.bcc.iter().map(String::as_str));
    }
    if let Some(reply) = headers.reply {
        if !reply.in_reply_to.is_empty() {
            email.in_reply_to(reply.in_reply_to.iter().map(String::as_str));
        }
        if !reply.references.is_empty() {
            email.references(reply.references.iter().map(String::as_str));
        }
    }
}

async fn required_identity_id_for<E>(session: &hail_jmap::Session, from: &str) -> Result<String, E>
where
    E: ProviderError,
{
    matching_identity_id::<E>(session, from)
        .await?
        .ok_or_else(E::sender_identity_unavailable)
}

async fn ensure_sender_identity<E>(session: &hail_jmap::Session, from: &str) -> Result<(), E>
where
    E: ProviderError,
{
    required_identity_id_for::<E>(session, from)
        .await
        .map(|_| ())
}

async fn matching_identity_id<E>(
    session: &hail_jmap::Session,
    from: &str,
) -> Result<Option<String>, E>
where
    E: ProviderError,
{
    let mut request = session.client().build();
    request.get_identity().properties([
        hail_jmap::jmap_client::identity::Property::Id,
        hail_jmap::jmap_client::identity::Property::Email,
    ]);
    let mut response = request
        .send_get_identity()
        .await
        .map_err(provider_error::<E>)?;
    Ok(response.take_list().into_iter().find_map(|mut identity| {
        let matches_from = identity
            .email()
            .is_some_and(|email| email.eq_ignore_ascii_case(from));
        let id = identity.take_id();
        (matches_from && !id.is_empty()).then_some(id)
    }))
}

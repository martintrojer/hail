use std::collections::HashMap;

use hail_core::MailClassification;
use hail_core::mail_render::{html_fragment_to_text, sanitize_and_strip_trackers};
use hail_jmap::jmap_client::email::Property;
use secrecy::SecretString;
use serde::Serialize;

use crate::state::AppState;

pub const MAIL_VIEW_PROPERTIES: &[Property] = &[
    Property::Id,
    Property::ThreadId,
    Property::From,
    Property::To,
    Property::Cc,
    Property::Bcc,
    Property::Subject,
    Property::Preview,
    Property::HtmlBody,
    Property::TextBody,
    Property::BodyValues,
    Property::ReceivedAt,
    Property::Keywords,
];

#[derive(Debug, Clone, Serialize)]
pub struct ThreadPreview {
    pub from: String,
    pub subject: String,
    pub preview: String,
}

pub fn looks_like_jmap_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn validate_thread_id(thread_id: &str) -> Result<(), axum::response::Response> {
    looks_like_jmap_id(thread_id)
        .then_some(())
        .ok_or_else(|| crate::routes::response::bad_request("invalid_thread_id"))
}

pub async fn thread_action_response<F, Fut>(
    user: &crate::middleware::auth::AuthUser,
    thread_id: &str,
    action: F,
) -> axum::response::Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<
            Output = Result<
                Option<crate::routes::undo::UndoToken>,
                crate::routes::threads::ThreadActionError,
            >,
        >,
{
    use axum::Json;
    use axum::response::IntoResponse;

    if let Err(response) = validate_thread_id(thread_id) {
        return response;
    }

    match action().await {
        Ok(undo) => Json(crate::routes::threads::ThreadVerbResponse { undo }).into_response(),
        Err(crate::routes::threads::ThreadActionError::NotFound) => {
            crate::routes::response::not_found("not_found")
        }
        Err(crate::routes::threads::ThreadActionError::Provider(err)) => {
            tracing::warn!(user_id = user.id, thread_id, error = %err, "thread action failed");
            crate::routes::response::internal()
        }
    }
}

pub trait ProviderError {
    fn provider(message: String) -> Self;

    fn sender_identity_unavailable() -> Self
    where
        Self: Sized,
    {
        Self::provider("sender identity not available".to_string())
    }
}

pub fn provider_error<E>(err: impl std::fmt::Display) -> E
where
    E: ProviderError,
{
    E::provider(err.to_string())
}

pub async fn jmap_session(
    state: &AppState,
    token: SecretString,
) -> Result<hail_jmap::Session, String> {
    hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
        .await
        .map_err(|err| err.to_string())
}

pub async fn email_ids_in_thread(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;

    let mut query = session
        .client()
        .email_query(
            Some(Filter::from(email_query::Filter::in_thread(thread_id))),
            None::<Vec<hail_jmap::jmap_client::core::query::Comparator<email_query::Comparator>>>,
        )
        .await
        .map_err(|err| err.to_string())?;
    Ok(query.take_ids())
}

pub async fn require_email_ids_in_thread(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let ids = email_ids_in_thread(session, thread_id).await?;
    if ids.is_empty() {
        return Err("thread_not_found".to_string());
    }
    Ok(ids)
}

pub async fn set_thread_keyword(
    session: &hail_jmap::Session,
    thread_id: &str,
    keyword: &str,
    enabled: bool,
) -> Result<(), String> {
    for email_id in require_email_ids_in_thread(session, thread_id).await? {
        session
            .client()
            .email_set_keyword(&email_id, keyword, enabled)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub async fn set_thread_keywords(
    session: &hail_jmap::Session,
    thread_id: &str,
    keywords: impl IntoIterator<Item = (&'static str, bool)> + Clone,
) -> Result<(), String> {
    let email_ids = require_email_ids_in_thread(session, thread_id).await?;
    for email_id in email_ids {
        for (keyword, enabled) in keywords.clone() {
            session
                .client()
                .email_set_keyword(&email_id, keyword, enabled)
                .await
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

pub async fn set_thread_mailboxes(
    session: &hail_jmap::Session,
    thread_id: &str,
    mailbox_ids: impl IntoIterator<Item = String> + Clone,
) -> Result<(), String> {
    for email_id in require_email_ids_in_thread(session, thread_id).await? {
        session
            .client()
            .email_set_mailboxes(&email_id, mailbox_ids.clone())
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub async fn required_drafts_mailbox_id(session: &hail_jmap::Session) -> Result<String, String> {
    hail_jmap::mailbox_id_by_role(session, hail_jmap::jmap_client::mailbox::Role::Drafts)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "drafts mailbox not found".to_string())
}

pub async fn trash_mailbox_id(session: &hail_jmap::Session) -> Result<Option<String>, String> {
    hail_jmap::mailbox_id_by_role(session, hail_jmap::jmap_client::mailbox::Role::Trash)
        .await
        .map_err(|err| err.to_string())
}

pub async fn move_thread_to_role(
    state: &AppState,
    token: SecretString,
    thread_id: &str,
    role: crate::routes::threads::MailboxRole,
) -> Result<(), String> {
    let session = jmap_session(state, token).await?;
    let mailbox_id = hail_jmap::mailbox_id_by_role(&session, role.jmap())
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("{} mailbox not found", role.name()))?;
    set_thread_mailboxes(&session, thread_id, [mailbox_id]).await
}

pub async fn hydrate_thread_previews(
    state: &AppState,
    user_id: i64,
    token: SecretString,
    context: &'static str,
    thread_ids: impl IntoIterator<Item = String>,
) -> HashMap<String, ThreadPreview> {
    let thread_ids = thread_ids.into_iter().collect::<Vec<_>>();
    if thread_ids.is_empty() {
        return HashMap::new();
    }

    let session = match jmap_session(state, token).await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(
                user_id,
                context,
                error = %err,
                "thread preview JMAP login failed; returning empty previews"
            );
            return HashMap::new();
        }
    };

    match latest_thread_previews(&session, thread_ids.clone()).await {
        Ok(previews) => previews,
        Err(err) => {
            tracing::warn!(
                user_id,
                context,
                error = %err,
                "thread preview JMAP hydration failed; returning empty previews"
            );
            HashMap::new()
        }
    }
}

pub async fn latest_thread_preview(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Option<ThreadPreview>, String> {
    Ok(latest_thread_previews(session, [thread_id.to_string()])
        .await?
        .remove(thread_id))
}

pub async fn latest_thread_previews(
    session: &hail_jmap::Session,
    thread_ids: impl IntoIterator<Item = String>,
) -> Result<HashMap<String, ThreadPreview>, String> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::query as email_query;

    let thread_ids = thread_ids.into_iter().collect::<Vec<_>>();
    if thread_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let filter = if thread_ids.len() == 1 {
        Filter::from(email_query::Filter::in_thread(thread_ids[0].clone()))
    } else {
        Filter::or(
            thread_ids
                .iter()
                .cloned()
                .map(email_query::Filter::in_thread)
                .map(Filter::from),
        )
    };

    let mut request = session.client().build();
    request
        .query_email()
        .filter(filter)
        .sort([email_query::Comparator::received_at().descending()])
        .limit(thread_ids.len())
        .arguments()
        .collapse_threads(true);
    let mut query = request
        .send_query_email()
        .await
        .map_err(|err| err.to_string())?;
    let preview_email_ids = query.take_ids();
    if preview_email_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut request = session.client().build();
    request.get_email().ids(preview_email_ids).properties([
        Property::Id,
        Property::ThreadId,
        Property::From,
        Property::Subject,
        Property::Preview,
    ]);
    let mut response = request
        .send_get_email()
        .await
        .map_err(|err| err.to_string())?;

    let mut previews = HashMap::with_capacity(thread_ids.len());
    for email in response.take_list() {
        let Some(thread_id) = email.thread_id() else {
            continue;
        };
        previews.insert(
            thread_id.to_string(),
            ThreadPreview {
                from: format_from(email.from()),
                subject: email.subject().unwrap_or_default().to_string(),
                preview: preview_from_email(&email),
            },
        );
    }

    Ok(previews)
}

pub async fn clear_thread_state(
    state: &AppState,
    actions: &dyn crate::routes::threads::ThreadActions,
    token: SecretString,
    user_id: i64,
    thread_id: &str,
) {
    for classification in MailClassification::ALL {
        let _ = actions
            .set_keyword(
                state,
                token.clone(),
                thread_id,
                classification.keyword(),
                false,
            )
            .await;
    }
    for keyword in ["$hail_setaside", "$hail_replylater"] {
        let _ = actions
            .set_keyword(state, token.clone(), thread_id, keyword, false)
            .await;
    }
    if let Err(err) = hail_db::clear_thread_sidecar_state(&state.db, user_id, thread_id).await {
        tracing::warn!(user_id, thread_id = %thread_id, error = %err, "thread sidecar cleanup failed");
    }
}

pub fn preview_from_email(email: &hail_jmap::jmap_client::email::Email) -> String {
    let jmap_preview = collapse_preview_whitespace(email.preview().unwrap_or_default());
    if !jmap_preview.is_empty() {
        return jmap_preview;
    }

    if let Some(text) = body_from_parts(email, email.text_body()) {
        let preview = collapse_preview_whitespace(&text);
        if !preview.is_empty() {
            return preview;
        }
    }

    if let Some(html) = body_from_parts(email, email.html_body()) {
        let sanitized = sanitize_and_strip_trackers(&html);
        let preview = collapse_preview_whitespace(&html_fragment_to_text(&sanitized.html));
        if !preview.is_empty() {
            return preview;
        }
    }

    String::new()
}

fn body_from_parts(
    email: &hail_jmap::jmap_client::email::Email,
    parts: Option<&[hail_jmap::jmap_client::email::EmailBodyPart]>,
) -> Option<String> {
    let mut body = String::new();
    for part in parts? {
        let Some(part_id) = part.part_id() else {
            continue;
        };
        let Some(value) = email.body_value(part_id) else {
            continue;
        };
        body.push_str(value.value());
    }
    Some(body)
}

fn collapse_preview_whitespace(input: &str) -> String {
    let mut preview = String::new();
    for word in input.split_whitespace() {
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(word);
        if preview.chars().count() >= 200 {
            return cap_preview(preview);
        }
    }
    cap_preview(preview)
}

fn cap_preview(input: String) -> String {
    let mut capped = String::new();
    for ch in input.chars().take(200) {
        capped.push(ch);
    }
    capped
}

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(|address| match address.name() {
            Some(name) if !name.is_empty() => format!("{} <{}>", name, address.email()),
            _ => address.email().to_string(),
        })
        .unwrap_or_default()
}

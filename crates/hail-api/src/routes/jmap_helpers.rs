use std::collections::HashMap;

use secrecy::SecretString;
use serde::Serialize;

use crate::state::AppState;

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

pub async fn drafts_mailbox_id(session: &hail_jmap::Session) -> Result<Option<String>, String> {
    hail_jmap::mailbox_id_by_role(session, hail_jmap::jmap_client::mailbox::Role::Drafts)
        .await
        .map_err(|err| err.to_string())
}

pub async fn required_drafts_mailbox_id(session: &hail_jmap::Session) -> Result<String, String> {
    drafts_mailbox_id(session)
        .await?
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

    let mut previews = HashMap::with_capacity(thread_ids.len());
    for thread_id in thread_ids {
        match latest_thread_preview(&session, &thread_id).await {
            Ok(Some(preview)) => {
                previews.insert(thread_id, preview);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    user_id,
                    context,
                    thread_id = %thread_id,
                    error = %err,
                    "thread preview JMAP hydration failed; leaving preview empty"
                );
            }
        }
    }
    previews
}

pub async fn latest_thread_preview(
    session: &hail_jmap::Session,
    thread_id: &str,
) -> Result<Option<ThreadPreview>, String> {
    use hail_jmap::jmap_client::core::query::Filter;
    use hail_jmap::jmap_client::email::Property;
    use hail_jmap::jmap_client::email::query as email_query;

    let mut request = session.client().build();
    request
        .query_email()
        .filter(Filter::from(email_query::Filter::in_thread(thread_id)))
        .sort([email_query::Comparator::received_at().descending()])
        .limit(1);
    let mut query = request
        .send_query_email()
        .await
        .map_err(|err| err.to_string())?;
    let Some(email_id) = query.take_ids().into_iter().next() else {
        return Ok(None);
    };

    let mut request = session.client().build();
    request.get_email().ids([email_id]).properties([
        Property::From,
        Property::Subject,
        Property::Preview,
    ]);
    let mut response = request
        .send_get_email()
        .await
        .map_err(|err| err.to_string())?;
    let Some(email) = response.take_list().into_iter().next() else {
        return Ok(None);
    };

    Ok(Some(ThreadPreview {
        from: format_from(email.from()),
        subject: email.subject().unwrap_or_default().to_string(),
        preview: email.preview().unwrap_or_default().to_string(),
    }))
}

pub async fn clear_thread_state(
    state: &AppState,
    actions: &dyn crate::routes::threads::ThreadActions,
    token: SecretString,
    user_id: i64,
    thread_id: &str,
) {
    for classification in crate::routes::threads::Classification::ALL {
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
    let _ = sqlx::query("DELETE FROM stack_positions WHERE user_id = ?1 AND thread_id = ?2")
        .bind(user_id)
        .bind(thread_id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query(
        "DELETE FROM bubble_ups WHERE user_id = ?1 AND thread_id = ?2 AND fired_at IS NULL",
    )
    .bind(user_id)
    .bind(thread_id)
    .execute(&state.db)
    .await;
}

fn format_from(from: Option<&[hail_jmap::jmap_client::email::EmailAddress]>) -> String {
    from.and_then(|addresses| addresses.first())
        .map(|address| match address.name() {
            Some(name) if !name.is_empty() => format!("{} <{}>", name, address.email()),
            _ => address.email().to_string(),
        })
        .unwrap_or_default()
}

//! Draft autosave endpoints.
//!
//! The SPA posts composer snapshots here; handlers validate the public JSON
//! surface, then delegate storage to an injectable [`DraftStore`]. The
//! production store writes text-only drafts through JMAP `Email/set` with the
//! `$draft` keyword and the user's Drafts mailbox. Tests inject a fake store so
//! auth/CSRF/validation/provider failures are covered without Stalwart.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use chrono::{DateTime, Utc};
use hail_jmap::jmap_client::core::set::SetObject;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const MAX_SUBJECT_CHARS: usize = 998;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_RECIPIENTS_PER_FIELD: usize = 200;

pub trait DraftStore: Send + Sync + 'static {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>>;

    fn update<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>>;
}

pub struct JmapDraftStore;

impl DraftStore for JmapDraftStore {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let session = login(state, token).await?;
            let drafts_mailbox_id = drafts_mailbox_id(&session).await?;

            let mut request = session.client().build();
            let create_id = {
                let email = request.set_email().create();
                email
                    .mailbox_ids([drafts_mailbox_id])
                    .keywords(["$draft"])
                    .from([from])
                    .to(draft.to.iter().map(String::as_str))
                    .subject(draft.subject);
                if !draft.cc.is_empty() {
                    email.cc(draft.cc.iter().map(String::as_str));
                }
                if !draft.bcc.is_empty() {
                    email.bcc(draft.bcc.iter().map(String::as_str));
                }
                set_text_body(email, draft.body_markdown);
                email.create_id().ok_or_else(|| {
                    DraftStoreError::Provider("Email/set create id missing".to_string())
                })?
            };

            let mut response = request.send_set_email().await.map_err(provider_error)?;
            let mut created = response.created(&create_id).map_err(provider_error)?;
            let id = created.take_id();
            if id.is_empty() {
                Err(DraftStoreError::Provider(
                    "Email/set created draft without id".to_string(),
                ))
            } else {
                Ok(id)
            }
        })
    }

    fn update<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let session = login(state, token).await?;
            let mut request = session.client().build();
            let email = request.set_email().update(draft_id);

            if let Some(to) = draft.to {
                email.to(to.iter().map(String::as_str));
            }
            if let Some(cc) = draft.cc {
                email.cc(cc.iter().map(String::as_str));
            }
            if let Some(bcc) = draft.bcc {
                email.bcc(bcc.iter().map(String::as_str));
            }
            if let Some(subject) = draft.subject {
                email.subject(subject);
            }
            if let Some(body_markdown) = draft.body_markdown {
                set_text_body(email, body_markdown);
            }

            let mut response = request.send_set_email().await.map_err(provider_error)?;
            response.updated(draft_id).map_err(provider_error)?;
            Ok(())
        })
    }
}

#[derive(Debug)]
pub enum DraftStoreError {
    Provider(String),
}

#[derive(Debug, Clone)]
pub struct DraftCreate {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_markdown: String,
}

#[derive(Debug, Clone, Default)]
pub struct DraftUpdate {
    pub to: Option<Vec<String>>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub subject: Option<String>,
    pub body_markdown: Option<String>,
}

pub fn router() -> Router<AppState> {
    router_with_store(Arc::new(JmapDraftStore))
}

pub fn router_with_store<S>(store: Arc<S>) -> Router<AppState>
where
    S: DraftStore,
{
    Router::new()
        .route("/api/drafts", post(create_draft::<S>))
        .route(
            "/api/drafts/{draft_id}",
            axum::routing::patch(update_draft::<S>),
        )
        .layer(Extension(store))
}

#[derive(Debug, Deserialize)]
struct DraftPayload {
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: Option<String>,
    body_markdown: Option<String>,
    attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct DraftResponse {
    draft_id: String,
    updated_at: DateTime<Utc>,
}

async fn create_draft<S>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(store): Extension<Arc<S>>,
    body: Result<Json<DraftPayload>, JsonRejection>,
) -> Response
where
    S: DraftStore,
{
    let Ok(Json(payload)) = body else {
        return bad_request("invalid_json");
    };
    let draft = match payload.into_create() {
        Ok(draft) => draft,
        Err(error) => return bad_request(error),
    };

    match store
        .create(&state, user.jmap_token.clone(), &user.email, draft)
        .await
    {
        Ok(draft_id) => (
            StatusCode::CREATED,
            Json(DraftResponse {
                draft_id,
                updated_at: Utc::now(),
            }),
        )
            .into_response(),
        Err(DraftStoreError::Provider(err)) => provider_failed(user.id, err),
    }
}

async fn update_draft<S>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(store): Extension<Arc<S>>,
    Path(draft_id): Path<String>,
    body: Result<Json<DraftPayload>, JsonRejection>,
) -> Response
where
    S: DraftStore,
{
    if !looks_like_jmap_id(&draft_id) {
        return bad_request("invalid_draft_id");
    }
    let Ok(Json(payload)) = body else {
        return bad_request("invalid_json");
    };
    let draft = match payload.into_update() {
        Ok(draft) => draft,
        Err(error) => return bad_request(error),
    };

    match store
        .update(&state, user.jmap_token.clone(), &draft_id, draft)
        .await
    {
        Ok(()) => Json(DraftResponse {
            draft_id,
            updated_at: Utc::now(),
        })
        .into_response(),
        Err(DraftStoreError::Provider(err)) => provider_failed(user.id, err),
    }
}

impl DraftPayload {
    fn into_create(self) -> Result<DraftCreate, &'static str> {
        validate_attachments(&self.attachments)?;
        let to = self.to.unwrap_or_default();
        let cc = self.cc.unwrap_or_default();
        let bcc = self.bcc.unwrap_or_default();
        let subject = self.subject.unwrap_or_default();
        let body_markdown = self.body_markdown.unwrap_or_default();

        validate_recipients("to", &to)?;
        validate_recipients("cc", &cc)?;
        validate_recipients("bcc", &bcc)?;
        validate_subject(&subject)?;
        validate_body(&body_markdown)?;

        Ok(DraftCreate {
            to,
            cc,
            bcc,
            subject,
            body_markdown,
        })
    }

    fn into_update(self) -> Result<DraftUpdate, &'static str> {
        validate_attachments(&self.attachments)?;
        if let Some(to) = &self.to {
            validate_recipients("to", to)?;
        }
        if let Some(cc) = &self.cc {
            validate_recipients("cc", cc)?;
        }
        if let Some(bcc) = &self.bcc {
            validate_recipients("bcc", bcc)?;
        }
        if let Some(subject) = &self.subject {
            validate_subject(subject)?;
        }
        if let Some(body_markdown) = &self.body_markdown {
            validate_body(body_markdown)?;
        }

        if self.to.is_none()
            && self.cc.is_none()
            && self.bcc.is_none()
            && self.subject.is_none()
            && self.body_markdown.is_none()
        {
            return Err("empty_patch");
        }

        Ok(DraftUpdate {
            to: self.to,
            cc: self.cc,
            bcc: self.bcc,
            subject: self.subject,
            body_markdown: self.body_markdown,
        })
    }
}

fn validate_attachments(attachments: &Option<Vec<serde_json::Value>>) -> Result<(), &'static str> {
    match attachments {
        Some(attachments) if !attachments.is_empty() => Err("attachments_not_supported"),
        _ => Ok(()),
    }
}

fn validate_recipients(field: &'static str, recipients: &[String]) -> Result<(), &'static str> {
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

fn looks_like_jmap_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

async fn login(
    state: &AppState,
    token: SecretString,
) -> Result<hail_jmap::Session, DraftStoreError> {
    hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
        .await
        .map_err(provider_error)
}

async fn drafts_mailbox_id(session: &hail_jmap::Session) -> Result<String, DraftStoreError> {
    use hail_jmap::jmap_client::mailbox::{Role, query::Filter};

    let mut response = session
        .client()
        .mailbox_query(Some(Filter::role(Role::Drafts)), None::<Vec<_>>)
        .await
        .map_err(provider_error)?;
    response
        .take_ids()
        .into_iter()
        .next()
        .ok_or_else(|| DraftStoreError::Provider("drafts mailbox not found".to_string()))
}

fn set_text_body(
    email: &mut hail_jmap::jmap_client::email::Email<hail_jmap::jmap_client::Set>,
    body: String,
) {
    use hail_jmap::jmap_client::email::EmailBodyPart;

    email
        .text_body(
            EmailBodyPart::new()
                .part_id("text")
                .content_type("text/plain"),
        )
        .body_value("text".to_string(), body);
}

fn provider_error(err: impl std::fmt::Display) -> DraftStoreError {
    DraftStoreError::Provider(err.to_string())
}

fn provider_failed(user_id: i64, err: String) -> Response {
    tracing::warn!(user_id, error = %err, "draft store failed");
    internal()
}

fn bad_request(error: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"error":"{error}"}}"#),
    )
        .into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}

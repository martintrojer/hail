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
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use hail_jmap::jmap_client::core::set::SetObject;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::{
    jmap_session, looks_like_jmap_id, provider_error, required_drafts_mailbox_id,
};
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

const MAX_SUBJECT_CHARS: usize = 998;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_RECIPIENTS_PER_FIELD: usize = 200;

/// OpenAPI tag for draft autosave endpoints.
pub const TAG: &str = "drafts";

pub trait DraftStore: Send + Sync + 'static {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>>;

    fn get<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DraftDetails>, DraftStoreError>> + Send + 'a>>;

    fn update<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>>;

    fn delete<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
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

    fn get<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DraftDetails>, DraftStoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            let mut request = session.client().build();
            let get_email = request.get_email();
            get_email.ids([draft_id]).properties([
                hail_jmap::jmap_client::email::Property::Id,
                hail_jmap::jmap_client::email::Property::Keywords,
                hail_jmap::jmap_client::email::Property::To,
                hail_jmap::jmap_client::email::Property::Cc,
                hail_jmap::jmap_client::email::Property::Bcc,
                hail_jmap::jmap_client::email::Property::Subject,
                hail_jmap::jmap_client::email::Property::TextBody,
                hail_jmap::jmap_client::email::Property::BodyValues,
            ]);
            get_email
                .arguments()
                .fetch_text_body_values(true)
                .max_body_value_bytes(MAX_BODY_BYTES);

            let mut response = request.send_get_email().await.map_err(provider_error)?;
            let Some(email) = response.take_list().pop() else {
                return Ok(None);
            };
            if !email
                .keywords()
                .into_iter()
                .any(|keyword| keyword == "$draft")
            {
                return Ok(None);
            }

            Ok(Some(DraftDetails {
                draft_id: draft_id.to_string(),
                to: addresses_from_jmap(email.to()),
                cc: addresses_from_jmap(email.cc()),
                bcc: addresses_from_jmap(email.bcc()),
                subject: email.subject().unwrap_or_default().to_string(),
                body_markdown: text_body_from_email(&email),
            }))
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
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            ensure_draft_email(&session, draft_id).await?;

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

    fn delete<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token).await.map_err(provider_error)?;
            ensure_draft_email(&session, draft_id).await?;
            session
                .client()
                .email_destroy(draft_id)
                .await
                .map_err(provider_error)
        })
    }
}

#[derive(Debug)]
pub enum DraftStoreError {
    NotFound,
    Provider(String),
}

impl crate::routes::jmap_helpers::ProviderError for DraftStoreError {
    fn provider(message: String) -> Self {
        Self::Provider(message)
    }
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DraftDetails {
    pub draft_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_markdown: String,
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_store(Arc::new(JmapDraftStore)))
}

/// Build the OpenAPI-tracked router for the production draft store.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_store(Arc::new(JmapDraftStore))
}

pub fn router_with_store<S>(store: Arc<S>) -> Router<AppState>
where
    S: DraftStore,
{
    Router::from(openapi_router_with_store(store))
}

fn openapi_router_with_store<S>(store: Arc<S>) -> OpenApiRouter<AppState>
where
    S: DraftStore,
{
    let store: Arc<dyn DraftStore> = store;
    OpenApiRouter::new()
        .routes(routes!(create_draft).layer(Extension(store.clone())))
        .routes(routes!(get_draft).layer(Extension(store.clone())))
        .routes(routes!(update_draft).layer(Extension(store.clone())))
        .routes(routes!(delete_draft).layer(Extension(store)))
}

#[derive(Debug, Deserialize, ToSchema)]
struct DraftPayload {
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    bcc: Option<Vec<String>>,
    subject: Option<String>,
    body_markdown: Option<String>,
    attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize, ToSchema)]
struct DraftResponse {
    draft_id: String,
    #[schema(value_type = String, format = DateTime)]
    updated_at: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/api/drafts",
    tag = TAG,
    request_body(content = DraftPayload, content_type = "application/json"),
    responses(
        (status = 201, description = "Draft created or autosaved.", body = DraftResponse),
        (status = 400, description = "Invalid draft payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "JMAP draft store failure."),
    ),
)]
async fn create_draft(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(store): Extension<Arc<dyn DraftStore>>,
    body: Result<Json<DraftPayload>, JsonRejection>,
) -> Response {
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
        Err(DraftStoreError::NotFound) => not_found(),
        Err(DraftStoreError::Provider(err)) => provider_failed(user.id, err),
    }
}

#[utoipa::path(
    get,
    path = "/api/drafts/{draft_id}",
    tag = TAG,
    params(
        ("draft_id" = String, Path, description = "JMAP draft email id to fetch."),
    ),
    responses(
        (status = 200, description = "Draft details for composer resume.", body = DraftDetails),
        (status = 400, description = "Invalid draft id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Draft not found or no longer a draft."),
        (status = 500, description = "JMAP draft store failure."),
    ),
)]
async fn get_draft(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(store): Extension<Arc<dyn DraftStore>>,
    Path(draft_id): Path<String>,
) -> Response {
    if !looks_like_jmap_id(&draft_id) {
        return bad_request("invalid_draft_id");
    }

    match store.get(&state, user.jmap_token.clone(), &draft_id).await {
        Ok(Some(draft)) => Json(draft).into_response(),
        Ok(None) => not_found(),
        Err(DraftStoreError::NotFound) => not_found(),
        Err(DraftStoreError::Provider(err)) => provider_failed(user.id, err),
    }
}

#[utoipa::path(
    patch,
    path = "/api/drafts/{draft_id}",
    tag = TAG,
    params(
        ("draft_id" = String, Path, description = "JMAP draft email id to update."),
    ),
    request_body(content = DraftPayload, content_type = "application/json"),
    responses(
        (status = 200, description = "Draft updated.", body = DraftResponse),
        (status = 400, description = "Invalid draft id or payload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Draft not found or no longer a draft."),
        (status = 500, description = "JMAP draft store failure."),
    ),
)]
async fn update_draft(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(store): Extension<Arc<dyn DraftStore>>,
    Path(draft_id): Path<String>,
    body: Result<Json<DraftPayload>, JsonRejection>,
) -> Response {
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
        Err(DraftStoreError::NotFound) => not_found(),
        Err(DraftStoreError::Provider(err)) => provider_failed(user.id, err),
    }
}

#[utoipa::path(
    delete,
    path = "/api/drafts/{draft_id}",
    tag = TAG,
    params(
        ("draft_id" = String, Path, description = "JMAP draft email id to delete."),
    ),
    responses(
        (status = 204, description = "Draft deleted."),
        (status = 400, description = "Invalid draft id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Draft not found or no longer a draft."),
        (status = 500, description = "JMAP draft store failure."),
    ),
)]
async fn delete_draft(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(store): Extension<Arc<dyn DraftStore>>,
    Path(draft_id): Path<String>,
) -> Response {
    if !looks_like_jmap_id(&draft_id) {
        return bad_request("invalid_draft_id");
    }

    match store
        .delete(&state, user.jmap_token.clone(), &draft_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(DraftStoreError::NotFound) => not_found(),
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

async fn ensure_draft_email(
    session: &hail_jmap::Session,
    draft_id: &str,
) -> Result<(), DraftStoreError> {
    let mut request = session.client().build();
    request.get_email().ids([draft_id]).properties([
        hail_jmap::jmap_client::email::Property::Id,
        hail_jmap::jmap_client::email::Property::Keywords,
    ]);
    let mut response = request.send_get_email().await.map_err(provider_error)?;
    let Some(email) = response.take_list().pop() else {
        return Err(DraftStoreError::NotFound);
    };
    if email
        .keywords()
        .into_iter()
        .any(|keyword| keyword == "$draft")
    {
        Ok(())
    } else {
        Err(DraftStoreError::NotFound)
    }
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

fn addresses_from_jmap(
    addresses: Option<&[hail_jmap::jmap_client::email::EmailAddress]>,
) -> Vec<String> {
    addresses
        .unwrap_or_default()
        .iter()
        .map(|address| address.email().to_string())
        .filter(|address| !address.is_empty())
        .collect()
}

fn text_body_from_email(email: &hail_jmap::jmap_client::email::Email) -> String {
    let Some(parts) = email.text_body() else {
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

fn provider_failed(user_id: i64, err: String) -> Response {
    tracing::warn!(user_id, error = %err, "draft store failed");
    internal()
}

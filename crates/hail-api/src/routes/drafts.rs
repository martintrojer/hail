//! Draft autosave endpoints.
//!
//! The SPA posts composer snapshots here; handlers validate the public JSON
//! surface, then delegate storage to an injectable [`DraftStore`]. The
//! production store writes sanitized HTML drafts through JMAP `Email/set` with
//! the `$draft` keyword and the user's Drafts mailbox. Tests inject a fake store so
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
use hail_core::mail_render::sanitize_outgoing_html;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::looks_like_jmap_id;
use crate::routes::outbound::{
    RenderedBody, html_to_plaintext, render_markdown, rendered_html, validate_attachments,
    validate_body, validate_recipients, validate_subject,
};
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

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

pub struct CacheDraftStore;

pub struct JmapDraftStore;

impl DraftStore for CacheDraftStore {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            state.mail.create_draft(draft_payload(from, draft)).await
                .map(|id| id.as_str().to_owned())
                .map_err(|err| DraftStoreError::Provider(err.to_string()))
        })
    }

    fn get<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DraftDetails>, DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            state.mail.get_draft(&hail_backend::BackendMsgId::new(draft_id.to_owned())).await
                .map(|draft| draft.map(|draft| DraftDetails {
                    draft_id: draft.id.as_str().to_owned(),
                    to: draft.to,
                    cc: draft.cc,
                    bcc: draft.bcc,
                    subject: draft.subject,
                    body_html: sanitize_outgoing_html(&draft.body_html),
                    body_markdown: draft.body_markdown,
                }))
                .map_err(|err| DraftStoreError::Provider(err.to_string()))
        })
    }

    fn update<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let existing = state.mail.get_draft(&hail_backend::BackendMsgId::new(draft_id.to_owned())).await
                .map_err(|err| DraftStoreError::Provider(err.to_string()))?
                .ok_or(DraftStoreError::NotFound)?;
            let body_markdown = draft.body_markdown.unwrap_or(existing.body_markdown);
            let payload = hail_cache::DraftPayload {
                from: String::new(),
                to: draft.to.unwrap_or(existing.to),
                cc: draft.cc.unwrap_or(existing.cc),
                bcc: draft.bcc.unwrap_or(existing.bcc),
                subject: draft.subject.unwrap_or(existing.subject),
                plain_text: body_markdown.clone(),
                html: draft.body_html.unwrap_or(existing.body_html),
                body_markdown,
            };
            state.mail.update_draft(&hail_backend::BackendMsgId::new(draft_id.to_owned()), payload).await
                .map_err(|err| DraftStoreError::Provider(err.to_string()))
        })
    }

    fn delete<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        Box::pin(async move {
            state.mail.delete_draft(&hail_backend::BackendMsgId::new(draft_id.to_owned())).await
                .map_err(|err| DraftStoreError::Provider(err.to_string()))
        })
    }
}

fn draft_payload(from: &str, draft: DraftCreate) -> hail_cache::DraftPayload {
    hail_cache::DraftPayload {
        from: from.to_owned(),
        to: draft.to,
        cc: draft.cc,
        bcc: draft.bcc,
        subject: draft.subject,
        plain_text: draft.body.plain_text,
        html: draft.body_html,
        body_markdown: draft.body_markdown,
    }
}

impl DraftStore for JmapDraftStore {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        from: &'a str,
        draft: DraftCreate,
    ) -> Pin<Box<dyn Future<Output = Result<String, DraftStoreError>> + Send + 'a>> {
        CacheDraftStore.create(state, token, from, draft)
    }

    fn get<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DraftDetails>, DraftStoreError>> + Send + 'a>> {
        CacheDraftStore.get(state, token, draft_id)
    }

    fn update<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
        draft: DraftUpdate,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        CacheDraftStore.update(state, token, draft_id, draft)
    }

    fn delete<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        draft_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DraftStoreError>> + Send + 'a>> {
        CacheDraftStore.delete(state, token, draft_id)
    }
}

#[derive(Debug)]
pub enum DraftStoreError {
    NotFound,
    SenderIdentityUnavailable,
    Provider(String),
}

impl crate::routes::jmap_helpers::ProviderError for DraftStoreError {
    fn provider(message: String) -> Self {
        Self::Provider(message)
    }

    fn sender_identity_unavailable() -> Self {
        Self::SenderIdentityUnavailable
    }
}

#[derive(Debug, Clone)]
pub struct DraftCreate {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: crate::routes::outbound::RenderedBody,
    pub body_markdown: String,
    pub body_html: String,
}

#[derive(Debug, Clone, Default)]
pub struct DraftUpdate {
    pub to: Option<Vec<String>>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub subject: Option<String>,
    pub body: Option<crate::routes::outbound::RenderedBody>,
    pub body_markdown: Option<String>,
    pub body_html: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DraftDetails {
    pub draft_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_html: String,
    pub body_markdown: String,
}

pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_store(Arc::new(CacheDraftStore)))
}

/// Build the OpenAPI-tracked router for the production draft store.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_store(Arc::new(CacheDraftStore))
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
    body_html: Option<String>,
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
        Err(DraftStoreError::NotFound) => not_found("not_found"),
        Err(DraftStoreError::SenderIdentityUnavailable) => {
            bad_request("sender_identity_unavailable")
        }
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
        Ok(None) => not_found("not_found"),
        Err(DraftStoreError::NotFound) => not_found("not_found"),
        Err(DraftStoreError::SenderIdentityUnavailable) => {
            bad_request("sender_identity_unavailable")
        }
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
        Err(DraftStoreError::NotFound) => not_found("not_found"),
        Err(DraftStoreError::SenderIdentityUnavailable) => {
            bad_request("sender_identity_unavailable")
        }
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
        Err(DraftStoreError::NotFound) => not_found("not_found"),
        Err(DraftStoreError::SenderIdentityUnavailable) => {
            bad_request("sender_identity_unavailable")
        }
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
        let body = render_draft_body(self.body_html.as_deref(), self.body_markdown.as_deref())?;
        let body_markdown = html_to_plaintext(&body.html);
        let body_html = body.html.clone();
        validate_recipients("to", &to, false)?;
        validate_recipients("cc", &cc, false)?;
        validate_recipients("bcc", &bcc, false)?;
        validate_subject(&subject)?;
        Ok(DraftCreate { to, cc, bcc, subject, body, body_markdown, body_html })
    }

    fn into_update(self) -> Result<DraftUpdate, &'static str> {
        validate_attachments(&self.attachments)?;
        if let Some(to) = &self.to { validate_recipients("to", to, false)?; }
        if let Some(cc) = &self.cc { validate_recipients("cc", cc, false)?; }
        if let Some(bcc) = &self.bcc { validate_recipients("bcc", bcc, false)?; }
        if let Some(subject) = &self.subject { validate_subject(subject)?; }
        let body = if self.body_html.is_some() || self.body_markdown.is_some() {
            Some(render_draft_body(self.body_html.as_deref(), self.body_markdown.as_deref())?)
        } else { None };
        let body_markdown = body.as_ref().map(|body| html_to_plaintext(&body.html));
        let body_html = body.as_ref().map(|body| body.html.clone());
        if self.to.is_none() && self.cc.is_none() && self.bcc.is_none() && self.subject.is_none() && body.is_none() {
            return Err("empty_patch");
        }
        Ok(DraftUpdate { to: self.to, cc: self.cc, bcc: self.bcc, subject: self.subject, body, body_markdown, body_html })
    }
}

fn render_draft_body(body_html: Option<&str>, body_markdown: Option<&str>) -> Result<RenderedBody, &'static str> {
    if let Some(html) = body_html {
        validate_body(html)?;
        if !html.trim().is_empty() {
            let body = rendered_html(html);
            validate_body(&body.html)?;
            return Ok(body);
        }
    }
    if let Some(markdown) = body_markdown {
        validate_body(markdown)?;
        if !markdown.trim().is_empty() {
            let body = render_markdown(markdown);
            validate_body(&body.html)?;
            return Ok(body);
        }
    }
    Ok(RenderedBody { plain_text: String::new(), html: String::new() })
}

fn provider_failed(user_id: i64, err: String) -> Response {
    tracing::warn!(user_id, error = %err, "draft store failed");
    internal()
}

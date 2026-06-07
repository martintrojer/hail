//! Attachment listing endpoint for the All Files SPA view.
//!
//! `GET /api/attachments` asks CachedMail for recent messages with attachments and
//! returns a flat, UI-ready list of file metadata with thread/message context.
//! Blob download URLs are scoped through hail-api so the browser never talks to
//! Stalwart/JMAP directly.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::response::{bad_request, internal, not_found};
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;

/// OpenAPI tag for attachment listing and download endpoints.
pub const TAG: &str = "attachments";

pub trait AttachmentProvider: Send + Sync + 'static {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<AttachmentItem>, AttachmentError>> + Send + 'a>>;

    fn download<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        blob_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, AttachmentError>> + Send + 'a>>;
}

pub struct CacheAttachmentProvider;

impl AttachmentProvider for CacheAttachmentProvider {
    fn list<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<AttachmentItem>, AttachmentError>> + Send + 'a>>
    {
        Box::pin(async move {
            state
                .mail
                .list_attachments(limit)
                .await
                .map(|items| items.into_iter().map(attachment_item_from_cached).collect())
                .map_err(|err| AttachmentError::provider(err.to_string()))
        })
    }

    fn download<'a>(
        &'a self,
        state: &'a AppState,
        _token: SecretString,
        blob_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, AttachmentError>> + Send + 'a>> {
        Box::pin(async move {
            state
                .mail
                .get_blob(&hail_backend::BlobRef::new(blob_id.to_owned()))
                .await
                .map(|bytes| Some(bytes.to_vec()))
                .map_err(|err| AttachmentError::provider(err.to_string()))
        })
    }
}

fn attachment_item_from_cached(item: hail_cache::CachedAttachment) -> AttachmentItem {
    let blob_id = item.blob_ref.as_str().to_owned();
    AttachmentItem {
        blob_id: blob_id.clone(),
        name: item.filename,
        type_: item.mime_type,
        size: usize::try_from(item.size_bytes).unwrap_or(usize::MAX),
        download_url: format!("/api/attachments/{}/download", urlencoding(&blob_id)),
        context: AttachmentContext {
            thread_id: item.context.thread_id,
            email_id: item.context.message_id.as_str().to_owned(),
            subject: item.context.subject,
            from: item.context.from,
            received_at: item.context.received_at,
            preview: item.context.preview,
        },
    }
}

#[derive(Debug)]
pub struct AttachmentError(String);

impl AttachmentError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    router_with_provider(Arc::new(CacheAttachmentProvider))
}

pub fn router_with_provider<P>(provider: Arc<P>) -> OpenApiRouter<AppState>
where
    P: AttachmentProvider,
{
    let provider: Arc<dyn AttachmentProvider> = provider;
    OpenApiRouter::new()
        .routes(routes!(list_attachments).layer(Extension(provider.clone())))
        .routes(routes!(download_attachment).layer(Extension(provider)))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListAttachmentsQuery {
    /// Maximum number of messages-with-attachments to inspect.
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AttachmentsResponse {
    pub items: Vec<AttachmentItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AttachmentItem {
    pub blob_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub size: usize,
    pub download_url: String,
    pub context: AttachmentContext,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AttachmentContext {
    pub thread_id: String,
    pub email_id: String,
    pub subject: String,
    pub from: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub received_at: Option<DateTime<Utc>>,
    pub preview: String,
}

#[utoipa::path(
    get,
    path = "/api/attachments",
    tag = TAG,
    params(ListAttachmentsQuery),
    responses(
        (status = 200, description = "Recent attachments with thread/message context.", body = AttachmentsResponse),
        (status = 400, description = "Invalid query."),
        (status = 401, description = "Missing or invalid session."),
        (status = 500, description = "Attachment listing failed."),
    ),
)]
async fn list_attachments(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn AttachmentProvider>>,
    Query(query): Query<ListAttachmentsQuery>,
) -> Response {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return bad_request("invalid_limit");
    }

    match provider.list(&state, user.jmap_token.clone(), limit).await {
        Ok(items) => Json(AttachmentsResponse { items }).into_response(),
        Err(err) => {
            tracing::warn!(user_id = user.id, error = %err.0, "attachment listing failed");
            internal()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/attachments/{blob_id}/download",
    tag = TAG,
    params(
        ("blob_id" = String, Path, description = "JMAP blob id to download."),
    ),
    responses(
        (status = 200, description = "Attachment bytes."),
        (status = 400, description = "Invalid blob id."),
        (status = 401, description = "Missing or invalid session."),
        (status = 404, description = "Blob not found."),
        (status = 500, description = "Attachment download failed."),
    ),
)]
async fn download_attachment(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(provider): Extension<Arc<dyn AttachmentProvider>>,
    Path(blob_id): Path<String>,
    Query(query): Query<DownloadAttachmentQuery>,
) -> Response {
    if blob_id.trim().is_empty() || blob_id.contains('/') || blob_id.contains('\\') {
        return bad_request("invalid_blob_id");
    }

    match provider
        .download(&state, user.jmap_token.clone(), &blob_id)
        .await
    {
        Ok(Some(bytes)) => {
            let inline = query.disposition.as_deref() == Some("inline");
            let content_type = if inline {
                match query
                    .r#type
                    .as_deref()
                    .filter(|value| is_safe_inline_image_type(value))
                {
                    Some(value) => HeaderValue::from_str(value)
                        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
                    None => return bad_request("invalid_inline_type"),
                }
            } else {
                HeaderValue::from_static("application/octet-stream")
            };
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, content_type);
            headers.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(if inline { "inline" } else { "attachment" }),
            );
            (StatusCode::OK, headers, bytes).into_response()
        }
        Ok(None) => not_found("blob"),
        Err(err) => {
            tracing::warn!(user_id = user.id, blob_id = %blob_id, error = %err.0, "attachment download failed");
            internal()
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DownloadAttachmentQuery {
    /// Optional inline rendering hint for safe inline image blobs.
    disposition: Option<String>,
    /// MIME type used only with `disposition=inline`; limited to safe raster image types.
    #[serde(rename = "type")]
    r#type: Option<String>,
}

fn is_safe_inline_image_type(content_type: &str) -> bool {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        content_type.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    )
}

fn urlencoding(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

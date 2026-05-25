//! Blob upload endpoint.
//!
//! Accepts multipart `file` parts from the SPA, enforces hail's per-file and
//! request-size limits, and uploads each part to JMAP. The live Stalwart call
//! is isolated behind [`BlobUploader`] so tests can use a fake backend.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::multipart::{MultipartError, MultipartRejection};
use axum::extract::{DefaultBodyLimit, Extension, Multipart, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use secrecy::SecretString;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::middleware::auth::AuthUser;
use crate::routes::jmap_helpers::jmap_session;
use crate::routes::response::{bad_request, internal};
use crate::state::AppState;

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 100 * 1024 * 1024;

/// OpenAPI tag for JMAP blob upload endpoints.
pub const TAG: &str = "blobs";

/// Dependency-injection seam for JMAP blob upload.
pub trait BlobUploader: Send + Sync + 'static {
    fn upload<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        bytes: Vec<u8>,
        content_type: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<UploadedBlob, BlobUploadError>> + Send + 'a>>;
}

/// Production uploader using `jmap-client`'s upload helper. The helper expands
/// the session `uploadUrl` `{accountId}` template and sends authenticated bytes.
pub struct JmapBlobUploader;

impl BlobUploader for JmapBlobUploader {
    fn upload<'a>(
        &'a self,
        state: &'a AppState,
        token: SecretString,
        bytes: Vec<u8>,
        content_type: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<UploadedBlob, BlobUploadError>> + Send + 'a>> {
        Box::pin(async move {
            let session = jmap_session(state, token)
                .await
                .map_err(BlobUploadError::new)?;
            let response = session
                .client()
                .upload(Some(session.account_id()), bytes, content_type.as_deref())
                .await
                .map_err(|err| BlobUploadError::new(err.to_string()))?;
            Ok(UploadedBlob {
                blob_id: response.blob_id().to_owned(),
                size: response.size(),
                type_: response.content_type().to_owned(),
            })
        })
    }
}

#[derive(Debug)]
pub struct BlobUploadError(String);

impl BlobUploadError {
    /// Construct an upload failure without exposing backend details to clients.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Build protected blob routes.
pub fn router() -> Router<AppState> {
    Router::from(openapi_router_with_uploader(Arc::new(JmapBlobUploader)))
}

/// Build the OpenAPI-tracked router for production blob uploads.
pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_uploader(Arc::new(JmapBlobUploader))
}

/// Test/helper router that injects a fake uploader. The 100 MiB total request
/// cap is enforced as an endpoint-local `DefaultBodyLimit` override.
pub fn router_with_uploader<U>(uploader: Arc<U>) -> Router<AppState>
where
    U: BlobUploader,
{
    Router::from(openapi_router_with_uploader(uploader))
}

fn openapi_router_with_uploader<U>(uploader: Arc<U>) -> OpenApiRouter<AppState>
where
    U: BlobUploader,
{
    let uploader: Arc<dyn BlobUploader> = uploader;
    OpenApiRouter::new()
        .routes(routes!(upload_blobs).layer(Extension(uploader)))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UploadedBlob {
    pub blob_id: String,
    pub size: usize,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct BlobUploadResponse {
    blobs: Vec<UploadedBlob>,
}

#[utoipa::path(
    post,
    path = "/api/blobs",
    tag = TAG,
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Blobs uploaded to JMAP.", body = BlobUploadResponse),
        (status = 400, description = "Invalid multipart upload."),
        (status = 401, description = "Missing or invalid session."),
        (status = 413, description = "Upload too large."),
        (status = 500, description = "Blob upload failed."),
    ),
)]
async fn upload_blobs(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(uploader): Extension<Arc<dyn BlobUploader>>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Response {
    let mut blobs = Vec::new();
    let Ok(mut multipart) = multipart else {
        return bad_request("invalid_multipart");
    };

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(err) => {
                tracing::debug!(error = %err, "multipart parse failed");
                return multipart_error(err);
            }
        };

        if field.name() != Some("file") {
            continue;
        }
        let content_type = field.content_type().map(ToOwned::to_owned);
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::debug!(error = %err, "multipart file read failed");
                return multipart_error(err);
            }
        };
        if bytes.len() > MAX_FILE_BYTES {
            return payload_too_large();
        }

        let uploaded = match uploader
            .upload(
                &state,
                user.jmap_token.clone(),
                bytes.to_vec(),
                content_type,
            )
            .await
        {
            Ok(uploaded) => uploaded,
            Err(err) => {
                tracing::warn!(user_id = user.id, error = %err.0, "blob upload failed");
                return internal();
            }
        };
        blobs.push(uploaded);
    }

    (StatusCode::CREATED, Json(BlobUploadResponse { blobs })).into_response()
}

fn multipart_error(err: MultipartError) -> Response {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        payload_too_large()
    } else {
        bad_request("invalid_multipart")
    }
}

fn payload_too_large() -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"payload_too_large"}"#,
    )
        .into_response()
}

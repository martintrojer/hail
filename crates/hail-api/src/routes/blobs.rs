//! Blob upload endpoint.
//!
//! Accepts multipart `file` parts from the SPA, enforces hail's per-file and
//! request-size limits, and uploads each part to JMAP. The live Stalwart call
//! is isolated behind [`BlobUploader`] so tests can use a fake backend.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Extension, Multipart, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use secrecy::SecretString;
use serde::Serialize;

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 100 * 1024 * 1024;

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
            let session = hail_jmap::login_bearer(&state.config.stalwart.jmap_url, token)
                .await
                .map_err(|err| BlobUploadError(err.to_string()))?;
            let response = session
                .client()
                .upload(Some(session.account_id()), bytes, content_type.as_deref())
                .await
                .map_err(|err| BlobUploadError(err.to_string()))?;
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

/// Build protected blob routes.
pub fn router() -> Router<AppState> {
    router_with_uploader(Arc::new(JmapBlobUploader))
}

/// Test/helper router that injects a fake uploader. The 100 MiB total request
/// cap is enforced as an endpoint-local `DefaultBodyLimit` override.
pub fn router_with_uploader<U>(uploader: Arc<U>) -> Router<AppState>
where
    U: BlobUploader,
{
    Router::new()
        .route("/api/blobs", post(upload_blobs::<U>))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(Extension(uploader))
}

#[derive(Debug, Clone, Serialize)]
pub struct UploadedBlob {
    pub blob_id: String,
    pub size: usize,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Debug, Serialize)]
struct BlobUploadResponse {
    blobs: Vec<UploadedBlob>,
}

async fn upload_blobs<U>(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(uploader): Extension<Arc<U>>,
    mut multipart: Multipart,
) -> Response
where
    U: BlobUploader,
{
    let mut blobs = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(err) => {
                tracing::debug!(error = %err, "multipart parse failed");
                return payload_too_large();
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
                return payload_too_large();
            }
        };
        if bytes.len() > MAX_FILE_BYTES {
            return payload_too_large();
        }

        let uploaded = match uploader
            .upload(&state, user.jmap_token.clone(), bytes.to_vec(), content_type)
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

fn payload_too_large() -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"payload_too_large"}"#,
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

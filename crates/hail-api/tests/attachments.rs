use http_body_util::BodyExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::require_auth;
use hail_api::routes::attachments::{
    AttachmentContext, AttachmentError, AttachmentItem, AttachmentProvider,
};
use hail_api::state::AppState;
use hail_test::{fixture_state, seed_session};
use secrecy::SecretString;
use tower::ServiceExt;

fn app(state: AppState, provider: Arc<FakeAttachmentProvider>) -> Router {
    let protected = Router::from(hail_api::routes::attachments::router_with_provider(
        provider,
    ))
    .layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone)]
struct FakeAttachmentProvider {
    list_result: Result<Vec<AttachmentItem>, String>,
    download_result: Result<Option<Vec<u8>>, String>,
    list_calls: Arc<Mutex<Vec<usize>>>,
    download_calls: Arc<Mutex<Vec<String>>>,
}

impl FakeAttachmentProvider {
    fn new(list_result: Result<Vec<AttachmentItem>, String>) -> Self {
        Self {
            list_result,
            download_result: Ok(Some(b"file bytes".to_vec())),
            list_calls: Arc::new(Mutex::new(Vec::new())),
            download_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn list_calls(&self) -> Vec<usize> {
        self.list_calls.lock().expect("list calls lock").clone()
    }

    fn download_calls(&self) -> Vec<String> {
        self.download_calls
            .lock()
            .expect("download calls lock")
            .clone()
    }
}

impl AttachmentProvider for FakeAttachmentProvider {
    fn list<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<AttachmentItem>, AttachmentError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.list_calls.lock().expect("list calls lock").push(limit);
            self.list_result.clone().map_err(AttachmentError::provider)
        })
    }

    fn download<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        blob_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, AttachmentError>> + Send + 'a>> {
        Box::pin(async move {
            self.download_calls
                .lock()
                .expect("download calls lock")
                .push(blob_id.to_string());
            self.download_result
                .clone()
                .map_err(AttachmentError::provider)
        })
    }
}

fn sample_item() -> AttachmentItem {
    AttachmentItem {
        blob_id: "blob-1".to_string(),
        name: "invoice.pdf".to_string(),
        type_: "application/pdf".to_string(),
        size: 42_000,
        download_url: "/api/attachments/blob-1/download".to_string(),
        context: AttachmentContext {
            thread_id: "thread-1".to_string(),
            email_id: "email-1".to_string(),
            subject: "May invoice".to_string(),
            from: "Billing <billing@example.org>".to_string(),
            received_at: None,
            preview: "Your invoice is attached".to_string(),
        },
    }
}

async fn get(
    state: AppState,
    sid: &str,
    provider: Arc<FakeAttachmentProvider>,
    uri: &str,
) -> axum::response::Response {
    app(state, provider)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header(header::COOKIE, format!("hail_session={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn list_attachments_returns_context_and_download_links() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "files@example.org").await;
    let provider = Arc::new(FakeAttachmentProvider::new(Ok(vec![sample_item()])));

    let resp = get(state, &sid, provider.clone(), "/api/attachments?limit=25").await;
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(provider.list_calls(), vec![25]);
    assert_eq!(json["items"][0]["name"], "invoice.pdf");
    assert_eq!(
        json["items"][0]["download_url"],
        "/api/attachments/blob-1/download"
    );
    assert_eq!(json["items"][0]["context"]["thread_id"], "thread-1");
    assert_eq!(json["items"][0]["context"]["subject"], "May invoice");
}

#[tokio::test]
async fn list_attachments_validates_limit() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "files@example.org").await;
    let provider = Arc::new(FakeAttachmentProvider::new(Ok(vec![sample_item()])));

    let resp = get(state, &sid, provider.clone(), "/api/attachments?limit=0").await;
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["error"], "invalid_limit");
    assert!(provider.list_calls().is_empty());
}

#[tokio::test]
async fn download_streams_blob_bytes_through_api() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "files@example.org").await;
    let provider = Arc::new(FakeAttachmentProvider::new(Ok(Vec::new())));

    let resp = get(
        state,
        &sid,
        provider.clone(),
        "/api/attachments/blob-1/download",
    )
    .await;
    let status = resp.status();
    let content_type = resp.headers().get(header::CONTENT_TYPE).cloned();
    let body = resp.into_body().collect().await.unwrap().to_bytes();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(provider.download_calls(), vec!["blob-1"]);
    assert_eq!(
        content_type.as_ref().and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(&body[..], b"file bytes");
}

#[tokio::test]
async fn download_can_render_safe_inline_images() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "files-inline@example.org").await;
    let provider = Arc::new(FakeAttachmentProvider::new(Ok(Vec::new())));

    let resp = get(
        state,
        &sid,
        provider.clone(),
        "/api/attachments/blob-image/download?disposition=inline&type=image%2Fpng",
    )
    .await;
    let status = resp.status();
    let content_type = resp.headers().get(header::CONTENT_TYPE).cloned();
    let content_disposition = resp.headers().get(header::CONTENT_DISPOSITION).cloned();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(provider.download_calls(), vec!["blob-image"]);
    assert_eq!(
        content_type.as_ref().and_then(|value| value.to_str().ok()),
        Some("image/png")
    );
    assert_eq!(
        content_disposition
            .as_ref()
            .and_then(|value| value.to_str().ok()),
        Some("inline")
    );
}

#[tokio::test]
async fn inline_download_rejects_non_image_types() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "files-inline-bad@example.org").await;
    let provider = Arc::new(FakeAttachmentProvider::new(Ok(Vec::new())));

    let resp = get(
        state,
        &sid,
        provider.clone(),
        "/api/attachments/blob-svg/download?disposition=inline&type=image%2Fsvg%2Bxml",
    )
    .await;
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "invalid_inline_type");
    assert_eq!(provider.download_calls(), vec!["blob-svg"]);
}

#[tokio::test]
async fn auth_required_for_attachments() {
    let (state, _key) = fixture_state().await;
    let provider = Arc::new(FakeAttachmentProvider::new(Ok(Vec::new())));

    let resp = app(state, provider)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/attachments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

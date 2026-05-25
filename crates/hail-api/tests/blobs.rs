use http_body_util::BodyExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::routes::blobs::{BlobUploadError, BlobUploader, UploadedBlob};
use hail_api::state::AppState;
use hail_test::{fixture_state, seed_session};
use secrecy::SecretString;
use tower::ServiceExt;

fn app(state: AppState, uploader: Arc<FakeUploader>) -> Router {
    let protected = hail_api::routes::blobs::router_with_uploader(uploader).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone)]
struct FakeUploader {
    fail: bool,
}

impl FakeUploader {
    fn ok() -> Arc<Self> {
        Arc::new(Self { fail: false })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self { fail: true })
    }
}

impl BlobUploader for FakeUploader {
    fn upload<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        bytes: Vec<u8>,
        content_type: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<UploadedBlob, BlobUploadError>> + Send + 'a>> {
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                return Err(BlobUploadError::new("synthetic uploader failure"));
            }

            Ok(UploadedBlob {
                blob_id: format!("blob-{}", bytes.len()),
                size: bytes.len(),
                type_: content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
            })
        })
    }
}

fn multipart_body(boundary: &str, content_type: &str, bytes: &[u8]) -> Vec<u8> {
    multipart_parts(boundary, &[("file", Some("test.bin"), content_type, bytes)])
}

fn multipart_parts(boundary: &str, parts: &[(&str, Option<&str>, &str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, filename, content_type, bytes) in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let filename = filename
            .map(|filename| format!("; filename=\"{filename}\""))
            .unwrap_or_default();
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"{filename}\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[tokio::test]
async fn one_file_upload_returns_blob_id() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "alice@example.org").await;
    let boundary = "hail-boundary";
    let body = multipart_body(boundary, "text/plain", b"hello");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["blobs"][0]["blob_id"], "blob-5");
    assert_eq!(json["blobs"][0]["size"], 5);
    assert_eq!(json["blobs"][0]["type"], "text/plain");
}

#[tokio::test]
async fn file_over_50mb_returns_413() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "bob@example.org").await;
    let boundary = "hail-boundary";
    let bytes = vec![b'x'; 50 * 1024 * 1024 + 1];
    let body = multipart_body(boundary, "application/octet-stream", &bytes);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn no_auth_returns_401() {
    let (state, _key) = fixture_state().await;
    let boundary = "hail-boundary";
    let body = multipart_body(boundary, "text/plain", b"hello");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_csrf_returns_403_before_upload() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "carol@example.org").await;
    let boundary = "hail-boundary";
    let body = multipart_body(boundary, "text/plain", b"hello");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::failing())
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "csrf_required");
}

#[tokio::test]
async fn no_file_parts_returns_empty_created_response() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "dana@example.org").await;
    let boundary = "hail-boundary";
    let body = multipart_parts(boundary, &[("note", None, "text/plain", b"ignored")]);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["blobs"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn multiple_file_parts_upload_in_order() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "erin@example.org").await;
    let boundary = "hail-boundary";
    let body = multipart_parts(
        boundary,
        &[
            ("file", Some("one.txt"), "text/plain", b"one"),
            (
                "file",
                Some("two.bin"),
                "application/octet-stream",
                b"twenty",
            ),
        ],
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["blobs"].as_array().unwrap().len(), 2);
    assert_eq!(json["blobs"][0]["blob_id"], "blob-3");
    assert_eq!(json["blobs"][0]["type"], "text/plain");
    assert_eq!(json["blobs"][1]["blob_id"], "blob-6");
    assert_eq!(json["blobs"][1]["type"], "application/octet-stream");
}

#[tokio::test]
async fn missing_boundary_returns_400() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "frank@example.org").await;

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(header::CONTENT_TYPE, "multipart/form-data")
        .body(Body::from(Vec::new()))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_multipart");
}

#[tokio::test]
async fn malformed_multipart_returns_stable_400_json() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "grace@example.org").await;
    let boundary = "hail-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"broken.bin\"\r\nContent-Type: text/plain\r\n\r\nunterminated"
    );

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::ok()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "invalid_multipart");
}

#[tokio::test]
async fn uploader_error_returns_stable_500_json() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "heidi@example.org").await;
    let boundary = "hail-boundary";
    let body = multipart_body(boundary, "text/plain", b"hello");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/blobs")
        .header(header::COOKIE, format!("hail_session={sid}"))
        .header(CSRF_HEADER, "1")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app(state, FakeUploader::failing())
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "internal");
}

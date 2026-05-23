use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use hail_api::middleware::auth::{CSRF_HEADER, require_auth};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::routes::blobs::{BlobUploadError, BlobUploader, UploadedBlob};
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use secrecy::SecretString;
use tower::ServiceExt;

async fn fixture_state() -> (AppState, [u8; KEY_LEN]) {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_blobs_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", hex::encode(key));
    }
    let config = Config::load_from(None).expect("load config");

    let state = AppState {
        db,
        config,
        server_key: Arc::new(key),
        login_limiter: Arc::new(IpRateLimiter::default()),
        events: hail_api::events::AppEventBus::default(),
    };
    (state, key)
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

async fn seed_session(state: &AppState, key: &[u8; KEY_LEN], email: &str) -> String {
    let now = Utc::now();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
         VALUES (?1, ?2, 0, ?3) RETURNING id",
    )
    .bind(email)
    .bind("account-id")
    .bind(now)
    .fetch_one(&state.db)
    .await
    .expect("insert user");

    let token_enc = hail_core::seal(b"dummy-token", key).expect("seal");
    let session_id = format!("{:064x}", user_id);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&token_enc)
    .bind(Some("test-ua"))
    .bind(now + Duration::days(30))
    .bind(now)
    .execute(&state.db)
    .await
    .expect("insert session");
    session_id
}

fn app(state: AppState, uploader: Arc<FakeUploader>) -> Router {
    let protected = hail_api::routes::blobs::router_with_uploader(uploader).layer(
        axum::middleware::from_fn_with_state(state.clone(), require_auth),
    );
    Router::new().merge(protected).with_state(state)
}

#[derive(Clone)]
struct FakeUploader;

impl BlobUploader for FakeUploader {
    fn upload<'a>(
        &'a self,
        _state: &'a AppState,
        _token: SecretString,
        bytes: Vec<u8>,
        content_type: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<UploadedBlob, BlobUploadError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(UploadedBlob {
                blob_id: format!("blob-{}", bytes.len()),
                size: bytes.len(),
                type_: content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
            })
        })
    }
}

fn multipart_body(boundary: &str, content_type: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"test.bin\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[tokio::test]
async fn one_file_upload_returns_blob_id() {
    let (state, key) = fixture_state().await;
    let sid = seed_session(&state, &key, "alice@example.org").await;
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

    let resp = app(state, Arc::new(FakeUploader)).oneshot(req).await.unwrap();
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
    let sid = seed_session(&state, &key, "bob@example.org").await;
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

    let resp = app(state, Arc::new(FakeUploader)).oneshot(req).await.unwrap();
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

    let resp = app(state, Arc::new(FakeUploader)).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

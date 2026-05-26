use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::KEY_LEN;
use hail_db::connect;
use hail_test::fixture_config;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state_with_public_url(
    webapp_dir: Option<std::path::PathBuf>,
    public_url: &str,
) -> AppState {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_static_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    let mut config = fixture_config(&url, &key);
    config.server.public_url = public_url.to_string();
    config.server.webapp_dir = webapp_dir;

    AppState {
        db,
        config,
        server_key: Arc::new(key),
        auth_rate_limiter: Arc::new(IpRateLimiter::default()),
        events: hail_api::events::AppEventBus::default(),
    }
}

async fn fixture_state(webapp_dir: Option<std::path::PathBuf>) -> AppState {
    fixture_state_with_public_url(webapp_dir, "http://localhost").await
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

fn assert_common_security_headers(headers: &HeaderMap) {
    assert_eq!(
        headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
    assert_eq!(headers.get(header::REFERRER_POLICY).unwrap(), "no-referrer");
    assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");

    let csp = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .expect("content-security-policy")
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(csp.contains("script-src 'self'"), "csp: {csp}");
    assert!(
        csp.contains("style-src 'self' 'unsafe-inline'"),
        "csp: {csp}"
    );
    assert!(csp.contains("img-src 'self' data: blob:"), "csp: {csp}");
    assert!(csp.contains("object-src 'none'"), "csp: {csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "csp: {csp}");
}

#[tokio::test]
async fn serves_spa_bundle_and_history_fallback_without_intercepting_api() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("assets")).expect("assets dir");
    std::fs::write(dir.path().join("index.html"), "<html>hail app</html>").expect("index");
    std::fs::write(dir.path().join("assets/test.js"), "console.log('hail');").expect("asset");

    let state = fixture_state(Some(dir.path().to_path_buf())).await;
    let app = hail_api::build_router(state, false);

    let root = get(app.clone(), "/").await;
    assert_eq!(root.status(), StatusCode::OK);
    assert_common_security_headers(root.headers());
    let root_csp = root
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(root_csp.contains("connect-src 'self' ws://localhost"));
    assert!(
        root.headers()
            .get(header::STRICT_TRANSPORT_SECURITY)
            .is_none()
    );
    let bytes = root.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), b"<html>hail app</html>");

    let asset = get(app.clone(), "/assets/test.js").await;
    assert_eq!(asset.status(), StatusCode::OK);
    let content_type = asset
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type")
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("application/javascript"),
        "unexpected content-type: {content_type}"
    );

    let history = get(app.clone(), "/some/spa/route").await;
    assert_eq!(history.status(), StatusCode::OK);
    let bytes = history.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes.as_ref(), b"<html>hail app</html>");

    let health = get(app, "/healthz").await;
    assert_eq!(health.status(), StatusCode::NO_CONTENT);
    assert_common_security_headers(health.headers());
}

#[tokio::test]
async fn security_headers_include_hsts_and_wss_when_public_url_is_https() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("index.html"), "<html>hail app</html>").expect("index");

    let state =
        fixture_state_with_public_url(Some(dir.path().to_path_buf()), "https://mail.example.test")
            .await;
    let app = hail_api::build_router(state, false);

    let spa = get(app.clone(), "/").await;
    assert_eq!(spa.status(), StatusCode::OK);
    assert_common_security_headers(spa.headers());
    assert_eq!(
        spa.headers()
            .get(header::STRICT_TRANSPORT_SECURITY)
            .unwrap(),
        "max-age=31536000; includeSubDomains"
    );
    let csp = spa
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        csp.contains("connect-src 'self' wss://mail.example.test"),
        "csp: {csp}"
    );

    let openapi = get(app, "/api/openapi.json").await;
    assert_eq!(openapi.status(), StatusCode::OK);
    assert_common_security_headers(openapi.headers());
    assert_eq!(
        openapi
            .headers()
            .get(header::STRICT_TRANSPORT_SECURITY)
            .unwrap(),
        "max-age=31536000; includeSubDomains"
    );
}

#[tokio::test]
async fn missing_webapp_dir_keeps_api_up_and_root_404s() {
    let missing = std::env::temp_dir().join(format!("hail-missing-webapp-{}", uuid_like()));
    assert!(!missing.exists());
    let state = fixture_state(Some(missing)).await;
    let app = hail_api::build_router(state, false);

    let health = get(app.clone(), "/healthz").await;
    assert_eq!(health.status(), StatusCode::NO_CONTENT);

    let root = get(app, "/").await;
    assert_eq!(root.status(), StatusCode::NOT_FOUND);
    assert_common_security_headers(root.headers());
}

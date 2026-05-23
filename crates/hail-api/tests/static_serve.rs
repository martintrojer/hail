use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::{Config, KEY_LEN};
use hail_db::connect;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn fixture_state(webapp_dir: Option<std::path::PathBuf>) -> AppState {
    let uniq = uuid_like();
    let url = format!("sqlite:file:hail_static_test_{uniq}?mode=memory&cache=shared");
    let db = connect(&url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");

    let key = [0x5Au8; KEY_LEN];
    unsafe {
        std::env::set_var("HAIL_DATABASE_URL", &url);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__BIND", "127.0.0.1:0");
        std::env::set_var("HAIL_SERVER__PUBLIC_URL", "http://localhost");
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", hex::encode(key));
        std::env::remove_var("HAIL_WEBAPP_DIR");
    }
    let mut config = Config::load_from(None).expect("load config");
    config.server.webapp_dir = webapp_dir;

    AppState {
        db,
        config,
        server_key: Arc::new(key),
        login_limiter: Arc::new(IpRateLimiter::default()),
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
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
}

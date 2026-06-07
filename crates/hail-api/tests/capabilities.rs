use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use hail_core::{MailBackend, MailCacheMode};
use hail_test::{fixture_state, seed_session};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

async fn get_capabilities(
    state: hail_api::state::AppState,
    session_id: &str,
) -> (StatusCode, serde_json::Value) {
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/capabilities")
        .header(header::COOKIE, format!("hail_session={session_id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn capabilities_returns_jmap_config_and_session_account() {
    let (mut state, key) = fixture_state().await;
    state.config.mail.backend = MailBackend::Jmap;
    state.config.mail.cache.mode = MailCacheMode::Full;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let (status, body) = get_capabilities(state, &sid).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "backend": "jmap",
            "cache_mode": "full",
            "supports_initial_import": false,
            "supports_principals_admin": true,
            "supports_bulk_archive": true,
            "supports_eventsource": true,
            "label_path_separator": "/",
            "accounts": [
                { "id": user_id, "email": "alice@example.org", "backend": "jmap" }
            ]
        })
    );
}

#[tokio::test]
async fn capabilities_returns_connected_mail_accounts_for_gmail() {
    let (mut state, key) = fixture_state().await;
    state.config.mail.backend = MailBackend::Gmail;
    state.config.mail.cache.mode = MailCacheMode::Bounded;
    let (user_id, sid) = seed_session(&state, &key, "owner@example.org").await;
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, display_email, \
          granted_scopes_json, consented_at, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?1, 'jmap-acct', 'gmail', 'gmail', 'gmail-1', 'you@gmail.com', 'You <you@gmail.com>', \
          '[]', ?2, ?3, 'active', ?2, ?2)",
    )
    .bind(user_id)
    .bind(now)
    .bind(vec![1_u8; 32])
    .execute(&state.db)
    .await
    .expect("insert mail account");

    let (status, body) = get_capabilities(state, &sid).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["backend"], "gmail");
    assert_eq!(body["cache_mode"], "bounded");
    assert_eq!(body["supports_initial_import"], true);
    assert_eq!(body["supports_principals_admin"], false);
    assert_eq!(body["supports_bulk_archive"], true);
    assert_eq!(body["supports_eventsource"], false);
    assert_eq!(body["label_path_separator"], "/");
    assert_eq!(
        body["accounts"],
        json!([{ "id": 1, "email": "You <you@gmail.com>", "backend": "gmail" }])
    );
}

#[tokio::test]
async fn capabilities_requires_authentication() {
    let (state, _key) = fixture_state().await;
    let app = hail_api::build_router(state, true);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/capabilities")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::CSRF_HEADER;
use hail_api::state::AppState;
use hail_test::{fixture_state, json_body, seed_session};
use tower::ServiceExt;

async fn request(
    state: AppState,
    method: Method,
    uri: &str,
    sid: Option<&str>,
    csrf: bool,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(sid) = sid {
        builder = builder.header(header::COOKIE, format!("hail_session={sid}"));
    }
    if csrf {
        builder = builder.header(CSRF_HEADER, "1");
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    hail_api::build_router(state, true)
        .oneshot(
            builder
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_owned())))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn get_returns_current_passphrase_and_persists_it_for_user() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "alice@example.org").await;

    let resp = request(
        state.clone(),
        Method::GET,
        "/api/speakeasy",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let first = json_body(resp).await;
    let phrase = first["speakeasy"]["passphrase"].as_str().unwrap();
    assert!(phrase.len() >= 33);
    assert_eq!(phrase.split('-').count(), 5);
    assert!(first["speakeasy"]["period"].as_str().unwrap().contains('-'));
    assert!(first["speakeasy"]["rotates_at"].as_str().is_some());
    assert!(first["speakeasy"]["generated_at"].as_str().is_some());
    assert!(first["speakeasy"]["manually_rotated_at"].is_null());

    let stored: (String, String) =
        sqlx::query_as("SELECT passphrase, period FROM speakeasy_passphrases WHERE user_id = ?1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .expect("stored speakeasy phrase");
    assert_eq!(stored.0, phrase);
    assert_eq!(stored.1, first["speakeasy"]["period"]);

    let resp = request(
        state,
        Method::GET,
        "/api/speakeasy",
        Some(&sid),
        false,
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["speakeasy"]["passphrase"], phrase);
}

#[tokio::test]
async fn rotate_changes_passphrase_without_logging_secret_in_audit_payload() {
    let (state, key) = fixture_state().await;
    let (user_id, sid) = seed_session(&state, &key, "rotate@example.org").await;

    let before = request(
        state.clone(),
        Method::GET,
        "/api/speakeasy",
        Some(&sid),
        false,
        None,
    )
    .await;
    let before_phrase = json_body(before).await["speakeasy"]["passphrase"]
        .as_str()
        .unwrap()
        .to_owned();

    let resp = request(
        state.clone(),
        Method::POST,
        "/api/speakeasy/rotate",
        Some(&sid),
        true,
        Some(r#"{"acknowledge_bypass_secret":true}"#),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let rotated = json_body(resp).await;
    let rotated_phrase = rotated["speakeasy"]["passphrase"].as_str().unwrap();
    assert_ne!(rotated_phrase, before_phrase);
    assert!(
        rotated["speakeasy"]["manually_rotated_at"]
            .as_str()
            .is_some()
    );

    let audit_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM audit_log WHERE user_id = ?1 AND action = 'speakeasy.rotate'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .expect("audit row");
    assert!(!audit_payload.contains(rotated_phrase));
    assert!(!audit_payload.contains(&before_phrase));
    assert!(audit_payload.contains("period"));
}

#[tokio::test]
async fn speakeasy_state_is_scoped_to_authenticated_user() {
    let (state, key) = fixture_state().await;
    let (_alice_id, alice_sid) = seed_session(&state, &key, "alice@example.org").await;
    let (_bob_id, bob_sid) = seed_session(&state, &key, "bob@example.org").await;

    let alice = request(
        state.clone(),
        Method::GET,
        "/api/speakeasy",
        Some(&alice_sid),
        false,
        None,
    )
    .await;
    let alice_phrase = json_body(alice).await["speakeasy"]["passphrase"]
        .as_str()
        .unwrap()
        .to_owned();

    let bob = request(
        state,
        Method::GET,
        "/api/speakeasy",
        Some(&bob_sid),
        false,
        None,
    )
    .await;
    let bob_phrase = json_body(bob).await["speakeasy"]["passphrase"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_ne!(alice_phrase, bob_phrase);
}

#[tokio::test]
async fn speakeasy_routes_require_auth_and_rotation_requires_csrf() {
    let (state, key) = fixture_state().await;
    let (_user_id, sid) = seed_session(&state, &key, "csrf@example.org").await;

    let no_auth = request(
        state.clone(),
        Method::GET,
        "/api/speakeasy",
        None,
        false,
        None,
    )
    .await;
    assert_eq!(no_auth.status(), StatusCode::UNAUTHORIZED);

    let no_csrf = request(
        state.clone(),
        Method::POST,
        "/api/speakeasy/rotate",
        Some(&sid),
        false,
        Some("{}"),
    )
    .await;
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(no_csrf).await["error"], "csrf_required");

    let with_csrf = request(
        state,
        Method::POST,
        "/api/speakeasy/rotate",
        Some(&sid),
        true,
        Some("{}"),
    )
    .await;
    assert_eq!(with_csrf.status(), StatusCode::OK);
}

#[tokio::test]
async fn openapi_includes_speakeasy_endpoints() {
    let (state, _key) = fixture_state().await;
    let resp = request(state, Method::GET, "/api/openapi.json", None, false, None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let spec = json_body(resp).await;
    let paths = spec["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/speakeasy"));
    assert!(paths.contains_key("/api/speakeasy/rotate"));
}

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use hail_api::middleware::auth::CSRF_HEADER;
use hail_api::middleware::session::SESSION_COOKIE;
use hail_api::routes::provider_accounts::{
    GmailAuthorizationRequest, GmailOAuthClient, GmailOAuthError, GmailProfile, GmailTokenExchange,
};
use hail_api::state::AppState;
use hail_core::{ProviderOAuthTokenKind, ProviderTokenContext};
use hail_db::provider_sync_audit::{
    NewProviderSyncAuditLog, ProviderSyncEventType, ProviderSyncOperationKind,
    ProviderSyncResultStatus, insert_provider_sync_audit_log,
};
use hail_test::{fixture_config, fixture_state, fresh_db_url, json_body, seed_session};
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

const GMAIL_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";
const GMAIL_SEND: &str = "https://www.googleapis.com/auth/gmail.send";

fn hostile_leak_error() -> &'static str {
    "api failed Authorization: Bearer ya29.api-secret access_token=ya29.api-secret refresh_token=1//api-refresh\n\nSubject: API Private\n\nAPI body must not be exposed"
}

fn assert_no_hostile_leak(surface: &str) {
    for forbidden in [
        "Bearer",
        "ya29.api-secret",
        "1//api-refresh",
        "Subject: API Private",
        "API body must not be exposed",
    ] {
        assert!(
            !surface.contains(forbidden),
            "surface leaked {forbidden:?}: {surface}"
        );
    }
}

#[derive(Default)]
struct FakeGmailOAuthClient {
    auth_requests: Mutex<Vec<GmailAuthorizationRequest>>,
    exchange_codes: Mutex<Vec<String>>,
    revoked_tokens: Mutex<Vec<String>>,
    exchange_error: Mutex<Option<String>>,
    revoke_error: Mutex<Option<String>>,
}

impl FakeGmailOAuthClient {
    fn fail_next_exchange(&self, message: impl Into<String>) {
        *self.exchange_error.lock().expect("exchange error") = Some(message.into());
    }

    fn fail_next_revoke(&self, message: impl Into<String>) {
        *self.revoke_error.lock().expect("revoke error") = Some(message.into());
    }
}

impl GmailOAuthClient for FakeGmailOAuthClient {
    fn authorization_url(&self, req: GmailAuthorizationRequest) -> Result<String, GmailOAuthError> {
        self.auth_requests
            .lock()
            .expect("auth requests")
            .push(req.clone());
        Ok(format!(
            "https://accounts.example.test/oauth?state={}&scope={}",
            req.state,
            req.scopes.join("%20")
        ))
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        _redirect_uri: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<GmailTokenExchange, GmailOAuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.exchange_codes
                .lock()
                .expect("exchange codes")
                .push(code.to_owned());
            if let Some(message) = self.exchange_error.lock().expect("exchange error").take() {
                return Err(GmailOAuthError::Exchange(message));
            }
            Ok(GmailTokenExchange {
                access_token: SecretString::from("ya29.access-token-secret"),
                refresh_token: Some(SecretString::from("1//refresh-token-secret")),
                expires_at: None,
                granted_scopes: vec![GMAIL_READONLY.to_owned(), GMAIL_SEND.to_owned()],
                profile: GmailProfile {
                    email: "Gmail.User@Example.COM".to_owned(),
                    history_id: Some("12345".to_owned()),
                },
            })
        })
    }

    fn revoke_refresh_token<'a>(
        &'a self,
        refresh_token: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<(), GmailOAuthError>> + Send + 'a>> {
        Box::pin(async move {
            self.revoked_tokens
                .lock()
                .expect("revoked tokens")
                .push(refresh_token.expose_secret().to_owned());
            if let Some(message) = self.revoke_error.lock().expect("revoke error").take() {
                return Err(GmailOAuthError::Revoke(message));
            }
            Ok(())
        })
    }
}

async fn app_state() -> (AppState, [u8; hail_core::KEY_LEN]) {
    let (mut state, key) = fixture_state().await;
    state.config.provider_import.gmail.oauth_client_id = Some("gmail-client-id".to_owned());
    (state, key)
}

async fn file_app_state() -> (AppState, [u8; hail_core::KEY_LEN]) {
    let (db_url, _guard) = fresh_db_url("provider-oauth-cas");
    let db = hail_db::connect(&db_url).await.expect("open sqlite");
    hail_db::migrate(&db).await.expect("migrate");
    let key = [0x5Au8; hail_core::KEY_LEN];
    let mut config = fixture_config(db_url, &key);
    config.provider_import.gmail.oauth_client_id = Some("gmail-client-id".to_owned());
    let state = AppState {
        db: db.clone(),
        config,
        server_key: Arc::new(key),
        auth_rate_limiter: Arc::new(hail_api::middleware::rate_limit::IpRateLimiter::default()),
        mail: hail_api::test_support::fake_cached_mail(db.clone()),
        events: hail_api::events::AppEventBus::default(),
    };
    (state, key)
}

fn app(state: AppState, client: Arc<FakeGmailOAuthClient>) -> Router {
    let protected = hail_api::routes::provider_accounts::router_with_client(client)
        .merge(hail_api::routes::provider_sync::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            hail_api::middleware::auth::require_auth,
        ));
    Router::new().merge(protected).with_state(state)
}

fn auth_request(method: Method, uri: &str, session_id: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(uri)
        .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
        .header(header::CONTENT_TYPE, "application/json");
    if method == Method::POST {
        builder = builder.header(CSRF_HEADER, "1");
    }
    builder.body(Body::empty()).unwrap()
}

async fn connect(
    state: AppState,
    client: Arc<FakeGmailOAuthClient>,
    session_id: &str,
) -> serde_json::Value {
    let resp = app(state, client)
        .oneshot(auth_request(
            Method::POST,
            "/api/provider-accounts/gmail/connect",
            session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    json_body(resp).await
}

fn state_from_connect_response(value: &serde_json::Value) -> String {
    let auth_url = value["authorization_url"]
        .as_str()
        .expect("authorization_url");
    url::Url::parse(auth_url)
        .expect("auth url")
        .query_pairs()
        .find_map(|(k, v)| (k == "state").then(|| v.into_owned()))
        .expect("state query")
}

#[tokio::test]
async fn gmail_connect_returns_read_send_authorization_url_and_persists_state_hash_only() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    let body = connect(state.clone(), client.clone(), &session_id).await;
    assert_eq!(
        body["scopes"],
        serde_json::json!([GMAIL_READONLY, GMAIL_SEND])
    );

    let state_token = state_from_connect_response(&body);
    assert_eq!(state_token.len(), 64);
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT token_hash, requested_scopes_json FROM provider_oauth_states")
            .fetch_all(&state.db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].0, state_token);
    assert_eq!(
        rows[0].1,
        serde_json::json!([GMAIL_READONLY, GMAIL_SEND]).to_string()
    );

    let requests = client.auth_requests.lock().expect("auth requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].client_id, "gmail-client-id");
    assert_eq!(
        requests[0].scopes,
        vec![GMAIL_READONLY.to_owned(), GMAIL_SEND.to_owned()]
    );
}

#[tokio::test]
async fn gmail_connect_requires_auth_and_csrf() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    let no_csrf = app(state.clone(), client.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/provider-accounts/gmail/connect")
                .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(no_csrf).await["error"], "csrf_required");

    let no_cookie = app(state, client.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/provider-accounts/gmail/connect")
                .header(CSRF_HEADER, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_cookie.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(no_cookie).await["error"], "unauthorized");
    assert!(
        client
            .auth_requests
            .lock()
            .expect("auth requests")
            .is_empty()
    );
}

fn redirect_location(headers: &HeaderMap) -> &str {
    headers
        .get(header::LOCATION)
        .expect("redirect location")
        .to_str()
        .expect("location is valid header value")
}

async fn connected_account_id(state: &AppState, user_id: i64) -> i64 {
    sqlx::query_scalar(
        "SELECT id FROM mail_accounts WHERE user_id = ?1 AND backend_kind = 'gmail'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap()
}

async fn provider_account_count(state: &AppState, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM mail_accounts WHERE user_id = ?1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await
        .unwrap()
}

async fn consumed_state_count(state: &AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM provider_oauth_states WHERE consumed_at IS NOT NULL")
        .fetch_one(&state.db)
        .await
        .unwrap()
}

async fn seed_second_session_for_user(
    state: &AppState,
    key: &[u8; hail_core::KEY_LEN],
    user_id: i64,
) -> String {
    let now = chrono::Utc::now();
    let token_enc = hail_core::seal(b"dummy-token", key).expect("seal");
    let session_id = format!("{:064x}", user_id + 10_000);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&token_enc)
    .bind(Some("test-ua-2"))
    .bind(now + chrono::Duration::days(30))
    .bind(now)
    .execute(&state.db)
    .await
    .expect("insert second session");
    session_id
}

#[tokio::test]
async fn gmail_callback_stores_encrypted_refresh_token_and_consumes_state_once() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);

    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(resp.headers()),
        "/provider-accounts?connected=gmail"
    );

    let account_id = connected_account_id(&state, user_id).await;
    let (provider_account_id, encrypted): (String, Vec<u8>) = sqlx::query_as(
        "SELECT provider_account_id, refresh_token_enc FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(provider_account_id, "gmail.user@example.com");
    assert_ne!(encrypted, b"1//refresh-token-secret");
    assert!(
        !encrypted
            .windows(b"refresh-token-secret".len())
            .any(|window| window == b"refresh-token-secret")
    );

    let context = ProviderTokenContext::new(
        user_id,
        account_id,
        "gmail",
        provider_account_id,
        ProviderOAuthTokenKind::Refresh,
    );
    let decrypted = hail_core::open_provider_oauth_token(&encrypted, &key, &context).unwrap();
    assert_eq!(decrypted.expose_secret(), "1//refresh-token-secret");

    let token_len: i64 =
        sqlx::query_scalar("SELECT length(refresh_token_enc) FROM mail_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(
        token_len >= 29,
        "active account must never persist placeholder token ciphertext"
    );

    let replay = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(replay.headers()),
        "/provider-accounts?error=invalid_oauth_state"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gmail_callback_concurrently_consumes_oauth_state_once() {
    let (state, key) = file_app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    let uri = Arc::new(format!(
        "/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code"
    ));
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let callback = |app: Router, barrier: Arc<tokio::sync::Barrier>, uri: Arc<String>| {
        let session_id = session_id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(auth_request(Method::GET, uri.as_str(), &session_id))
                .await
                .unwrap()
        })
    };
    let first = callback(
        app(state.clone(), client.clone()),
        barrier.clone(),
        uri.clone(),
    );
    let second = callback(
        app(state.clone(), client.clone()),
        barrier.clone(),
        uri.clone(),
    );
    barrier.wait().await;

    let first = first.await.unwrap();
    let second = second.await.unwrap();
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    assert_eq!(second.status(), StatusCode::SEE_OTHER);
    let locations = [
        redirect_location(first.headers()).to_owned(),
        redirect_location(second.headers()).to_owned(),
    ];
    assert_eq!(
        locations
            .iter()
            .filter(|location| location.as_str() == "/provider-accounts?connected=gmail")
            .count(),
        1,
        "exactly one callback should connect: {locations:?}"
    );
    assert_eq!(
        locations
            .iter()
            .filter(|location| location.as_str() == "/provider-accounts?error=invalid_oauth_state")
            .count(),
        1,
        "exactly one callback should lose the state CAS: {locations:?}"
    );
    assert_eq!(
        client
            .exchange_codes
            .lock()
            .expect("exchange codes")
            .as_slice(),
        ["oauth-code"]
    );

    let account_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM mail_accounts WHERE user_id = ?1 AND backend_kind = 'gmail'",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(account_count, 1);
    let consumed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_oauth_states WHERE user_id = ?1 AND consumed_at IS NOT NULL",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(consumed_count, 1);
}

#[tokio::test]
async fn gmail_callback_rejects_unauthenticated_cross_user_cross_session_and_expired_state() {
    let (state, key) = app_state().await;
    let (alice_id, alice_session) = seed_session(&state, &key, "alice@example.com").await;
    let (_bob_id, bob_session) = seed_session(&state, &key, "bob@example.com").await;
    let alice_second_session = seed_second_session_for_user(&state, &key, alice_id).await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &alice_session).await);
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");

    let no_cookie = app(state.clone(), client.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(&uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_cookie.status(), StatusCode::UNAUTHORIZED);

    for session_id in [&bob_session, &alice_second_session] {
        let resp = app(state.clone(), client.clone())
            .oneshot(auth_request(Method::GET, &uri, session_id))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            redirect_location(resp.headers()),
            "/provider-accounts?error=invalid_oauth_state"
        );
    }
    assert_eq!(provider_account_count(&state, alice_id).await, 0);
    assert_eq!(consumed_state_count(&state).await, 0);
    assert!(
        client
            .exchange_codes
            .lock()
            .expect("exchange codes")
            .is_empty()
    );

    sqlx::query("UPDATE provider_oauth_states SET expires_at = ?1 WHERE consumed_at IS NULL")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .execute(&state.db)
        .await
        .unwrap();
    let expired = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &alice_session))
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(expired.headers()),
        "/provider-accounts?error=invalid_oauth_state"
    );
    assert_eq!(provider_account_count(&state, alice_id).await, 0);
    assert_eq!(consumed_state_count(&state).await, 0);
    assert!(
        client
            .exchange_codes
            .lock()
            .expect("exchange codes")
            .is_empty()
    );
}

#[tokio::test]
async fn gmail_callback_exchange_failure_consumes_state_without_creating_account() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    client.fail_next_exchange("temporary upstream failure");
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=bad-code");

    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(resp.headers()),
        "/provider-accounts?error=oauth_exchange_failed"
    );
    assert_eq!(
        client
            .exchange_codes
            .lock()
            .expect("exchange codes")
            .as_slice(),
        ["bad-code"]
    );
    assert_eq!(provider_account_count(&state, user_id).await, 0);
    assert_eq!(consumed_state_count(&state).await, 1);

    let replay = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(replay.headers()),
        "/provider-accounts?error=invalid_oauth_state"
    );
    assert_eq!(
        client
            .exchange_codes
            .lock()
            .expect("exchange codes")
            .as_slice(),
        ["bad-code"]
    );
    assert_eq!(provider_account_count(&state, user_id).await, 0);
}

#[tokio::test]
async fn gmail_callback_redirects_safe_errors_to_provider_accounts_page() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);

    for (uri, expected_location) in [
        (
            "/api/provider-accounts/gmail/callback?state=visible-state-token&error=access_denied&code=visible-code",
            "/provider-accounts?error=oauth_denied",
        ),
        (
            "/api/provider-accounts/gmail/callback?code=visible-code",
            "/provider-accounts?error=missing_state",
        ),
        (
            "/api/provider-accounts/gmail/callback?state=visible-state-token",
            "/provider-accounts?error=missing_code",
        ),
    ] {
        let resp = app(state.clone(), client.clone())
            .oneshot(auth_request(Method::GET, uri, &session_id))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(redirect_location(resp.headers()), expected_location);
        assert!(!redirect_location(resp.headers()).contains("visible-state-token"));
        assert!(!redirect_location(resp.headers()).contains("visible-code"));
    }

    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(
            Method::GET,
            &format!("/api/provider-accounts/gmail/callback?state={state_token}"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(resp.headers()),
        "/provider-accounts?error=missing_code"
    );
    assert_eq!(
        client.exchange_codes.lock().expect("exchange codes").len(),
        0
    );
}

#[tokio::test]
async fn sync_status_lists_only_importable_connected_gmail_accounts_with_canonical_statuses() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let (other_user_id, _other_session) = seed_session(&state, &key, "bob@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();

    async fn insert_account(
        state: &AppState,
        user_id: i64,
        provider_account_id: &str,
        email: &str,
        sync_status: &str,
        token: Option<Vec<u8>>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO mail_accounts \
             (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, display_email, \
              granted_scopes_json, refresh_token_enc, last_profile_history_id, profile_synced_at, sync_status, \
              last_sync_attempted_at, last_sync_succeeded_at, next_sync_after, sync_backoff_secs, \
              last_error_class, last_error_message, disconnected_at, revoked_at, created_at, updated_at) \
             VALUES (?1, 'acct-a', 'gmail', 'gmail', ?2, ?3, ?4, '[]', \
                     ?5, 'history-9', ?6, ?7, ?6, ?6, ?8, 120, \
                     'gmail_rate_limit', 'rate limited', \
                     CASE WHEN ?7 = 'disconnected' THEN ?6 ELSE NULL END, \
                     CASE WHEN ?7 = 'revoked' THEN ?6 ELSE NULL END, ?6, ?6) \
             RETURNING id",
        )
        .bind(user_id)
        .bind(provider_account_id)
        .bind(email)
        .bind(format!("{email} display"))
        .bind(token)
        .bind(now)
        .bind(sync_status)
        .bind(now + chrono::Duration::minutes(10))
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    let active_id = insert_account(
        &state,
        user_id,
        "gmail-active",
        "bravo@gmail.example",
        "active",
        Some(vec![1_u8; 29]),
        now,
    )
    .await;
    let error_id = insert_account(
        &state,
        user_id,
        "gmail-error",
        "alpha@gmail.example",
        "error",
        Some(vec![2_u8; 29]),
        now,
    )
    .await;
    let initial_id = insert_account(
        &state,
        user_id,
        "gmail-initial",
        "charlie@gmail.example",
        "initial_sync",
        Some(vec![3_u8; 29]),
        now,
    )
    .await;
    let revoked_id = insert_account(
        &state,
        user_id,
        "gmail-revoked",
        "delta@gmail.example",
        "revoked",
        None,
        now,
    )
    .await;
    let disconnected_id = insert_account(
        &state,
        user_id,
        "gmail-disconnected",
        "echo@gmail.example",
        "disconnected",
        None,
        now,
    )
    .await;
    let disabled_id = insert_account(
        &state,
        user_id,
        "gmail-disabled",
        "foxtrot@gmail.example",
        "disabled",
        None,
        now,
    )
    .await;
    let other_id = insert_account(
        &state,
        other_user_id,
        "gmail-bob",
        "bob@gmail.example",
        "active",
        Some(vec![4_u8; 29]),
        now,
    )
    .await;

    insert_provider_sync_audit_log(
        &state.db,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id: error_id,
            operation_kind: ProviderSyncOperationKind::Sync,
            event_type: ProviderSyncEventType::SyncStarted,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Started,
            safe_error_code: None,
            safe_error_class: None,
            safe_error_message: None,
            metadata_json: None,
        },
    )
    .await
    .unwrap();
    insert_provider_sync_audit_log(
        &state.db,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id: error_id,
            operation_kind: ProviderSyncOperationKind::Failure,
            event_type: ProviderSyncEventType::SyncFailed,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Failed,
            safe_error_code: None,
            safe_error_class: Some("gmail_rate_limit"),
            safe_error_message: Some("provider asked us to retry later"),
            metadata_json: None,
        },
    )
    .await
    .unwrap();

    let resp = app(state.clone(), client)
        .oneshot(auth_request(
            Method::GET,
            "/api/provider-accounts/sync-status",
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let accounts = body["accounts"].as_array().expect("accounts array");
    let ids: Vec<i64> = accounts
        .iter()
        .map(|account| account["id"].as_i64().expect("account id"))
        .collect();
    assert_eq!(ids, vec![error_id, active_id, initial_id]);
    assert!(!ids.contains(&revoked_id));
    assert!(!ids.contains(&disconnected_id));
    assert!(!ids.contains(&disabled_id));
    assert!(!ids.contains(&other_id));

    let statuses: Vec<&str> = accounts
        .iter()
        .map(|account| account["sync_status"].as_str().expect("sync_status"))
        .collect();
    assert_eq!(statuses, vec!["error", "active", "initial_sync"]);
    for status in statuses {
        assert!(
            ["active", "error", "initial_sync"].contains(&status),
            "sync-status endpoint returned non-importable or non-canonical status: {status}"
        );
    }

    let error_account = &accounts[0];
    assert_eq!(error_account["id"], error_id);
    assert_eq!(error_account["provider_kind"], "gmail");
    assert_eq!(error_account["provider_email"], "alpha@gmail.example");
    assert!(error_account["last_sync_succeeded_at"].as_str().is_some());
    assert!(error_account["next_sync_after"].as_str().is_some());
    assert_eq!(error_account["sync_backoff_secs"], 120);
    assert_eq!(error_account["last_error_class"], "gmail_rate_limit");
    assert!(error_account["last_error_message"].is_null());
    assert_eq!(error_account["last_profile_history_id"], "history-9");
    assert_eq!(
        error_account["last_sync_event"]["event_type"],
        "sync_failed"
    );
    assert_eq!(error_account["last_sync_event"]["result_status"], "failed");
    assert!(
        error_account["last_sync_event"]["created_at"]
            .as_str()
            .is_some()
    );
    assert_eq!(
        error_account["last_error_event"]["event_type"],
        "sync_failed"
    );
    assert_eq!(error_account["last_error_event"]["result_status"], "failed");
    assert_eq!(
        error_account["last_error_event"]["safe_error_message"],
        "provider asked us to retry later"
    );
    assert!(
        error_account["last_error_event"]["created_at"]
            .as_str()
            .is_some()
    );
}

#[tokio::test]
async fn provider_account_response_returns_internal_error_for_corrupt_granted_scopes_json() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let account_id: i64 = sqlx::query_scalar("SELECT id FROM mail_accounts LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();

    sqlx::query("DROP TRIGGER mail_accounts_json_state_update")
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE mail_accounts SET granted_scopes_json = ?1 WHERE id = ?2")
        .bind("not-json")
        .bind(account_id)
        .execute(&state.db)
        .await
        .unwrap();

    let resp = app(state, client)
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/disconnect"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn sync_status_output_redacts_hostile_error_fields() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();

    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          granted_scopes_json, refresh_token_enc, sync_status, last_error_class, last_error_message, \
          created_at, updated_at) \
         VALUES (?1, 'acct-a', 'gmail', 'gmail', 'gmail-alice', 'alice@gmail.example', '[]', \
                 ?2, 'error', 'hostile_error', ?3, ?4, ?4) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(vec![9_u8; 29])
    .bind(hostile_leak_error())
    .bind(now)
    .fetch_one(&state.db)
    .await
    .unwrap();
    insert_provider_sync_audit_log(
        &state.db,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id: account_id,
            operation_kind: ProviderSyncOperationKind::Failure,
            event_type: ProviderSyncEventType::SyncFailed,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Failed,
            safe_error_code: Some("hostile_error"),
            safe_error_class: Some("hostile_error"),
            safe_error_message: Some(hostile_leak_error()),
            metadata_json: None,
        },
    )
    .await
    .unwrap();

    let resp = app(state.clone(), client)
        .oneshot(auth_request(
            Method::GET,
            "/api/provider-accounts/sync-status",
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let rendered = serde_json::to_string(&body).expect("response json");
    assert_no_hostile_leak(&rendered);
    assert!(body["accounts"][0]["last_error_message"].is_null());
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn manual_sync_trigger_marks_only_importable_accounts_due_without_running_gmail() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let (other_user_id, _other_session) = seed_session(&state, &key, "bob@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let next_sync = now + chrono::Duration::hours(1);

    async fn insert_account(
        state: &AppState,
        user_id: i64,
        provider_account_id: &str,
        status: &str,
        token: Option<Vec<u8>>,
        next_sync: chrono::DateTime<chrono::Utc>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO mail_accounts \
             (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
              granted_scopes_json, refresh_token_enc, sync_status, next_sync_after, sync_backoff_secs, \
              last_error_class, last_error_message, disconnected_at, revoked_at, created_at, updated_at) \
             VALUES (?1, 'acct-a', 'gmail', 'gmail', ?2, ?3, '[]', \
                     ?4, ?5, ?6, 300, 'network', 'timeout', \
                     CASE WHEN ?5 = 'disconnected' THEN ?7 ELSE NULL END, \
                     CASE WHEN ?5 = 'revoked' THEN ?7 ELSE NULL END, ?7, ?7) \
             RETURNING id",
        )
        .bind(user_id)
        .bind(provider_account_id)
        .bind(format!("{provider_account_id}@gmail.example"))
        .bind(token)
        .bind(status)
        .bind(next_sync)
        .bind(now)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    let account_id = insert_account(
        &state,
        user_id,
        "gmail-alice",
        "error",
        Some(vec![3_u8; 29]),
        next_sync,
        now,
    )
    .await;
    let other_account_id = insert_account(
        &state,
        other_user_id,
        "gmail-bob",
        "active",
        Some(vec![4_u8; 29]),
        next_sync,
        now,
    )
    .await;
    let revoked_id = insert_account(
        &state,
        user_id,
        "gmail-revoked-sync",
        "revoked",
        None,
        next_sync,
        now,
    )
    .await;
    let disconnected_id = insert_account(
        &state,
        user_id,
        "gmail-disconnected-sync",
        "disconnected",
        None,
        next_sync,
        now,
    )
    .await;
    let disabled_id = insert_account(
        &state,
        user_id,
        "gmail-disabled-sync",
        "disabled",
        None,
        next_sync,
        now,
    )
    .await;

    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/sync"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["account"]["id"], account_id);
    assert_eq!(body["account"]["sync_status"], "error");
    assert!(body["account"]["next_sync_after"].is_null());
    assert!(body["account"]["sync_backoff_secs"].is_null());
    assert_eq!(
        client.exchange_codes.lock().expect("exchange codes").len(),
        0
    );

    let row: (String, Option<String>, Option<i64>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT sync_status, next_sync_after, sync_backoff_secs, last_error_class, last_error_message \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "error".to_owned(),
            None,
            None,
            Some("network".to_owned()),
            Some("timeout".to_owned())
        )
    );

    for denied_id in [other_account_id, revoked_id, disconnected_id, disabled_id] {
        let resp = app(state.clone(), client.clone())
            .oneshot(auth_request(
                Method::POST,
                &format!("/api/provider-accounts/{denied_id}/sync"),
                &session_id,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "account {denied_id}");
    }

    let denied_rows: Vec<(i64, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT id, next_sync_after, sync_backoff_secs, last_error_class \
         FROM mail_accounts WHERE id IN (?1, ?2, ?3, ?4) ORDER BY id",
    )
    .bind(other_account_id)
    .bind(revoked_id)
    .bind(disconnected_id)
    .bind(disabled_id)
    .fetch_all(&state.db)
    .await
    .unwrap();
    assert_eq!(denied_rows.len(), 4);
    for (_id, denied_next_sync, denied_backoff, denied_error_class) in denied_rows {
        assert!(denied_next_sync.is_some());
        assert_eq!(denied_backoff, Some(300));
        assert_eq!(denied_error_class.as_deref(), Some("network"));
    }
}

#[tokio::test]
async fn reimport_requires_auth() {
    let (state, _key) = app_state().await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    let resp = app(state, client)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/provider-accounts/123/reimport")
                .header(CSRF_HEADER, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(resp).await["error"], "unauthorized");
}

#[tokio::test]
async fn stop_requires_auth() {
    let (state, _key) = app_state().await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    let resp = app(state, client)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/provider-accounts/123/stop")
                .header(CSRF_HEADER, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(resp).await["error"], "unauthorized");
}

#[tokio::test]
async fn stop_active_provider_import_pauses_and_audits_for_owner_only() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let (other_user_id, _other_session) = seed_session(&state, &key, "bob@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let next_sync = now + chrono::Duration::hours(1);

    async fn insert_account(
        state: &AppState,
        user_id: i64,
        provider_account_id: &str,
        status: &str,
        next_sync: chrono::DateTime<chrono::Utc>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO mail_accounts \
             (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
              granted_scopes_json, refresh_token_enc, sync_status, next_sync_after, sync_backoff_secs, \
              last_error_class, last_error_message, created_at, updated_at) \
             VALUES (?1, 'acct-a', 'gmail', 'gmail', ?2, ?3, '[]', \
                     ?4, ?5, ?6, 300, 'network', 'timeout', ?7, ?7) \
             RETURNING id",
        )
        .bind(user_id)
        .bind(provider_account_id)
        .bind(format!("{provider_account_id}@gmail.example"))
        .bind(vec![3_u8; 29])
        .bind(status)
        .bind(next_sync)
        .bind(now)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    let active_id = insert_account(
        &state,
        user_id,
        "gmail-active-stop",
        "active",
        next_sync,
        now,
    )
    .await;
    let other_id = insert_account(
        &state,
        other_user_id,
        "gmail-bob-stop",
        "active",
        next_sync,
        now,
    )
    .await;

    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{active_id}/stop"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["account"]["id"], active_id);
    assert_eq!(body["account"]["sync_status"], "paused");
    assert!(body["account"]["next_sync_after"].is_null());
    assert!(body["account"]["sync_backoff_secs"].is_null());
    assert_eq!(body["account"]["last_error_class"], "operator_paused");
    assert_eq!(
        body["account"]["last_error_event"]["event_type"],
        "sync_paused"
    );

    let row: (String, Option<String>, Option<i64>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT sync_status, next_sync_after, sync_backoff_secs, last_error_class, last_error_message \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(active_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "paused".to_owned(),
            None,
            None,
            Some("operator_paused".to_owned()),
            None,
        )
    );

    let event: (String, String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT operation_kind, event_type, safe_error_class, safe_error_message \
         FROM provider_sync_events WHERE provider_account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(active_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(event.0, "sync");
    assert_eq!(event.1, "sync_paused");
    assert_eq!(event.2.as_deref(), Some("operator_paused"));
    assert_eq!(event.3.as_deref(), Some("Gmail import paused by operator"));

    let resp = app(state.clone(), client)
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{other_id}/stop"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stop_paused_provider_import_conflicts() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          granted_scopes_json, refresh_token_enc, sync_status, created_at, updated_at) \
         VALUES (?1, 'acct-a', 'gmail', 'gmail', 'gmail-paused-stop', 'paused-stop@gmail.example', \
                 '[]', ?2, 'paused', ?3, ?3) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(vec![3_u8; 29])
    .bind(now)
    .fetch_one(&state.db)
    .await
    .unwrap();

    let resp = app(state, client)
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/stop"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(resp).await["error"],
        "provider_sync_not_in_flight"
    );
}

#[tokio::test]
async fn reimport_disconnected_account_returns_not_found() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          granted_scopes_json, sync_status, disconnected_at, created_at, updated_at) \
         VALUES (?1, 'acct-a', 'gmail', 'gmail', 'gmail-disconnected-reimport', 'disconnected-reimport@gmail.example', \
                 '[]', 'disconnected', ?2, ?2, ?2) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .unwrap();

    let resp = app(state.clone(), client)
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/reimport"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(resp).await["error"], "provider_account_not_found");

    let row: (String, Option<String>, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT sync_status, disconnected_at, refresh_token_enc FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "disconnected");
    assert!(row.1.is_some());
    assert!(row.2.is_none());
}

#[tokio::test]
async fn reimport_active_account_resets_cursors_and_error_state() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let account_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO mail_accounts
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, display_email,
          granted_scopes_json, refresh_token_enc, last_profile_history_id, profile_synced_at,
          initial_sync_completed_at, backfill_cursor_json, sync_status, last_sync_attempted_at,
          last_sync_succeeded_at, next_sync_after, sync_backoff_secs, last_error_class, last_error_message,
          created_at, updated_at)
         VALUES (?1, 'acct-a', 'gmail', 'gmail', 'gmail-active-reimport', 'active-reimport@gmail.example',
                 'Active Reimport', '[]', ?2, 'history-99', ?3, ?3,
                 '{"kind":"gmail_historical_v1","completed":false}', 'error', ?3, ?3, ?4, 900,
                 'gmail_rate_limit', 'rate limited', ?3, ?3)
         RETURNING id"#,
    )
    .bind(user_id)
    .bind(vec![7_u8; 29])
    .bind(now)
    .bind(now + chrono::Duration::hours(1))
    .fetch_one(&state.db)
    .await
    .unwrap();
    insert_provider_sync_audit_log(
        &state.db,
        NewProviderSyncAuditLog {
            user_id,
            provider_account_id: account_id,
            operation_kind: ProviderSyncOperationKind::Failure,
            event_type: ProviderSyncEventType::SyncFailed,
            provider_message_id: None,
            result_status: ProviderSyncResultStatus::Failed,
            safe_error_code: Some("gmail_rate_limit"),
            safe_error_class: Some("gmail_rate_limit"),
            safe_error_message: Some("provider asked us to retry later"),
            metadata_json: None,
        },
    )
    .await
    .unwrap();

    let resp = app(state.clone(), client)
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/reimport"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["account"]["id"], account_id);
    assert_eq!(body["account"]["sync_status"], "initial_sync");
    assert!(body["account"]["last_profile_history_id"].is_null());
    assert!(body["account"]["last_error_class"].is_null());
    assert!(body["account"]["last_error_event"].is_null());
    assert!(body["account"]["next_sync_after"].is_null());
    assert!(body["account"]["sync_backoff_secs"].is_null());

    let row: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT sync_status, last_profile_history_id, profile_synced_at, initial_sync_completed_at, \
                backfill_cursor_json, last_error_class, sync_backoff_secs \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "initial_sync");
    assert!(row.1.is_none());
    assert!(row.2.is_none());
    assert!(row.3.is_none());
    assert!(row.4.is_none());
    assert!(row.5.is_none());
    assert!(row.6.is_none());
}

#[tokio::test]
async fn reimport_while_initial_sync_returns_conflict() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          granted_scopes_json, refresh_token_enc, last_profile_history_id, sync_status, \
          created_at, updated_at) \
         VALUES (?1, 'acct-a', 'gmail', 'gmail', 'gmail-initial-reimport', 'initial-reimport@gmail.example', \
                 '[]', ?2, 'history-100', 'initial_sync', ?3, ?3) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(vec![8_u8; 29])
    .bind(now)
    .fetch_one(&state.db)
    .await
    .unwrap();

    let resp = app(state.clone(), client)
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/reimport"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(resp).await["error"],
        "provider_reimport_already_running"
    );

    let cursor: Option<String> =
        sqlx::query_scalar("SELECT last_profile_history_id FROM mail_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(cursor.as_deref(), Some("history-100"));
}

#[tokio::test]
async fn manual_sync_trigger_requires_csrf() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    let resp = app(state.clone(), client.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/provider-accounts/123/sync")
                .header(header::COOKIE, format!("{SESSION_COOKIE}={session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn provider_account_mutations_require_auth() {
    let (state, _key) = app_state().await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    for uri in [
        "/api/provider-accounts/123/sync",
        "/api/provider-accounts/123/reimport",
        "/api/provider-accounts/123/disconnect",
    ] {
        let resp = app(state.clone(), client.clone())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header(CSRF_HEADER, "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(json_body(resp).await["error"], "unauthorized", "{uri}");
    }
}

#[tokio::test]
async fn disconnect_requires_csrf_and_owner_session() {
    let (state, key) = app_state().await;
    let (alice_id, alice_session) = seed_session(&state, &key, "alice@example.com").await;
    let (_bob_id, bob_session) = seed_session(&state, &key, "bob@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &alice_session).await);
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &alice_session))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let account_id = connected_account_id(&state, alice_id).await;
    let disconnect_uri = format!("/api/provider-accounts/{account_id}/disconnect");

    let no_csrf = app(state.clone(), client.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(&disconnect_uri)
                .header(header::COOKIE, format!("{SESSION_COOKIE}={alice_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
    assert!(
        client
            .revoked_tokens
            .lock()
            .expect("revoked tokens")
            .is_empty()
    );

    let other_user = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::POST, &disconnect_uri, &bob_session))
        .await
        .unwrap();
    assert_eq!(other_user.status(), StatusCode::NOT_FOUND);
    assert!(
        client
            .revoked_tokens
            .lock()
            .expect("revoked tokens")
            .is_empty()
    );

    let (status, token): (String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT sync_status, refresh_token_enc FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(status, "active");
    assert!(token.is_some());
}

#[tokio::test]
async fn disconnect_revokes_refresh_token_and_clears_local_secret() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let account_id = connected_account_id(&state, _user_id).await;

    let disconnect_uri = format!("/api/provider-accounts/{account_id}/disconnect");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::POST, &disconnect_uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["sync_status"], "disconnected");

    assert_eq!(
        client
            .revoked_tokens
            .lock()
            .expect("revoked tokens")
            .as_slice(),
        ["1//refresh-token-secret"]
    );
    let (status, token): (String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT sync_status, refresh_token_enc FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(status, "disconnected");
    assert!(token.is_none());
}

#[tokio::test]
async fn disconnect_succeeds_locally_when_provider_revoke_fails_and_hides_token_material() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let account_id = connected_account_id(&state, user_id).await;
    client.fail_next_revoke(hostile_leak_error());

    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/disconnect"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["sync_status"], "disconnected");
    let rendered = serde_json::to_string(&body).expect("response json");
    assert!(!rendered.contains("refresh-token-secret"));
    assert_no_hostile_leak(&rendered);

    assert_eq!(
        client
            .revoked_tokens
            .lock()
            .expect("revoked tokens")
            .as_slice(),
        ["1//refresh-token-secret"]
    );
    let (status, token, token_ref, key_id, disconnected_at): (
        String,
        Option<Vec<u8>>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT sync_status, refresh_token_enc, refresh_token_ref, refresh_token_key_id, disconnected_at \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(status, "disconnected");
    assert!(token.is_none());
    assert!(token_ref.is_none());
    assert!(key_id.is_none());
    assert!(disconnected_at.is_some());
}

#[tokio::test]
async fn disconnect_already_disconnected_account_returns_not_found_without_revoke() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let now = chrono::Utc::now();
    let account_id: i64 = sqlx::query_scalar(
        "INSERT INTO mail_accounts \
         (user_id, jmap_account_id, backend_kind, provider_kind, provider_account_id, provider_email, \
          granted_scopes_json, sync_status, disconnected_at, created_at, updated_at) \
         VALUES (?1, 'acct-a', 'gmail', 'gmail', 'gmail-disconnected', 'disconnected@gmail.example', \
                 '[]', 'disconnected', ?2, ?2, ?2) \
         RETURNING id",
    )
    .bind(user_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    .unwrap();

    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(
            Method::POST,
            &format!("/api/provider-accounts/{account_id}/disconnect"),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(resp).await["error"], "provider_account_not_found");
    assert!(
        client
            .revoked_tokens
            .lock()
            .expect("revoked tokens")
            .is_empty()
    );

    let row: (String, Option<String>, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT sync_status, disconnected_at, refresh_token_enc FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "disconnected");
    assert!(row.1.is_some());
    assert!(row.2.is_none());
}

#[tokio::test]
async fn reconnect_clears_stale_disconnect_error_and_retry_state() {
    let (state, key) = app_state().await;
    let (user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());
    let state_token =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    let uri = format!("/api/provider-accounts/gmail/callback?state={state_token}&code=oauth-code");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let account_id = connected_account_id(&state, user_id).await;
    let original_token: Vec<u8> =
        sqlx::query_scalar("SELECT refresh_token_enc FROM mail_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.db)
            .await
            .unwrap();

    let stale_time = chrono::Utc::now() - chrono::Duration::days(1);
    sqlx::query(
        "UPDATE mail_accounts \
         SET sync_status = 'disconnected', refresh_token_enc = NULL, refresh_token_key_id = NULL, \
             disconnected_at = ?2, revoked_at = ?2, last_error_class = 'old_error', \
             last_error_message = 'old failure', next_sync_after = ?2, sync_backoff_secs = 900 \
         WHERE id = ?1",
    )
    .bind(account_id)
    .bind(stale_time)
    .execute(&state.db)
    .await
    .unwrap();

    let reconnect_state =
        state_from_connect_response(&connect(state.clone(), client.clone(), &session_id).await);
    let reconnect_uri =
        format!("/api/provider-accounts/gmail/callback?state={reconnect_state}&code=oauth-code-2");
    let resp = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &reconnect_uri, &session_id))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        redirect_location(resp.headers()),
        "/provider-accounts?connected=gmail"
    );

    let row: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Vec<u8>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT sync_status, disconnected_at, revoked_at, last_error_class, last_error_message, \
                sync_backoff_secs, refresh_token_enc, next_sync_after \
         FROM mail_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(row.0, "active");
    assert!(row.1.is_none(), "disconnected_at should be cleared");
    assert!(row.2.is_none(), "revoked_at should be cleared");
    assert!(row.3.is_none(), "last_error_class should be cleared");
    assert!(row.4.is_none(), "last_error_message should be cleared");
    assert!(row.5.is_none(), "sync_backoff_secs should be cleared");
    assert_ne!(
        row.6, original_token,
        "refresh token ciphertext should be replaced"
    );
    assert!(
        row.7.is_none(),
        "next_sync_after should be cleared for immediate import eligibility"
    );
}

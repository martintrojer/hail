use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hail_api::middleware::auth::CSRF_HEADER;
use hail_api::middleware::session::SESSION_COOKIE;
use hail_api::routes::provider_accounts::{
    GmailAuthorizationRequest, GmailOAuthClient, GmailOAuthError, GmailProfile, GmailTokenExchange,
};
use hail_api::state::AppState;
use hail_core::{ProviderOAuthTokenKind, ProviderTokenContext};
use hail_test::{fixture_state, json_body, seed_session};
use secrecy::{ExposeSecret, SecretString};
use tower::ServiceExt;

const GMAIL_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";

#[derive(Default)]
struct FakeGmailOAuthClient {
    auth_requests: Mutex<Vec<GmailAuthorizationRequest>>,
    exchange_codes: Mutex<Vec<String>>,
    revoked_tokens: Mutex<Vec<String>>,
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
            Ok(GmailTokenExchange {
                access_token: SecretString::from("ya29.access-token-secret"),
                refresh_token: Some(SecretString::from("1//refresh-token-secret")),
                expires_at: None,
                granted_scopes: vec![GMAIL_READONLY.to_owned()],
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
            Ok(())
        })
    }
}

async fn app_state() -> (AppState, [u8; hail_core::KEY_LEN]) {
    let (mut state, key) = fixture_state().await;
    state.config.provider_import.gmail.oauth_client_id = Some("gmail-client-id".to_owned());
    (state, key)
}

fn app(state: AppState, client: Arc<FakeGmailOAuthClient>) -> Router {
    let protected = hail_api::routes::provider_accounts::router_with_client(client).layer(
        axum::middleware::from_fn_with_state(
            state.clone(),
            hail_api::middleware::auth::require_auth,
        ),
    );
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
async fn gmail_connect_returns_readonly_authorization_url_and_persists_state_hash_only() {
    let (state, key) = app_state().await;
    let (_user_id, session_id) = seed_session(&state, &key, "alice@example.com").await;
    let client = Arc::new(FakeGmailOAuthClient::default());

    let body = connect(state.clone(), client.clone(), &session_id).await;
    assert_eq!(body["scopes"], serde_json::json!([GMAIL_READONLY]));

    let state_token = state_from_connect_response(&body);
    assert_eq!(state_token.len(), 64);
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT token_hash, requested_scopes_json FROM provider_oauth_states")
            .fetch_all(&state.db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].0, state_token);
    assert_eq!(rows[0].1, serde_json::json!([GMAIL_READONLY]).to_string());

    let requests = client.auth_requests.lock().expect("auth requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].client_id, "gmail-client-id");
    assert_eq!(requests[0].scopes, vec![GMAIL_READONLY.to_owned()]);
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
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["provider_kind"], "gmail");
    assert_eq!(body["provider_email"], "gmail.user@example.com");
    assert_eq!(body["granted_scopes"], serde_json::json!([GMAIL_READONLY]));
    assert_eq!(body["last_profile_history_id"], "12345");

    let account_id = body["id"].as_i64().expect("account id");
    let (provider_account_id, encrypted): (String, Vec<u8>) = sqlx::query_as(
        "SELECT provider_account_id, refresh_token_enc FROM provider_accounts WHERE id = ?1",
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

    let replay = app(state.clone(), client.clone())
        .oneshot(auth_request(Method::GET, &uri, &session_id))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
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
    let account_id = json_body(resp).await["id"].as_i64().unwrap();

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
        "SELECT sync_status, refresh_token_enc FROM provider_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(status, "disconnected");
    assert!(token.is_none());
}

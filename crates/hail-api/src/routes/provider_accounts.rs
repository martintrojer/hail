//! Gmail OAuth connect/callback/disconnect endpoints for provider import mode.

use std::{future::Future, pin::Pin, sync::Arc};

use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use chrono::{Duration, Utc};
use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use url::Url;
use utoipa::ToSchema;
use utoipa_axum::{
    router::{OpenApiRouter, UtoipaMethodRouterExt},
    routes,
};

use crate::{
    middleware::auth::{AuthSession, AuthUser},
    routes::response::{error_response, internal},
    state::AppState,
};

pub const TAG: &str = "provider-accounts";
const GMAIL_PROVIDER_KIND: &str = "gmail";
const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const OAUTH_STATE_TTL_MINUTES: i64 = 10;
const PROVIDER_REFRESH_TOKEN_KEY_ID: &str = "server_key:v1";

#[derive(Debug, Clone)]
pub struct GmailAuthorizationRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub state: String,
    pub scopes: Vec<String>,
}

#[derive(Clone)]
pub struct GmailTokenExchange {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub granted_scopes: Vec<String>,
    pub profile: GmailProfile,
}

impl std::fmt::Debug for GmailTokenExchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GmailTokenExchange")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .field("granted_scopes", &self.granted_scopes)
            .field("profile", &self.profile)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct GmailProfile {
    pub email: String,
    pub history_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GmailOAuthError {
    #[error("gmail oauth is not configured")]
    NotConfigured,
    #[error("gmail oauth authorization URL could not be built")]
    AuthorizationUrl,
    #[error("gmail oauth exchange failed: {0}")]
    Exchange(String),
    #[error("gmail token revoke failed: {0}")]
    Revoke(String),
}

pub trait GmailOAuthClient: Send + Sync + 'static {
    fn authorization_url(&self, req: GmailAuthorizationRequest) -> Result<String, GmailOAuthError>;
    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        redirect_uri: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<GmailTokenExchange, GmailOAuthError>> + Send + 'a>>;
    fn revoke_refresh_token<'a>(
        &'a self,
        refresh_token: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<(), GmailOAuthError>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct LiveGmailOAuthClient {
    http: reqwest::Client,
    client_id: Option<String>,
    client_secret: Option<SecretString>,
    auth_url: String,
    token_url: String,
    revoke_url: String,
    gmail_api_url: String,
}

impl LiveGmailOAuthClient {
    #[must_use]
    pub fn from_config(config: &hail_core::Config) -> Self {
        let gmail = &config.provider_import.gmail;
        Self {
            http: reqwest::Client::new(),
            client_id: gmail.oauth_client_id.clone(),
            client_secret: gmail.oauth_client_secret.clone(),
            auth_url: gmail
                .oauth_auth_url
                .clone()
                .unwrap_or_else(|| "https://accounts.google.com/o/oauth2/v2/auth".to_string()),
            token_url: gmail
                .oauth_token_url
                .clone()
                .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()),
            revoke_url: gmail
                .oauth_revoke_url
                .clone()
                .unwrap_or_else(|| "https://oauth2.googleapis.com/revoke".to_string()),
            gmail_api_url: gmail
                .api_base_url
                .clone()
                .unwrap_or_else(|| "https://gmail.googleapis.com".to_string()),
        }
    }
}

impl GmailOAuthClient for LiveGmailOAuthClient {
    fn authorization_url(&self, req: GmailAuthorizationRequest) -> Result<String, GmailOAuthError> {
        let mut url = Url::parse(&self.auth_url).map_err(|_| GmailOAuthError::AuthorizationUrl)?;
        url.query_pairs_mut()
            .append_pair("client_id", &req.client_id)
            .append_pair("redirect_uri", &req.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &req.scopes.join(" "))
            .append_pair("state", &req.state)
            .append_pair("access_type", "offline")
            .append_pair("include_granted_scopes", "true")
            .append_pair("prompt", "consent");
        Ok(url.into())
    }

    fn exchange_code<'a>(
        &'a self,
        code: &'a str,
        redirect_uri: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<GmailTokenExchange, GmailOAuthError>> + Send + 'a>>
    {
        Box::pin(async move {
            let client_id = self
                .client_id
                .as_deref()
                .ok_or(GmailOAuthError::NotConfigured)?;
            let client_secret = self
                .client_secret
                .as_ref()
                .ok_or(GmailOAuthError::NotConfigured)?;
            let body = {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                form.append_pair("code", code);
                form.append_pair("client_id", client_id);
                form.append_pair("client_secret", client_secret.expose_secret());
                form.append_pair("redirect_uri", redirect_uri);
                form.append_pair("grant_type", "authorization_code");
                form.finish()
            };

            let token: GoogleTokenResponse = self
                .http
                .post(&self.token_url)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|err| GmailOAuthError::Exchange(err.to_string()))?
                .error_for_status()
                .map_err(|err| GmailOAuthError::Exchange(err.to_string()))?
                .json()
                .await
                .map_err(|err| GmailOAuthError::Exchange(err.to_string()))?;

            let profile_url = format!(
                "{}/gmail/v1/users/me/profile",
                self.gmail_api_url.trim_end_matches('/')
            );
            let profile: GoogleProfileResponse = self
                .http
                .get(profile_url)
                .bearer_auth(token.access_token.expose_secret())
                .send()
                .await
                .map_err(|err| GmailOAuthError::Exchange(err.to_string()))?
                .error_for_status()
                .map_err(|err| GmailOAuthError::Exchange(err.to_string()))?
                .json()
                .await
                .map_err(|err| GmailOAuthError::Exchange(err.to_string()))?;

            Ok(GmailTokenExchange {
                access_token: token.access_token,
                refresh_token: token.refresh_token,
                expires_at: token
                    .expires_in
                    .and_then(|s| i64::try_from(s).ok())
                    .map(|s| Utc::now() + Duration::seconds(s)),
                granted_scopes: token
                    .scope
                    .as_deref()
                    .map(split_scopes)
                    .unwrap_or_else(|| vec![GMAIL_READONLY_SCOPE.to_string()]),
                profile: GmailProfile {
                    email: profile.email_address,
                    history_id: profile.history_id,
                },
            })
        })
    }

    fn revoke_refresh_token<'a>(
        &'a self,
        refresh_token: SecretString,
    ) -> Pin<Box<dyn Future<Output = Result<(), GmailOAuthError>> + Send + 'a>> {
        Box::pin(async move {
            let body = {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                form.append_pair("token", refresh_token.expose_secret());
                form.finish()
            };
            self.http
                .post(&self.revoke_url)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|err| GmailOAuthError::Revoke(err.to_string()))?
                .error_for_status()
                .map_err(|err| GmailOAuthError::Revoke(err.to_string()))?;
            Ok(())
        })
    }
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    #[serde(deserialize_with = "deserialize_secret")]
    access_token: SecretString,
    #[serde(default, deserialize_with = "deserialize_optional_secret")]
    refresh_token: Option<SecretString>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleProfileResponse {
    email_address: String,
    history_id: Option<String>,
}

fn deserialize_secret<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(SecretString::from)
}

fn deserialize_optional_secret<'de, D>(deserializer: D) -> Result<Option<SecretString>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(|value| value.map(SecretString::from))
}

pub fn router() -> Router<AppState> {
    router_with_client(Arc::new(LiveGmailOAuthClient::from_config_default()))
}

pub fn openapi_router() -> OpenApiRouter<AppState> {
    openapi_router_with_client(Arc::new(LiveGmailOAuthClient::from_config_default()))
}

impl LiveGmailOAuthClient {
    fn from_config_default() -> LiveGmailOAuthClient {
        LiveGmailOAuthClient {
            http: reqwest::Client::new(),
            client_id: None,
            client_secret: None,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            revoke_url: "https://oauth2.googleapis.com/revoke".to_string(),
            gmail_api_url: "https://gmail.googleapis.com".to_string(),
        }
    }
}

pub fn router_with_client<P>(client: Arc<P>) -> Router<AppState>
where
    P: GmailOAuthClient,
{
    let client: Arc<dyn GmailOAuthClient> = client;
    Router::new()
        .route(
            "/api/provider-accounts/gmail/connect",
            axum::routing::post(connect_gmail),
        )
        .route(
            "/api/provider-accounts/gmail/callback",
            axum::routing::get(gmail_callback),
        )
        .route(
            "/api/provider-accounts/{id}/disconnect",
            axum::routing::post(disconnect_provider_account),
        )
        .layer(Extension(client))
}

pub fn openapi_router_with_client<P>(client: Arc<P>) -> OpenApiRouter<AppState>
where
    P: GmailOAuthClient,
{
    let client: Arc<dyn GmailOAuthClient> = client;
    OpenApiRouter::new()
        .routes(routes!(connect_gmail).layer(Extension(client.clone())))
        .routes(routes!(gmail_callback).layer(Extension(client.clone())))
        .routes(routes!(disconnect_provider_account).layer(Extension(client)))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GmailConnectResponse {
    pub authorization_url: String,
    #[schema(value_type = Vec<String>)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProviderAccountResponse {
    pub id: i64,
    pub provider_kind: String,
    pub provider_account_id: String,
    pub provider_email: String,
    pub display_email: Option<String>,
    pub granted_scopes: Vec<String>,
    pub sync_status: String,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub cached_access_token_expires_at: Option<chrono::DateTime<Utc>>,
    pub last_profile_history_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GmailCallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

#[utoipa::path(post, path = "/api/provider-accounts/gmail/connect", tag = TAG,
    responses((status = 200, description = "Gmail OAuth authorization URL.", body = GmailConnectResponse)))]
async fn connect_gmail(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Extension(session): Extension<AuthSession>,
    Extension(client): Extension<Arc<dyn GmailOAuthClient>>,
) -> Response {
    let Some(client_id) = state.config.provider_import.gmail.oauth_client_id.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "gmail_oauth_not_configured",
        );
    };
    let redirect_uri = gmail_redirect_uri(&state);
    let scopes = vec![GMAIL_READONLY_SCOPE.to_string()];
    let state_token = match create_oauth_state(
        &state.db,
        user.id,
        &session.id,
        &redirect_uri,
        &scopes,
    )
    .await
    {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, "gmail oauth: state creation failed");
            return internal();
        }
    };
    let authorization_url = match client.authorization_url(GmailAuthorizationRequest {
        client_id,
        redirect_uri,
        state: state_token,
        scopes: scopes.clone(),
    }) {
        Ok(url) => url,
        Err(err) => {
            tracing::warn!(error = %err, user_id = user.id, "gmail oauth: authorization URL failed");
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "gmail_oauth_not_configured",
            );
        }
    };
    Json(GmailConnectResponse {
        authorization_url,
        scopes,
    })
    .into_response()
}

#[utoipa::path(get, path = "/api/provider-accounts/gmail/callback", tag = TAG,
    params(("state" = Option<String>, Query), ("code" = Option<String>, Query), ("error" = Option<String>, Query)),
    responses((status = 303, description = "Redirects to provider accounts SPA route after Gmail OAuth callback.")))]
async fn gmail_callback(
    State(state): State<AppState>,
    Query(query): Query<GmailCallbackQuery>,
    Extension(user): Extension<AuthUser>,
    Extension(session): Extension<AuthSession>,
    Extension(client): Extension<Arc<dyn GmailOAuthClient>>,
) -> Response {
    if query.error.is_some() {
        return provider_accounts_callback_redirect("error", "oauth_denied");
    }
    let Some(state_token) = query.state.as_deref() else {
        return provider_accounts_callback_redirect("error", "missing_state");
    };
    let Some(code) = query.code.as_deref() else {
        return provider_accounts_callback_redirect("error", "missing_code");
    };
    let oauth_state = match consume_oauth_state(&state.db, user.id, &session.id, state_token).await
    {
        Ok(Some(row)) => row,
        Ok(None) => return provider_accounts_callback_redirect("error", "invalid_oauth_state"),
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, "gmail oauth: state lookup failed");
            return provider_accounts_callback_redirect("error", "callback_failed");
        }
    };
    let exchange = match client.exchange_code(code, &oauth_state.redirect_uri).await {
        Ok(exchange) => exchange,
        Err(err) => {
            tracing::warn!(error = %err, user_id = user.id, "gmail oauth: exchange failed");
            return provider_accounts_callback_redirect("error", "oauth_exchange_failed");
        }
    };
    match upsert_provider_account(&state, &user, exchange).await {
        Ok(_) => provider_accounts_callback_redirect("connected", GMAIL_PROVIDER_KIND),
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, "gmail oauth: account store failed");
            provider_accounts_callback_redirect("error", "callback_failed")
        }
    }
}

fn provider_accounts_callback_redirect(key: &str, value: &str) -> Response {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(key, value)
        .finish();
    Redirect::to(&format!("/provider-accounts?{query}")).into_response()
}

#[utoipa::path(post, path = "/api/provider-accounts/{id}/disconnect", tag = TAG,
    params(("id" = i64, Path)),
    responses((status = 200, description = "Provider account disconnected.", body = ProviderAccountResponse)))]
async fn disconnect_provider_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Extension(user): Extension<AuthUser>,
    Extension(client): Extension<Arc<dyn GmailOAuthClient>>,
) -> Response {
    let token = match load_refresh_token_for_disconnect(&state, user.id, id).await {
        Ok(token) => token,
        Err(sqlx::Error::RowNotFound) => {
            return error_response(StatusCode::NOT_FOUND, "provider_account_not_found");
        }
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, provider_account_id = id, "gmail oauth: disconnect token load failed");
            return internal();
        }
    };
    if let Some(refresh_token) = token {
        if let Err(err) = client.revoke_refresh_token(refresh_token).await {
            tracing::warn!(error = %err, user_id = user.id, provider_account_id = id, "gmail oauth: provider revoke failed; disconnecting locally");
        }
    }
    match mark_provider_account_disconnected(&state.db, user.id, id).await {
        Ok(account) => Json(account).into_response(),
        Err(sqlx::Error::RowNotFound) => {
            error_response(StatusCode::NOT_FOUND, "provider_account_not_found")
        }
        Err(err) => {
            tracing::error!(error = %err, user_id = user.id, provider_account_id = id, "gmail oauth: disconnect update failed");
            internal()
        }
    }
}

#[derive(Debug)]
struct OAuthStateRow {
    redirect_uri: String,
}

async fn create_oauth_state(
    db: &SqlitePool,
    user_id: i64,
    session_id: &str,
    redirect_uri: &str,
    scopes: &[String],
) -> Result<String, sqlx::Error> {
    let token = random_token().map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    let token_hash = token_hash(&token);
    let now = Utc::now();
    let expires_at = now + Duration::minutes(OAUTH_STATE_TTL_MINUTES);
    sqlx::query(
        "INSERT INTO provider_oauth_states (token_hash, user_id, session_id, provider_kind, redirect_uri, requested_scopes_json, expires_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(session_id)
    .bind(GMAIL_PROVIDER_KIND)
    .bind(redirect_uri)
    .bind(serde_json::to_string(scopes).unwrap_or_else(|_| "[]".to_string()))
    .bind(expires_at)
    .bind(now)
    .execute(db)
    .await?;
    Ok(token)
}

async fn consume_oauth_state(
    db: &SqlitePool,
    user_id: i64,
    session_id: &str,
    state: &str,
) -> Result<Option<OAuthStateRow>, sqlx::Error> {
    let now = Utc::now();
    let state_hash = token_hash(state);
    let row = sqlx::query_as::<_, (String,)>(
        "UPDATE provider_oauth_states SET consumed_at = ?1 \
         WHERE token_hash = ?2 AND user_id = ?3 AND session_id = ?4 AND provider_kind = ?5 AND consumed_at IS NULL AND expires_at > ?6 \
         RETURNING redirect_uri",
    )
    .bind(now)
    .bind(state_hash)
    .bind(user_id)
    .bind(session_id)
    .bind(GMAIL_PROVIDER_KIND)
    .bind(now)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(redirect_uri,)| OAuthStateRow { redirect_uri }))
}

async fn upsert_provider_account(
    state: &AppState,
    user: &AuthUser,
    exchange: GmailTokenExchange,
) -> Result<ProviderAccountResponse, sqlx::Error> {
    let profile_email = exchange.profile.email.trim().to_lowercase();
    let provider_account_id = profile_email.clone();
    let scopes = normalize_scopes(exchange.granted_scopes);
    if !scopes.iter().any(|scope| scope == GMAIL_READONLY_SCOPE) {
        return Err(sqlx::Error::Protocol(
            "gmail.readonly scope missing".to_string(),
        ));
    }
    let Some(refresh_token) = exchange.refresh_token else {
        return Err(sqlx::Error::Protocol(
            "gmail refresh token missing".to_string(),
        ));
    };

    let now = Utc::now();
    let jmap_account_id: String =
        sqlx::query_scalar("SELECT jmap_account_id FROM users WHERE id = ?1")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
    let scopes_json =
        serde_json::to_string(&scopes).map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    let mut tx = state.db.begin().await?;
    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO provider_accounts \
         (user_id, jmap_account_id, provider_kind, provider_account_id, provider_email, display_email, granted_scopes_json, consented_at, \
          refresh_token_enc, refresh_token_key_id, cached_access_token_expires_at, access_token_refreshed_at, last_profile_history_id, profile_synced_at, \
          sync_status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, x'', ?9, ?10, ?8, ?11, ?8, 'active', ?8, ?8) \
         ON CONFLICT(user_id, provider_kind, provider_account_id) DO UPDATE SET \
          jmap_account_id = excluded.jmap_account_id, provider_email = excluded.provider_email, display_email = excluded.display_email, \
          granted_scopes_json = excluded.granted_scopes_json, consented_at = excluded.consented_at, refresh_token_enc = excluded.refresh_token_enc, \
          refresh_token_key_id = excluded.refresh_token_key_id, \
          cached_access_token_expires_at = excluded.cached_access_token_expires_at, access_token_refreshed_at = excluded.access_token_refreshed_at, \
          last_profile_history_id = excluded.last_profile_history_id, profile_synced_at = excluded.profile_synced_at, sync_status = 'active', \
          disconnected_at = NULL, revoked_at = NULL, last_error_class = NULL, last_error_message = NULL, updated_at = excluded.updated_at \
         RETURNING id",
    )
    .bind(user.id)
    .bind(jmap_account_id)
    .bind(GMAIL_PROVIDER_KIND)
    .bind(&provider_account_id)
    .bind(&profile_email)
    .bind(&profile_email)
    .bind(&scopes_json)
    .bind(now)
    .bind(PROVIDER_REFRESH_TOKEN_KEY_ID)
    .bind(exchange.expires_at)
    .bind(exchange.profile.history_id.as_deref())
    .fetch_one(&mut *tx)
    .await?;

    let context = hail_core::ProviderTokenContext::new(
        user.id,
        row_id,
        GMAIL_PROVIDER_KIND,
        &provider_account_id,
        hail_core::ProviderOAuthTokenKind::Refresh,
    );
    let encrypted = hail_core::seal_provider_oauth_token(
        &hail_core::ProviderOAuthToken::from(refresh_token),
        &state.server_key,
        &context,
    )
    .map_err(|err| sqlx::Error::Protocol(err.to_string()))?
    .into_bytes();

    sqlx::query(
        "UPDATE provider_accounts SET refresh_token_enc = ?1, updated_at = ?2 WHERE id = ?3",
    )
    .bind(encrypted)
    .bind(now)
    .bind(row_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    load_provider_account_response(&state.db, user.id, row_id).await
}

async fn load_refresh_token_for_disconnect(
    state: &AppState,
    user_id: i64,
    provider_account_id: i64,
) -> Result<Option<SecretString>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, Vec<u8>)>(
        "SELECT provider_account_id, refresh_token_enc FROM provider_accounts WHERE id = ?1 AND user_id = ?2 AND sync_status != 'disconnected'",
    )
    .bind(provider_account_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    let (provider_external_id, ciphertext) = row;
    let context = hail_core::ProviderTokenContext::new(
        user_id,
        provider_account_id,
        GMAIL_PROVIDER_KIND,
        provider_external_id,
        hail_core::ProviderOAuthTokenKind::Refresh,
    );
    let token = hail_core::open_provider_oauth_token(&ciphertext, &state.server_key, &context)
        .map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
    Ok(Some(SecretString::from(token.expose_secret().to_string())))
}

async fn mark_provider_account_disconnected(
    db: &SqlitePool,
    user_id: i64,
    provider_account_id: i64,
) -> Result<ProviderAccountResponse, sqlx::Error> {
    let now = Utc::now();
    sqlx::query(
        "UPDATE provider_accounts SET refresh_token_enc = NULL, refresh_token_ref = NULL, refresh_token_key_id = NULL, sync_status = 'disconnected', disconnected_at = ?1, updated_at = ?1 WHERE id = ?2 AND user_id = ?3",
    )
    .bind(now)
    .bind(provider_account_id)
    .bind(user_id)
    .execute(db)
    .await?;
    load_provider_account_response(db, user_id, provider_account_id).await
}

async fn load_provider_account_response(
    db: &SqlitePool,
    user_id: i64,
    provider_account_id: i64,
) -> Result<ProviderAccountResponse, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, String, String, String, Option<String>, String, String, Option<chrono::DateTime<Utc>>, Option<String>)>(
        "SELECT id, provider_kind, provider_account_id, provider_email, display_email, granted_scopes_json, sync_status, cached_access_token_expires_at, last_profile_history_id \
         FROM provider_accounts WHERE id = ?1 AND user_id = ?2",
    )
    .bind(provider_account_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(row_to_response(row))
}

fn row_to_response(
    row: (
        i64,
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        Option<chrono::DateTime<Utc>>,
        Option<String>,
    ),
) -> ProviderAccountResponse {
    let (
        id,
        provider_kind,
        provider_account_id,
        provider_email,
        display_email,
        granted_scopes_json,
        sync_status,
        cached_access_token_expires_at,
        last_profile_history_id,
    ) = row;
    ProviderAccountResponse {
        id,
        provider_kind,
        provider_account_id,
        provider_email,
        display_email,
        granted_scopes: serde_json::from_str(&granted_scopes_json).unwrap_or_default(),
        sync_status,
        cached_access_token_expires_at,
        last_profile_history_id,
    }
}

fn gmail_redirect_uri(state: &AppState) -> String {
    format!(
        "{}/api/provider-accounts/gmail/callback",
        state.config.server.public_url.trim_end_matches('/')
    )
}

fn random_token() -> Result<String, rand::rand_core::OsError> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut bytes)?;
    Ok(hex::encode(bytes))
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn split_scopes(scopes: &str) -> Vec<String> {
    normalize_scopes(scopes.split_whitespace().map(str::to_string).collect())
}

fn normalize_scopes(scopes: Vec<String>) -> Vec<String> {
    let mut scopes: Vec<String> = scopes
        .into_iter()
        .filter(|scope| !scope.trim().is_empty())
        .collect();
    scopes.sort();
    scopes.dedup();
    scopes
}

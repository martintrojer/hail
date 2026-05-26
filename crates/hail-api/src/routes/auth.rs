//! `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me`.
//!
//! Login flow (design.md §7.3, §10):
//!   1. Validate credentials against Stalwart via `hail_jmap::login_basic`.
//!   2. Upsert the `users` row. If `[admin]` is configured in `hail.toml`,
//!      that Stalwart-authenticated email is elevated on successful login;
//!      otherwise the first successful login becomes admin (DD-9).
//!   3. Synthesize a JMAP bearer token. For Stalwart, the bearer token
//!      IS the Basic credentials base64-encoded (`base64("user:pass")`);
//!      Stalwart's HTTP layer accepts that under both `Authorization:
//!      Basic …` and `Authorization: Bearer …`. We chose this over
//!      issuing a fresh server token because:
//!        * `jmap-client` doesn't expose a "create OAuth-style token"
//!          call on Stalwart at this version;
//!        * round-tripping the same credentials keeps the server side
//!          unchanged and the failure mode obvious — if the password
//!          changes, every old session breaks loudly at the next request.
//!          We encrypt the bearer at rest via AES-256-GCM (`hail_core::seal`)
//!          so the SQLite file alone does not yield usable tokens.
//!   4. Mint a 256-bit random session id (hex-encoded — `2 * 32 = 64`
//!      chars — never derived from any user input).
//!   5. INSERT the row with 30-day expiry, set the cookie, return the
//!      `user` payload.
//!
//! Logout: DELETE the session row (best-effort — missing rows still
//! return 204) and clear the cookie with `Max-Age=0`.
//!
//! `me`: served behind the auth middleware, returns the `AuthUser` view.

use std::future::Future;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Extension, FromRequest, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use chrono::{Duration, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[cfg(feature = "__test-stubs")]
use std::sync::Arc;

use crate::middleware::auth::AuthUser;
use crate::middleware::session::{
    SESSION_TTL_DAYS, basic_bearer, build_session_cookie, clear_session_cookie, new_session_id,
    session_cookie_value,
};
use crate::routes::response::{bad_request, error_response, internal};
use crate::state::AppState;

/// Public JSON representation of a user. Mirrors the v1 schema in
/// design.md §6.2. `jmap_account_id` and `created_at` are intentionally
/// NOT exposed — those are server-side bookkeeping.
#[derive(Debug, Serialize)]
pub struct UserView {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub is_admin: bool,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: SecretString,
}

#[derive(Debug, Serialize)]
struct UserEnvelope {
    user: UserView,
}

/// Build the *public* auth subrouter (login + logout). These two routes
/// MUST NOT sit behind the auth middleware: login is how you get auth in
/// the first place, and logout has to work even with a stale/missing
/// cookie.
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
}

/// Build the *protected* auth subrouter (`me`). Mounted under the auth
/// middleware in `main.rs`.
pub fn protected_router() -> Router<AppState> {
    Router::new().route("/api/auth/me", axum::routing::get(me))
}

/// `POST /api/auth/login`. See module docs.
async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    let body = match parse_login_request(request, &state).await {
        Ok(body) => body,
        Err(response) => return response,
    };

    login_impl(state, addr, headers, body, authenticate_login).await
}

async fn login_impl<F, Fut>(
    state: AppState,
    addr: SocketAddr,
    headers: HeaderMap,
    body: LoginRequest,
    authenticate: F,
) -> Response
where
    F: FnOnce(String, String, SecretString) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    // Rate-limit BEFORE we touch Stalwart so spraying credentials is
    // cheap to absorb. Prefer X-Forwarded-For for reverse-proxy deployments;
    // otherwise use the direct peer address from ConnectInfo.
    let ip =
        crate::middleware::rate_limit::client_ip(&headers, Some(addr.ip())).unwrap_or(addr.ip());
    if !state.auth_rate_limiter.check(ip) {
        tracing::warn!(%ip, "login: rate-limited");
        return crate::middleware::rate_limit::too_many_requests();
    }

    let email = body.email.trim().to_lowercase();
    let password = body.password;

    // 1. Validate with Stalwart.
    let session = match authenticate(
        state.config.stalwart.jmap_url.clone(),
        email.clone(),
        password.clone(),
    )
    .await
    {
        Ok(account_id) => account_id,
        Err(err) => {
            // GENERIC error to the client (design.md §10.1 logging hygiene);
            // detailed reason to the trace log so an operator can debug.
            tracing::info!(error = %err, "login: jmap auth failed");
            return invalid_credentials();
        }
    };
    let jmap_account_id = session;

    // 2. Upsert user. Inside one transaction so the "first user becomes
    // admin" check can't race with a concurrent first login.
    let now = Utc::now();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "login: tx begin failed");
            return internal();
        }
    };

    let (user_id, db_email, display_name, is_admin_int) = match upsert_authenticated_user(
        &mut tx,
        state
            .config
            .admin
            .as_ref()
            .map(|admin| admin.email.as_str()),
        &email,
        &jmap_account_id,
        now,
    )
    .await
    {
        Ok(row) => row,
        Err(err) => {
            tracing::error!(error = %err, "login: user upsert failed");
            return internal();
        }
    };

    // 3. Synthesize the JMAP bearer token: base64("email:password").
    // Stalwart accepts the same blob under `Authorization: Bearer …`,
    // so storing it lets us reuse the same value for future requests
    // without re-prompting for a password. The plaintext is held only
    // long enough to encrypt-and-forget; we never log it.
    let bearer = basic_bearer(&email, &password);
    let token_enc = match hail_core::seal(bearer.as_bytes(), &state.server_key) {
        Ok(b) => b,
        Err(err) => {
            tracing::error!(error = %err, "login: token seal failed");
            return internal();
        }
    };

    // 4. 256-bit random session id, hex encoded. Never derived from any
    // user input (design.md §10.1: "opaque id, 256-bit").
    let session_id = match new_session_id() {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(error = %err, "login: failed to draw session id from OS RNG");
            return internal();
        }
    };

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    let expires_at = now + Duration::days(SESSION_TTL_DAYS);

    if let Err(err) = sqlx::query(
        "INSERT INTO sessions (id, user_id, jmap_token_enc, user_agent, expires_at, created_at, last_used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(&token_enc)
    .bind(user_agent.as_deref())
    .bind(expires_at)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %err, "login: session insert failed");
        return internal();
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "login: tx commit failed");
        return internal();
    }

    let user_view = UserView {
        id: user_id,
        email: db_email,
        display_name,
        is_admin: is_admin_int != 0,
    };

    let body = match serde_json::to_string(&UserEnvelope { user: user_view }) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(error = %err, "login: response serialize failed");
            return internal();
        }
    };

    let cookie = build_session_cookie(&session_id);
    tracing::info!(user_id, "login: success");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::SET_COOKIE, cookie),
        ],
        body,
    )
        .into_response()
}

async fn authenticate_login(
    jmap_url: String,
    email: String,
    password: SecretString,
) -> Result<String, String> {
    let session = hail_jmap::login_basic(&jmap_url, &email, password)
        .await
        .map_err(|err| err.to_string())?;
    Ok(session.account_id().to_string())
}

async fn parse_login_request(request: Request, state: &AppState) -> Result<LoginRequest, Response> {
    Json::<LoginRequest>::from_request(request, state)
        .await
        .map(|Json(body)| body)
        .map_err(|_rejection| bad_request("bad_request"))
}

#[cfg(feature = "__test-stubs")]
pub type TestLoginProvider = Arc<
    dyn Fn(
            String,
            String,
            SecretString,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

#[cfg(feature = "__test-stubs")]
pub async fn test_login_with_provider(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    request: Request,
    provider: TestLoginProvider,
) -> Response {
    let body = match parse_login_request(request, &state).await {
        Ok(body) => body,
        Err(response) => return response,
    };

    login_impl(
        state,
        addr,
        headers,
        body,
        move |jmap_url, email, password| provider(jmap_url, email, password),
    )
    .await
}

/// `POST /api/auth/logout`. Always 204. We delete the row if the cookie
/// matches one we know; we always clear the cookie client-side so a stale
/// or attacker-supplied cookie can't keep coming back.
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(value) = session_cookie_value(&headers) {
        // Best-effort delete. We deliberately don't expose whether the
        // session existed.
        if let Err(err) = sqlx::query("DELETE FROM sessions WHERE id = ?1")
            .bind(value)
            .execute(&state.db)
            .await
        {
            tracing::warn!(error = %err, "logout: session delete failed");
        }
    }
    let clear = clear_session_cookie();
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, HeaderValue::from_str(&clear).unwrap())],
    )
        .into_response()
}

/// `GET /api/auth/me`. Behind the auth middleware. The middleware has
/// already done the heavy lifting; we just translate the extension to a
/// JSON view. We re-fetch `display_name` from the DB so it's always
/// fresh (it's not part of `AuthUser`).
async fn me(State(state): State<AppState>, Extension(user): Extension<AuthUser>) -> Response {
    let display_name: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?1")
            .bind(user.id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .flatten();

    let view = UserView {
        id: user.id,
        email: user.email,
        display_name,
        is_admin: user.is_admin,
    };
    Json(UserEnvelope { user: view }).into_response()
}

async fn upsert_authenticated_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    config_admin_email: Option<&str>,
    email: &str,
    jmap_account_id: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(i64, String, Option<String>, i64), sqlx::Error> {
    let any_admin_exists: bool =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM users WHERE is_admin = 1)")
            .fetch_one(&mut **tx)
            .await
            .map(|n| n != 0)
            .unwrap_or(true); // err on the side of NOT auto-elevating.

    // Admin bootstrap policy:
    //
    // * With `[admin]`, Stalwart remains the source of truth. We do not
    //   seed a fake hail user at startup because `users.jmap_account_id`
    //   must come from a real JMAP login. Instead, the configured email is
    //   elevated on its first successful Stalwart login (and on later
    //   logins if an existing local row somehow lost `is_admin`).
    // * Without `[admin]`, preserve the first-run fallback: the first
    //   successful login becomes admin if no admin row exists.
    let promote_to_admin = should_promote_to_admin(config_admin_email, email, any_admin_exists);

    sqlx::query_as(
        "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(email) DO UPDATE SET \
           jmap_account_id = excluded.jmap_account_id, \
           is_admin = CASE \
             WHEN excluded.is_admin = 1 THEN 1 \
             ELSE users.is_admin \
           END \
         RETURNING id, email, display_name, is_admin",
    )
    .bind(email)
    .bind(jmap_account_id)
    .bind(if promote_to_admin { 1_i64 } else { 0_i64 })
    .bind(now)
    .fetch_one(&mut **tx)
    .await
}

fn should_promote_to_admin(
    config_admin_email: Option<&str>,
    login_email: &str,
    any_admin_exists: bool,
) -> bool {
    if let Some(admin_email) = config_admin_email {
        return admin_email.trim().eq_ignore_ascii_case(login_email);
    }

    !any_admin_exists
}

fn invalid_credentials() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "invalid_credentials")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_tx() -> sqlx::Transaction<'static, sqlx::Sqlite> {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("open memory db");
        sqlx::query(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                email TEXT NOT NULL UNIQUE,
                jmap_account_id TEXT NOT NULL,
                display_name TEXT,
                is_admin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&db)
        .await
        .expect("create users");
        db.begin().await.expect("begin tx")
    }

    #[tokio::test]
    async fn configured_admin_login_inserts_admin_with_real_account_id() {
        let mut tx = memory_tx().await;
        let (_id, email, _display_name, is_admin) = upsert_authenticated_user(
            &mut tx,
            Some("Operator@Example.Org"),
            "operator@example.org",
            "acct-real",
            Utc::now(),
        )
        .await
        .expect("upsert user");

        assert_eq!(email, "operator@example.org");
        assert_eq!(is_admin, 1);
        let account_id: String = sqlx::query_scalar("SELECT jmap_account_id FROM users")
            .fetch_one(&mut *tx)
            .await
            .expect("select account id");
        assert_eq!(account_id, "acct-real");
    }

    #[tokio::test]
    async fn configured_admin_login_elevates_existing_non_admin_row() {
        let mut tx = memory_tx().await;
        sqlx::query(
            "INSERT INTO users (email, jmap_account_id, is_admin, created_at)
             VALUES ('operator@example.org', 'old-acct', 0, ?1)",
        )
        .bind(Utc::now())
        .execute(&mut *tx)
        .await
        .expect("seed user");

        let (_id, _email, _display_name, is_admin) = upsert_authenticated_user(
            &mut tx,
            Some("operator@example.org"),
            "operator@example.org",
            "new-acct",
            Utc::now(),
        )
        .await
        .expect("upsert user");

        assert_eq!(is_admin, 1);
        let (account_id, is_admin): (String, i64) =
            sqlx::query_as("SELECT jmap_account_id, is_admin FROM users")
                .fetch_one(&mut *tx)
                .await
                .expect("select user");
        assert_eq!(account_id, "new-acct");
        assert_eq!(is_admin, 1);
    }

    #[test]
    fn configured_admin_email_promotes_even_when_admin_exists() {
        assert!(should_promote_to_admin(
            Some("Operator@Example.Org"),
            "operator@example.org",
            true,
        ));
    }

    #[test]
    fn configured_admin_does_not_promote_other_users() {
        assert!(!should_promote_to_admin(
            Some("operator@example.org"),
            "alice@example.org",
            false,
        ));
    }

    #[test]
    fn first_login_promotes_only_without_configured_admin_and_existing_admin() {
        assert!(should_promote_to_admin(None, "alice@example.org", false));
        assert!(!should_promote_to_admin(None, "bob@example.org", true));
    }
}

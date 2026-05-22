//! `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me`.
//!
//! Login flow (design.md §7.3, §10):
//!   1. Validate credentials against Stalwart via `hail_jmap::login_basic`.
//!   2. Upsert the `users` row (first login becomes `is_admin=1` iff the
//!      operator did not pre-provision an `[admin]` in `hail.toml` AND
//!      no admin exists yet — DD-9).
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
//!      We encrypt the bearer at rest via AES-256-GCM (`hail_core::seal`)
//!      so the SQLite file alone does not yield usable tokens.
//!   4. Mint a 256-bit random session id (hex-encoded — `2 * 32 = 64`
//!      chars — never derived from any user input).
//!   5. INSERT the row with 30-day expiry, set the cookie, return the
//!      `user` payload.
//!
//! Logout: DELETE the session row (best-effort — missing rows still
//! return 204) and clear the cookie with `Max-Age=0`.
//!
//! `me`: served behind the auth middleware, returns the `AuthUser` view.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Extension, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{Duration, Utc};
use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::middleware::auth::{AuthUser, SESSION_COOKIE};
use crate::state::AppState;

/// Session lifetime — 30 days, per design.md §10.1 ("30-day sliding TTL").
const SESSION_TTL_DAYS: i64 = 30;
/// `Max-Age` value matching `SESSION_TTL_DAYS` in seconds.
const SESSION_MAX_AGE_SECS: i64 = SESSION_TTL_DAYS * 24 * 60 * 60;

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
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Response {
    // Rate-limit BEFORE we touch Stalwart so spraying credentials is
    // cheap to absorb. We key on the connecting peer's IP — fine for our
    // single-host deployment, where there is no upstream proxy
    // injecting `X-Forwarded-For` we'd need to trust.
    if !state.login_limiter.check(addr.ip()) {
        tracing::warn!(ip = %addr.ip(), "login: rate-limited");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"rate_limited"}"#,
        )
            .into_response();
    }

    let email = body.email.trim().to_lowercase();
    let password = body.password;

    // 1. Validate with Stalwart.
    let session = match hail_jmap::login_basic(
        &state.config.stalwart.jmap_url,
        &email,
        password.clone(),
    )
    .await
    {
        Ok(s) => s,
        Err(err) => {
            // GENERIC error to the client (design.md §10.1 logging hygiene);
            // detailed reason to the trace log so an operator can debug.
            tracing::info!(error = %err, "login: jmap auth failed");
            return invalid_credentials();
        }
    };
    let jmap_account_id = session.account_id().to_string();

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

    // "First user becomes admin" applies only when the operator did NOT
    // pre-provision an `[admin]` block (DD-9). Otherwise the admin is
    // expected to come from the `/setup` wizard (separate task) or a
    // manual `users` row.
    let admin_preprovisioned = state.config.admin.is_some();
    let any_admin_exists: bool =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM users WHERE is_admin = 1)")
            .fetch_one(&mut *tx)
            .await
            .map(|n| n != 0)
            .unwrap_or(true); // err on the side of NOT auto-elevating.

    let promote_to_admin = !admin_preprovisioned && !any_admin_exists;

    let upsert_sql = "INSERT INTO users (email, jmap_account_id, is_admin, created_at) \
                      VALUES (?1, ?2, ?3, ?4) \
                      ON CONFLICT(email) DO UPDATE SET jmap_account_id = excluded.jmap_account_id \
                      RETURNING id, email, display_name, is_admin";
    let (user_id, db_email, display_name, is_admin_int): (i64, String, Option<String>, i64) =
        match sqlx::query_as(upsert_sql)
            .bind(&email)
            .bind(&jmap_account_id)
            .bind(if promote_to_admin { 1_i64 } else { 0_i64 })
            .bind(now)
            .fetch_one(&mut *tx)
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
    let basic_plain = format!("{}:{}", email, password.expose_secret());
    let bearer = B64.encode(basic_plain.as_bytes());
    let token_enc = match hail_core::seal(bearer.as_bytes(), &state.server_key) {
        Ok(b) => b,
        Err(err) => {
            tracing::error!(error = %err, "login: token seal failed");
            return internal();
        }
    };

    // 4. 256-bit random session id, hex encoded. Never derived from any
    // user input (design.md §10.1: "opaque id, 256-bit").
    let mut id_bytes = [0u8; 32];
    if rand::rngs::OsRng.try_fill_bytes(&mut id_bytes).is_err() {
        tracing::error!("login: failed to draw session id from OS RNG");
        return internal();
    }
    let session_id = hex::encode(id_bytes);

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

    let cookie = build_session_cookie(&session_id, SESSION_MAX_AGE_SECS);
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

/// `POST /api/auth/logout`. Always 204. We delete the row if the cookie
/// matches one we know; we always clear the cookie client-side so a stale
/// or attacker-supplied cookie can't keep coming back.
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(value) = read_session_cookie(&headers) {
        // Best-effort delete. We deliberately don't expose whether the
        // session existed.
        if let Err(err) = sqlx::query("DELETE FROM sessions WHERE id = ?1")
            .bind(&value)
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

/// Build the `Set-Cookie` value for a fresh session. We hardcode every
/// flag in design.md §10.1 so the security posture is obvious from this
/// single line:
///   * `HttpOnly`           — not visible to JS in the SPA.
///   * `Secure`             — cookie only sent over HTTPS in production.
///   * `SameSite=Lax`       — top-level navigation only carries the
///                            cookie cross-site; combined with the
///                            mandatory `X-Hail-Request` header on
///                            mutations this gives CSRF defence in depth.
///   * `Path=/`             — visible to every hail route.
///   * `Max-Age=2592000`    — 30 days, matching the row's expiry.
fn build_session_cookie(session_id: &str, max_age_secs: i64) -> String {
    format!(
        "{name}={value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}",
        name = SESSION_COOKIE,
        value = session_id,
        max_age = max_age_secs,
    )
}

/// `Set-Cookie` value used by logout to invalidate the cookie client-side.
fn clear_session_cookie() -> String {
    format!(
        "{name}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0",
        name = SESSION_COOKIE
    )
}

/// Find the `hail_session` cookie value, if any. Walks the `Cookie`
/// header manually to avoid pulling in `axum-extra::cookie` for this one
/// call — see the middleware module for the same parser.
fn read_session_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim_start();
        if let Some((k, v)) = pair.split_once('=') {
            if k == SESSION_COOKIE {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn invalid_credentials() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"invalid_credentials"}"#,
    )
        .into_response()
}

fn internal() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"internal"}"#,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_carries_all_security_flags() {
        let c = build_session_cookie("deadbeef", 2_592_000);
        // Each flag from design.md §10.1 must be present verbatim.
        assert!(c.contains("HttpOnly"), "HttpOnly missing: {c}");
        assert!(c.contains("Secure"), "Secure missing: {c}");
        assert!(c.contains("SameSite=Lax"), "SameSite=Lax missing: {c}");
        assert!(c.contains("Path=/"), "Path=/ missing: {c}");
        assert!(c.contains("Max-Age=2592000"), "Max-Age missing: {c}");
        assert!(
            c.starts_with("hail_session=deadbeef"),
            "name/value missing: {c}"
        );
    }

    #[test]
    fn cleared_cookie_has_max_age_zero() {
        let c = clear_session_cookie();
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("HttpOnly"));
    }
}

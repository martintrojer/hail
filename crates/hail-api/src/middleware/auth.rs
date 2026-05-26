//! Cookie-session authentication middleware (design.md §10.1).
//!
//! Responsibilities:
//!   1. Read the `hail_session` cookie.
//!   2. Look up the row in `sessions`, verify `expires_at > now`,
//!      bump `last_used_at`.
//!   3. Decrypt `jmap_token_enc` with the server key and attach `AuthUser`
//!      (id, email, is_admin, plaintext JMAP token) plus `AuthSession`
//!      (session id and expiry) as Axum `Extension`s.
//!   4. For mutating methods (POST/PUT/PATCH/DELETE) demand the
//!      `X-Hail-Request: 1` header — same-origin defence belt-and-braces
//!      with `SameSite=Lax` (CSRF, §10.1).
//!
//! Any failure short-circuits to a generic 401 (auth) or 403 (CSRF);
//! the detailed reason lives only in the trace log.
//!
//! Constant-time equality is used when comparing the session id — even
//! though primary-key lookup happens server-side, the cookie value is
//! attacker-controlled and the safe habit is to never branch on it.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;
use secrecy::SecretString;
use subtle::ConstantTimeEq;

use crate::middleware::session::{
    SESSION_COOKIE, SESSION_TTL_DAYS, build_session_cookie, cookie_value,
};
use crate::state::AppState;
/// CSRF header demanded on mutating methods. Same value the SPA sends.
pub const CSRF_HEADER: &str = "X-Hail-Request";

/// Information about the authenticated principal. Inserted as a request
/// extension by [`require_auth`] for downstream handlers to extract via
/// `Extension<AuthUser>`.
///
/// `jmap_token` is stored as [`SecretString`] so Debug-formatting it (or
/// accidentally `format!("{:?}", auth_user)`-ing the whole struct) prints
/// `[REDACTED]` instead of the token. We deliberately do NOT derive
/// `Debug` on this struct ourselves, even with `SecretString`, because
/// it's easier to audit the rule "AuthUser never appears in tracing
/// macros" than "AuthUser appears only via specific safe field access".
#[derive(Clone)]
pub struct AuthUser {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
    pub jmap_token: SecretString,
}

/// Authenticated session metadata inserted by [`require_auth`]. Handlers that
/// accept durable work can persist this opaque reference/expiry without seeing
/// or copying the plaintext JMAP token.
#[derive(Clone)]
pub struct AuthSession {
    pub id: String,
    pub expires_at: chrono::DateTime<Utc>,
}

/// Generic 401 body. Identical for every failure mode so a malicious
/// caller can't fingerprint "bad cookie" vs "expired" vs "decrypt
/// failed". Specifics go to tracing only.
fn unauthorized() -> Response {
    crate::routes::response::error_response(StatusCode::UNAUTHORIZED, "unauthorized")
}

/// Generic 403 for missing CSRF header.
fn forbidden_csrf() -> Response {
    crate::routes::response::error_response(StatusCode::FORBIDDEN, "csrf_required")
}

/// Constant-time string compare. Returns true iff the strings are equal
/// in length and content. Used when the value is attacker-controlled
/// (here, the cookie before lookup).
fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// Returns `true` for HTTP methods that mutate server state and therefore
/// need CSRF protection. GET/HEAD/OPTIONS are exempt.
fn is_mutating(method: &Method) -> bool {
    matches!(
        method,
        &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
    )
}

/// Axum middleware that enforces session auth + CSRF. Mount via
/// `axum::middleware::from_fn_with_state(state, require_auth)`.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    // CSRF check first: if it fails we don't even need to touch the DB.
    if is_mutating(req.method())
        && req.headers().get(CSRF_HEADER).map(|v| v.as_bytes()) != Some(b"1")
    {
        tracing::debug!(
            method = %req.method(),
            path = %req.uri().path(),
            "csrf header missing on mutating request",
        );
        return forbidden_csrf();
    }

    let Some(cookie) = cookie_value(req.headers(), SESSION_COOKIE) else {
        tracing::debug!("auth: no session cookie");
        return unauthorized();
    };
    // Copy out of the borrow on `req.headers()` so we can `await` below.
    let cookie = cookie.to_string();

    // Length / charset sanity — the value we mint is 64 hex chars. We
    // reject anything else BEFORE hitting the DB. Constant-time compare
    // on the length isn't possible but length is not a secret.
    if cookie.len() != 64 || !cookie.bytes().all(|b| b.is_ascii_hexdigit()) {
        tracing::debug!("auth: malformed session cookie");
        return unauthorized();
    }

    // Look up the session + the joined user row in one round-trip.
    let row = match sqlx::query_as::<
        _,
        (
            String,
            i64,
            Vec<u8>,
            chrono::DateTime<Utc>,
            i64,
            String,
            i64,
        ),
    >(
        "SELECT s.id, s.user_id, s.jmap_token_enc, s.expires_at, \
                u.id, u.email, u.is_admin \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.id = ?1",
    )
    .bind(&cookie)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::debug!("auth: session not found");
            return unauthorized();
        }
        Err(err) => {
            tracing::warn!(error = %err, "auth: session lookup failed");
            return unauthorized();
        }
    };

    let (session_id, _user_id_dup, token_enc, expires_at, user_id, email, is_admin) = row;

    // Belt-and-braces: constant-time compare the returned id back against
    // the cookie. SQLite's `=` is the actual lookup, but if we ever moved
    // to a prefix index or a cache, this would still hold.
    if !ct_eq(&session_id, &cookie) {
        tracing::warn!("auth: session id mismatch after lookup");
        return unauthorized();
    }

    // Expiry check.
    let now = Utc::now();
    if expires_at <= now {
        tracing::debug!("auth: session expired");
        return unauthorized();
    }

    // Decrypt the JMAP token. Failure here means the row is corrupt or the
    // server key rotated without re-encrypting — operator problem, not
    // user problem. Generic 401 to the client; warn to the log.
    let token_bytes = match hail_core::open(&token_enc, &state.server_key) {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!(error = %err, user_id, "auth: token decrypt failed");
            return unauthorized();
        }
    };
    let token = match String::from_utf8(token_bytes) {
        Ok(s) => SecretString::from(s),
        Err(_) => {
            tracing::warn!(user_id, "auth: decrypted token was not UTF-8");
            return unauthorized();
        }
    };

    // Best-effort sliding-session refresh. Auth has already succeeded, so
    // a write failure should not fail the request, but downstream durable work
    // must only see the extended expiry if it actually reached SQLite.
    let refreshed_expires_at = now + chrono::Duration::days(SESSION_TTL_DAYS);
    let mut session_expires_at = expires_at;
    let mut refresh_cookie = false;
    if let Err(err) =
        sqlx::query("UPDATE sessions SET last_used_at = ?1, expires_at = ?2 WHERE id = ?3")
            .bind(now)
            .bind(refreshed_expires_at)
            .bind(&cookie)
            .execute(&state.db)
            .await
    {
        tracing::warn!(error = %err, "auth: sliding session refresh failed");
    } else {
        session_expires_at = refreshed_expires_at;
        refresh_cookie = true;
    }

    req.extensions_mut().insert(AuthUser {
        id: user_id,
        email,
        is_admin: is_admin != 0,
        jmap_token: token,
    });
    req.extensions_mut().insert(AuthSession {
        id: session_id,
        expires_at: session_expires_at,
    });

    let mut response = next.run(req).await;
    if refresh_cookie {
        match header::HeaderValue::from_str(&build_session_cookie(&cookie)) {
            Ok(cookie) => {
                response.headers_mut().append(header::SET_COOKIE, cookie);
            }
            Err(err) => {
                tracing::warn!(error = %err, "auth: failed to build session refresh cookie");
            }
        }
    }
    response
}

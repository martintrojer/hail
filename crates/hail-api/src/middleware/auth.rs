//! Cookie-session authentication middleware (design.md §10.1).
//!
//! Responsibilities:
//!   1. Read the `hail_session` cookie.
//!   2. Look up the row in `sessions`, verify `expires_at > now`,
//!      bump `last_used_at`.
//!   3. Decrypt `jmap_token_enc` with the server key and attach an
//!      `AuthUser` (id, email, is_admin, plaintext JMAP token) as an
//!      Axum `Extension`.
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
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use secrecy::SecretString;
use subtle::ConstantTimeEq;

use crate::middleware::session::{SESSION_COOKIE, cookie_value};
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

/// Generic 401 body. Identical for every failure mode so a malicious
/// caller can't fingerprint "bad cookie" vs "expired" vs "decrypt
/// failed". Specifics go to tracing only.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"unauthorized"}"#.to_string(),
    )
        .into_response()
}

/// Generic 403 for missing CSRF header.
fn forbidden_csrf() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"csrf_required"}"#.to_string(),
    )
        .into_response()
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

    // Best-effort `last_used_at` bump. We don't fail the request if this
    // INSERT-side-effect fails — auth has already succeeded.
    if let Err(err) = sqlx::query("UPDATE sessions SET last_used_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(&cookie)
        .execute(&state.db)
        .await
    {
        tracing::warn!(error = %err, "auth: last_used_at bump failed");
    }

    req.extensions_mut().insert(AuthUser {
        id: user_id,
        email,
        is_admin: is_admin != 0,
        jmap_token: token,
    });

    next.run(req).await
}

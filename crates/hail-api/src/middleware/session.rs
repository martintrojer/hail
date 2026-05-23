//! Shared authentication session primitives.
//!
//! Login, logout, first-run setup, and the auth middleware all agree on
//! these values and wire formats. Keeping them here avoids silent drift in
//! cookie flags, TTLs, bearer-token construction, and session-id shape.

use axum::http::{HeaderMap, header};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};

/// Cookie name used for the browser session id.
pub const SESSION_COOKIE: &str = "hail_session";
/// Session lifetime — 30 days, per design.md §10.1 ("30-day sliding TTL").
pub const SESSION_TTL_DAYS: i64 = 30;
/// `Max-Age` value matching [`SESSION_TTL_DAYS`] in seconds.
pub const SESSION_MAX_AGE_SECS: i64 = SESSION_TTL_DAYS * 24 * 60 * 60;

/// Build the bearer token Stalwart accepts for later JMAP requests.
///
/// Stalwart accepts `base64("email:password")` under both
/// `Authorization: Basic …` and `Authorization: Bearer …`.
pub fn basic_bearer(email: &str, password: &SecretString) -> String {
    B64.encode(format!("{}:{}", email, password.expose_secret()).as_bytes())
}

/// Error returned when the OS random-number generator cannot mint a session id.
#[derive(Debug, thiserror::Error)]
#[error("failed to draw session id from OS RNG")]
pub struct SessionIdError;

/// Mint a 256-bit random session id, hex-encoded as 64 lowercase chars.
pub fn new_session_id() -> Result<String, SessionIdError> {
    let mut id_bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut id_bytes)
        .map_err(|_| SessionIdError)?;
    Ok(hex::encode(id_bytes))
}

/// Build the `Set-Cookie` value for a fresh session. We hardcode every
/// flag in design.md §10.1 so the security posture is obvious from this
/// single line:
/// - `HttpOnly` — not visible to JS in the SPA.
/// - `Secure` — cookie only sent over HTTPS in production.
/// - `SameSite=Lax` — top-level navigation only carries the cookie cross-site;
///   combined with the mandatory `X-Hail-Request` header on mutations this gives
///   CSRF defence in depth.
/// - `Path=/` — visible to every hail route.
/// - `Max-Age=2592000` — 30 days, matching the row's expiry.
pub fn build_session_cookie(session_id: &str) -> String {
    format!(
        "{name}={value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}",
        name = SESSION_COOKIE,
        value = session_id,
        max_age = SESSION_MAX_AGE_SECS,
    )
}

/// `Set-Cookie` value used by logout to invalidate the cookie client-side.
pub fn clear_session_cookie() -> String {
    format!(
        "{name}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0",
        name = SESSION_COOKIE
    )
}

/// Extract a named cookie value from the request headers.
pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim_start();
        if let Some((k, v)) = pair.split_once('=')
            && k == name
        {
            return Some(v);
        }
    }
    None
}

/// Find the `hail_session` cookie value, if any.
pub fn session_cookie_value(headers: &HeaderMap) -> Option<&str> {
    cookie_value(headers, SESSION_COOKIE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn basic_bearer_base64_encodes_email_password_pair() {
        let password = SecretString::from("correct horse".to_string());
        assert_eq!(
            basic_bearer("alice@example.org", &password),
            "YWxpY2VAZXhhbXBsZS5vcmc6Y29ycmVjdCBob3JzZQ=="
        );
    }

    #[test]
    fn new_session_id_is_256_bit_lowercase_hex() {
        let id = new_session_id().expect("random session id");
        assert_eq!(id.len(), 64);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id, id.to_ascii_lowercase());
    }

    #[test]
    fn session_cookie_carries_all_security_flags() {
        let c = build_session_cookie("deadbeef");
        assert_eq!(SESSION_MAX_AGE_SECS, 2_592_000);
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
    fn cleared_cookie_has_max_age_zero_and_security_flags() {
        let c = clear_session_cookie();
        assert!(c.starts_with("hail_session="));
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("Secure"));
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Path=/"));
    }

    #[test]
    fn session_cookie_value_reads_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; hail_session=abc123; other=value"),
        );
        assert_eq!(session_cookie_value(&headers), Some("abc123"));
    }
}

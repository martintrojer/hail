//! Test-only stub route used by `tests/auth.rs` to exercise the CSRF /
//! auth-middleware behaviour on a "downstream mutating endpoint". Only
//! compiled under `#[cfg(test)]` so it can never leak into a production
//! binary.

use axum::{
    Extension, Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::{middleware::auth::AuthUser, state::AppState};

#[derive(Serialize)]
struct TokenProbe {
    token_len: usize,
    token_hash: String,
    email: String,
    is_admin: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/threads/test",
            post(|| async { StatusCode::NO_CONTENT }),
        )
        .route("/api/auth/test-token", get(token_probe))
}

async fn token_probe(Extension(user): Extension<AuthUser>) -> Json<TokenProbe> {
    let token = user.jmap_token.expose_secret();
    Json(TokenProbe {
        token_len: token.len(),
        token_hash: fnv1a64_hex(token.as_bytes()),
        email: user.email,
        is_admin: user.is_admin,
    })
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

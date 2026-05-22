//! Test-only stub route used by `tests/auth.rs` to exercise the CSRF /
//! auth-middleware behaviour on a "downstream mutating endpoint". Only
//! compiled under `#[cfg(test)]` so it can never leak into a production
//! binary.

use axum::{Router, http::StatusCode, routing::post};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/api/threads/test",
        post(|| async { StatusCode::NO_CONTENT }),
    )
}

//! HTTP middleware: authentication / CSRF / rate limiting.
//!
//! Kept small and dependency-light so the middleware itself is easy to
//! audit. See `docs/design.md` §10 (security model) for the threat model
//! these layers cover.

pub mod auth;
pub mod rate_limit;
pub mod security_headers;
pub mod session;

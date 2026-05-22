//! Process-wide shared state for `hail-api`.
//!
//! Mirrors `hail_worker::state::AppState` so both binaries share the same
//! mental model: one `Arc<AppState>` carries the SQLx pool, the parsed
//! config, and the parsed 32-byte AES-256-GCM server key. We keep the
//! struct cheap-to-clone (the pool / key are already `Arc`-shaped or
//! `Copy`).

use std::sync::Arc;

use hail_core::Config;
use sqlx::SqlitePool;

/// Process-wide shared state. Cloned into every handler via `axum`'s
/// `State` extractor.
#[derive(Clone)]
pub struct AppState {
    /// SQLx pool against the hail sidecar database.
    pub db: SqlitePool,
    /// API configuration loaded from TOML + env.
    pub config: Config,
    /// 32-byte AES-256-GCM key, parsed from `config.secrets.server_key`
    /// at startup. Wrapped in `Arc<[u8; KEY_LEN]>` rather than copied so
    /// the bytes live in one heap allocation and never appear in `Debug`
    /// output (we don't derive Debug on AppState for the same reason —
    /// see the manual impl below).
    pub server_key: Arc<[u8; hail_core::KEY_LEN]>,
    /// In-memory rate limiter for `/api/auth/login` (5 attempts / 60s
    /// per remote IP). See `middleware::rate_limit`.
    pub login_limiter: Arc<crate::middleware::rate_limit::IpRateLimiter>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `server_key` and `config.secrets` from Debug.
        f.debug_struct("AppState")
            .field("db", &"<SqlitePool>")
            .field("database_url", &self.config.database_url)
            .field("stalwart_url", &self.config.stalwart.jmap_url)
            .field("server_key", &"<redacted>")
            .finish()
    }
}

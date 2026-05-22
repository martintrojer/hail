//! Process-wide shared state for `hail-api`.
//!
//! Mirrors `hail_worker::state::AppState` so both binaries share the same
//! mental model: one `Arc<AppState>` carries the SQLx pool and the parsed
//! config to every handler / background task. Subsequent tasks (auth,
//! sessions, JMAP) will hang client caches and signer keys here.

use hail_core::Config;
use sqlx::SqlitePool;

/// Process-wide shared state. Cheap to clone via `Arc` (or via
/// `axum::extract::State`, which uses `Clone` under the hood).
//
// `dead_code` is allowed because the skeleton stores `config` for follow-up
// tasks (auth, views, verbs) that will read it. Remove once those land.
#[allow(dead_code)]
#[derive(Clone)]
pub struct AppState {
    /// SQLx pool against the hail sidecar database.
    pub db: SqlitePool,
    /// API configuration loaded from TOML + env.
    pub config: Config,
}

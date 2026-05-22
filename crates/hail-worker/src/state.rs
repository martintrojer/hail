//! Process-wide shared state for `hail-worker`.
//!
//! Per design.md §8.2, `AppState` is wrapped in `Arc` and handed to every
//! task (supervisor, scheduler, per-user EventSource streams). It owns the
//! SQLite pool, the parsed [`Config`], and the token decryptor used by the
//! per-user supervisors to open their bearer JMAP token at rest (DD-8).

use std::sync::Arc;

use hail_core::Config;
use sqlx::SqlitePool;

use crate::crypto::TokenDecryptor;

/// Process-wide shared state. Cheap to clone via `Arc`.
pub struct AppState {
    /// SQLx pool against the hail sidecar database.
    pub db: SqlitePool,
    /// Worker configuration loaded from TOML + env.
    pub config: Config,
    /// Decryptor for `sessions.jmap_token_enc`. Trait-object so tests
    /// can inject a fake; production wires `HailCoreOpener` once the
    /// `auth-login` wave merges (see `crypto.rs` TODO).
    pub token_decryptor: Arc<dyn TokenDecryptor>,
}

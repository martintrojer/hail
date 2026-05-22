//! Process-wide shared state for `hail-worker`.
//!
//! Per design.md §8.2, `AppState` is wrapped in `Arc` and handed to every
//! task (supervisor, scheduler, per-user EventSource streams). It owns the
//! SQLite pool and the parsed `Config`. Per-user JMAP client caches will
//! land here in the `jmap-eventsource` task.

use sqlx::SqlitePool;

use crate::config::Config;

/// Process-wide shared state. Cheap to clone via `Arc`.
//
// `dead_code` is allowed because the skeleton stores fields that follow-up
// tasks (`jmap-eventsource`, `screener-routing`) will read. Remove once those
// land.
#[allow(dead_code)]
pub struct AppState {
    /// SQLx pool against the hail sidecar database.
    pub db: SqlitePool,
    /// Worker configuration loaded from the environment.
    pub config: Config,
}

//! Runtime configuration for `hail-worker`, populated from environment variables.
//!
//! Keep this surface tiny on purpose: the worker is meant to boot from a couple
//! of env vars (matching what `deploy/docker-compose.yml` will pass in). Anything
//! richer belongs in `hail.toml` parsed by `hail-api` and handed across.

use std::env;

/// Default DB URL used when `HAIL_DB_URL` is unset.
///
/// Relative `sqlite://hail.db` means "file `hail.db` in the worker's cwd";
/// the Compose deployment will mount this onto a volume.
const DEFAULT_DB_URL: &str = "sqlite://hail.db";

/// Default Stalwart JMAP endpoint, matching the dev Compose port.
const DEFAULT_STALWART_URL: &str = "http://localhost:8080";

/// Default supervisor tick cadence in seconds. Matches design.md §8.1 item 4
/// (60s scheduler tick); we use 30 here as a placeholder. Override via
/// `HAIL_TICK_SECS` — primarily so smoke tests don't have to wait 30s.
const DEFAULT_TICK_SECS: u64 = 30;

/// Worker configuration. Cheap to clone; held by `AppState`.
#[derive(Debug, Clone)]
pub struct Config {
    /// `sqlx` URL for the hail sidecar SQLite database.
    pub database_url: String,
    /// Base URL of the Stalwart JMAP server.
    pub stalwart_url: String,
    /// Supervisor tick cadence in seconds.
    pub tick_secs: u64,
}

impl Config {
    /// Build a `Config` from env vars, falling back to development defaults.
    ///
    /// Never fails: missing or malformed vars resolve to the defaults above.
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("HAIL_DB_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string()),
            stalwart_url: env::var("HAIL_STALWART_URL")
                .unwrap_or_else(|_| DEFAULT_STALWART_URL.to_string()),
            tick_secs: env::var("HAIL_TICK_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_TICK_SECS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_env_unset() {
        // SAFETY: tests in this module are single-threaded by `cargo test` default
        // for env-mutating tests; we just want to assert the fallback strings.
        // Use `remove_var` to ensure determinism even if the host sets these.
        // SAFETY: `remove_var` is `unsafe` on edition 2024 — single-threaded test ok.
        unsafe {
            env::remove_var("HAIL_DB_URL");
            env::remove_var("HAIL_STALWART_URL");
        }
        // SAFETY: single-threaded test.
        unsafe {
            env::remove_var("HAIL_TICK_SECS");
        }
        let cfg = Config::from_env();
        assert_eq!(cfg.database_url, DEFAULT_DB_URL);
        assert_eq!(cfg.stalwart_url, DEFAULT_STALWART_URL);
        assert_eq!(cfg.tick_secs, DEFAULT_TICK_SECS);
    }
}

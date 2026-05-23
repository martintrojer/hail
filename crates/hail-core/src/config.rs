//! Unified configuration loader for `hail-api` and `hail-worker`.
//!
//! Layered with [`figment`]:
//!   1. TOML file at `$HAIL_CONFIG`, falling back to `/etc/hail/hail.toml`
//!      then `./hail.toml` (so dev shells and containers both work without
//!      flags). A missing file is fine as long as the env layer fills in
//!      every required field.
//!   2. Environment variables prefixed `HAIL_`, with double underscore
//!      denoting nesting — e.g. `HAIL_STALWART__JMAP_URL` maps to
//!      `stalwart.jmap_url`. Env wins over TOML.
//!
//! The structure mirrors `deploy/hail.example.toml` and the data-model /
//! security sections of `docs/design.md`:
//!   - DD-7: `database_url` is the sqlx URL to the SQLite sidecar.
//!   - DD-8: `secrets.server_key` is the AES-GCM key (32+ bytes) used to
//!     encrypt JMAP tokens at rest. Required.
//!   - DD-9: `[admin]` is optional; its absence triggers the first-run
//!     wizard at `/setup`.

use std::path::{Path, PathBuf};

use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// Default TOML location for production / container deployments.
const DEFAULT_CONFIG_PATH: &str = "/etc/hail/hail.toml";
/// Fallback for local dev — looked at only if `DEFAULT_CONFIG_PATH` is absent.
const FALLBACK_CONFIG_PATH: &str = "./hail.toml";
/// Env prefix; `HAIL_FOO__BAR` becomes `foo.bar`.
const ENV_PREFIX: &str = "HAIL_";
/// Nested separator inside env-var names. Figment translates this to `.`.
const ENV_NESTED_SEP: &str = "__";
/// Minimum usable length of the AES-GCM server key, in bytes. DD-8 calls
/// for a 256-bit key; we accept either 32 decoded hex bytes or at least
/// 32 non-hex raw bytes so operators can rotate to a larger raw key
/// without code changes.
const MIN_SERVER_KEY_BYTES: usize = 32;

/// Top-level configuration. Cheap to clone (held inside `Arc<AppState>`).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// sqlx URL for the SQLite sidecar. e.g. `sqlite:///var/lib/hail/hail.db`.
    pub database_url: String,
    /// Stalwart JMAP / management endpoints.
    pub stalwart: StalwartConfig,
    /// HTTP server bind + public URL (for cookie scope / CORS).
    pub server: ServerConfig,
    /// Optional admin block. When `None`, the first-run wizard at `/setup`
    /// is active (DD-9).
    #[serde(default)]
    pub admin: Option<AdminConfig>,
    /// Encryption secrets. The server key is required.
    pub secrets: SecretsConfig,
}

/// Stalwart endpoints. `management_url` is optional because it's only used
/// by the admin surface; most reads/writes go through `jmap_url`.
#[derive(Debug, Clone, Deserialize)]
pub struct StalwartConfig {
    /// Base URL of the Stalwart JMAP HTTP endpoint.
    pub jmap_url: String,
    /// Stalwart management API (`/manage` on the same host, typically).
    #[serde(default)]
    pub management_url: Option<String>,
}

/// HTTP server settings consumed by `hail-api`. `hail-worker` ignores these
/// but parses them anyway so the same `hail.toml` works for both bins.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// `host:port` to bind, e.g. `0.0.0.0:8080`.
    pub bind: String,
    /// Externally-visible URL — used for cookie domain + same-origin CSRF.
    pub public_url: String,
    /// Directory containing the built SPA bundle. If unset, hail-api checks
    /// `HAIL_WEBAPP_DIR`, then falls back to `/srv/hail/webapp`.
    #[serde(default)]
    pub webapp_dir: Option<PathBuf>,
}

/// Pre-provisioned admin login. Setting this block opts out of the wizard
/// (DD-9). Stalwart remains the password/account source of truth: hail does
/// not create a local user row until this email successfully logs in through
/// JMAP, at which point the row is marked `is_admin=1` using the real
/// `jmap_account_id` returned by Stalwart.
#[derive(Debug, Clone, Deserialize)]
pub struct AdminConfig {
    /// Admin login email.
    pub email: String,
    /// Deprecated/no-op. Hail authenticates configured admins against
    /// Stalwart on login and never reads this value. Kept only so older
    /// configs/env can continue to parse while operators remove it.
    #[serde(default)]
    pub password_hash: Option<String>,
    /// Display name shown in the UI.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Encryption / signing secrets. Currently just the AES-GCM key used to
/// wrap JMAP tokens at rest (DD-8).
#[derive(Debug, Clone, Deserialize)]
pub struct SecretsConfig {
    /// Key material for AES-256-GCM token encryption. Prefer 64 ASCII
    /// hex characters from `openssl rand -hex 32`; non-hex raw strings of
    /// at least 32 bytes are also accepted, with the first 32 bytes used.
    /// Stored as [`SecretString`] so it never lands in `Debug`/logs.
    pub server_key: SecretString,
}

/// Errors surfaced by [`Config::load`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// No TOML file found at any of the candidate paths AND env didn't
    /// fully cover the schema. The path embedded is the highest-priority
    /// location we looked at.
    #[error("hail config not found (looked at {0}); set HAIL_CONFIG or create hail.toml")]
    NotFound(PathBuf),
    /// Figment couldn't parse or merge the layers. Wraps the upstream
    /// error verbatim — it already points at file + field.
    #[error(transparent)]
    Parse(Box<figment::Error>),
    /// `secrets.server_key` was empty or does not provide at least
    /// `MIN_SERVER_KEY_BYTES` of usable key material.
    #[error("invalid server_key: {0}")]
    InvalidServerKey(String),
}

impl Config {
    /// Load the unified config (TOML + env overrides). See module docs.
    pub fn load() -> Result<Self, ConfigError> {
        let path = resolve_config_path();
        Self::load_from(path.as_deref())
    }

    /// Like [`Self::load`] but with an explicit TOML path. Useful for
    /// tests and tools that don't want to mutate the global env.
    pub fn load_from(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut fig = Figment::new();
        if let Some(p) = path {
            fig = fig.merge(Toml::file(p));
        }
        // Env overrides last so they win. `split("__")` turns the double
        // underscore into nested keys. `HAIL_DATABASE_URL` → `database_url`.
        fig = fig.merge(Env::prefixed(ENV_PREFIX).split(ENV_NESTED_SEP));

        let cfg: Config = fig.extract().map_err(|e| {
            // Distinguish "no config at all" from a real parse error.
            if path.is_none() && env_layer_empty() {
                ConfigError::NotFound(PathBuf::from(DEFAULT_CONFIG_PATH))
            } else {
                ConfigError::Parse(Box::new(e))
            }
        })?;

        validate_server_key(&cfg.secrets.server_key)?;
        Ok(cfg)
    }
}

/// Walk `HAIL_CONFIG` → `/etc/hail/hail.toml` → `./hail.toml`, returning
/// the first one that exists. `None` is a valid outcome — the env layer
/// can still satisfy the schema.
fn resolve_config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HAIL_CONFIG") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    for candidate in [DEFAULT_CONFIG_PATH, FALLBACK_CONFIG_PATH] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// True if no `HAIL_*` env vars are set — used to disambiguate "missing
/// file" from "parse error" when both layers fail to produce a config.
fn env_layer_empty() -> bool {
    std::env::vars().all(|(k, _)| !k.starts_with(ENV_PREFIX))
}

/// Validate the server key: non-empty, and either 64+ hex characters
/// (representing at least 32 bytes) or a non-hex raw string of at least
/// 32 bytes. `parse_server_key` uses the decoded 64-character hex value
/// or the first 32 raw bytes; it does not base64-decode keys.
fn validate_server_key(key: &SecretString) -> Result<(), ConfigError> {
    let raw = key.expose_secret();
    if raw.is_empty() {
        return Err(ConfigError::InvalidServerKey(
            "empty; set HAIL_SECRETS__SERVER_KEY or [secrets].server_key in TOML".to_string(),
        ));
    }

    // Try hex first.
    if let Some(bytes) = decode_hex(raw) {
        if bytes >= MIN_SERVER_KEY_BYTES {
            return Ok(());
        }
        return Err(ConfigError::InvalidServerKey(format!(
            "hex-decoded length {bytes} < required {MIN_SERVER_KEY_BYTES} bytes"
        )));
    }

    // Fall back to raw byte length. Anything ≥ 32 bytes is acceptable;
    // parse_server_key will use the first 32 bytes directly.
    if raw.len() >= MIN_SERVER_KEY_BYTES {
        return Ok(());
    }

    Err(ConfigError::InvalidServerKey(format!(
        "{} chars; need ≥ {} bytes (hex-encode 32 random bytes: `openssl rand -hex 32`)",
        raw.len(),
        MIN_SERVER_KEY_BYTES
    )))
}

/// Hex-decode `s` and return its byte length. `None` if `s` isn't a pure
/// hex string — we don't actually need the bytes, just the length.
fn decode_hex(s: &str) -> Option<usize> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(s.len() / 2)
}

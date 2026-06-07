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
use serde::{Deserialize, Deserializer, de};

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

/// Top-level configuration. Cheap to clone (held inside `Arc<AppState>`).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// sqlx URL for the SQLite sidecar. e.g. `sqlite:///var/lib/hail/hail.db`.
    pub database_url: String,
    /// Mail backend flavour and cache policy. Defaults preserve the existing
    /// self-host/JMAP deployment shape when `[mail]` is omitted.
    #[serde(default)]
    pub mail: MailConfig,
    /// Stalwart JMAP / management endpoints.
    pub stalwart: StalwartConfig,
    /// HTTP server bind + public URL (for cookie scope / CORS).
    pub server: ServerConfig,
    /// Optional provider import settings. Gmail OAuth credentials are required
    /// only when provider import mode is enabled in the UI/API.
    #[serde(default)]
    pub provider_import: ProviderImportConfig,
    /// Optional admin block. When `None`, the first-run wizard at `/setup`
    /// can be used only if explicit setup bootstrap config is also present.
    #[serde(default)]
    pub admin: Option<AdminConfig>,
    /// First-run setup bootstrap guard. Operators must explicitly enable the
    /// wizard POST and provide a one-time bootstrap token out-of-band so a
    /// public empty deployment cannot be claimed by whoever reaches it first.
    #[serde(default)]
    pub setup: SetupConfig,
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

/// Provider import settings. Optional so non-Gmail deployments can run without
/// OAuth credentials until they enable provider-backed import mode.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderImportConfig {
    #[serde(default)]
    pub gmail: GmailProviderConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GmailProviderConfig {
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub oauth_client_secret: Option<SecretString>,
    #[serde(default)]
    pub oauth_auth_url: Option<String>,
    #[serde(default)]
    pub oauth_token_url: Option<String>,
    #[serde(default)]
    pub oauth_revoke_url: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    /// Optional smoke/development safety bound for the initial Gmail backfill.
    ///
    /// Defaults to `None` so production imports keep the full historical
    /// window. Set this for live smoke/dev environments with small local
    /// Stalwart blob quotas; the worker leaves the account in `initial_sync`
    /// with a durable backfill cursor when the bound is reached.
    #[serde(default, deserialize_with = "empty_string_as_none_usize")]
    pub initial_import_max_messages: Option<usize>,
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

/// First-run setup bootstrap settings. These do not affect the generic
/// `/api/setup/state` response: the UI may show the wizard for an empty
/// deployment, but `/api/setup/admin` still requires this explicit operator
/// enablement plus the matching token.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SetupConfig {
    /// Enables POST /api/setup/admin when no admin exists and `[admin]` is not
    /// configured. Defaults to false so an accidentally public empty instance
    /// cannot be claimed.
    #[serde(default)]
    pub bootstrap_enabled: bool,
    /// Operator-provided shared secret required in the setup form. Prefer an
    /// env var (`HAIL_SETUP__BOOTSTRAP_TOKEN`) over TOML for deployments.
    #[serde(default)]
    pub bootstrap_token: Option<SecretString>,
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

/// Mail backend flavour and cache policy.
#[derive(Debug, Clone)]
pub struct MailConfig {
    pub backend: MailBackend,
    pub gmail: MailGmailConfig,
    pub jmap: MailJmapConfig,
    pub cache: MailCacheConfig,
}

impl Default for MailConfig {
    fn default() -> Self {
        Self::default_for_backend(MailBackend::Jmap)
    }
}

impl<'de> Deserialize<'de> for MailConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawMailConfig {
            #[serde(default)]
            backend: Option<MailBackend>,
            #[serde(default)]
            gmail: Option<MailGmailConfig>,
            #[serde(default)]
            jmap: Option<MailJmapConfig>,
            #[serde(default)]
            cache: Option<RawMailCacheConfig>,
        }

        let raw = RawMailConfig::deserialize(deserializer)?;
        let backend = raw.backend.unwrap_or_default();
        Ok(Self {
            backend,
            gmail: raw.gmail.unwrap_or_default(),
            jmap: raw.jmap.unwrap_or_default(),
            cache: raw
                .cache
                .map(|cache| cache.into_config(backend))
                .unwrap_or_else(|| MailCacheConfig::default_for_backend(backend)),
        })
    }
}

impl MailConfig {
    fn default_for_backend(backend: MailBackend) -> Self {
        Self {
            backend,
            gmail: MailGmailConfig::default(),
            jmap: MailJmapConfig::default(),
            cache: MailCacheConfig::default_for_backend(backend),
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.cache.backfill == MailBackfill::Incremental && self.cache.mode == MailCacheMode::Off
        {
            return Err(ConfigError::InvalidMail(
                "mail.cache.backfill=incremental requires mail.cache.mode to be bounded or full"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailBackend {
    Gmail,
    Jmap,
}

impl Default for MailBackend {
    fn default() -> Self {
        Self::Jmap
    }
}

#[derive(Clone, Deserialize, Default)]
pub struct MailGmailConfig {
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub oauth_client_secret: Option<SecretString>,
    #[serde(default)]
    pub oauth_auth_url: Option<String>,
    #[serde(default)]
    pub oauth_token_url: Option<String>,
    #[serde(default)]
    pub oauth_revoke_url: Option<String>,
}

impl std::fmt::Debug for MailGmailConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MailGmailConfig")
            .field("oauth_client_id", &self.oauth_client_id)
            .field(
                "oauth_client_secret",
                &self.oauth_client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("oauth_auth_url", &self.oauth_auth_url)
            .field("oauth_token_url", &self.oauth_token_url)
            .field("oauth_revoke_url", &self.oauth_revoke_url)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MailJmapConfig {
    pub jmap_url: String,
    #[serde(default)]
    pub management_url: Option<String>,
}

impl Default for MailJmapConfig {
    fn default() -> Self {
        Self {
            jmap_url: "http://stalwart:8080".to_string(),
            management_url: Some("http://stalwart:8080".to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MailCacheConfig {
    pub mode: MailCacheMode,
    pub keep_days: u32,
    pub keep_max_msgs: u64,
    pub keep_max_bytes: u64,
    pub backfill: MailBackfill,
    pub blob_root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawMailCacheConfig {
    #[serde(default)]
    mode: Option<MailCacheMode>,
    #[serde(default)]
    keep_days: Option<u32>,
    #[serde(default)]
    keep_max_msgs: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_human_size")]
    keep_max_bytes: Option<u64>,
    #[serde(default)]
    backfill: Option<MailBackfill>,
    #[serde(default)]
    blob_root: Option<PathBuf>,
}

impl RawMailCacheConfig {
    fn into_config(self, backend: MailBackend) -> MailCacheConfig {
        let defaults = MailCacheConfig::default_for_backend(backend);
        MailCacheConfig {
            mode: self.mode.unwrap_or(defaults.mode),
            keep_days: self.keep_days.unwrap_or(defaults.keep_days),
            keep_max_msgs: self.keep_max_msgs.unwrap_or(defaults.keep_max_msgs),
            keep_max_bytes: self.keep_max_bytes.unwrap_or(defaults.keep_max_bytes),
            backfill: self.backfill.unwrap_or(defaults.backfill),
            blob_root: self.blob_root.unwrap_or(defaults.blob_root),
        }
    }
}

impl MailCacheConfig {
    fn default_for_backend(backend: MailBackend) -> Self {
        Self {
            mode: MailCacheMode::Bounded,
            keep_days: default_cache_keep_days(),
            keep_max_msgs: default_cache_keep_max_msgs(),
            keep_max_bytes: default_cache_keep_max_bytes(),
            backfill: match backend {
                MailBackend::Gmail => MailBackfill::Incremental,
                MailBackend::Jmap => MailBackfill::Off,
            },
            blob_root: default_blob_root(),
        }
    }
}

impl Default for MailCacheConfig {
    fn default() -> Self {
        Self::default_for_backend(MailBackend::Jmap)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailCacheMode {
    Off,
    Bounded,
    Full,
}

impl Default for MailCacheMode {
    fn default() -> Self {
        Self::Bounded
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailBackfill {
    Off,
    Incremental,
}

impl Default for MailBackfill {
    fn default() -> Self {
        Self::Off
    }
}

fn default_cache_keep_days() -> u32 {
    90
}

fn default_cache_keep_max_msgs() -> u64 {
    50_000
}

fn default_cache_keep_max_bytes() -> u64 {
    5 * 1024 * 1024 * 1024
}

fn default_blob_root() -> PathBuf {
    PathBuf::from("/var/lib/hail/blobs")
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
    /// `secrets.server_key` was empty or invalid.
    #[error("invalid server_key: {0}")]
    InvalidServerKey(String),
    /// Cross-field mail/cache validation failed.
    #[error("invalid mail config: {0}")]
    InvalidMail(String),
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
        cfg.mail.validate()?;
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

/// Deserialize an optional usize while accepting an empty string as unset.
///
/// Compose entries like `VAR=${VAR:-}` materialize an unset env var as `""`.
/// Figment then feeds that string into Serde; without this helper an optional
/// numeric field fails to parse before application defaults can apply.
fn empty_string_as_none_usize<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalUsizeVisitor;

    impl<'de> de::Visitor<'de> for OptionalUsizeVisitor {
        type Value = Option<usize>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a non-negative integer, null, or an empty string")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            usize::try_from(value).map(Some).map_err(E::custom)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            usize::try_from(value).map(Some).map_err(E::custom)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                Ok(None)
            } else {
                value.parse::<usize>().map(Some).map_err(E::custom)
            }
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(OptionalUsizeVisitor)
}

fn deserialize_optional_human_size<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalHumanSizeVisitor;

    impl<'de> de::Visitor<'de> for OptionalHumanSizeVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a byte count, a humanized size string, null, or an empty string")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u64::try_from(value).map(Some).map_err(E::custom)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                parse_human_size(trimmed).map(Some).map_err(E::custom)
            }
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(OptionalHumanSizeVisitor)
}

fn parse_human_size(value: &str) -> Result<u64, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("size must not be empty".to_string());
    }

    let mut number_end = 0;
    for (idx, ch) in value.char_indices() {
        if ch.is_ascii_digit() || ch == '_' {
            number_end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if number_end == 0 {
        return Err(format!("invalid size {value:?}: missing number"));
    }

    let number = value[..number_end].replace('_', "");
    let amount = number.parse::<u64>().map_err(|err| err.to_string())?;
    let unit = value[number_end..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" | "byte" | "bytes" => 1,
        "kb" | "kib" => 1024,
        "mb" | "mib" => 1024_u64.pow(2),
        "gb" | "gib" => 1024_u64.pow(3),
        "tb" | "tib" => 1024_u64.pow(4),
        other => {
            return Err(format!(
                "invalid size unit {other:?}; expected B, KiB, MiB, GiB, or TiB"
            ));
        }
    };

    amount
        .checked_mul(multiplier)
        .ok_or_else(|| format!("size {value:?} overflows u64 bytes"))
}

/// Validate the server key by using the same parser that runtime crypto uses.
/// This accepts exactly 64 ASCII hex characters (`openssl rand -hex 32`) or a
/// non-hex raw string of at least 32 bytes. Longer pure-hex strings are rejected
/// instead of silently becoming raw ASCII at runtime.
fn validate_server_key(key: &SecretString) -> Result<(), ConfigError> {
    crate::parse_server_key(key).map(|_| ()).map_err(|err| {
        let msg = if key.expose_secret().is_empty() {
            "empty; set HAIL_SECRETS__SERVER_KEY or [secrets].server_key in TOML".to_string()
        } else {
            err.to_string()
        };
        ConfigError::InvalidServerKey(msg)
    })
}

//! Integration tests for `hail_core::Config`.
//!
//! These tests mutate process env vars (a global). Cargo runs tests inside
//! a single binary in parallel, so every test acquires `ENV_LOCK` first to
//! serialize access. Without it, one test's `clear_hail_env` races another
//! test's `set_var` and both see arbitrary state.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // Recover from poisoned mutex — a panicked test still left the env in a
    // state we'll clean up at the top of the next test.
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

use hail_core::{Config, ConfigError};
use secrecy::ExposeSecret;

/// Clear every `HAIL_*` env var so prior tests / the host shell don't bleed
/// into the current run. `remove_var` is `unsafe` on edition 2024 because
/// concurrent access is UB; we accept the precondition (serial tests).
fn clear_hail_env() {
    let keys: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| k.starts_with("HAIL_"))
        .collect();
    for k in keys {
        // SAFETY: serial single-thread test execution within this binary.
        unsafe {
            std::env::remove_var(&k);
        }
    }
}

/// Write a TOML to a temp file and return both the tempdir guard (so it
/// outlives the test) and the path.
fn write_toml(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hail.toml");
    fs::write(&path, body).expect("write toml");
    (dir, path)
}

const VALID_KEY_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"; // 64 hex chars = 32 bytes

const FULL_TOML: &str = r#"
database_url = "sqlite::memory:"

[stalwart]
jmap_url = "http://stalwart.local:8080"
management_url = "http://stalwart.local:8080/manage"

[server]
bind = "0.0.0.0:8080"
public_url = "https://hail.test"
webapp_dir = "/tmp/hail-webapp"

[admin]
email = "ops@hail.test"
display_name = "Ops"

[setup]
bootstrap_enabled = true
bootstrap_token = "operator-only-bootstrap-token"

[provider_import.gmail]
oauth_client_id = "gmail-client-id.apps.googleusercontent.com"
oauth_client_secret = "gmail-client-secret"
initial_import_max_messages = 37

[secrets]
server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;

#[test]
fn loads_full_toml_fields() {
    let _guard = env_lock();
    clear_hail_env();
    let (_dir, path) = write_toml(FULL_TOML);
    let cfg = Config::load_from(Some(&path)).expect("load");

    assert_eq!(cfg.database_url, "sqlite::memory:");
    assert_eq!(cfg.stalwart.jmap_url, "http://stalwart.local:8080");
    assert_eq!(
        cfg.stalwart.management_url.as_deref(),
        Some("http://stalwart.local:8080/manage")
    );
    assert_eq!(cfg.server.bind, "0.0.0.0:8080");
    assert_eq!(cfg.server.public_url, "https://hail.test");
    assert_eq!(
        cfg.server.webapp_dir.as_deref(),
        Some(std::path::Path::new("/tmp/hail-webapp"))
    );

    let admin = cfg.admin.as_ref().expect("[admin] present");
    assert_eq!(admin.email, "ops@hail.test");
    assert_eq!(admin.display_name.as_deref(), Some("Ops"));
    assert!(cfg.setup.bootstrap_enabled);
    assert_eq!(
        cfg.setup
            .bootstrap_token
            .as_ref()
            .expect("setup bootstrap token")
            .expose_secret(),
        "operator-only-bootstrap-token"
    );
    assert_eq!(
        cfg.provider_import.gmail.oauth_client_id.as_deref(),
        Some("gmail-client-id.apps.googleusercontent.com")
    );
    assert_eq!(
        cfg.provider_import
            .gmail
            .oauth_client_secret
            .as_ref()
            .expect("gmail client secret")
            .expose_secret(),
        "gmail-client-secret"
    );
    assert_eq!(
        cfg.provider_import.gmail.initial_import_max_messages,
        Some(37)
    );

    assert_eq!(cfg.secrets.server_key.expose_secret(), VALID_KEY_HEX);
}

#[test]
fn env_overrides_toml() {
    let _guard = env_lock();
    clear_hail_env();
    let override_key: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    // SAFETY: serial single-thread test execution.
    unsafe {
        std::env::set_var("HAIL_SECRETS__SERVER_KEY", override_key);
        std::env::set_var("HAIL_STALWART__JMAP_URL", "http://override:9999");
    }
    let (_dir, path) = write_toml(FULL_TOML);
    let cfg = Config::load_from(Some(&path)).expect("load");

    assert_eq!(cfg.secrets.server_key.expose_secret(), override_key);
    assert_eq!(cfg.stalwart.jmap_url, "http://override:9999");
    // Unrelated fields still come from TOML.
    assert_eq!(cfg.server.bind, "0.0.0.0:8080");

    clear_hail_env();
}

#[test]
fn empty_initial_import_max_env_is_treated_as_unset() {
    let _guard = env_lock();
    clear_hail_env();
    unsafe {
        std::env::set_var(
            "HAIL_PROVIDER_IMPORT__GMAIL__INITIAL_IMPORT_MAX_MESSAGES",
            "",
        );
    }
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;
    let (_dir, path) = write_toml(toml);
    let cfg = Config::load_from(Some(&path)).expect("load with empty optional numeric env");
    assert_eq!(cfg.provider_import.gmail.initial_import_max_messages, None);

    clear_hail_env();
}

#[test]
fn numeric_initial_import_max_env_still_overrides_toml() {
    let _guard = env_lock();
    clear_hail_env();
    unsafe {
        std::env::set_var(
            "HAIL_PROVIDER_IMPORT__GMAIL__INITIAL_IMPORT_MAX_MESSAGES",
            "250",
        );
    }
    let (_dir, path) = write_toml(FULL_TOML);
    let cfg = Config::load_from(Some(&path)).expect("load with numeric optional env");
    assert_eq!(
        cfg.provider_import.gmail.initial_import_max_messages,
        Some(250)
    );

    clear_hail_env();
}

#[test]
fn missing_server_key_errors_clearly() {
    let _guard = env_lock();
    clear_hail_env();
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = ""
"#;
    let (_dir, path) = write_toml(toml);
    let err = Config::load_from(Some(&path)).expect_err("must reject empty key");
    match err {
        ConfigError::InvalidServerKey(msg) => {
            assert!(
                msg.contains("empty") || msg.contains("HAIL_SECRETS__SERVER_KEY"),
                "expected guidance message, got: {msg}"
            );
        }
        other => panic!("expected InvalidServerKey, got {other:?}"),
    }
}

#[test]
fn short_server_key_errors_with_byte_count() {
    let _guard = env_lock();
    clear_hail_env();
    // 30 hex chars = 15 bytes — well below the 32-byte minimum.
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = "0123456789abcdef0123456789abcd"
"#;
    let (_dir, path) = write_toml(toml);
    let err = Config::load_from(Some(&path)).expect_err("must reject short key");
    assert!(matches!(err, ConfigError::InvalidServerKey(_)));
}

#[test]
fn longer_pure_hex_server_key_is_rejected() {
    let _guard = env_lock();
    clear_hail_env();
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef00"
"#;
    let (_dir, path) = write_toml(toml);
    let err = Config::load_from(Some(&path)).expect_err("must reject 66-char hex key");
    assert!(matches!(err, ConfigError::InvalidServerKey(_)));
}

#[test]
fn setup_bootstrap_defaults_to_disabled() {
    let _guard = env_lock();
    clear_hail_env();
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;
    let (_dir, path) = write_toml(toml);
    let cfg = Config::load_from(Some(&path)).expect("load");
    assert!(!cfg.setup.bootstrap_enabled);
    assert!(cfg.setup.bootstrap_token.is_none());
}

#[test]
fn setup_bootstrap_env_overrides_toml() {
    let _guard = env_lock();
    clear_hail_env();
    unsafe {
        std::env::set_var("HAIL_SETUP__BOOTSTRAP_ENABLED", "true");
        std::env::set_var("HAIL_SETUP__BOOTSTRAP_TOKEN", "from-env-token");
    }
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;
    let (_dir, path) = write_toml(toml);
    let cfg = Config::load_from(Some(&path)).expect("load");
    assert!(cfg.setup.bootstrap_enabled);
    assert_eq!(
        cfg.setup
            .bootstrap_token
            .as_ref()
            .expect("bootstrap token")
            .expose_secret(),
        "from-env-token"
    );

    clear_hail_env();
}

#[test]
fn admin_block_optional() {
    let _guard = env_lock();
    clear_hail_env();
    let toml = r#"
database_url = "sqlite::memory:"
[stalwart]
jmap_url = "http://x"
[server]
bind = "0.0.0.0:8080"
public_url = "https://x"
[secrets]
server_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;
    let (_dir, path) = write_toml(toml);
    let cfg = Config::load_from(Some(&path)).expect("load");
    assert!(
        cfg.admin.is_none(),
        "no [admin] block => Config.admin == None"
    );
}

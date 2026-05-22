//! Temporary local crypto adapter.
//!
//! `hail-core::crypto::open` is being built in parallel by the
//! `auth-login` task (worker-1). To unblock this task we define a thin
//! trait so the per-user supervisor can decrypt the session's
//! `jmap_token_enc` blob via dependency injection — the production
//! adapter and the test fake share one shape.
//!
//! TODO: swap [`HailCoreOpener`] for `hail_core::crypto::open` once the
//! auth-login wave merges, and delete this module.

use secrecy::SecretString;

/// Outcome of attempting to decrypt a JMAP token blob.
///
/// Variants are unused until `hail_core::crypto::open` lands and the
/// production decryptor stops `todo!()`-ing; the supervisor pattern-
/// matches on this enum for FATAL classification, so the variants
/// must exist now.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum DecryptError {
    /// The ciphertext was malformed (wrong length, bad tag, etc.).
    #[error("ciphertext is malformed: {0}")]
    Malformed(&'static str),
    /// AES-GCM verification failed — either the key is wrong or the
    /// blob was tampered with. Treated as FATAL by the supervisor.
    #[error("authentication failed")]
    AuthFailed,
}

/// Decrypts a session's `jmap_token_enc` blob into the plaintext bearer
/// token. Stays object-safe so the supervisor can hold an
/// `Arc<dyn TokenDecryptor>`.
pub trait TokenDecryptor: Send + Sync {
    /// Decrypt `enc` using the configured server key, returning the
    /// resulting bearer token wrapped in [`SecretString`] so it cannot
    /// land in logs by accident.
    fn decrypt(&self, enc: &[u8]) -> Result<SecretString, DecryptError>;
}

/// Production adapter that will delegate to `hail_core::crypto::open`
/// once auth-login lands. For now it panics on use, so any code that
/// actually opens a JMAP session in a non-test context will surface a
/// loud failure instead of a silent misroute. Tests inject a fake
/// decryptor; the top-level supervisor wires this real one.
pub struct HailCoreOpener {
    /// Server key from `Config::secrets.server_key`. Retained for the
    /// future call to `hail_core::crypto::open`.
    _server_key: SecretString,
}

impl HailCoreOpener {
    /// Construct an opener bound to the given server key material.
    #[must_use]
    pub fn new(server_key: SecretString) -> Self {
        Self {
            _server_key: server_key,
        }
    }
}

impl TokenDecryptor for HailCoreOpener {
    fn decrypt(&self, _enc: &[u8]) -> Result<SecretString, DecryptError> {
        // Smoke-test / dev escape hatch: when `HAIL_INSECURE_DECRYPT=1`
        // is set we hand back an empty bearer token. This lets the
        // SIGINT-graceful-shutdown smoke test exercise the full
        // per-user supervisor path (including the JMAP connect await)
        // without depending on auth-login. NEVER set this in prod.
        if std::env::var("HAIL_INSECURE_DECRYPT").as_deref() == Ok("1") {
            return Ok(SecretString::from(String::new()));
        }
        // TODO: replace with `hail_core::crypto::open(self._server_key, enc)`
        // once the auth-login wave lands. Until then a per-user
        // supervisor that reaches this point fails fast.
        todo!(
            "hail_core::crypto::open is not yet on main; \
             waiting on the auth-login task"
        )
    }
}

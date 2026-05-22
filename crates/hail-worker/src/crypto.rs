//! Local crypto adapter for the worker.
//!
//! The actual AES-256-GCM primitive lives in [`hail_core::crypto`]
//! (shipped by the auth-login wave). This module is a thin adapter
//! that wraps it behind a [`TokenDecryptor`] trait so the per-user
//! supervisor can inject a fake in tests and the real opener in prod.

use hail_core::crypto::{self, KEY_LEN};
use secrecy::{ExposeSecret, SecretString};

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

/// Production adapter. Delegates to [`hail_core::crypto::open`] using
/// a server key parsed at construction.
pub struct HailCoreOpener {
    server_key: [u8; KEY_LEN],
}

impl HailCoreOpener {
    /// Construct an opener bound to the given server key material.
    /// The key is parsed via [`hail_core::crypto::parse_server_key`]
    /// (accepts 64-char hex or ≥32 raw bytes).
    pub fn new(server_key: SecretString) -> Result<Self, hail_core::crypto::CryptoError> {
        let key = crypto::parse_server_key(&server_key)?;
        Ok(Self { server_key: key })
    }
}

impl Drop for HailCoreOpener {
    fn drop(&mut self) {
        // Zero the in-memory key on drop. The `aes-gcm` crate already
        // zeroizes its internal state; this is belt-and-suspenders for
        // our own copy.
        self.server_key.fill(0);
    }
}

impl TokenDecryptor for HailCoreOpener {
    fn decrypt(&self, enc: &[u8]) -> Result<SecretString, DecryptError> {
        match crypto::open(enc, &self.server_key) {
            Ok(plaintext) => match String::from_utf8(plaintext) {
                Ok(s) => Ok(SecretString::from(s)),
                Err(_) => Err(DecryptError::Malformed("plaintext is not valid utf-8")),
            },
            Err(crypto::CryptoError::Malformed) => {
                Err(DecryptError::Malformed("ciphertext too short or malformed"))
            }
            Err(crypto::CryptoError::Decrypt) => Err(DecryptError::AuthFailed),
            Err(crypto::CryptoError::InvalidKey(_)) | Err(crypto::CryptoError::Rng) => {
                // Key was validated at construction and we don't generate
                // a nonce here; reaching either branch means an internal
                // invariant broke. Surface as AuthFailed.
                Err(DecryptError::AuthFailed)
            }
        }
    }
}

// Silence dead_code warning on ExposeSecret import (used transitively via parse_server_key).
const _: fn(&SecretString) = |s| {
    let _ = s.expose_secret();
};

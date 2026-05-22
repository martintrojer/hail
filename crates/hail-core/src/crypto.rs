//! AES-256-GCM helpers used to wrap JMAP bearer tokens at rest (DD-8).
//!
//! The hail process holds the symmetric key in memory; sessions persist
//! only the AEAD ciphertext in `sessions.jmap_token_enc`. Compromise of
//! the SQLite file alone therefore does not yield usable JMAP tokens.
//!
//! Wire format produced by [`seal`] / consumed by [`open`]:
//! ```text
//!   ┌──────────────┬──────────────────────────────────┬──────────┐
//!   │ nonce (12 B) │ ciphertext (plaintext.len() B)   │ tag (16) │
//!   └──────────────┴──────────────────────────────────┴──────────┘
//! ```
//! The `aes-gcm` crate's `encrypt` returns `ciphertext || tag` as a single
//! buffer, so the on-disk layout is exactly `nonce || that`. The
//! ciphertext is non-deterministic — each `seal` draws a fresh random
//! nonce from the OS CSPRNG. (Reusing a nonce under the same key would
//! be catastrophic for GCM; this is the standard countermeasure.)

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::TryRngCore;
use secrecy::{ExposeSecret, SecretString};

/// AES-GCM IV size in bytes (96-bit, the standard).
pub const NONCE_LEN: usize = 12;
/// AES-256 key size in bytes.
pub const KEY_LEN: usize = 32;
/// GCM authentication tag size in bytes.
pub const TAG_LEN: usize = 16;

/// Errors surfaced by the crypto helpers. Variants are deliberately
/// coarse so the API doesn't leak structure that a chosen-ciphertext
/// adversary could probe (open() collapses every authentication failure
/// onto the same `Decrypt` variant).
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Operator-supplied server key was unusable (wrong length, bad hex).
    /// Surfaced only at startup; never on hot-path requests.
    #[error("invalid server key: {0}")]
    InvalidKey(&'static str),

    /// Ciphertext was shorter than `NONCE_LEN + TAG_LEN`, i.e. structurally
    /// invalid. Distinguished from `Decrypt` only to give the caller a
    /// chance to log "corrupted row" vs "wrong key / tampered".
    #[error("ciphertext too short")]
    Malformed,

    /// AEAD verification failed: tampered ciphertext, wrong key, or
    /// truncated tag. We deliberately do NOT distinguish these to the
    /// caller — they're cryptographically indistinguishable from the
    /// attacker's point of view, and we don't want to give them a clue.
    #[error("decryption failed")]
    Decrypt,

    /// OS CSPRNG refused to produce 12 bytes. Effectively unreachable on
    /// a live OS but propagated so we never silently fall back to a
    /// predictable nonce.
    #[error("failed to draw nonce from OS RNG")]
    Rng,
}

/// Encrypt `plaintext` under `key` with a fresh random 96-bit nonce.
/// Output layout: `nonce (12) || ciphertext || tag (16)`.
///
/// The output never depends on `plaintext` length beyond an additive
/// `NONCE_LEN + TAG_LEN` overhead — fine for short tokens.
pub fn seal(plaintext: &[u8], key: &[u8; KEY_LEN]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce_bytes)
        .map_err(|_| CryptoError::Rng)?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(nonce, plaintext)
        // `aes-gcm`'s `encrypt` only fails on internal invariant violations
        // (e.g. plaintext too long for GCM's 64 GiB limit). Treat as opaque.
        .map_err(|_| CryptoError::Decrypt)?;

    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt a buffer produced by [`seal`] under the same `key`. Returns
/// the original plaintext, or [`CryptoError`] if the AEAD tag failed to
/// verify (tampering, truncation, wrong key — indistinguishable).
pub fn open(ciphertext: &[u8], key: &[u8; KEY_LEN]) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::Malformed);
    }
    let (nonce_bytes, body) = ciphertext.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), body)
        .map_err(|_| CryptoError::Decrypt)
}

/// Parse the operator-provided server key into the 32-byte form
/// AES-256-GCM expects.
///
/// Accepts:
///   - 64 ASCII hex characters (the `openssl rand -hex 32` form), or
///   - 32+ raw bytes — the first 32 are taken as the key.
///
/// `Config::load` has already enforced the byte-length minimum, so this
/// is mostly a hex decode + memcpy. The output is never logged.
pub fn parse_server_key(s: &SecretString) -> Result<[u8; KEY_LEN], CryptoError> {
    let raw = s.expose_secret();

    // Hex path: 64 hex chars → 32 bytes.
    if raw.len() == 2 * KEY_LEN && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        let decoded =
            hex::decode(raw).map_err(|_| CryptoError::InvalidKey("hex decode failed"))?;
        let mut out = [0u8; KEY_LEN];
        out.copy_from_slice(&decoded);
        return Ok(out);
    }

    // Raw-byte path: ≥ 32 bytes, take the first 32. Matches the
    // looser side of `validate_server_key` in `config.rs`.
    if raw.len() >= KEY_LEN {
        let mut out = [0u8; KEY_LEN];
        out.copy_from_slice(&raw.as_bytes()[..KEY_LEN]);
        return Ok(out);
    }

    Err(CryptoError::InvalidKey(
        "server key must be 64 hex chars or ≥ 32 raw bytes",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn k(byte: u8) -> [u8; KEY_LEN] {
        [byte; KEY_LEN]
    }

    #[test]
    fn roundtrip() {
        let key = k(0xAB);
        let pt = b"hello, jmap bearer token";
        let ct = seal(pt, &key).expect("seal");
        // Nonce-prefixed → at least NONCE_LEN + TAG_LEN bytes longer than plaintext.
        assert_eq!(ct.len(), pt.len() + NONCE_LEN + TAG_LEN);
        let recovered = open(&ct, &key).expect("open");
        assert_eq!(recovered, pt);
    }

    #[test]
    fn nonces_are_unique() {
        // Two encryptions of the same plaintext must produce different
        // ciphertexts — that's how we know the nonce is random.
        let key = k(0x01);
        let a = seal(b"x", &key).unwrap();
        let b = seal(b"x", &key).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn tamper_detected() {
        let key = k(0x77);
        let mut ct = seal(b"sensitive", &key).expect("seal");
        // Flip a byte in the body — the AEAD tag must reject it.
        let mid = NONCE_LEN + 2;
        ct[mid] ^= 0x40;
        assert!(matches!(open(&ct, &key), Err(CryptoError::Decrypt)));
    }

    #[test]
    fn wrong_key_rejected() {
        let pt = b"another secret";
        let ct = seal(pt, &k(0x11)).expect("seal");
        assert!(matches!(open(&ct, &k(0x22)), Err(CryptoError::Decrypt)));
    }

    #[test]
    fn truncated_ciphertext_is_malformed() {
        let ct = seal(b"abc", &k(0x33)).expect("seal");
        // Trim to less than nonce + tag.
        let short = &ct[..NONCE_LEN + TAG_LEN - 1];
        assert!(matches!(open(short, &k(0x33)), Err(CryptoError::Malformed)));
    }

    #[test]
    fn parse_hex_key() {
        // 64 hex chars = 32 bytes.
        let hex = "0".repeat(64);
        let parsed = parse_server_key(&SecretString::from(hex)).expect("hex parse");
        assert_eq!(parsed, [0u8; KEY_LEN]);
    }

    #[test]
    fn parse_raw_key_takes_first_32() {
        // 40 raw bytes — first 32 should land in the key.
        let raw: String = (0..40).map(|i| (b'a' + (i % 26)) as char).collect();
        let parsed = parse_server_key(&SecretString::from(raw.clone())).expect("raw parse");
        assert_eq!(&parsed[..], &raw.as_bytes()[..KEY_LEN]);
    }

    #[test]
    fn parse_rejects_short_key() {
        let short = SecretString::from("tooshort");
        assert!(matches!(
            parse_server_key(&short),
            Err(CryptoError::InvalidKey(_))
        ));
    }
}

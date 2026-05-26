//! Provider OAuth token crypto helpers.
//!
//! Gmail/provider refresh tokens are long-lived credentials. Store them only as
//! AES-256-GCM ciphertext in `provider_accounts.refresh_token_enc`, using the
//! same server-key primitive that wraps JMAP session bearer tokens. Access
//! tokens are normally short-lived memory values, but this module supports both
//! token kinds so OAuth code has one secret-safe representation.
//!
//! The AEAD additional authenticated data binds ciphertext to non-secret account
//! metadata. Copying an encrypted token blob to a different provider account or
//! decrypting it as a different token kind fails authentication instead of
//! returning plaintext.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};

use crate::crypto::{self, CryptoError, KEY_LEN};

const PROVIDER_TOKEN_AAD_VERSION: &str = "hail-provider-oauth-token:v1";

/// OAuth token kind protected by provider-token crypto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOAuthTokenKind {
    /// Long-lived OAuth refresh token stored encrypted in `hail.db`.
    Refresh,
    /// Short-lived OAuth access token. Usually memory-only, but uses the same
    /// wrapper when tests or future queues need an encrypted form.
    Access,
}

impl ProviderOAuthTokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Access => "access",
        }
    }
}

/// Non-secret provider account metadata authenticated with each token blob.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderTokenContext {
    pub user_id: i64,
    pub provider_account_row_id: i64,
    pub provider_kind: String,
    pub provider_account_id: String,
    pub token_kind: ProviderOAuthTokenKind,
}

impl ProviderTokenContext {
    /// Construct context for a token associated with one `provider_accounts` row.
    pub fn new(
        user_id: i64,
        provider_account_row_id: i64,
        provider_kind: impl Into<String>,
        provider_account_id: impl Into<String>,
        token_kind: ProviderOAuthTokenKind,
    ) -> Self {
        Self {
            user_id,
            provider_account_row_id,
            provider_kind: provider_kind.into(),
            provider_account_id: provider_account_id.into(),
            token_kind,
        }
    }

    fn aad(&self) -> Vec<u8> {
        // Length-prefix variable strings to avoid ambiguous concatenations.
        let provider_kind = self.provider_kind.as_bytes();
        let provider_account_id = self.provider_account_id.as_bytes();
        let mut out = Vec::with_capacity(
            PROVIDER_TOKEN_AAD_VERSION.len()
                + 1
                + 8
                + 8
                + 1
                + provider_kind.len()
                + 1
                + provider_account_id.len()
                + 1
                + self.token_kind.as_str().len(),
        );
        out.extend_from_slice(PROVIDER_TOKEN_AAD_VERSION.as_bytes());
        out.push(0);
        out.extend_from_slice(&self.user_id.to_be_bytes());
        out.extend_from_slice(&self.provider_account_row_id.to_be_bytes());
        append_len_prefixed(&mut out, provider_kind);
        append_len_prefixed(&mut out, provider_account_id);
        append_len_prefixed(&mut out, self.token_kind.as_str().as_bytes());
        out
    }
}

impl fmt::Debug for ProviderTokenContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderTokenContext")
            .field("user_id", &self.user_id)
            .field("provider_account_row_id", &self.provider_account_row_id)
            .field("provider_kind", &self.provider_kind)
            .field("provider_account_id", &self.provider_account_id)
            .field("token_kind", &self.token_kind)
            .finish()
    }
}

fn append_len_prefixed(out: &mut Vec<u8>, value: &[u8]) {
    let len = u32::try_from(value.len()).expect("provider token AAD field exceeds u32 length");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value);
}

/// Plaintext OAuth token wrapper. `Debug` is intentionally redacted.
#[derive(Clone)]
pub struct ProviderOAuthToken(SecretString);

impl ProviderOAuthToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(SecretString::from(token.into()))
    }

    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl From<SecretString> for ProviderOAuthToken {
    fn from(token: SecretString) -> Self {
        Self(token)
    }
}

impl fmt::Debug for ProviderOAuthToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderOAuthToken([REDACTED])")
    }
}

/// Ciphertext wrapper for provider OAuth tokens. `Debug` omits bytes so log
/// records cannot grow a copy-pastable credential blob.
#[derive(Clone, PartialEq, Eq)]
pub struct EncryptedProviderOAuthToken(Vec<u8>);

impl EncryptedProviderOAuthToken {
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for EncryptedProviderOAuthToken {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<Vec<u8>> for EncryptedProviderOAuthToken {
    fn from(ciphertext: Vec<u8>) -> Self {
        Self(ciphertext)
    }
}

impl fmt::Debug for EncryptedProviderOAuthToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedProviderOAuthToken")
            .field("ciphertext", &"[REDACTED]")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Provider-token crypto errors. Variants never include token plaintext.
#[derive(Debug, thiserror::Error)]
pub enum ProviderTokenCryptoError {
    #[error("provider token encryption failed")]
    Encrypt,

    #[error("provider token ciphertext is malformed")]
    Malformed,

    #[error("provider token authentication failed")]
    Decrypt,

    #[error("decrypted provider token is not valid UTF-8")]
    Utf8,
}

/// Encrypt an OAuth token under the hail server key and provider context.
pub fn seal_provider_oauth_token(
    token: &ProviderOAuthToken,
    key: &[u8; KEY_LEN],
    context: &ProviderTokenContext,
) -> Result<EncryptedProviderOAuthToken, ProviderTokenCryptoError> {
    crypto::seal_with_aad(
        token.expose_secret().as_bytes(),
        key,
        context.aad().as_slice(),
    )
    .map(EncryptedProviderOAuthToken)
    .map_err(|_| ProviderTokenCryptoError::Encrypt)
}

/// Decrypt an OAuth token under the hail server key and provider context.
pub fn open_provider_oauth_token(
    ciphertext: impl AsRef<[u8]>,
    key: &[u8; KEY_LEN],
    context: &ProviderTokenContext,
) -> Result<ProviderOAuthToken, ProviderTokenCryptoError> {
    let plaintext = crypto::open_with_aad(ciphertext.as_ref(), key, context.aad().as_slice())
        .map_err(|err| match err {
            CryptoError::Malformed => ProviderTokenCryptoError::Malformed,
            CryptoError::Decrypt => ProviderTokenCryptoError::Decrypt,
            CryptoError::InvalidKey(_) | CryptoError::Rng => ProviderTokenCryptoError::Decrypt,
        })?;
    String::from_utf8(plaintext)
        .map(ProviderOAuthToken::new)
        .map_err(|_| ProviderTokenCryptoError::Utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(byte: u8) -> [u8; KEY_LEN] {
        [byte; KEY_LEN]
    }

    fn context(kind: ProviderOAuthTokenKind) -> ProviderTokenContext {
        ProviderTokenContext::new(42, 7, "gmail", "gmail-account-123", kind)
    }

    #[test]
    fn provider_refresh_token_roundtrips() {
        let token = ProviderOAuthToken::new("1//refresh-secret-token");
        let key = k(0xA5);
        let context = context(ProviderOAuthTokenKind::Refresh);

        let encrypted = seal_provider_oauth_token(&token, &key, &context).expect("seal");
        assert_ne!(encrypted.as_bytes(), token.expose_secret().as_bytes());
        assert!(
            !encrypted
                .as_bytes()
                .windows(token.expose_secret().len())
                .any(|window| window == token.expose_secret().as_bytes())
        );

        let recovered = open_provider_oauth_token(&encrypted, &key, &context).expect("open");
        assert_eq!(recovered.expose_secret(), token.expose_secret());
    }

    #[test]
    fn provider_access_token_roundtrips() {
        let token = ProviderOAuthToken::new("ya29.access-secret-token");
        let key = k(0xB6);
        let context = context(ProviderOAuthTokenKind::Access);

        let encrypted = seal_provider_oauth_token(&token, &key, &context).expect("seal");
        let recovered = open_provider_oauth_token(&encrypted, &key, &context).expect("open");

        assert_eq!(recovered.expose_secret(), token.expose_secret());
    }

    #[test]
    fn wrong_key_fails_without_plaintext() {
        let plaintext = "1//wrong-key-secret";
        let token = ProviderOAuthToken::new(plaintext);
        let context = context(ProviderOAuthTokenKind::Refresh);
        let encrypted = seal_provider_oauth_token(&token, &k(0x11), &context).expect("seal");

        let err = open_provider_oauth_token(&encrypted, &k(0x22), &context).expect_err("wrong key");

        assert!(matches!(err, ProviderTokenCryptoError::Decrypt));
        let display = err.to_string();
        let debug = format!("{err:?}");
        assert!(!display.contains(plaintext));
        assert!(!debug.contains(plaintext));
    }

    #[test]
    fn authenticated_context_must_match() {
        let token = ProviderOAuthToken::new("1//context-bound-secret");
        let key = k(0xC7);
        let refresh_context = context(ProviderOAuthTokenKind::Refresh);
        let encrypted = seal_provider_oauth_token(&token, &key, &refresh_context).expect("seal");

        let access_context = context(ProviderOAuthTokenKind::Access);
        let err = open_provider_oauth_token(&encrypted, &key, &access_context)
            .expect_err("token kind mismatch must fail");

        assert!(matches!(err, ProviderTokenCryptoError::Decrypt));
    }

    #[test]
    fn debug_output_redacts_token_material() {
        let plaintext = "ya29.debug-secret-token";
        let token = ProviderOAuthToken::new(plaintext);
        let key = k(0xD8);
        let context = context(ProviderOAuthTokenKind::Access);
        let encrypted = seal_provider_oauth_token(&token, &key, &context).expect("seal");
        let err = open_provider_oauth_token(&encrypted, &k(0xD9), &context).expect_err("wrong key");

        for rendered in [
            format!("{token:?}"),
            format!("{encrypted:?}"),
            format!("{context:?}"),
            format!("{err:?}"),
            err.to_string(),
        ] {
            assert!(!rendered.contains(plaintext), "leaked in {rendered}");
            assert!(
                !rendered.contains("debug-secret-token"),
                "leaked in {rendered}"
            );
        }
    }
}

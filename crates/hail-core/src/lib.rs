//! Domain types shared between `hail-api` and `hail-worker`.
//!
//! For now this crate is mostly the unified configuration loader; future
//! tasks will park shared API types and error enums here as the binaries
//! grow.

pub mod blob;
pub mod config;
pub mod crypto;
pub mod mail_classification;
pub mod mail_render;
pub mod provider_tokens;
pub mod screener;

pub use blob::{BlobId, BlobIdParseError, BlobKind};
pub use config::{
    AdminConfig, Config, ConfigError, GmailProviderConfig, MailBackend, MailBackfill,
    MailCacheConfig, MailCacheMode, MailConfig, MailGmailConfig, MailJmapConfig,
    ProviderImportConfig, SecretsConfig, ServerConfig, SetupConfig, StalwartConfig,
};
pub use crypto::{
    CryptoError, KEY_LEN, NONCE_LEN, TAG_LEN, open, open_with_aad, parse_server_key, seal,
    seal_with_aad,
};
pub use mail_classification::{HAIL_SPAM_KEYWORD, MailClassification, SPAM_KEYWORD};
pub use mail_render::{
    BlockedTracker, SanitizedHtml, sanitize_and_strip_trackers, sanitize_outgoing_html,
};
pub use provider_tokens::{
    EncryptedProviderOAuthToken, ProviderOAuthToken, ProviderOAuthTokenKind, ProviderTokenContext,
    ProviderTokenCryptoError, open_provider_oauth_token, seal_provider_oauth_token,
};
pub use screener::{
    Classification, ScreenerDecision, ScreenerRule, ScreenerRuleParseError, lookup_rule,
    normalize_sender,
};

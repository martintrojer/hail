//! Domain types shared between `hail-api` and `hail-worker`.
//!
//! For now this crate is mostly the unified configuration loader; future
//! tasks will park shared API types and error enums here as the binaries
//! grow.

pub mod config;
pub mod crypto;
pub mod mail_classification;
pub mod mail_render;
pub mod screener;

pub use config::{
    AdminConfig, Config, ConfigError, SecretsConfig, ServerConfig, SetupConfig, StalwartConfig,
};
pub use crypto::{CryptoError, KEY_LEN, NONCE_LEN, TAG_LEN, open, parse_server_key, seal};
pub use mail_classification::{HAIL_SPAM_KEYWORD, MailClassification, SPAM_KEYWORD};
pub use mail_render::{BlockedTracker, SanitizedHtml, sanitize_and_strip_trackers};
pub use screener::{
    Classification, ScreenerDecision, ScreenerRule, ScreenerRuleParseError, lookup_rule,
    normalize_sender,
};

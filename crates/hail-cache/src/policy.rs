//! Cache policy types shared by cache modules.

use hail_core::{MailBackfill, MailCacheConfig, MailCacheMode};

/// Cache mode reused from hail-core configuration.
pub type CacheMode = MailCacheMode;

/// Cache backfill policy reused from hail-core configuration.
pub type CacheBackfill = MailBackfill;

/// Cache policy used by `CachedMail`.
///
/// This mirrors the `[mail.cache]` config block but avoids coupling callers to
/// blob-store path configuration after construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePolicy {
    pub mode: MailCacheMode,
    pub keep_days: u32,
    pub keep_max_msgs: u64,
    pub keep_max_bytes: u64,
    pub backfill: MailBackfill,
}

impl CachePolicy {
    #[must_use]
    pub const fn new(
        mode: MailCacheMode,
        keep_days: u32,
        keep_max_msgs: u64,
        keep_max_bytes: u64,
        backfill: MailBackfill,
    ) -> Self {
        Self {
            mode,
            keep_days,
            keep_max_msgs,
            keep_max_bytes,
            backfill,
        }
    }
}

impl From<&MailCacheConfig> for CachePolicy {
    fn from(value: &MailCacheConfig) -> Self {
        Self {
            mode: value.mode,
            keep_days: value.keep_days,
            keep_max_msgs: value.keep_max_msgs,
            keep_max_bytes: value.keep_max_bytes,
            backfill: value.backfill,
        }
    }
}

impl From<MailCacheConfig> for CachePolicy {
    fn from(value: MailCacheConfig) -> Self {
        Self::from(&value)
    }
}

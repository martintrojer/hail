use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Stalwart spam verdict keyword for mail classified as junk.
pub const SPAM_KEYWORD: &str = "$Junk";
/// Hail-owned marker applied after spam-flagged mail is routed to Junk.
pub const HAIL_SPAM_KEYWORD: &str = "$hail_spam";

/// Canonical hail-owned routing classification for incoming mail.
///
/// These values are stored in sidecar rule rows as lowercase strings and are
/// represented in JMAP as hail-owned keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MailClassification {
    Imbox,
    Feed,
    Papertrail,
}

impl MailClassification {
    pub const ALL: [Self; 3] = [Self::Imbox, Self::Feed, Self::Papertrail];

    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Imbox => "$hail_imbox",
            Self::Feed => "$hail_feed",
            Self::Papertrail => "$hail_papertrail",
        }
    }

    #[must_use]
    pub fn from_keyword(kw: &str) -> Option<Self> {
        match kw {
            "$hail_imbox" => Some(Self::Imbox),
            "$hail_feed" => Some(Self::Feed),
            "$hail_papertrail" => Some(Self::Papertrail),
            _ => None,
        }
    }

    #[must_use]
    pub const fn db_value(self) -> &'static str {
        match self {
            Self::Imbox => "imbox",
            Self::Feed => "feed",
            Self::Papertrail => "papertrail",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "imbox" => Some(Self::Imbox),
            "feed" => Some(Self::Feed),
            "papertrail" => Some(Self::Papertrail),
            _ => None,
        }
    }
}

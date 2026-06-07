//! Backend-agnostic mail provider trait and shared domain types.
//!
//! This crate defines the narrow seam between hail's cache layer and concrete
//! mail providers. It intentionally contains no backend implementations.

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod conformance;

pub use hail_core::MailClassification;

/// Crate-local result type for backend operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by [`MailBackend`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The backend does not support the requested operation.
    #[error("backend capability is not supported: {capability}")]
    UnsupportedCapability { capability: &'static str },

    /// The requested backend object does not exist.
    #[error("backend object was not found: {kind} {id}")]
    NotFound { kind: &'static str, id: String },

    /// Backend rejected or could not understand caller input.
    #[error("invalid backend request: {0}")]
    InvalidRequest(String),

    /// Authentication or authorization failed.
    #[error("backend authentication failed")]
    Authentication,

    /// Backend rate limited the operation.
    #[error("backend rate limited the operation")]
    RateLimited,

    /// The backend or network is temporarily unavailable.
    #[error("backend is temporarily unavailable")]
    TemporarilyUnavailable,

    /// Catch-all for implementation-specific failures.
    #[error("backend error: {0}")]
    Other(String),
}

/// Backend-advertised feature flags and small typed limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub supports_initial_import: bool,
    pub supports_eventsource: bool,
    pub supports_principals_admin: bool,
    pub supports_send: bool,
    pub native_threading: bool,
    pub max_attachment_size: u64,
    pub label_path_separator: char,
}

/// Stable provider-native message identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BackendMsgId(pub String);

impl BackendMsgId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable provider-native blob identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlobRef(pub String);

impl BlobRef {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Structured metadata for a message attachment or inline related part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub blob_ref: Option<BlobRef>,
    pub inline: bool,
    pub content_id: Option<String>,
}

/// Provider-native submission identifier returned by send operations.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SubmissionId(pub String);

impl SubmissionId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque backend sync cursor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SyncCursor(pub String);

impl SyncCursor {
    #[must_use]
    pub fn new(cursor: impl Into<String>) -> Self {
        Self(cursor.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Backend keyword or label identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Keyword(pub String);

impl Keyword {
    #[must_use]
    pub fn new(keyword: impl Into<String>) -> Self {
        Self(keyword.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn from_classification(classification: MailClassification) -> Self {
        Self(classification.keyword().to_owned())
    }
}

/// Canonical mailbox roles understood by hail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Archive,
    Drafts,
    Sent,
    Trash,
    Junk,
    Important,
    AllMail,
    Custom,
}

/// Minimal query model accepted by backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub scope: QueryScope,
    pub text: Option<String>,
    pub mailbox_role: Option<MailboxRole>,
    pub keywords: Vec<Keyword>,
    pub newer_than_epoch_secs: Option<i64>,
    pub older_than_epoch_secs: Option<i64>,
}

impl Query {
    #[must_use]
    pub fn all() -> Self {
        Self {
            scope: QueryScope::All,
            text: None,
            mailbox_role: None,
            keywords: Vec::new(),
            newer_than_epoch_secs: None,
            older_than_epoch_secs: None,
        }
    }
}

/// High-level backend search/list scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryScope {
    All,
    Search,
    Thread,
}

/// Page request for stable backend listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
}

impl PageRequest {
    #[must_use]
    pub const fn first(limit: u32) -> Self {
        Self {
            limit,
            cursor: None,
        }
    }
}

/// A backend page of items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

/// SMTP-style envelope for outbound submissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
}

/// Backend-neutral mailbox metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    pub role: MailboxRole,
    pub parent_id: Option<String>,
}

/// Backend-neutral principal metadata for admin-capable providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
}

/// Lightweight message metadata paired with raw RFC822 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMessage {
    pub id: BackendMsgId,
    pub thread_id: Option<String>,
    pub rfc822: Bytes,
    pub keywords: Vec<Keyword>,
    pub envelope: Option<Envelope>,
    pub received_at_epoch_secs: Option<i64>,
    pub size_bytes: Option<u64>,
    pub blob_refs: Vec<BlobRef>,
    pub attachments: Vec<AttachmentMeta>,
    pub metadata: BTreeMap<String, String>,
}

/// Backend-native change translated into hail's common sync shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    MessageCreated {
        id: BackendMsgId,
        raw_ref: Option<RawMessage>,
    },
    MessageUpdated {
        id: BackendMsgId,
        /// Current provider keyword state when the backend can cheaply
        /// hydrate it for an update notification. Cache sync should treat
        /// this as authoritative over the delta fields when present.
        keywords: Option<Vec<Keyword>>,
        keywords_added: Vec<Keyword>,
        keywords_removed: Vec<Keyword>,
    },
    MessageDeleted {
        id: BackendMsgId,
    },
    MailboxRoleChanged {
        id: BackendMsgId,
        role: MailboxRole,
    },
}

/// Narrow async seam implemented by concrete mail providers.
#[async_trait]
pub trait MailBackend: Send + Sync + 'static {
    fn capabilities(&self) -> &'static Capabilities;

    async fn list_message_ids(
        &self,
        query: &Query,
        page: &PageRequest,
    ) -> Result<Page<BackendMsgId>>;

    async fn get_message(&self, id: &BackendMsgId) -> Result<RawMessage>;

    async fn fetch_blob(&self, id: &BlobRef) -> Result<Bytes>;

    async fn set_keywords(
        &self,
        id: &BackendMsgId,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> Result<()>;

    async fn move_to_role(&self, id: &BackendMsgId, role: MailboxRole) -> Result<()>;

    async fn delete_permanently(&self, id: &BackendMsgId) -> Result<()>;

    async fn send(&self, rfc822: &[u8], envelope: &Envelope) -> Result<SubmissionId>;

    async fn poll_changes(&self, cursor: &SyncCursor) -> Result<(Vec<Change>, SyncCursor)>;

    async fn watch_changes(&self) -> BoxStream<'static, Change>;

    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>>;

    async fn list_principals(&self) -> Result<Vec<Principal>>;
}

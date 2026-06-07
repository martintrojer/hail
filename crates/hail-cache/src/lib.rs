//! Cache facade for backend-agnostic mail reads, mutations, and sends.
//!
//! This crate owns the seam that `hail-api` and `hail-worker` will use once the
//! read-through/write-through cache lands.  The current implementation is only
//! the compile-time skeleton: it wires together SQLite, a blob store, a selected
//! [`MailBackend`], and a cache policy, but intentionally performs no caching or
//! backend I/O yet.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use hail_backend::{
    BackendMsgId, BlobRef, Envelope, Keyword, MailBackend, MailboxRole, SubmissionId,
};
use hail_blob_store::BlobStore;
use hail_core::{MailBackfill, MailCacheConfig, MailCacheMode};
use serde::{Deserialize, Serialize};
pub use sqlx::SqlitePool;
use std::sync::Arc;

/// Crate-local result type for cache operations.
pub type Result<T> = std::result::Result<T, CacheError>;

/// Errors surfaced by [`CachedMail`].
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The facade method is reserved for a downstream cache implementation task.
    #[error("cache operation is not implemented yet: {operation}")]
    NotImplemented { operation: &'static str },

    /// SQLite access failed.
    #[error(transparent)]
    Db(#[from] sqlx::Error),

    /// Blob store access failed.
    #[error(transparent)]
    Blob(#[from] hail_blob_store::BlobStoreError),

    /// The selected upstream backend failed.
    #[error(transparent)]
    Backend(#[from] hail_backend::Error),
}

/// Cache mode reused from hail-core configuration.
pub type CacheMode = MailCacheMode;

/// Cache backfill policy reused from hail-core configuration.
pub type CacheBackfill = MailBackfill;

/// Cache policy used by [`CachedMail`].
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

/// Read-through/write-through mail facade.
pub struct CachedMail {
    db: SqlitePool,
    blobs: Arc<dyn BlobStore>,
    backend: Box<dyn MailBackend>,
    policy: CachePolicy,
}

impl CachedMail {
    /// Construct a cache facade from its persistent stores, selected backend,
    /// and cache policy.
    pub fn new(
        db: SqlitePool,
        blobs: Arc<dyn BlobStore>,
        backend: Box<dyn MailBackend>,
        policy: impl Into<CachePolicy>,
    ) -> Self {
        Self {
            db,
            blobs,
            backend,
            policy: policy.into(),
        }
    }

    /// Borrow the SQLite pool backing cache metadata and queues.
    #[must_use]
    pub const fn db(&self) -> &SqlitePool {
        &self.db
    }

    /// Borrow the configured blob store.
    #[must_use]
    pub fn blobs(&self) -> &dyn BlobStore {
        self.blobs.as_ref()
    }

    /// Borrow the selected upstream backend.
    #[must_use]
    pub fn backend(&self) -> &dyn MailBackend {
        self.backend.as_ref()
    }

    /// Borrow the active cache policy.
    #[must_use]
    pub const fn policy(&self) -> &CachePolicy {
        &self.policy
    }

    /// List a collapsed mail view, shaped like today's `MailViewProvider::list`.
    pub async fn list_view(
        &self,
        view: MailView,
        cursor: Option<String>,
        limit: usize,
        opts: MailViewListOpts,
    ) -> Result<MailViewPage> {
        let _ = (view, cursor, limit, opts);
        Err(CacheError::NotImplemented {
            operation: "list_view",
        })
    }

    /// Count a collapsed mail view, shaped like today's `MailViewProvider::count`.
    pub async fn count_view(&self, view: MailView, unread_only: bool) -> Result<usize> {
        let _ = (view, unread_only);
        Err(CacheError::NotImplemented {
            operation: "count_view",
        })
    }

    /// Fetch a thread/conversation for rendering.
    pub async fn get_thread(&self, thread_id: &str) -> Result<Thread> {
        let _ = thread_id;
        Err(CacheError::NotImplemented {
            operation: "get_thread",
        })
    }

    /// Fetch cached or backend metadata for one message.
    pub async fn get_message(&self, id: &BackendMsgId) -> Result<CachedMessage> {
        let _ = id;
        Err(CacheError::NotImplemented {
            operation: "get_message",
        })
    }

    /// Fetch the raw RFC822 body for one message.
    pub async fn get_message_body(&self, id: &BackendMsgId) -> Result<Bytes> {
        let _ = id;
        Err(CacheError::NotImplemented {
            operation: "get_message_body",
        })
    }

    /// Fetch an attachment/body blob by backend blob reference.
    pub async fn get_blob(&self, id: &BlobRef) -> Result<Bytes> {
        let _ = id;
        Err(CacheError::NotImplemented {
            operation: "get_blob",
        })
    }

    /// Search mail, shaped like today's `SearchProvider::search`.
    pub async fn search(
        &self,
        q: &str,
        mailbox: Option<SearchMailbox>,
        limit: usize,
    ) -> Result<Vec<MailSearchResult>> {
        let _ = (q, mailbox, limit);
        Err(CacheError::NotImplemented {
            operation: "search",
        })
    }

    /// Mutate backend/cache keywords on a message or thread target.
    pub async fn mutate_keywords(
        &self,
        target: MailTarget<'_>,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> Result<()> {
        let _ = (target, add, remove);
        Err(CacheError::NotImplemented {
            operation: "mutate_keywords",
        })
    }

    /// Move a message or thread target to a canonical mailbox role.
    pub async fn move_to_role(&self, target: MailTarget<'_>, role: MailboxRole) -> Result<()> {
        let _ = (target, role);
        Err(CacheError::NotImplemented {
            operation: "move_to_role",
        })
    }

    /// Enqueue or perform an outbound send through the write-through path.
    pub async fn send_enqueue(&self, rfc822: &[u8], envelope: &Envelope) -> Result<SubmissionId> {
        let _ = (rfc822, envelope);
        Err(CacheError::NotImplemented {
            operation: "send_enqueue",
        })
    }
}

/// Mail view selector mirrored from the current API route surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailView {
    Imbox,
    Feed,
    Papertrail,
    Drafts,
    Trash,
    Spam,
    Archive,
}

/// Lightweight list options mirrored from the current view provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailViewListOpts {
    pub feed_render: FeedRenderMode,
}

/// Feed render option placeholder for cache-owned view assembly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedRenderMode {
    #[default]
    WithoutRemoteImages,
    WithRemoteImages,
}

/// Page returned by [`CachedMail::list_view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailViewPage {
    pub items: Vec<MailViewItem>,
    pub next_cursor: Option<String>,
}

/// One collapsed row in a mail view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailViewItem {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub preview: String,
    pub received_at: Option<DateTime<Utc>>,
    pub unread: bool,
    pub message_count: usize,
    pub unread_count: usize,
    pub classification: MailView,
    pub labels: Vec<CachedLabel>,
    pub feed_html: Option<String>,
    pub feed_html_with_images: Option<String>,
    pub feed_blocked_trackers: Option<Vec<BlockedTracker>>,
    pub feed_blocked_images: Option<usize>,
}

/// Search mailbox selector mirrored from the current API route surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMailbox {
    Imbox,
    Feed,
    Papertrail,
    Archive,
    Trash,
    Drafts,
}

/// One mail search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailSearchResult {
    pub thread_id: String,
    pub email_id: String,
    pub from: String,
    pub subject: String,
    pub preview: String,
    pub message_count: usize,
    pub unread_count: usize,
    pub unread: bool,
    pub received_at: Option<DateTime<Utc>>,
    pub labels: Vec<CachedLabel>,
}

/// Render-ready thread placeholder returned by [`CachedMail::get_thread`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub thread_id: String,
    pub messages: Vec<CachedMessage>,
}

/// Cache-shaped message metadata placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedMessage {
    pub id: BackendMsgId,
    pub thread_id: Option<String>,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub preview: String,
    pub received_at: Option<DateTime<Utc>>,
    pub unread: bool,
    pub keywords: Vec<Keyword>,
    pub size_bytes: Option<u64>,
    pub blob_refs: Vec<BlobRef>,
}

/// A label attached to a cached mail row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedLabel {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
}

/// Tracker removal metadata for rendered feed cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedTracker {
    pub src: String,
    pub reason: String,
}

/// Mutation target for message- or thread-scoped operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailTarget<'a> {
    Message(&'a BackendMsgId),
    Thread(&'a str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use hail_backend::{
        BlobRef, Capabilities, Change, Mailbox, Page, PageRequest, Principal, Query, RawMessage,
        SyncCursor,
    };
    use hail_blob_store::FilesystemBlobStore;

    static CAPABILITIES: Capabilities = Capabilities {
        supports_initial_import: false,
        supports_eventsource: false,
        supports_principals_admin: false,
        supports_send: true,
        native_threading: false,
        max_attachment_size: 0,
        label_path_separator: '/',
    };

    struct StubBackend;

    #[async_trait::async_trait]
    impl MailBackend for StubBackend {
        fn capabilities(&self) -> &'static Capabilities {
            &CAPABILITIES
        }

        async fn list_message_ids(
            &self,
            _query: &Query,
            _page: &PageRequest,
        ) -> hail_backend::Result<Page<BackendMsgId>> {
            Ok(Page::empty())
        }

        async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
            Ok(RawMessage {
                id: id.clone(),
                thread_id: None,
                rfc822: Bytes::new(),
                keywords: Vec::new(),
                envelope: None,
                received_at_epoch_secs: None,
                size_bytes: None,
                blob_refs: Vec::new(),
                metadata: Default::default(),
            })
        }

        async fn fetch_blob(&self, _id: &BlobRef) -> hail_backend::Result<Bytes> {
            Ok(Bytes::new())
        }

        async fn set_keywords(
            &self,
            _id: &BackendMsgId,
            _add: &[Keyword],
            _remove: &[Keyword],
        ) -> hail_backend::Result<()> {
            Ok(())
        }

        async fn move_to_role(
            &self,
            _id: &BackendMsgId,
            _role: MailboxRole,
        ) -> hail_backend::Result<()> {
            Ok(())
        }

        async fn delete_permanently(&self, _id: &BackendMsgId) -> hail_backend::Result<()> {
            Ok(())
        }

        async fn send(
            &self,
            _rfc822: &[u8],
            _envelope: &Envelope,
        ) -> hail_backend::Result<SubmissionId> {
            Ok(SubmissionId::new("stub-submission"))
        }

        async fn poll_changes(
            &self,
            cursor: &SyncCursor,
        ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
            Ok((Vec::new(), cursor.clone()))
        }

        async fn watch_changes(&self) -> futures_util::stream::BoxStream<'static, Change> {
            Box::pin(stream::empty())
        }

        async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
            Ok(Vec::new())
        }

        async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn constructs_cached_mail_with_sqlite_blob_store_and_backend() {
        let pool = hail_db::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        hail_db::migrate(&pool).await.expect("run migrations");
        let tempdir = tempfile::tempdir().expect("create temp blob dir");
        let blobs = Arc::new(FilesystemBlobStore::new(tempdir.path()));
        let backend = Box::new(StubBackend);
        let policy = CachePolicy::new(
            MailCacheMode::Bounded,
            90,
            50_000,
            5 * 1024 * 1024,
            MailBackfill::Off,
        );

        let cache = CachedMail::new(pool.clone(), blobs, backend, policy.clone());

        assert_eq!(cache.policy(), &policy);
        assert_eq!(cache.backend().capabilities(), &CAPABILITIES);
        let _: &SqlitePool = cache.db();
        let _: &dyn BlobStore = cache.blobs();
        let err = cache
            .list_view(MailView::Imbox, None, 50, MailViewListOpts::default())
            .await
            .expect_err("skeleton should not implement list_view yet");
        assert!(matches!(
            err,
            CacheError::NotImplemented {
                operation: "list_view"
            }
        ));
    }
}

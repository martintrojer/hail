//! Cache facade for backend-agnostic mail reads, mutations, and sends.
//!
//! This crate owns the seam that `hail-api` and `hail-worker` use for
//! read-through/write-through mail access.  Metadata read-through lives in
//! `readthrough`; downstream body/blob, write-through, sync, and eviction work
//! should add sibling modules instead of growing this facade.

mod bodies;
mod error;
mod policy;
mod readthrough;
mod search;
mod sync;
mod types;
mod writethrough;

use bytes::Bytes;
use hail_backend::{
    BackendMsgId, BlobRef, Envelope, Keyword, MailBackend, MailboxRole, SubmissionId,
};
use hail_blob_store::BlobStore;
pub use policy::{CacheBackfill, CacheMode, CachePolicy};
pub use sqlx::SqlitePool;
use std::sync::Arc;
pub use types::{
    BlockedTracker, CachedLabel, CachedMessage, FeedRenderMode, MailSearchResult, MailTarget,
    MailView, MailViewItem, MailViewListOpts, MailViewPage, SearchMailbox, SearchResultSource,
    Thread,
};

pub use error::CacheError;

const DEFAULT_ACCOUNT_ID: i64 = 1;

/// Crate-local result type for cache operations.
pub type Result<T> = std::result::Result<T, CacheError>;

/// Read-through/write-through mail facade.
pub struct CachedMail {
    db: SqlitePool,
    blobs: Arc<dyn BlobStore>,
    backend: Box<dyn MailBackend>,
    policy: CachePolicy,
    account_id: i64,
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
        Self::with_account_id(db, blobs, backend, policy, DEFAULT_ACCOUNT_ID)
    }

    /// Construct a cache facade for a specific mail account row.
    pub fn with_account_id(
        db: SqlitePool,
        blobs: Arc<dyn BlobStore>,
        backend: Box<dyn MailBackend>,
        policy: impl Into<CachePolicy>,
        account_id: i64,
    ) -> Self {
        Self {
            db,
            blobs,
            backend,
            policy: policy.into(),
            account_id,
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

    /// Mail account id used in cache table keys.
    #[must_use]
    pub const fn account_id(&self) -> i64 {
        self.account_id
    }

    /// Fetch a thread/conversation for rendering.
    pub async fn get_thread(&self, thread_id: &str) -> Result<Thread> {
        self.get_thread_readthrough(thread_id).await
    }

    /// Fetch the raw RFC822 body for one message.
    pub async fn get_message_body(&self, id: &BackendMsgId) -> Result<Bytes> {
        self.get_message_body_readthrough(id).await
    }

    /// Fetch an attachment/body blob by backend blob reference.
    pub async fn get_blob(&self, id: &BlobRef) -> Result<Bytes> {
        self.get_blob_readthrough(id).await
    }

    /// Search mail, shaped like today's `SearchProvider::search`.
    pub async fn search(
        &self,
        q: &str,
        mailbox: Option<SearchMailbox>,
        limit: usize,
    ) -> Result<Vec<MailSearchResult>> {
        self.search_cached_mail(q, mailbox, limit).await
    }

    /// Mutate backend/cache keywords on a message or thread target.
    pub async fn mutate_keywords(
        &self,
        target: MailTarget<'_>,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> Result<()> {
        writethrough::mutate_keywords(self, target, add, remove).await
    }

    /// Move a message or thread target to a canonical mailbox role.
    pub async fn move_to_role(&self, target: MailTarget<'_>, role: MailboxRole) -> Result<()> {
        writethrough::move_to_role(self, target, role).await
    }

    /// Permanently delete a message or every cached message in a thread target.
    pub async fn delete_permanently(&self, target: MailTarget<'_>) -> Result<()> {
        let ids = match target {
            MailTarget::Message(id) => vec![id.clone()],
            MailTarget::Thread(thread_id) => {
                let thread = self.get_thread(thread_id).await?;
                thread
                    .messages
                    .into_iter()
                    .map(|message| message.id)
                    .collect()
            }
        };
        for id in ids {
            self.backend.delete_permanently(&id).await?;
        }
        Ok(())
    }

    /// Count queued outbound mutations that have not yet been applied upstream.
    pub async fn pending_sync_count(&self) -> Result<i64> {
        writethrough::pending_sync_count(self).await
    }

    /// Enqueue or perform an outbound send through the write-through path.
    pub async fn send_enqueue(&self, rfc822: &[u8], envelope: &Envelope) -> Result<SubmissionId> {
        let _ = (rfc822, envelope);
        Err(CacheError::NotImplemented {
            operation: "send_enqueue",
        })
    }
}

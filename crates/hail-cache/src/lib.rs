//! Cache facade for backend-agnostic mail reads, mutations, and sends.
//!
//! This crate owns the seam that `hail-api` and `hail-worker` use for
//! read-through/write-through mail access.  Metadata read-through lives in
//! `readthrough`; downstream body/blob, write-through, sync, and eviction work
//! should add sibling modules instead of growing this facade.

mod api_facade;
mod bodies;
mod error;
mod eviction;
mod policy;
mod readthrough;
mod search;
mod sync;
mod types;
mod writethrough;

use bytes::Bytes;
pub use eviction::{
    EvictionStats, evict_account_bodies, load_account_policies, refresh_pinned_messages,
    refresh_pinned_messages_conn,
};
use hail_backend::{
    BackendMsgId, BlobRef, Envelope, Keyword, MailBackend, MailboxRole, SubmissionId,
};
use hail_blob_store::BlobStore;
pub use policy::{CacheBackfill, CacheMode, CachePolicy};
pub use sqlx::SqlitePool;
use std::sync::Arc;
pub use types::{
    BlockedTracker, CachedAttachment, CachedAttachmentContext, CachedLabel, CachedMessage,
    CachedMessageBody, ComposeSubmission, DraftMessage, DraftPayload, FeedRenderMode,
    MailSearchResult, MailTarget, MailView, MailViewItem, MailViewListOpts, MailViewPage,
    MailboxSnapshot, OutboundPayload, ReplyContext, ScreenerDecision, ScreenerMessage,
    ScreenerSenderPreview, SearchMailbox, SearchResultSource, Thread,
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
        self.backend.send(rfc822, envelope).await.map_err(Into::into)
    }

    /// Create a backend-visible draft placeholder for scheduling or immediate send.
    pub async fn create_draft(&self, draft: DraftPayload) -> Result<BackendMsgId> {
        self.create_draft_cached(draft).await
    }

    /// Read one draft message for composer resume.
    pub async fn get_draft(&self, id: &BackendMsgId) -> Result<Option<DraftMessage>> {
        self.get_draft_cached(id).await
    }

    /// Update a cached/backend draft by replacing its stored body.
    pub async fn update_draft(&self, id: &BackendMsgId, draft: DraftPayload) -> Result<()> {
        self.update_draft_cached(id, draft).await
    }

    /// Delete a draft by provider id.
    pub async fn delete_draft(&self, id: &BackendMsgId) -> Result<()> {
        self.delete_permanently(MailTarget::Message(id)).await
    }

    /// Submit a composed outbound payload.
    pub async fn submit_composed(&self, payload: OutboundPayload) -> Result<ComposeSubmission> {
        self.submit_composed_cached(payload).await
    }

    /// Build reply metadata from cached/backend thread data.
    pub async fn reply_context(&self, thread_id: &str) -> Result<Option<ReplyContext>> {
        self.reply_context_cached(thread_id).await
    }

    /// List attachment metadata with message context.
    pub async fn list_attachments(&self, limit: usize) -> Result<Vec<CachedAttachment>> {
        self.list_attachments_cached(limit).await
    }

    /// Upload a local blob through the cache blob store.
    pub async fn upload_blob(&self, bytes: &[u8]) -> Result<hail_core::BlobId> {
        self.blobs.put(hail_core::BlobKind::Att, bytes).await.map_err(Into::into)
    }

    /// Fetch message metadata and RFC822 body together.
    pub async fn get_message_with_body(&self, id: &BackendMsgId) -> Result<CachedMessageBody> {
        let message = self.get_message(id).await?;
        let rfc822 = self.get_message_body(id).await?;
        Ok(CachedMessageBody { message, rfc822 })
    }

    /// Enrich pending screener senders from backend/cache without exposing provider APIs to routes.
    pub async fn screener_previews(
        &self,
        senders: &[String],
        limit_per_sender: usize,
    ) -> Result<Vec<ScreenerSenderPreview>> {
        self.screener_previews_cached(senders, limit_per_sender).await
    }

    /// Apply a screener decision to historical messages from the sender.
    pub async fn apply_screener_backfill(
        &self,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Keyword>,
    ) -> Result<()> {
        self.apply_screener_backfill_cached(sender, decision, classify_as).await
    }

    /// Undo a denied screener decision by routing sender history back to inbox/classification.
    pub async fn undo_screener_deny(&self, sender: &str, classify_as: Keyword) -> Result<()> {
        self.undo_screener_deny_cached(sender, classify_as).await
    }

    /// Restore one thread classification through cache keyword mutations.
    pub async fn restore_thread_classification(
        &self,
        thread_id: &str,
        classification: Keyword,
        stale: &[Keyword],
    ) -> Result<()> {
        self.mutate_keywords(MailTarget::Thread(thread_id), &[classification], stale)
            .await
    }

    /// Restore thread mailboxes when only role-level cache semantics are available.
    pub async fn restore_mailboxes(&self, snapshots: &[MailboxSnapshot]) -> Result<()> {
        self.restore_mailboxes_cached(snapshots).await
    }
}


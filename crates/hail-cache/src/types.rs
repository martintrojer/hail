//! Public data shapes returned by the cache facade.

use chrono::{DateTime, Utc};
use hail_backend::{AttachmentMeta, BackendMsgId, BlobRef, Envelope, Keyword, SubmissionId};
use serde::{Deserialize, Serialize};

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

/// Page returned by `CachedMail::list_view`.
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
    pub source: SearchResultSource,
}

/// Where a search result was served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchResultSource {
    Local,
    Backend,
}

/// Render-ready thread placeholder returned by `CachedMail::get_thread`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub thread_id: String,
    pub messages: Vec<CachedMessage>,
}

/// Cache-shaped message metadata.
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


/// Full cached/backfilled message body plus metadata for rendering surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedMessageBody {
    pub message: CachedMessage,
    pub rfc822: bytes::Bytes,
}

/// Attachment metadata paired with message context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAttachment {
    pub blob_ref: BlobRef,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub inline: bool,
    pub content_id: Option<String>,
    pub context: CachedAttachmentContext,
}

/// Message context for an attachment list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAttachmentContext {
    pub thread_id: String,
    pub message_id: BackendMsgId,
    pub subject: String,
    pub from: String,
    pub received_at: Option<DateTime<Utc>>,
    pub preview: String,
}

/// Draft body and envelope details used by the API composer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftMessage {
    pub id: BackendMsgId,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_html: String,
    pub body_markdown: String,
}

/// Payload for creating or updating a draft through the cache facade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftPayload {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub plain_text: String,
    pub html: String,
    pub body_markdown: String,
}

/// Reply metadata extracted from the latest message in a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyContext {
    pub to: Vec<String>,
    pub subject: String,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
}

/// Sent/draft message submission payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPayload {
    pub rfc822: Vec<u8>,
    pub envelope: Envelope,
}

/// Result from a cache-mediated compose send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeSubmission {
    pub message_id: BackendMsgId,
    pub submission_id: SubmissionId,
}

/// Screener decision for applying history backfill through CachedMail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenerDecision {
    Approve,
    Deny,
}

/// CachedMail-owned screener preview row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenerMessage {
    pub email_id: BackendMsgId,
    pub subject: String,
    pub preview: String,
    pub from: String,
    pub received_at: Option<DateTime<Utc>>,
}

/// CachedMail-owned screener sender preview aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenerSenderPreview {
    pub sender: String,
    pub message_count: usize,
    pub emails: Vec<ScreenerMessage>,
}

/// Preserve or restore prior provider mailbox ids when available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxSnapshot {
    pub message_id: BackendMsgId,
    pub mailbox_ids: Vec<String>,
}

/// Attachment metadata from backend RawMessage translated into cache DTOs.
impl CachedAttachment {
    #[must_use]
    pub fn from_meta(meta: AttachmentMeta, context: CachedAttachmentContext) -> Option<Self> {
        Some(Self {
            blob_ref: meta.blob_ref?,
            filename: meta.filename,
            mime_type: meta.mime_type,
            size_bytes: meta.size_bytes,
            inline: meta.inline,
            content_id: meta.content_id,
            context,
        })
    }
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

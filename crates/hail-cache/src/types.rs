//! Public data shapes returned by the cache facade.

use chrono::{DateTime, Utc};
use hail_backend::{BackendMsgId, BlobRef, Keyword};
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

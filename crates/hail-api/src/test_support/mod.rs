//! Test helpers for constructing AppState with a fake mail backend.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend, Mailbox,
    MailboxRole, Page, PageRequest, Principal, Query, RawMessage, SubmissionId, SyncCursor,
};
use hail_blob_store::FilesystemBlobStore;
use hail_cache::{CachePolicy, CachedMail};
use hail_core::MailBackfill;

static CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: false,
    max_attachment_size: 0,
    label_path_separator: '/',
};

#[derive(Clone, Default)]
pub struct FakeMailBackend {
    messages: Arc<HashMap<BackendMsgId, RawMessage>>,
    blobs: Arc<HashMap<BlobRef, Bytes>>,
    order: Arc<Vec<BackendMsgId>>,
}

impl FakeMailBackend {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MailBackend for FakeMailBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        _query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
        Ok(Page {
            items: self.order.iter().take(limit).cloned().collect(),
            next_cursor: None,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        self.messages
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
    }

    async fn fetch_blob(&self, id: &BlobRef) -> hail_backend::Result<Bytes> {
        self.blobs
            .get(id)
            .cloned()
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "blob",
                id: id.as_str().to_owned(),
            })
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
        Ok(SubmissionId::new("fake-submission"))
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(&self) -> futures_core::stream::BoxStream<'static, Change> {
        Box::pin(stream::empty())
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Ok(Vec::new())
    }
}

#[must_use]
pub fn fake_cached_mail(db: sqlx::SqlitePool) -> Arc<CachedMail> {
    Arc::new(CachedMail::new(
        db,
        Arc::new(FilesystemBlobStore::new(
            std::env::temp_dir().join("hail-api-test-blobs"),
        )),
        Box::new(FakeMailBackend::empty()),
        CachePolicy::new(hail_core::MailCacheMode::Off, 0, 0, 0, MailBackfill::Off),
    ))
}

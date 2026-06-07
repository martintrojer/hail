use async_trait::async_trait;
use bytes::Bytes;
use futures_core::stream::BoxStream;
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, MailBackend, Mailbox, Page,
    PageRequest, Principal, Query, RawMessage, Result, SubmissionId, SyncCursor,
};

static CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: false,
    native_threading: false,
    max_attachment_size: 0,
    label_path_separator: '/',
};

struct NoopBackend;

#[async_trait]
impl MailBackend for NoopBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        _query: &Query,
        _page: &PageRequest,
    ) -> Result<Page<BackendMsgId>> {
        Ok(Page::empty())
    }

    async fn get_message(&self, id: &BackendMsgId) -> Result<RawMessage> {
        Err(hail_backend::Error::NotFound {
            kind: "message",
            id: id.as_str().to_owned(),
        })
    }

    async fn fetch_blob(&self, id: &BlobRef) -> Result<Bytes> {
        Err(hail_backend::Error::NotFound {
            kind: "blob",
            id: id.as_str().to_owned(),
        })
    }

    async fn set_keywords(
        &self,
        _id: &BackendMsgId,
        _add: &[hail_backend::Keyword],
        _remove: &[hail_backend::Keyword],
    ) -> Result<()> {
        Ok(())
    }

    async fn move_to_role(
        &self,
        _id: &BackendMsgId,
        _role: hail_backend::MailboxRole,
    ) -> Result<()> {
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> Result<()> {
        Ok(())
    }

    async fn send(&self, _rfc822: &[u8], _envelope: &Envelope) -> Result<SubmissionId> {
        Err(hail_backend::Error::UnsupportedCapability { capability: "send" })
    }

    async fn poll_changes(&self, cursor: &SyncCursor) -> Result<(Vec<Change>, SyncCursor)> {
        Ok((Vec::new(), cursor.clone()))
    }

    async fn watch_changes(&self) -> BoxStream<'static, Change> {
        Box::pin(futures_util::stream::empty())
    }

    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
        Ok(Vec::new())
    }

    async fn list_principals(&self) -> Result<Vec<Principal>> {
        Ok(Vec::new())
    }
}

async fn run_conformance_smoke<B: MailBackend>(backend: &B) {
    assert!(!backend.capabilities().supports_send);

    let page = backend
        .list_message_ids(&Query::all(), &PageRequest::first(10))
        .await
        .expect("list_message_ids should succeed");
    assert!(page.items.is_empty());

    let cursor = SyncCursor::new("initial");
    let (changes, next_cursor) = backend
        .poll_changes(&cursor)
        .await
        .expect("poll_changes should succeed");
    assert!(changes.is_empty());
    assert_eq!(next_cursor, cursor);
}

#[tokio::test]
async fn noop_fixture_satisfies_conformance_scaffold() {
    run_conformance_smoke(&NoopBackend).await;
}

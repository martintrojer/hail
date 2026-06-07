use async_trait::async_trait;
use bytes::Bytes;
use futures_core::stream::BoxStream;
use hail_backend::conformance::{
    MailBackendConformance, PrincipalExpectation, run_mail_backend_conformance,
};
use hail_backend::{
    AttachmentMeta, BackendMsgId, BlobRef, Capabilities, Change, Envelope, Error, Keyword,
    MailBackend, Mailbox, MailboxRole, Page, PageRequest, Principal, Query, RawMessage, Result,
    SubmissionId, SyncCursor,
};
use std::collections::BTreeMap;

static CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: false,
    native_threading: false,
    max_attachment_size: 0,
    label_path_separator: '/',
};

#[derive(Clone)]
struct InMemoryBackend {
    fixture: MailBackendConformance,
}

impl InMemoryBackend {
    fn new(fixture: MailBackendConformance) -> Self {
        Self { fixture }
    }
}

#[async_trait]
impl MailBackend for InMemoryBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        _query: &Query,
        _page: &PageRequest,
    ) -> Result<Page<BackendMsgId>> {
        Ok(Page {
            items: self.fixture.listed_message_ids.clone(),
            next_cursor: self.fixture.listed_next_cursor.clone(),
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> Result<RawMessage> {
        if id == &self.fixture.message_id {
            Ok(self.fixture.expected_message.clone())
        } else {
            Err(Error::NotFound {
                kind: "message",
                id: id.as_str().to_owned(),
            })
        }
    }

    async fn fetch_blob(&self, id: &BlobRef) -> Result<Bytes> {
        if id == &self.fixture.blob_ref {
            Ok(self.fixture.expected_blob.clone())
        } else {
            Err(Error::NotFound {
                kind: "blob",
                id: id.as_str().to_owned(),
            })
        }
    }

    async fn set_keywords(
        &self,
        _id: &BackendMsgId,
        _add: &[Keyword],
        _remove: &[Keyword],
    ) -> Result<()> {
        Ok(())
    }

    async fn move_to_role(&self, _id: &BackendMsgId, _role: MailboxRole) -> Result<()> {
        Ok(())
    }

    async fn delete_permanently(&self, _id: &BackendMsgId) -> Result<()> {
        Ok(())
    }

    async fn send(&self, _rfc822: &[u8], _envelope: &Envelope) -> Result<SubmissionId> {
        Err(Error::UnsupportedCapability { capability: "send" })
    }

    async fn poll_changes(&self, _cursor: &SyncCursor) -> Result<(Vec<Change>, SyncCursor)> {
        Ok((
            self.fixture.expected_changes.clone(),
            self.fixture.expected_next_cursor.clone(),
        ))
    }

    async fn watch_changes(&self) -> BoxStream<'static, Change> {
        Box::pin(futures_util::stream::empty())
    }

    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
        Ok(self.fixture.expected_mailboxes.clone())
    }

    async fn list_principals(&self) -> Result<Vec<Principal>> {
        Err(Error::UnsupportedCapability {
            capability: "principals_admin",
        })
    }
}

fn fixture() -> MailBackendConformance {
    let message_id = BackendMsgId::new("fixture-message-1");
    let blob_ref = BlobRef::new("fixture-message-1:blob-1");
    let rfc822 = Bytes::from_static(
        b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: fixture\r\n\r\nHello",
    );
    MailBackendConformance {
        expected_capabilities: CAPABILITIES,
        listed_message_ids: vec![message_id.clone()],
        listed_next_cursor: Some("next-page".to_string()),
        message_id: message_id.clone(),
        expected_message: RawMessage {
            id: message_id.clone(),
            thread_id: Some("thread-1".to_string()),
            rfc822,
            keywords: vec![Keyword::new("INBOX")],
            envelope: Some(Envelope {
                mail_from: "sender@example.org".to_string(),
                rcpt_to: vec!["user@example.org".to_string()],
            }),
            received_at_epoch_secs: Some(1_700_000_000),
            size_bytes: Some(64),
            blob_refs: vec![blob_ref.clone()],
            attachments: vec![AttachmentMeta {
                filename: "fixture.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 11,
                blob_ref: Some(blob_ref.clone()),
                inline: false,
                content_id: None,
            }],
            metadata: BTreeMap::from([("source".to_string(), "in-memory".to_string())]),
        },
        blob_ref,
        expected_blob: Bytes::from_static(b"blob bytes"),
        keyword_additions: vec![Keyword::new("$seen")],
        keyword_removals: vec![Keyword::new("$flagged")],
        move_role: MailboxRole::Trash,
        send_rfc822:
            b"From: sender@example.org\r\nTo: user@example.org\r\nSubject: send\r\n\r\nBody"
                .to_vec(),
        send_envelope: Envelope {
            mail_from: "sender@example.org".to_string(),
            rcpt_to: vec!["user@example.org".to_string()],
        },
        expected_submission_id: SubmissionId::new("unsupported"),
        poll_cursor: SyncCursor::new("cursor-1"),
        expected_changes: vec![Change::MessageUpdated {
            id: message_id,
            keywords: None,
            keywords_added: vec![Keyword::new("INBOX")],
            keywords_removed: Vec::new(),
        }],
        expected_next_cursor: SyncCursor::new("cursor-2"),
        expected_mailboxes: vec![Mailbox {
            id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            role: MailboxRole::Inbox,
            parent_id: None,
        }],
        principals: PrincipalExpectation::Unsupported,
    }
}

#[tokio::test]
async fn in_memory_fixture_satisfies_shared_conformance() {
    let fixture = fixture();
    let backend = InMemoryBackend::new(fixture.clone());
    run_mail_backend_conformance(&backend, &fixture).await;
}

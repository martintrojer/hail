//! Shared MailBackend conformance harness.
//!
//! Concrete backends can call [`run_mail_backend_conformance`] from their own
//! integration tests with a recorded or in-memory fixture. The harness is kept
//! in this crate so every provider implementation exercises the same trait
//! contract at the cache/backend seam.

use crate::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Error, Keyword, MailBackend, Mailbox,
    MailboxRole, PageRequest, Principal, RawMessage, SubmissionId, SyncCursor,
};

/// Expected shape for the optional principal-management operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalExpectation {
    /// The backend advertises principal administration and should return these
    /// principals.
    Supported(Vec<Principal>),
    /// The backend does not support principal administration and should return
    /// [`Error::UnsupportedCapability`].
    Unsupported,
}

/// Fixture data and expectations for the shared MailBackend conformance suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailBackendConformance {
    pub expected_capabilities: Capabilities,
    pub listed_message_ids: Vec<BackendMsgId>,
    pub listed_next_cursor: Option<String>,
    pub message_id: BackendMsgId,
    pub expected_message: RawMessage,
    pub blob_ref: BlobRef,
    pub expected_blob: bytes::Bytes,
    pub keyword_additions: Vec<Keyword>,
    pub keyword_removals: Vec<Keyword>,
    pub move_role: MailboxRole,
    pub send_rfc822: Vec<u8>,
    pub send_envelope: Envelope,
    pub expected_submission_id: SubmissionId,
    pub poll_cursor: SyncCursor,
    pub expected_changes: Vec<Change>,
    pub expected_next_cursor: SyncCursor,
    pub expected_mailboxes: Vec<Mailbox>,
    pub principals: PrincipalExpectation,
}

/// Run the shared conformance suite against a backend instance.
///
/// The suite intentionally calls every [`MailBackend`] method once with fixture
/// data. Provider-specific tests should use a fake/recorded transport and then
/// assert their native requests if they need stronger protocol-shape checks.
pub async fn run_mail_backend_conformance<B>(backend: &B, fixture: &MailBackendConformance)
where
    B: MailBackend,
{
    assert_eq!(backend.capabilities(), &fixture.expected_capabilities);

    let page = backend
        .list_message_ids(&crate::Query::all(), &PageRequest::first(10))
        .await
        .expect("list_message_ids should succeed");
    assert_eq!(page.items, fixture.listed_message_ids);
    assert_eq!(page.next_cursor, fixture.listed_next_cursor);

    let message = backend
        .get_message(&fixture.message_id)
        .await
        .expect("get_message should succeed");
    let expected_keywords = fixture
        .expected_message
        .keywords
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let actual_keywords = message
        .keywords
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut expected_message = fixture.expected_message.clone();
    let mut actual_message = message;
    expected_message.keywords.clear();
    actual_message.keywords.clear();
    assert_eq!(actual_message, expected_message);
    assert_eq!(actual_keywords, expected_keywords);

    let blob = backend
        .fetch_blob(&fixture.blob_ref)
        .await
        .expect("fetch_blob should succeed");
    assert_eq!(blob, fixture.expected_blob);

    backend
        .set_keywords(
            &fixture.message_id,
            &fixture.keyword_additions,
            &fixture.keyword_removals,
        )
        .await
        .expect("set_keywords should succeed");

    backend
        .move_to_role(&fixture.message_id, fixture.move_role)
        .await
        .expect("move_to_role should succeed");

    if fixture.expected_capabilities.supports_send {
        let submission = backend
            .send(&fixture.send_rfc822, &fixture.send_envelope)
            .await
            .expect("send should succeed when supports_send is true");
        assert_eq!(submission, fixture.expected_submission_id);
    } else {
        let error = backend
            .send(&fixture.send_rfc822, &fixture.send_envelope)
            .await
            .expect_err("send should fail when supports_send is false");
        assert!(matches!(error, Error::UnsupportedCapability { .. }));
    }

    let (changes, next_cursor) = backend
        .poll_changes(&fixture.poll_cursor)
        .await
        .expect("poll_changes should succeed");
    assert_eq!(
        normalized_changes(changes),
        normalized_changes(fixture.expected_changes.clone())
    );
    assert_eq!(next_cursor, fixture.expected_next_cursor);

    let mailboxes = backend
        .list_mailboxes()
        .await
        .expect("list_mailboxes should succeed");
    assert_eq!(mailboxes, fixture.expected_mailboxes);

    match &fixture.principals {
        PrincipalExpectation::Supported(expected) => {
            let principals = backend
                .list_principals()
                .await
                .expect("list_principals should succeed when supported");
            assert_eq!(&principals, expected);
        }
        PrincipalExpectation::Unsupported => {
            let error = backend
                .list_principals()
                .await
                .expect_err("list_principals should fail when unsupported");
            assert!(matches!(error, Error::UnsupportedCapability { .. }));
        }
    }

    // Shape check: the method must return a stream that can be constructed and
    // dropped without requiring a live network subscription.
    let changes_stream = backend.watch_changes().await;
    drop(changes_stream);

    backend
        .delete_permanently(&fixture.message_id)
        .await
        .expect("delete_permanently should succeed");
}

fn normalized_changes(changes: Vec<Change>) -> Vec<Change> {
    changes
        .into_iter()
        .map(|change| match change {
            Change::MessageUpdated {
                id,
                mut keywords,
                mut keywords_added,
                mut keywords_removed,
            } => {
                if let Some(keywords) = keywords.as_mut() {
                    keywords.sort();
                }
                keywords_added.sort();
                keywords_removed.sort();
                Change::MessageUpdated {
                    id,
                    keywords,
                    keywords_added,
                    keywords_removed,
                }
            }
            other => other,
        })
        .collect()
}

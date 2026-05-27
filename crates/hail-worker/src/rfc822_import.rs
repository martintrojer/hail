//! Stalwart RFC822 import primitive for provider import mode.
//!
//! This boundary creates local Stalwart mail objects from provider-supplied raw
//! RFC822. Production uses standard JMAP `Blob/upload` + `Email/import` through
//! `jmap-client::Client::email_import_account`. Callers still own durable
//! provider idempotency in `provider_message_mappings`; this module returns the
//! stable JMAP ids needed for that mapping and provides a fake importer for
//! scheduler/dedupe tests.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use hail_jmap::jmap_client::email::{Property as EmailProperty, query as email_query};
use hail_jmap::jmap_client::mailbox::{Role, query::Filter as MailboxFilter};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rfc822ImportRequest {
    pub raw_rfc822: Vec<u8>,
    pub mailbox_ids: Vec<String>,
    pub keywords: Vec<String>,
    pub received_at: Option<i64>,
    pub provider_message_id: Option<String>,
}

impl Rfc822ImportRequest {
    #[must_use]
    pub fn into_mailbox(raw_rfc822: Vec<u8>, mailbox_id: impl Into<String>) -> Self {
        Self {
            raw_rfc822,
            mailbox_ids: vec![mailbox_id.into()],
            keywords: Vec::new(),
            received_at: None,
            provider_message_id: None,
        }
    }

    #[must_use]
    pub fn with_provider_message_id(mut self, provider_message_id: impl Into<String>) -> Self {
        self.provider_message_id = Some(provider_message_id.into());
        self
    }

    #[must_use]
    pub fn with_keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn with_received_at(mut self, received_at: i64) -> Self {
        self.received_at = Some(received_at);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedRfc822Message {
    pub jmap_email_id: String,
    pub jmap_thread_id: Option<String>,
    pub jmap_mailbox_ids: Vec<String>,
    pub rfc822_message_ids: Vec<String>,
    pub duplicate: bool,
}

#[async_trait]
pub trait Rfc822Importer: Send + Sync {
    async fn import_rfc822(
        &self,
        request: Rfc822ImportRequest,
    ) -> Result<ImportedRfc822Message, Rfc822ImportError>;
}

pub struct StalwartJmapRfc822Importer {
    session: hail_jmap::Session,
}

impl StalwartJmapRfc822Importer {
    #[must_use]
    pub fn new(session: hail_jmap::Session) -> Self {
        Self { session }
    }

    pub async fn inbox_request(
        &self,
        raw_rfc822: Vec<u8>,
    ) -> Result<Rfc822ImportRequest, Rfc822ImportError> {
        Ok(Rfc822ImportRequest::into_mailbox(
            raw_rfc822,
            self.inbox_id().await?,
        ))
    }

    pub async fn inbox_id(&self) -> Result<String, Rfc822ImportError> {
        let mut query = self
            .session
            .client()
            .mailbox_query(Some(MailboxFilter::role(Role::Inbox)), None::<Vec<_>>)
            .await
            .map_err(jmap_error)?;
        query
            .take_ids()
            .into_iter()
            .next()
            .ok_or(Rfc822ImportError::MissingInbox)
    }

    async fn lookup_existing_by_message_id(
        &self,
        message_id: &str,
    ) -> Result<Option<ImportedRfc822Message>, Rfc822ImportError> {
        let mut query = self
            .session
            .client()
            .email_query(
                Some(email_query::Filter::header("Message-ID", Some(message_id))),
                None::<Vec<_>>,
            )
            .await
            .map_err(jmap_error)?;
        let Some(email_id) = query.take_ids().into_iter().next() else {
            return Ok(None);
        };
        self.hydrate_imported_email(&email_id, true).await.map(Some)
    }

    async fn hydrate_imported_email(
        &self,
        email_id: &str,
        duplicate: bool,
    ) -> Result<ImportedRfc822Message, Rfc822ImportError> {
        let properties = [
            EmailProperty::Id,
            EmailProperty::ThreadId,
            EmailProperty::MailboxIds,
            EmailProperty::MessageId,
        ];
        let email = self
            .session
            .client()
            .email_get(email_id, Some(properties))
            .await
            .map_err(jmap_error)?
            .ok_or_else(|| Rfc822ImportError::ImportedEmailMissing {
                email_id: email_id.to_string(),
            })?;
        let jmap_email_id = email
            .id()
            .map(str::to_owned)
            .ok_or(Rfc822ImportError::MissingImportedEmailId)?;
        Ok(ImportedRfc822Message {
            jmap_email_id,
            jmap_thread_id: email.thread_id().map(str::to_owned),
            jmap_mailbox_ids: email.mailbox_ids().into_iter().map(str::to_owned).collect(),
            rfc822_message_ids: email.message_id().unwrap_or_default().to_vec(),
            duplicate,
        })
    }
}

#[async_trait]
impl Rfc822Importer for StalwartJmapRfc822Importer {
    async fn import_rfc822(
        &self,
        request: Rfc822ImportRequest,
    ) -> Result<ImportedRfc822Message, Rfc822ImportError> {
        validate_request(&request)?;

        if let Some(message_id) = first_message_id(&request.raw_rfc822) {
            if let Some(existing) = self.lookup_existing_by_message_id(&message_id).await? {
                return Ok(existing);
            }
        }

        let keywords = (!request.keywords.is_empty()).then_some(request.keywords);
        let email = self
            .session
            .client()
            .email_import_account(
                self.session.account_id(),
                request.raw_rfc822,
                request.mailbox_ids,
                keywords,
                request.received_at,
            )
            .await
            .map_err(jmap_error)?;
        let jmap_email_id = email
            .id()
            .map(str::to_owned)
            .ok_or(Rfc822ImportError::MissingImportedEmailId)?;

        self.hydrate_imported_email(&jmap_email_id, false).await
    }
}

#[derive(Debug, Default)]
pub struct FakeRfc822Importer {
    state: Mutex<FakeState>,
    fail_once: Mutex<HashMap<String, Rfc822ImportError>>,
}

#[derive(Debug, Default)]
struct FakeState {
    next_id: u64,
    by_provider_id: HashMap<String, ImportedRfc822Message>,
    by_message_id: HashMap<String, ImportedRfc822Message>,
    imports: Vec<Rfc822ImportRequest>,
}

impl FakeRfc822Importer {
    #[must_use]
    pub fn imports(&self) -> Vec<Rfc822ImportRequest> {
        self.state
            .lock()
            .expect("fake importer mutex")
            .imports
            .clone()
    }

    #[must_use]
    pub fn local_message_count(&self) -> usize {
        self.state.lock().expect("fake importer mutex").next_id as usize
    }

    pub fn fail_next_for_provider_message_id(
        &self,
        provider_message_id: impl Into<String>,
        error: Rfc822ImportError,
    ) {
        self.fail_once
            .lock()
            .expect("fake importer fail mutex")
            .insert(provider_message_id.into(), error);
    }
}

#[async_trait]
impl Rfc822Importer for FakeRfc822Importer {
    async fn import_rfc822(
        &self,
        request: Rfc822ImportRequest,
    ) -> Result<ImportedRfc822Message, Rfc822ImportError> {
        validate_request(&request)?;
        if let Some(provider_id) = request.provider_message_id.as_deref()
            && let Some(error) = self
                .fail_once
                .lock()
                .expect("fake importer fail mutex")
                .remove(provider_id)
        {
            return Err(error);
        }
        let mut state = self.state.lock().expect("fake importer mutex");
        state.imports.push(request.clone());

        if let Some(provider_id) = request.provider_message_id.as_deref() {
            if let Some(existing) = state.by_provider_id.get(provider_id) {
                return Ok(as_duplicate(existing));
            }
        }
        let message_id = first_message_id(&request.raw_rfc822);
        if let Some(message_id) = message_id.as_deref() {
            if let Some(existing) = state.by_message_id.get(message_id) {
                return Ok(as_duplicate(existing));
            }
        }

        state.next_id += 1;
        let id = state.next_id;
        let imported = ImportedRfc822Message {
            jmap_email_id: format!("email-{id}"),
            jmap_thread_id: Some(format!("thread-{id}")),
            jmap_mailbox_ids: request.mailbox_ids.clone(),
            rfc822_message_ids: message_id.iter().cloned().collect(),
            duplicate: false,
        };
        if let Some(provider_id) = request.provider_message_id.as_deref() {
            state
                .by_provider_id
                .insert(provider_id.to_string(), imported.clone());
        }
        if let Some(message_id) = message_id {
            state.by_message_id.insert(message_id, imported.clone());
        }
        Ok(imported)
    }
}

fn validate_request(request: &Rfc822ImportRequest) -> Result<(), Rfc822ImportError> {
    if request.raw_rfc822.is_empty() {
        return Err(Rfc822ImportError::EmptyMessage);
    }
    if request.mailbox_ids.is_empty() {
        return Err(Rfc822ImportError::NoTargetMailbox);
    }
    Ok(())
}

fn as_duplicate(imported: &ImportedRfc822Message) -> ImportedRfc822Message {
    let mut duplicate = imported.clone();
    duplicate.duplicate = true;
    duplicate
}

fn jmap_error(error: hail_jmap::jmap_client::Error) -> Rfc822ImportError {
    Rfc822ImportError::Jmap(error.to_string())
}

pub(crate) fn first_message_id(raw_rfc822: &[u8]) -> Option<String> {
    let headers = String::from_utf8_lossy(raw_rfc822);
    let header_block = headers
        .split("\r\n\r\n")
        .next()
        .and_then(|part| part.split("\n\n").next())
        .unwrap_or(headers.as_ref());
    let unfolded = unfold_headers(header_block);
    unfolded.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("Message-ID")
            .then(|| normalize_message_id(value.trim()))
            .flatten()
    })
}

fn unfold_headers(headers: &str) -> String {
    let mut out = String::new();
    for line in headers.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            out.push(' ');
            out.push_str(line.trim());
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line.trim_end_matches('\r'));
        }
    }
    out
}

fn normalize_message_id(value: &str) -> Option<String> {
    let candidate = value.trim();
    let candidate = if let Some(start) = candidate.find('<') {
        let after_start = &candidate[start + 1..];
        if let Some(end) = after_start.find('>') {
            &after_start[..end]
        } else {
            after_start
        }
    } else {
        candidate
            .split_whitespace()
            .find(|part| part.contains('@'))
            .unwrap_or(candidate)
    };
    let candidate = candidate.trim();
    (!candidate.is_empty()).then(|| candidate.to_string())
}

#[derive(Debug, Error)]
pub enum Rfc822ImportError {
    #[error("RFC822 import requires non-empty message bytes")]
    EmptyMessage,
    #[error("RFC822 import requires at least one target mailbox")]
    NoTargetMailbox,
    #[error("JMAP account has no Inbox mailbox for provider imports")]
    MissingInbox,
    #[error("Stalwart accepted RFC822 import but did not return an Email id")]
    MissingImportedEmailId,
    #[error("Stalwart imported Email id {email_id} but Email/get could not hydrate it")]
    ImportedEmailMissing { email_id: String },
    #[error("Stalwart JMAP RFC822 import failed: {0}")]
    Jmap(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_raw(message_id: &str) -> Vec<u8> {
        format!(
            "From: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nMessage-ID: <{message_id}>\r\nSubject: hello\r\n\r\nBody"
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn fake_import_returns_stable_jmap_ids() {
        let importer = FakeRfc822Importer::default();
        let imported = importer
            .import_rfc822(
                Rfc822ImportRequest::into_mailbox(sample_raw("m1@example.com"), "inbox")
                    .with_provider_message_id("gmail-1")
                    .with_keywords(["$seen"]),
            )
            .await
            .expect("import succeeds");

        assert_eq!(imported.jmap_email_id, "email-1");
        assert_eq!(imported.jmap_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(imported.jmap_mailbox_ids, vec!["inbox"]);
        assert_eq!(imported.rfc822_message_ids, vec!["m1@example.com"]);
        assert!(!imported.duplicate);
        assert_eq!(importer.imports().len(), 1);
    }

    #[tokio::test]
    async fn fake_import_is_idempotent_by_provider_message_id() {
        let importer = FakeRfc822Importer::default();
        let first = importer
            .import_rfc822(
                Rfc822ImportRequest::into_mailbox(sample_raw("m2@example.com"), "inbox")
                    .with_provider_message_id("gmail-2"),
            )
            .await
            .expect("first import");
        let second = importer
            .import_rfc822(
                Rfc822ImportRequest::into_mailbox(sample_raw("different@example.com"), "inbox")
                    .with_provider_message_id("gmail-2"),
            )
            .await
            .expect("second import");

        assert_eq!(second.jmap_email_id, first.jmap_email_id);
        assert!(second.duplicate);
    }

    #[tokio::test]
    async fn fake_import_dedupes_by_rfc822_message_id() {
        let importer = FakeRfc822Importer::default();
        let first = importer
            .import_rfc822(Rfc822ImportRequest::into_mailbox(
                sample_raw("m3@example.com"),
                "inbox",
            ))
            .await
            .expect("first import");
        let second = importer
            .import_rfc822(Rfc822ImportRequest::into_mailbox(
                sample_raw("m3@example.com"),
                "archive",
            ))
            .await
            .expect("duplicate import");

        assert_eq!(second.jmap_email_id, first.jmap_email_id);
        assert_eq!(second.jmap_mailbox_ids, vec!["inbox"]);
        assert!(second.duplicate);
    }

    #[test]
    fn message_id_parser_handles_folded_headers() {
        let raw = b"From: a@example.com\r\nMessage-ID: <folded\r\n @example.com>\r\n\r\nBody";
        assert_eq!(
            first_message_id(raw).as_deref(),
            Some("folded @example.com")
        );
    }

    #[tokio::test]
    async fn rejects_empty_or_mailboxless_imports() {
        let importer = FakeRfc822Importer::default();
        assert!(matches!(
            importer
                .import_rfc822(Rfc822ImportRequest::into_mailbox(Vec::new(), "inbox"))
                .await,
            Err(Rfc822ImportError::EmptyMessage)
        ));
        assert!(matches!(
            importer
                .import_rfc822(Rfc822ImportRequest {
                    raw_rfc822: sample_raw("m4@example.com"),
                    mailbox_ids: Vec::new(),
                    keywords: Vec::new(),
                    received_at: None,
                    provider_message_id: None,
                })
                .await,
            Err(Rfc822ImportError::NoTargetMailbox)
        ));
    }
}

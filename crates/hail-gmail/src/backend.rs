use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use hail_backend::{
    AttachmentMeta, BackendMsgId, BlobRef, Capabilities, Change, Envelope, Error as BackendError,
    Keyword, MailBackend, Mailbox, MailboxRole, Page, PageRequest, Principal, Query, RawMessage,
    SubmissionId, SyncCursor,
};
use tokio_util::sync::CancellationToken;

use crate::gmail_client::{
    BatchModifyMessagesRequest, GmailApiErrorKind, GmailClient, GmailClientError, GmailMessagePart,
    GmailTokenSource, ListHistoryParams, ListMessagesParams, ModifyMessageRequest, RawGmailMessage,
};
use crate::gmail_outbound_smtp::{
    GmailOutboundSmtpClient, GmailOutboundSmtpError, GmailRawOutboundMessage, LettreGmailSmtpSender,
};

const GMAIL_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: true,
    supports_eventsource: false,
    supports_principals_admin: false,
    supports_send: true,
    native_threading: true,
    max_attachment_size: 25 * 1024 * 1024,
    label_path_separator: '/',
};

const WATCH_POLL_INTERVAL: Duration = Duration::from_secs(30);

pub struct GmailBackend<T, S = LettreGmailSmtpSender> {
    gmail: Arc<GmailClient<T>>,
    smtp: Arc<GmailOutboundSmtpClient<T, S>>,
    cancel: CancellationToken,
}

impl<T> GmailBackend<T, LettreGmailSmtpSender>
where
    T: GmailTokenSource + Clone,
{
    pub fn new(http: reqwest::Client, token_source: T) -> Result<Self, GmailClientError> {
        let gmail = Arc::new(GmailClient::new(http, token_source.clone())?);
        let smtp = Arc::new(GmailOutboundSmtpClient::new(
            token_source,
            LettreGmailSmtpSender,
        ));
        Ok(Self {
            gmail,
            smtp,
            cancel: CancellationToken::new(),
        })
    }
}

impl<T, S> GmailBackend<T, S>
where
    T: GmailTokenSource,
{
    pub fn from_parts(gmail: GmailClient<T>, smtp: GmailOutboundSmtpClient<T, S>) -> Self {
        Self {
            gmail: Arc::new(gmail),
            smtp: Arc::new(smtp),
            cancel: CancellationToken::new(),
        }
    }

    pub fn with_cancel_token(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn gmail_client(&self) -> &GmailClient<T> {
        &self.gmail
    }
}

#[async_trait]
impl<T, S> MailBackend for GmailBackend<T, S>
where
    T: GmailTokenSource + Send + Sync + 'static,
    S: crate::gmail_outbound_smtp::GmailSmtpSender + Send + Sync + 'static,
{
    fn capabilities(&self) -> &'static Capabilities {
        &GMAIL_CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        let params = ListMessagesParams {
            max_results: Some(u16::try_from(page.limit.min(500)).unwrap_or(500)),
            page_token: page.cursor.clone(),
            query: gmail_query(query),
            label_ids: query
                .mailbox_role
                .and_then(gmail_label_for_role)
                .map(str::to_string)
                .into_iter()
                .collect(),
            include_spam_trash: matches!(
                query.mailbox_role,
                Some(MailboxRole::Junk | MailboxRole::Trash)
            ),
        };
        let response = self
            .gmail
            .list_messages(&params)
            .await
            .map_err(map_gmail_error)?;
        Ok(Page {
            items: response
                .messages
                .into_iter()
                .map(|msg| BackendMsgId::new(msg.id))
                .collect(),
            next_cursor: response.next_page_token,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        let raw = self
            .gmail
            .get_raw_message(id.as_str())
            .await
            .map_err(map_gmail_error)?;
        Ok(raw_message_from_gmail(raw))
    }

    async fn fetch_blob(&self, id: &BlobRef) -> hail_backend::Result<Bytes> {
        let Some((message_id, attachment_id)) = id.as_str().split_once(':') else {
            return Err(BackendError::InvalidRequest(
                "Gmail blob refs must be message_id:attachment_id".to_string(),
            ));
        };
        let bytes = self
            .gmail
            .get_attachment(message_id, attachment_id)
            .await
            .map_err(map_gmail_error)?;
        Ok(Bytes::from(bytes))
    }

    async fn set_keywords(
        &self,
        id: &BackendMsgId,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> hail_backend::Result<()> {
        let request = modify_request_for_keyword_delta(add, remove);
        self.gmail
            .modify_message(id.as_str(), &request)
            .await
            .map(|_| ())
            .map_err(map_gmail_error)
    }

    async fn move_to_role(&self, id: &BackendMsgId, role: MailboxRole) -> hail_backend::Result<()> {
        let Some(label) = gmail_label_for_role(role) else {
            return Err(BackendError::UnsupportedCapability {
                capability: "custom_mailbox_role_move",
            });
        };
        let request = BatchModifyMessagesRequest {
            ids: vec![id.as_str().to_string()],
            add_label_ids: vec![label.to_string()],
            remove_label_ids: labels_removed_for_role(role),
        };
        self.gmail
            .batch_modify_messages(&request)
            .await
            .map_err(map_gmail_error)
    }

    async fn delete_permanently(&self, id: &BackendMsgId) -> hail_backend::Result<()> {
        self.gmail
            .delete_message(id.as_str())
            .await
            .map_err(map_gmail_error)
    }

    async fn send(&self, rfc822: &[u8], envelope: &Envelope) -> hail_backend::Result<SubmissionId> {
        let message = GmailRawOutboundMessage {
            mail_from: envelope.mail_from.clone(),
            rcpt_to: envelope.rcpt_to.clone(),
            rfc822: rfc822.to_vec(),
        };
        let submission = self.smtp.send_raw(&message).await.map_err(map_smtp_error)?;
        Ok(SubmissionId::new(submission.id))
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        let mut params = ListHistoryParams::new(cursor.as_str());
        params.max_results = Some(500);
        params.history_types = vec![
            "messageAdded".to_string(),
            "labelAdded".to_string(),
            "labelRemoved".to_string(),
        ];
        let mut changes = Vec::new();
        let mut next_cursor = cursor.as_str().to_string();
        let mut seen = BTreeSet::new();

        loop {
            let response = self
                .gmail
                .list_history(&params)
                .await
                .map_err(map_gmail_error)?;
            if let Some(history_id) = response.history_id {
                next_cursor = history_id;
            }
            for record in response.history {
                next_cursor = record.id.clone();
                for added in record.messages_added {
                    let id = BackendMsgId::new(added.message.id);
                    if seen.insert(("created", id.as_str().to_string())) {
                        changes.push(Change::MessageCreated { id, raw_ref: None });
                    }
                }
                for added in record.labels_added {
                    if added.label_ids.iter().any(|label| label == "TRASH") {
                        changes.push(Change::MailboxRoleChanged {
                            id: BackendMsgId::new(added.message.id),
                            role: MailboxRole::Trash,
                        });
                    } else {
                        let label_ids = added.label_ids;
                        changes.push(Change::MessageUpdated {
                            id: BackendMsgId::new(added.message.id),
                            keywords: None,
                            keywords_added: keywords_for_added_gmail_labels(&label_ids),
                            keywords_removed: keywords_for_removed_hail_labels_on_gmail_add(
                                label_ids,
                            ),
                        });
                    }
                }
                for removed in record.labels_removed {
                    let label_ids = removed.label_ids;
                    changes.push(Change::MessageUpdated {
                        id: BackendMsgId::new(removed.message.id),
                        keywords: None,
                        keywords_added: keywords_for_added_hail_labels_on_gmail_remove(&label_ids),
                        keywords_removed: keywords_for_removed_gmail_labels(label_ids),
                    });
                }
            }
            match response.next_page_token {
                Some(token) => params.page_token = Some(token),
                None => return Ok((changes, SyncCursor::new(next_cursor))),
            }
        }
    }

    async fn watch_changes(&self) -> BoxStream<'static, Change> {
        let backend = self.clone_for_watch();
        let cancel = self.cancel.clone();
        stream::unfold((backend, cancel, None::<SyncCursor>), |(backend, cancel, cursor)| async move {
            let mut interval = tokio::time::interval(WATCH_POLL_INTERVAL);
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return None,
                    _ = interval.tick() => {
                        let cursor = match cursor.clone() {
                            Some(cursor) => cursor,
                            None => match backend.gmail.profile().await {
                                Ok(profile) => SyncCursor::new(profile.history_id.unwrap_or_default()),
                                Err(_) => continue,
                            },
                        };
                        match backend.poll_changes(&cursor).await {
                            Ok((changes, next_cursor)) => {
                                let mut items = changes.into_iter();
                                if let Some(first) = items.next() {
                                    let rest = stream::iter(items.collect::<Vec<_>>());
                                    let chained = stream::once(async move { first }).chain(rest).boxed();
                                    return Some((chained, (backend, cancel, Some(next_cursor))));
                                }
                                return Some((stream::empty().boxed(), (backend, cancel, Some(next_cursor))));
                            }
                            Err(_) => continue,
                        }
                    }
                }
            }
        })
        .flat_map(|s| s)
        .boxed()
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        let labels = self
            .gmail
            .list_labels()
            .await
            .map_err(map_gmail_error)?
            .labels;
        Ok(labels
            .into_iter()
            .map(|label| Mailbox {
                role: role_for_gmail_label(&label.id),
                id: label.id,
                name: label.name,
                parent_id: None,
            })
            .collect())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Err(BackendError::UnsupportedCapability {
            capability: "principals_admin",
        })
    }
}

impl<T, S> GmailBackend<T, S> {
    fn clone_for_watch(&self) -> Self {
        Self {
            gmail: Arc::clone(&self.gmail),
            smtp: Arc::clone(&self.smtp),
            cancel: self.cancel.clone(),
        }
    }
}

fn gmail_query(query: &Query) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(text) = query.text.as_deref().filter(|text| !text.trim().is_empty()) {
        parts.push(text.trim().to_string());
    }
    if let Some(newer) = query.newer_than_epoch_secs {
        parts.push(format!("after:{newer}"));
    }
    if let Some(older) = query.older_than_epoch_secs {
        parts.push(format!("before:{older}"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn raw_message_from_gmail(raw: RawGmailMessage) -> RawMessage {
    let mut metadata = BTreeMap::new();
    if let Some(history_id) = raw.history_id {
        metadata.insert("gmail_history_id".to_string(), history_id);
    }
    let id = raw.id;
    let attachments = raw
        .payload
        .as_ref()
        .map(|payload| attachments_from_gmail_payload(&id, payload))
        .unwrap_or_default();
    RawMessage {
        id: BackendMsgId::new(id),
        thread_id: raw.thread_id,
        rfc822: Bytes::from(raw.rfc822),
        keywords: keywords_from_gmail_labels(raw.label_ids),
        envelope: None,
        received_at_epoch_secs: None,
        size_bytes: None,
        blob_refs: Vec::new(),
        attachments,
        metadata,
    }
}

fn attachments_from_gmail_payload(
    message_id: &str,
    payload: &GmailMessagePart,
) -> Vec<AttachmentMeta> {
    let mut attachments = Vec::new();
    collect_gmail_attachments(message_id, payload, &mut attachments);
    attachments
}

fn collect_gmail_attachments(
    message_id: &str,
    part: &GmailMessagePart,
    attachments: &mut Vec<AttachmentMeta>,
) {
    let filename = part.filename.clone().unwrap_or_default();
    let attachment_id = part.body.attachment_id.as_deref();
    if !filename.is_empty() || attachment_id.is_some() {
        let content_id = header_value(part, "Content-ID").and_then(normalize_content_id);
        let content_disposition = header_value(part, "Content-Disposition");
        attachments.push(AttachmentMeta {
            filename,
            mime_type: part
                .mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            size_bytes: part.body.size.unwrap_or_default(),
            blob_ref: attachment_id
                .map(|attachment_id| BlobRef::new(format!("{message_id}:{attachment_id}"))),
            inline: is_inline_disposition(content_disposition) || content_id.is_some(),
            content_id,
        });
    }

    for sub_part in &part.parts {
        collect_gmail_attachments(message_id, sub_part, attachments);
    }
}

fn header_value<'a>(part: &'a GmailMessagePart, name: &str) -> Option<&'a str> {
    part.headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

fn is_inline_disposition(value: Option<&str>) -> bool {
    value
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("inline"))
}

fn normalize_content_id(value: &str) -> Option<String> {
    let trimmed = value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn modify_request_for_keyword_delta(add: &[Keyword], remove: &[Keyword]) -> ModifyMessageRequest {
    let mut request = ModifyMessageRequest::default();
    for keyword in add {
        if keyword.as_str() == "$seen" {
            push_unique_string(&mut request.remove_label_ids, "UNREAD");
        } else {
            push_unique_string(&mut request.add_label_ids, gmail_label_for_keyword(keyword));
        }
    }
    for keyword in remove {
        if keyword.as_str() == "$seen" {
            push_unique_string(&mut request.add_label_ids, "UNREAD");
        } else {
            push_unique_string(&mut request.remove_label_ids, gmail_label_for_keyword(keyword));
        }
    }
    request
}

fn push_unique_string(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn keywords_from_gmail_labels(label_ids: Vec<String>) -> Vec<Keyword> {
    let mut has_unread = false;
    let mut keywords = Vec::new();
    for label in label_ids {
        if label == "UNREAD" {
            has_unread = true;
        } else {
            keywords.push(Keyword::new(label));
        }
    }
    if !has_unread {
        keywords.push(Keyword::new("$seen"));
    }
    keywords
}

fn keywords_for_added_gmail_labels(label_ids: &[String]) -> Vec<Keyword> {
    label_ids
        .iter()
        .filter(|label| label.as_str() != "UNREAD")
        .map(|label| Keyword::new(label.clone()))
        .collect()
}

fn keywords_for_removed_hail_labels_on_gmail_add(label_ids: Vec<String>) -> Vec<Keyword> {
    label_ids
        .into_iter()
        .filter(|label| label == "UNREAD")
        .map(|_| Keyword::new("$seen"))
        .collect()
}

fn keywords_for_added_hail_labels_on_gmail_remove(label_ids: &[String]) -> Vec<Keyword> {
    label_ids
        .iter()
        .filter(|label| label.as_str() == "UNREAD")
        .map(|_| Keyword::new("$seen"))
        .collect()
}

fn keywords_for_removed_gmail_labels(label_ids: Vec<String>) -> Vec<Keyword> {
    label_ids
        .into_iter()
        .filter(|label| label != "UNREAD")
        .map(Keyword::new)
        .collect()
}

fn gmail_label_for_keyword(keyword: &Keyword) -> String {
    match keyword.as_str() {
        "$draft" => "DRAFT".to_string(),
        "$flagged" => "STARRED".to_string(),
        value => value.to_string(),
    }
}

fn gmail_label_for_role(role: MailboxRole) -> Option<&'static str> {
    match role {
        MailboxRole::Inbox => Some("INBOX"),
        MailboxRole::Sent => Some("SENT"),
        MailboxRole::Drafts => Some("DRAFT"),
        MailboxRole::Trash => Some("TRASH"),
        MailboxRole::Junk => Some("SPAM"),
        MailboxRole::Important => Some("IMPORTANT"),
        MailboxRole::AllMail | MailboxRole::Archive => Some("INBOX"),
        MailboxRole::Custom => None,
    }
}

fn labels_removed_for_role(role: MailboxRole) -> Vec<String> {
    match role {
        MailboxRole::Archive => vec!["INBOX".to_string()],
        MailboxRole::Inbox => vec!["SPAM".to_string(), "TRASH".to_string()],
        MailboxRole::Trash => vec!["INBOX".to_string(), "SPAM".to_string()],
        MailboxRole::Junk => vec!["INBOX".to_string(), "TRASH".to_string()],
        MailboxRole::Sent
        | MailboxRole::Drafts
        | MailboxRole::Important
        | MailboxRole::AllMail
        | MailboxRole::Custom => Vec::new(),
    }
}

fn role_for_gmail_label(label: &str) -> MailboxRole {
    match label {
        "INBOX" => MailboxRole::Inbox,
        "SENT" => MailboxRole::Sent,
        "DRAFT" => MailboxRole::Drafts,
        "TRASH" => MailboxRole::Trash,
        "SPAM" => MailboxRole::Junk,
        "IMPORTANT" => MailboxRole::Important,
        _ => MailboxRole::Custom,
    }
}

fn map_gmail_error(error: GmailClientError) -> BackendError {
    match error {
        GmailClientError::Api {
            kind: GmailApiErrorKind::Unauthorized | GmailApiErrorKind::PermissionDenied,
            ..
        } => BackendError::Authentication,
        GmailClientError::Api {
            kind: GmailApiErrorKind::NotFound,
            message,
            ..
        } => BackendError::NotFound {
            kind: "gmail",
            id: message,
        },
        GmailClientError::Api {
            kind: GmailApiErrorKind::RateLimited,
            ..
        } => BackendError::RateLimited,
        GmailClientError::Api {
            kind: GmailApiErrorKind::Transient,
            ..
        }
        | GmailClientError::Request(_) => BackendError::TemporarilyUnavailable,
        GmailClientError::Api { message, .. } => BackendError::Other(message),
        other => BackendError::Other(other.to_string()),
    }
}

fn map_smtp_error(error: GmailOutboundSmtpError) -> BackendError {
    match error {
        GmailOutboundSmtpError::Authentication | GmailOutboundSmtpError::Token(_) => {
            BackendError::Authentication
        }
        GmailOutboundSmtpError::Timeout => BackendError::TemporarilyUnavailable,
        other => BackendError::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use secrecy::SecretString;

    use super::*;
    use crate::gmail_client::{
        GmailMessagePart, GmailMessagePartBody, GmailMessagePartHeader, StaticGmailTokenSource,
    };
    use crate::gmail_outbound_smtp::{
        GmailOutboundMessage, GmailRawOutboundMessage, GmailSmtpSender, GmailSmtpSubmission,
    };

    #[derive(Clone, Default)]
    struct CapturingSmtpSender {
        captured: Arc<Mutex<Vec<GmailRawOutboundMessage>>>,
    }

    #[async_trait]
    impl GmailSmtpSender for CapturingSmtpSender {
        async fn send_message(
            &self,
            _access_token: SecretString,
            _message: &GmailOutboundMessage,
        ) -> Result<(), GmailOutboundSmtpError> {
            unreachable!("GmailBackend::send must use raw RFC822 SMTP passthrough")
        }

        async fn send_raw_message(
            &self,
            _access_token: SecretString,
            message: &GmailRawOutboundMessage,
        ) -> Result<GmailSmtpSubmission, GmailOutboundSmtpError> {
            self.captured
                .lock()
                .expect("capture mutex")
                .push(message.clone());
            Ok(GmailSmtpSubmission {
                id: "250 2.0.0 queued-as-abc123".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn backend_send_passthroughs_rfc822_and_envelope() {
        let token_source = StaticGmailTokenSource::new(SecretString::from("test-token"));
        let gmail = GmailClient::new(reqwest::Client::new(), token_source.clone()).unwrap();
        let sender = CapturingSmtpSender::default();
        let captured = Arc::clone(&sender.captured);
        let smtp = GmailOutboundSmtpClient::new(token_source, sender);
        let backend = GmailBackend::from_parts(gmail, smtp);
        let rfc822 = b"From: Header From <header-from@example.org>\r\nTo: Header To <header-to@example.org>\r\nSubject: Exact bytes\r\n\r\nBody with \0 bytes and UTF-8 \xF0\x9F\x93\xA7";
        let envelope = Envelope {
            mail_from: "smtp-from@example.org".to_string(),
            rcpt_to: vec![
                "smtp-to@example.org".to_string(),
                "smtp-bcc@example.org".to_string(),
            ],
        };

        let submission = backend.send(rfc822, &envelope).await.unwrap();

        assert_eq!(submission.as_str(), "250 2.0.0 queued-as-abc123");
        let captured = captured.lock().expect("capture mutex");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].mail_from, envelope.mail_from);
        assert_eq!(captured[0].rcpt_to, envelope.rcpt_to);
        assert_eq!(captured[0].rfc822, rfc822);
    }

    #[test]
    fn raw_message_from_gmail_extracts_nested_attachment_metadata() {
        let raw = RawGmailMessage {
            id: "msg-attachments".to_string(),
            thread_id: Some("thread-attachments".to_string()),
            history_id: Some("hist-attachments".to_string()),
            label_ids: vec!["INBOX".to_string()],
            rfc822: b"From: sender@example.org\r\n\r\nbody".to_vec(),
            payload: Some(GmailMessagePart {
                part_id: Some("0".to_string()),
                mime_type: Some("multipart/mixed".to_string()),
                filename: Some(String::new()),
                headers: Vec::new(),
                body: GmailMessagePartBody::default(),
                parts: vec![
                    GmailMessagePart {
                        part_id: Some("1".to_string()),
                        mime_type: Some("text/plain".to_string()),
                        filename: Some(String::new()),
                        headers: Vec::new(),
                        body: GmailMessagePartBody {
                            attachment_id: None,
                            size: Some(12),
                        },
                        parts: Vec::new(),
                    },
                    GmailMessagePart {
                        part_id: Some("2".to_string()),
                        mime_type: Some("application/pdf".to_string()),
                        filename: Some("invoice.pdf".to_string()),
                        headers: vec![GmailMessagePartHeader {
                            name: "Content-Disposition".to_string(),
                            value: "attachment; filename=\"invoice.pdf\"".to_string(),
                        }],
                        body: GmailMessagePartBody {
                            attachment_id: Some("att-pdf".to_string()),
                            size: Some(42_000),
                        },
                        parts: Vec::new(),
                    },
                    GmailMessagePart {
                        part_id: Some("3".to_string()),
                        mime_type: Some("multipart/related".to_string()),
                        filename: Some(String::new()),
                        headers: Vec::new(),
                        body: GmailMessagePartBody::default(),
                        parts: vec![GmailMessagePart {
                            part_id: Some("3.1".to_string()),
                            mime_type: Some("image/png".to_string()),
                            filename: Some(String::new()),
                            headers: vec![
                                GmailMessagePartHeader {
                                    name: "Content-ID".to_string(),
                                    value: "<logo@example.org>".to_string(),
                                },
                                GmailMessagePartHeader {
                                    name: "Content-Disposition".to_string(),
                                    value: "inline; filename=logo.png".to_string(),
                                },
                            ],
                            body: GmailMessagePartBody {
                                attachment_id: Some("att-logo".to_string()),
                                size: Some(1_337),
                            },
                            parts: Vec::new(),
                        }],
                    },
                ],
            }),
        };

        let message = raw_message_from_gmail(raw);

        assert_eq!(
            message.attachments,
            vec![
                AttachmentMeta {
                    filename: "invoice.pdf".to_string(),
                    mime_type: "application/pdf".to_string(),
                    size_bytes: 42_000,
                    blob_ref: Some(BlobRef::new("msg-attachments:att-pdf")),
                    inline: false,
                    content_id: None,
                },
                AttachmentMeta {
                    filename: String::new(),
                    mime_type: "image/png".to_string(),
                    size_bytes: 1_337,
                    blob_ref: Some(BlobRef::new("msg-attachments:att-logo")),
                    inline: true,
                    content_id: Some("logo@example.org".to_string()),
                },
            ]
        );
    }

    #[test]
    fn raw_message_from_gmail_maps_gmail_unread_inverse_to_seen_keyword() {
        let read = raw_message_from_gmail(RawGmailMessage {
            id: "read-msg".to_string(),
            thread_id: None,
            history_id: None,
            label_ids: vec!["INBOX".to_string()],
            rfc822: b"From: sender@example.org\r\n\r\nbody".to_vec(),
            payload: None,
        });
        assert!(read.keywords.contains(&Keyword::new("$seen")));
        assert!(read.keywords.contains(&Keyword::new("INBOX")));
        assert!(!read.keywords.contains(&Keyword::new("UNREAD")));

        let unread = raw_message_from_gmail(RawGmailMessage {
            id: "unread-msg".to_string(),
            thread_id: None,
            history_id: None,
            label_ids: vec!["INBOX".to_string(), "UNREAD".to_string()],
            rfc822: b"From: sender@example.org\r\n\r\nbody".to_vec(),
            payload: None,
        });
        assert!(!unread.keywords.contains(&Keyword::new("$seen")));
        assert!(unread.keywords.contains(&Keyword::new("INBOX")));
        assert!(!unread.keywords.contains(&Keyword::new("UNREAD")));
    }

    #[test]
    fn modify_request_for_keyword_delta_inverts_seen_and_unread_label() {
        let mark_read = modify_request_for_keyword_delta(&[Keyword::new("$seen")], &[]);
        assert_eq!(mark_read.add_label_ids, Vec::<String>::new());
        assert_eq!(mark_read.remove_label_ids, vec!["UNREAD"]);

        let mark_unread = modify_request_for_keyword_delta(&[], &[Keyword::new("$seen")]);
        assert_eq!(mark_unread.add_label_ids, vec!["UNREAD"]);
        assert_eq!(mark_unread.remove_label_ids, Vec::<String>::new());
    }

    #[test]
    fn modify_request_for_keyword_delta_maps_other_keywords_by_name() {
        let request = modify_request_for_keyword_delta(
            &[Keyword::new("$flagged"), Keyword::new("Custom")],
            &[Keyword::new("$draft"), Keyword::new("Old")],
        );

        assert_eq!(request.add_label_ids, vec!["STARRED", "Custom"]);
        assert_eq!(request.remove_label_ids, vec!["DRAFT", "Old"]);
    }

    #[test]
    fn gmail_backend_advertises_send_capability() {
        assert!(GMAIL_CAPABILITIES.supports_send);
    }
}

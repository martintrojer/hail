use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use hail_backend::{
    BackendMsgId, BlobRef, Capabilities, Change, Envelope, Error as BackendError, Keyword,
    MailBackend, Mailbox, MailboxRole, Page, PageRequest, Principal, Query, RawMessage,
    SubmissionId, SyncCursor,
};
use tokio_util::sync::CancellationToken;

use crate::gmail_client::{
    BatchModifyMessagesRequest, GmailApiErrorKind, GmailClient, GmailClientError, GmailTokenSource,
    ListHistoryParams, ListMessagesParams, ModifyMessageRequest, RawGmailMessage,
};
use crate::gmail_outbound_smtp::{
    GmailOutboundMessage, GmailOutboundSmtp, GmailOutboundSmtpClient, GmailOutboundSmtpError,
    LettreGmailSmtpSender,
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
        let request = ModifyMessageRequest {
            add_label_ids: add.iter().map(gmail_label_for_keyword).collect(),
            remove_label_ids: remove.iter().map(gmail_label_for_keyword).collect(),
        };
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
        let message = GmailOutboundMessage {
            from: envelope.mail_from.clone(),
            to: envelope.rcpt_to.clone(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            plain_text: String::from_utf8_lossy(rfc822).into_owned(),
            html: String::from_utf8_lossy(rfc822).into_owned(),
        };
        self.smtp
            .send_gmail(&message)
            .await
            .map_err(map_smtp_error)?;
        Ok(SubmissionId::new("gmail-smtp"))
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
                        changes.push(Change::MessageUpdated {
                            id: BackendMsgId::new(added.message.id),
                            keywords_added: added.label_ids.into_iter().map(Keyword::new).collect(),
                            keywords_removed: Vec::new(),
                        });
                    }
                }
                for removed in record.labels_removed {
                    changes.push(Change::MessageUpdated {
                        id: BackendMsgId::new(removed.message.id),
                        keywords_added: Vec::new(),
                        keywords_removed: removed.label_ids.into_iter().map(Keyword::new).collect(),
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
    RawMessage {
        id: BackendMsgId::new(raw.id),
        thread_id: raw.thread_id,
        rfc822: Bytes::from(raw.rfc822),
        keywords: raw.label_ids.into_iter().map(Keyword::new).collect(),
        envelope: None,
        received_at_epoch_secs: None,
        size_bytes: None,
        blob_refs: Vec::new(),
        // TODO(gmail attachments): populate structured AttachmentMeta from the
        // Gmail payload parts; the cache currently sees no attachment rows for
        // Gmail-backed messages until this is wired.
        attachments: Vec::new(),
        metadata,
    }
}

fn gmail_label_for_keyword(keyword: &Keyword) -> String {
    match keyword.as_str() {
        "$seen" => "UNREAD".to_string(),
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

//! `hail-backend` implementation backed by JMAP Mail.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::stream::BoxStream;
use futures_util::{StreamExt, stream};
use hail_backend::{
    AttachmentMeta, BackendMsgId, BlobRef, Capabilities, Change, Envelope, Keyword, MailBackend,
    Mailbox, MailboxRole, Page, PageRequest, Principal, Query, QueryScope, RawMessage,
    SubmissionId, SyncCursor,
};
use jmap_client::email::{Email, Property};
use tokio_util::sync::CancellationToken;

use crate::Session;
use crate::management::{ManagementError, ManagementPrincipal, ManagementSession};
use secrecy::SecretString;

const MAX_CHANGES_PER_POLL: usize = 256;
const EVENTSOURCE_PING_SECS: u32 = 30;

/// Capabilities exposed by the Stalwart/JMAP backend.
pub const JMAP_BACKEND_CAPABILITIES: Capabilities = Capabilities {
    supports_initial_import: false,
    supports_eventsource: true,
    supports_principals_admin: true,
    supports_send: true,
    native_threading: true,
    max_attachment_size: u64::MAX,
    label_path_separator: '/',
};

/// MailBackend implementation over an authenticated JMAP session.
pub struct JmapBackend {
    session: Arc<Session>,
    management: Option<Arc<ManagementSession>>,
    cancel: CancellationToken,
}

impl JmapBackend {
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self::with_cancel(session, CancellationToken::new())
    }

    #[must_use]
    pub fn with_cancel(session: Session, cancel: CancellationToken) -> Self {
        Self {
            session: Arc::new(session),
            management: None,
            cancel,
        }
    }

    #[must_use]
    pub fn with_management(session: Session, management: ManagementSession) -> Self {
        Self::with_management_and_cancel(session, management, CancellationToken::new())
    }

    pub async fn with_management_bearer(
        session: Session,
        management_url: &str,
        bearer: SecretString,
    ) -> hail_backend::Result<Self> {
        let management = ManagementSession::connect(management_url, bearer)
            .await
            .map_err(map_management_error)?;
        Ok(Self::with_management(session, management))
    }

    #[must_use]
    pub fn with_management_and_cancel(
        session: Session,
        management: ManagementSession,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            session: Arc::new(session),
            management: Some(Arc::new(management)),
            cancel,
        }
    }

    #[must_use]
    pub fn attach_management(mut self, management: ManagementSession) -> Self {
        self.management = Some(Arc::new(management));
        self
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        self.session.as_ref()
    }

    #[must_use]
    pub fn management(&self) -> Option<&ManagementSession> {
        self.management.as_deref()
    }

    pub async fn create_domain(&self, domain: &str) -> hail_backend::Result<()> {
        self.management_session()?
            .create_domain(domain)
            .await
            .map_err(map_management_error)
    }

    pub async fn create_individual(
        &self,
        email: &str,
        password: &SecretString,
        display_name: Option<&str>,
    ) -> hail_backend::Result<Principal> {
        let management = self.management_session()?;
        let created_id = management
            .create_individual(email, password, display_name)
            .await
            .map_err(map_management_error)?;
        if let Some(id) = created_id {
            return Ok(Principal {
                id,
                email: email.to_string(),
                display_name: display_name.map(str::to_owned),
            });
        }
        let principal = management
            .list_individuals()
            .await
            .map_err(map_management_error)?
            .into_iter()
            .find(|principal| principal.name.eq_ignore_ascii_case(email))
            .map(management_principal_to_backend)
            .unwrap_or_else(|| Principal {
                id: email.to_string(),
                email: email.to_string(),
                display_name: display_name.map(str::to_owned),
            });
        Ok(principal)
    }

    pub async fn provision_principal(
        &self,
        domain: &str,
        email: &str,
        password: &SecretString,
        display_name: Option<&str>,
    ) -> hail_backend::Result<Principal> {
        self.create_domain(domain).await?;
        self.create_individual(email, password, display_name).await
    }

    pub async fn destroy_individual(&self, email: &str) -> hail_backend::Result<()> {
        self.management_session()?
            .destroy_individual(email)
            .await
            .map_err(map_management_error)
    }

    pub async fn reset_individual_secret(
        &self,
        email: &str,
        password: &SecretString,
    ) -> hail_backend::Result<Option<Principal>> {
        let management = self.management_session()?;
        let reset_id = management
            .reset_individual_secret(email, password)
            .await
            .map_err(map_management_error)?;
        let Some(id) = reset_id else {
            return Ok(None);
        };
        Ok(Some(
            management
                .list_individuals()
                .await
                .map_err(map_management_error)?
                .into_iter()
                .find(|principal| principal.id == id)
                .map(management_principal_to_backend)
                .unwrap_or_else(|| Principal {
                    id,
                    email: email.to_string(),
                    display_name: None,
                }),
        ))
    }

    fn management_session(&self) -> hail_backend::Result<&ManagementSession> {
        self.management
            .as_deref()
            .ok_or(hail_backend::Error::UnsupportedCapability {
                capability: "stalwart principals admin requires a management session",
            })
    }

    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn cancel_watchers(&self) {
        self.cancel.cancel();
    }

    async fn default_identity_id(&self, from: &str) -> hail_backend::Result<String> {
        let mut request = self.session.client().build();
        request.get_identity().properties([
            jmap_client::identity::Property::Id,
            jmap_client::identity::Property::Email,
        ]);
        let mut response = request.send_get_identity().await.map_err(map_jmap_error)?;
        response
            .take_list()
            .into_iter()
            .find_map(|mut identity| {
                let matches = identity
                    .email()
                    .is_some_and(|email| email.eq_ignore_ascii_case(from));
                let id = identity.take_id();
                (matches && !id.is_empty()).then_some(id)
            })
            .ok_or_else(|| {
                hail_backend::Error::InvalidRequest(
                    "no JMAP identity matches envelope.mail_from".to_string(),
                )
            })
    }

    async fn keyword_state_changes(&self, ids: &[String]) -> hail_backend::Result<Vec<Change>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut request = self.session.client().build();
        request
            .get_email()
            .ids(ids.iter().cloned())
            .properties([Property::Id, Property::Keywords]);
        let mut response = request.send_get_email().await.map_err(map_jmap_error)?;
        let mut keywords_by_id = response
            .take_list()
            .into_iter()
            .filter_map(|email| {
                let id = email.id()?.to_owned();
                let keywords = email.keywords().into_iter().map(Keyword::new).collect();
                Some((id, keywords))
            })
            .collect::<BTreeMap<_, _>>();
        Ok(ids
            .iter()
            .filter_map(|id| {
                keywords_by_id.remove(id).map(|keywords| Change::MessageUpdated {
                    id: BackendMsgId::new(id.clone()),
                    keywords: Some(keywords),
                    keywords_added: Vec::new(),
                    keywords_removed: Vec::new(),
                })
            })
            .collect())
    }
}

#[async_trait]
impl MailBackend for JmapBackend {
    fn capabilities(&self) -> &'static Capabilities {
        &JMAP_BACKEND_CAPABILITIES
    }

    async fn list_message_ids(
        &self,
        query: &Query,
        page: &PageRequest,
    ) -> hail_backend::Result<Page<BackendMsgId>> {
        let mut request = self.session.client().build();
        let query_request = request.query_email();
        if let Some(filter) = query_to_filter(self.session.as_ref(), query).await? {
            query_request.filter(filter);
        }
        query_request.sort([jmap_client::email::query::Comparator::received_at().descending()]);
        if let Some(cursor) = page.cursor.as_ref() {
            query_request.anchor(cursor).anchor_offset(1);
        }
        let limit = usize::try_from(page.limit).map_err(|_| {
            hail_backend::Error::InvalidRequest("page limit is too large".to_string())
        })?;
        query_request.limit(limit.saturating_add(1));
        let mut response = request.send_query_email().await.map_err(map_jmap_error)?;
        let mut ids = response
            .take_ids()
            .into_iter()
            .map(BackendMsgId::new)
            .collect::<Vec<_>>();
        let has_more = ids.len() > limit;
        if has_more {
            ids.truncate(limit);
        }
        let next_cursor = has_more
            .then(|| ids.last().map(|id| id.as_str().to_owned()))
            .flatten();
        Ok(Page {
            items: ids,
            next_cursor,
        })
    }

    async fn get_message(&self, id: &BackendMsgId) -> hail_backend::Result<RawMessage> {
        let email = self
            .session
            .client()
            .email_get(id.as_str(), Some(raw_message_properties()))
            .await
            .map_err(map_jmap_error)?
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "email",
                id: id.as_str().to_owned(),
            })?;
        let blob_id = email.blob_id().ok_or_else(|| {
            hail_backend::Error::Other(format!("JMAP Email/get omitted blobId for {}", id.as_str()))
        })?;
        let rfc822 = self
            .session
            .client()
            .download(blob_id)
            .await
            .map_err(map_jmap_error)?;
        Ok(raw_message_from_email(email, Bytes::from(rfc822)))
    }

    async fn fetch_blob(&self, id: &BlobRef) -> hail_backend::Result<Bytes> {
        self.session
            .client()
            .download(id.as_str())
            .await
            .map(Bytes::from)
            .map_err(map_jmap_error)
    }

    async fn set_keywords(
        &self,
        id: &BackendMsgId,
        add: &[Keyword],
        remove: &[Keyword],
    ) -> hail_backend::Result<()> {
        for keyword in add {
            self.session
                .client()
                .email_set_keyword(id.as_str(), keyword.as_str(), true)
                .await
                .map_err(map_jmap_error)?;
        }
        for keyword in remove {
            self.session
                .client()
                .email_set_keyword(id.as_str(), keyword.as_str(), false)
                .await
                .map_err(map_jmap_error)?;
        }
        Ok(())
    }

    async fn move_to_role(&self, id: &BackendMsgId, role: MailboxRole) -> hail_backend::Result<()> {
        let jmap_role =
            mailbox_role_to_jmap(role).ok_or(hail_backend::Error::UnsupportedCapability {
                capability: "custom mailbox role move",
            })?;
        let mailbox_id = crate::mailbox_id_by_role(self.session.as_ref(), jmap_role)
            .await
            .map_err(map_jmap_error)?
            .ok_or_else(|| hail_backend::Error::NotFound {
                kind: "mailbox_role",
                id: format!("{role:?}"),
            })?;
        self.session
            .client()
            .email_set_mailboxes(id.as_str(), [mailbox_id])
            .await
            .map_err(map_jmap_error)?;
        Ok(())
    }

    async fn delete_permanently(&self, id: &BackendMsgId) -> hail_backend::Result<()> {
        self.session
            .client()
            .email_destroy(id.as_str())
            .await
            .map_err(map_jmap_error)
    }

    async fn send(&self, rfc822: &[u8], envelope: &Envelope) -> hail_backend::Result<SubmissionId> {
        if envelope.rcpt_to.is_empty() {
            return Err(hail_backend::Error::InvalidRequest(
                "envelope.rcpt_to must not be empty".to_string(),
            ));
        }
        let sent_id =
            crate::mailbox_id_by_role(self.session.as_ref(), jmap_client::mailbox::Role::Sent)
                .await
                .map_err(map_jmap_error)?
                .ok_or_else(|| hail_backend::Error::NotFound {
                    kind: "mailbox_role",
                    id: "sent".to_string(),
                })?;
        let email = self
            .session
            .client()
            .email_import_account(
                self.session.account_id(),
                rfc822.to_vec(),
                [sent_id],
                None::<Vec<String>>,
                None,
            )
            .await
            .map_err(map_jmap_error)?;
        let email_id = email.id().ok_or_else(|| {
            hail_backend::Error::Other("JMAP Email/import response omitted id".to_string())
        })?;
        let identity_id = self.default_identity_id(&envelope.mail_from).await?;
        let mut submission = self
            .session
            .client()
            .email_submission_create_envelope(
                email_id,
                identity_id,
                envelope.mail_from.as_str(),
                envelope.rcpt_to.iter().map(String::as_str),
            )
            .await
            .map_err(map_jmap_error)?;
        let submission_id = submission.take_id();
        if submission_id.is_empty() {
            Err(hail_backend::Error::Other(
                "JMAP EmailSubmission/set response omitted id".to_string(),
            ))
        } else {
            Ok(SubmissionId::new(submission_id))
        }
    }

    async fn poll_changes(
        &self,
        cursor: &SyncCursor,
    ) -> hail_backend::Result<(Vec<Change>, SyncCursor)> {
        let mut response = self
            .session
            .client()
            .email_changes(cursor.as_str(), Some(MAX_CHANGES_PER_POLL))
            .await
            .map_err(map_jmap_error)?;
        let mut changes = Vec::with_capacity(response.total_changes());
        changes.extend(
            response
                .created()
                .iter()
                .cloned()
                .map(|id| Change::MessageCreated {
                    id: BackendMsgId::new(id),
                    raw_ref: None,
                }),
        );
        changes.extend(self.keyword_state_changes(response.updated()).await?);
        changes.extend(
            response
                .destroyed()
                .iter()
                .cloned()
                .map(|id| Change::MessageDeleted {
                    id: BackendMsgId::new(id),
                }),
        );
        Ok((changes, SyncCursor::new(response.take_new_state())))
    }

    async fn watch_changes(&self) -> BoxStream<'static, Change> {
        let session = Arc::clone(&self.session);
        let cancel = self.cancel.clone();
        Box::pin(stream::unfold(
            WatchState::new(session, cancel),
            next_watch_change,
        ))
    }

    async fn list_mailboxes(&self) -> hail_backend::Result<Vec<Mailbox>> {
        let mut request = self.session.client().build();
        request.get_mailbox().properties([
            jmap_client::mailbox::Property::Id,
            jmap_client::mailbox::Property::Name,
            jmap_client::mailbox::Property::Role,
            jmap_client::mailbox::Property::ParentId,
        ]);
        let mut response = request.send_get_mailbox().await.map_err(map_jmap_error)?;
        Ok(response
            .take_list()
            .into_iter()
            .filter_map(mailbox_from_jmap)
            .collect())
    }

    async fn list_principals(&self) -> hail_backend::Result<Vec<Principal>> {
        Ok(self
            .management_session()?
            .list_individuals()
            .await
            .map_err(map_management_error)?
            .into_iter()
            .map(management_principal_to_backend)
            .collect())
    }
}

struct WatchState {
    session: Arc<Session>,
    cancel: CancellationToken,
    cursor: Option<SyncCursor>,
    pending: Vec<Change>,
}

impl WatchState {
    fn new(session: Arc<Session>, cancel: CancellationToken) -> Self {
        Self {
            session,
            cancel,
            cursor: None,
            pending: Vec::new(),
        }
    }
}

async fn next_watch_change(mut state: WatchState) -> Option<(Change, WatchState)> {
    loop {
        if let Some(change) = state.pending.pop() {
            return Some((change, state));
        }
        if state.cancel.is_cancelled() {
            return None;
        }
        let mut event_stream = match state
            .session
            .client()
            .event_source(
                Some([jmap_client::DataType::Email]),
                false,
                Some(EVENTSOURCE_PING_SECS),
                None,
            )
            .await
        {
            Ok(event_stream) => event_stream,
            Err(_) => {
                tokio::select! {
                    _ = state.cancel.cancelled() => return None,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                }
            }
        };

        loop {
            tokio::select! {
                _ = state.cancel.cancelled() => return None,
                notification = event_stream.next() => {
                    match notification {
                        Some(Ok(jmap_client::event_source::PushNotification::StateChange(changes))) => {
                            if let Some(next_cursor) = email_state_for_account(&changes, state.session.account_id()) {
                                let since = state.cursor.clone().unwrap_or_else(|| SyncCursor::new(next_cursor.clone()));
                                let backend = JmapBackend {
                                    session: Arc::clone(&state.session),
                                    management: None,
                                    cancel: state.cancel.clone(),
                                };
                                match backend.poll_changes(&since).await {
                                    Ok((changes, cursor)) => {
                                        state.cursor = Some(cursor);
                                        state.pending = changes.into_iter().rev().collect();
                                        break;
                                    }
                                    Err(_) => state.cursor = Some(SyncCursor::new(next_cursor)),
                                }
                            }
                        }
                        Some(Ok(jmap_client::event_source::PushNotification::CalendarAlert(_))) => {}
                        Some(Err(_)) | None => break,
                    }
                }
            }
        }
    }
}

fn email_state_for_account(
    changes: &jmap_client::event_source::Changes,
    account_id: &str,
) -> Option<String> {
    changes.changes(account_id)?.find_map(|(data_type, state)| {
        (*data_type == jmap_client::DataType::Email).then(|| state.clone())
    })
}

async fn query_to_filter(
    session: &Session,
    query: &Query,
) -> hail_backend::Result<Option<jmap_client::core::query::Filter<jmap_client::email::query::Filter>>>
{
    use jmap_client::core::query::Filter;
    use jmap_client::email::query as email_query;

    let mut conditions = Vec::new();
    match query.scope {
        QueryScope::All => {}
        QueryScope::Search => {
            if let Some(text) = query.text.as_ref().filter(|text| !text.trim().is_empty()) {
                conditions.push(Filter::from(email_query::Filter::text(text.clone())));
            }
        }
        QueryScope::Thread => {
            let thread_id = query.text.as_ref().ok_or_else(|| {
                hail_backend::Error::InvalidRequest(
                    "thread query requires text=thread_id".to_string(),
                )
            })?;
            conditions.push(Filter::from(email_query::Filter::in_thread(thread_id)));
        }
    }
    if let Some(role) = query.mailbox_role {
        if let Some(jmap_role) = mailbox_role_to_jmap(role) {
            let Some(mailbox_id) = crate::mailbox_id_by_role(session, jmap_role)
                .await
                .map_err(map_jmap_error)?
            else {
                return Ok(None);
            };
            conditions.push(Filter::from(email_query::Filter::in_mailbox(mailbox_id)));
        }
    }
    conditions.extend(query.keywords.iter().map(|keyword| {
        Filter::from(email_query::Filter::has_keyword(
            keyword.as_str().to_owned(),
        ))
    }));
    if let Some(newer) = query.newer_than_epoch_secs {
        conditions.push(Filter::from(email_query::Filter::after(newer)));
    }
    if let Some(older) = query.older_than_epoch_secs {
        conditions.push(Filter::from(email_query::Filter::before(older)));
    }
    Ok(match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => Some(Filter::and(conditions)),
    })
}

fn raw_message_properties() -> Vec<Property> {
    vec![
        Property::Id,
        Property::BlobId,
        Property::ThreadId,
        Property::Keywords,
        Property::ReceivedAt,
        Property::Size,
        Property::From,
        Property::To,
        Property::Cc,
        Property::Bcc,
        Property::Attachments,
    ]
}

fn raw_message_from_email(email: Email, rfc822: Bytes) -> RawMessage {
    let id = BackendMsgId::new(email.id().unwrap_or_default());
    let mut blob_refs = Vec::new();
    if let Some(blob_id) = email.blob_id() {
        blob_refs.push(BlobRef::new(blob_id));
    }
    let mut attachments = Vec::new();
    if let Some(parts) = email.attachments() {
        for part in parts {
            if let Some(blob_id) = part.blob_id() {
                blob_refs.push(BlobRef::new(blob_id));
            }
            attachments.push(AttachmentMeta {
                filename: part.name().unwrap_or_default().to_string(),
                mime_type: part
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string(),
                size_bytes: part.size() as u64,
                blob_ref: part.blob_id().map(BlobRef::new),
                inline: part.content_disposition() == Some("inline") || part.content_id().is_some(),
                content_id: part.content_id().map(str::to_owned),
            });
        }
    }

    let mut metadata = BTreeMap::new();
    if let Some(subject) = email.subject() {
        metadata.insert("subject".to_string(), subject.to_string());
    }
    insert_addresses(&mut metadata, "from", email.from());
    insert_addresses(&mut metadata, "to", email.to());
    insert_addresses(&mut metadata, "cc", email.cc());
    insert_addresses(&mut metadata, "bcc", email.bcc());

    RawMessage {
        id,
        thread_id: email.thread_id().map(str::to_owned),
        rfc822,
        keywords: email.keywords().into_iter().map(Keyword::new).collect(),
        envelope: email.from().and_then(|from| {
            let mail_from = from.first()?.email().to_string();
            let rcpt_to = email
                .to()
                .unwrap_or_default()
                .iter()
                .map(|addr| addr.email().to_string())
                .collect();
            Some(Envelope { mail_from, rcpt_to })
        }),
        received_at_epoch_secs: email.received_at(),
        size_bytes: Some(email.size() as u64),
        blob_refs,
        attachments,
        metadata,
    }
}

fn insert_addresses(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    addresses: Option<&[jmap_client::email::EmailAddress]>,
) {
    if let Some(addresses) = addresses {
        let value = addresses
            .iter()
            .map(|address| address.email())
            .collect::<Vec<_>>()
            .join(",");
        if !value.is_empty() {
            metadata.insert(key.to_string(), value);
        }
    }
}

fn mailbox_from_jmap(mailbox: jmap_client::mailbox::Mailbox) -> Option<Mailbox> {
    let id = mailbox.id()?.to_owned();
    Some(Mailbox {
        id,
        name: mailbox.name().unwrap_or_default().to_owned(),
        role: mailbox_role_from_jmap(mailbox.role()),
        parent_id: mailbox.parent_id().map(str::to_owned),
    })
}

fn management_principal_to_backend(principal: ManagementPrincipal) -> Principal {
    let email = principal
        .emails
        .iter()
        .find(|email| email.contains('@'))
        .cloned()
        .unwrap_or_else(|| principal.name.clone());
    Principal {
        id: principal.id,
        email,
        display_name: principal.description,
    }
}

fn mailbox_role_to_jmap(role: MailboxRole) -> Option<jmap_client::mailbox::Role> {
    match role {
        MailboxRole::Inbox => Some(jmap_client::mailbox::Role::Inbox),
        MailboxRole::Archive => Some(jmap_client::mailbox::Role::Archive),
        MailboxRole::Drafts => Some(jmap_client::mailbox::Role::Drafts),
        MailboxRole::Sent => Some(jmap_client::mailbox::Role::Sent),
        MailboxRole::Trash => Some(jmap_client::mailbox::Role::Trash),
        MailboxRole::Junk => Some(jmap_client::mailbox::Role::Junk),
        MailboxRole::Important => Some(jmap_client::mailbox::Role::Important),
        MailboxRole::AllMail | MailboxRole::Custom => None,
    }
}

fn mailbox_role_from_jmap(role: jmap_client::mailbox::Role) -> MailboxRole {
    match role {
        jmap_client::mailbox::Role::Inbox => MailboxRole::Inbox,
        jmap_client::mailbox::Role::Archive => MailboxRole::Archive,
        jmap_client::mailbox::Role::Drafts => MailboxRole::Drafts,
        jmap_client::mailbox::Role::Sent => MailboxRole::Sent,
        jmap_client::mailbox::Role::Trash => MailboxRole::Trash,
        jmap_client::mailbox::Role::Junk => MailboxRole::Junk,
        jmap_client::mailbox::Role::Important => MailboxRole::Important,
        jmap_client::mailbox::Role::Other(_) | jmap_client::mailbox::Role::None => {
            MailboxRole::Custom
        }
    }
}

fn map_management_error(error: ManagementError) -> hail_backend::Error {
    match error {
        ManagementError::Api { status, .. } if matches!(status.as_u16(), 401 | 403) => {
            hail_backend::Error::Authentication
        }
        ManagementError::Api { status, .. } if status.as_u16() == 404 => {
            hail_backend::Error::NotFound {
                kind: "stalwart_management",
                id: "management endpoint".to_string(),
            }
        }
        ManagementError::Api { status, .. } if status.as_u16() == 429 => {
            hail_backend::Error::RateLimited
        }
        ManagementError::Api { status, .. } if status.is_server_error() => {
            hail_backend::Error::TemporarilyUnavailable
        }
        ManagementError::Http(_) => hail_backend::Error::TemporarilyUnavailable,
        other => hail_backend::Error::Other(other.to_string()),
    }
}

fn map_jmap_error(error: jmap_client::Error) -> hail_backend::Error {
    match &error {
        jmap_client::Error::Problem(problem) => match problem.status {
            Some(401 | 403) => hail_backend::Error::Authentication,
            Some(404) => hail_backend::Error::NotFound {
                kind: "jmap",
                id: problem.detail().unwrap_or("not found").to_string(),
            },
            Some(429) => hail_backend::Error::RateLimited,
            Some(408 | 500..=599) => hail_backend::Error::TemporarilyUnavailable,
            _ => hail_backend::Error::Other(error.to_string()),
        },
        jmap_client::Error::Server(status)
            if status.starts_with("401") || status.starts_with("403") =>
        {
            hail_backend::Error::Authentication
        }
        jmap_client::Error::Server(status) if status.starts_with("429") => {
            hail_backend::Error::RateLimited
        }
        jmap_client::Error::Server(status) if status.starts_with("5") => {
            hail_backend::Error::TemporarilyUnavailable
        }
        _ => hail_backend::Error::Other(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jmap_capabilities_advertise_principals_admin() {
        assert!(JMAP_BACKEND_CAPABILITIES.supports_principals_admin);
    }

    #[test]
    fn management_principal_prefers_email_address() {
        let principal = management_principal_to_backend(ManagementPrincipal {
            id: "user-id".to_string(),
            name: "alice".to_string(),
            description: Some("Alice".to_string()),
            emails: vec!["alice@example.org".to_string()],
        });

        assert_eq!(principal.id, "user-id");
        assert_eq!(principal.email, "alice@example.org");
        assert_eq!(principal.display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn management_principal_falls_back_to_name() {
        let principal = management_principal_to_backend(ManagementPrincipal {
            id: "domain-id".to_string(),
            name: "example.org".to_string(),
            description: None,
            emails: Vec::new(),
        });

        assert_eq!(principal.email, "example.org");
    }
}

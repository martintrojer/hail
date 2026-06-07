//! Higher-level cache facade operations used by API mail routes.

use std::collections::{BTreeMap, HashSet};

use bytes::Bytes;
use chrono::Utc;
use hail_backend::{BackendMsgId, Keyword, MailboxRole, PageRequest, Query, QueryScope, RawMessage};
use hail_core::{MailClassification, mail_render::html_fragment_to_text};

use crate::{
    CachedAttachment, CachedAttachmentContext, CachedMail, ComposeSubmission, DraftMessage,
    DraftPayload, MailTarget, MailboxSnapshot, OutboundPayload, ReplyContext, Result,
    ScreenerDecision, ScreenerMessage, ScreenerSenderPreview,
};

const DRAFT_KEYWORD: &str = "$draft";
const HAIL_SCREENED_KEYWORD: &str = "$hail_screened";
const TRASH_KEYWORD: &str = "$trash";
const JUNK_KEYWORD: &str = "$Junk";

impl CachedMail {
    pub(crate) async fn create_draft_cached(&self, draft: DraftPayload) -> Result<BackendMsgId> {
        let rfc822 = render_rfc822(&draft, None);
        let envelope = hail_backend::Envelope {
            mail_from: draft.from.clone(),
            rcpt_to: draft.to.clone(),
        };
        let raw = RawMessage {
            id: generated_message_id("draft"),
            thread_id: None,
            rfc822: Bytes::from(rfc822),
            keywords: vec![Keyword::new(DRAFT_KEYWORD)],
            envelope: Some(envelope),
            received_at_epoch_secs: Some(Utc::now().timestamp()),
            size_bytes: None,
            blob_refs: Vec::new(),
            attachments: Vec::new(),
            metadata: draft_metadata(&draft),
        };
        let id = raw.id.clone();
        crate::readthrough::upsert_raw_metadata(self.db(), self.account_id(), raw).await?;
        Ok(id)
    }

    pub(crate) async fn get_draft_cached(
        &self,
        id: &BackendMsgId,
    ) -> Result<Option<DraftMessage>> {
        let message = match self.get_message(id).await {
            Ok(message) => message,
            Err(crate::CacheError::Backend(hail_backend::Error::NotFound { .. })) => return Ok(None),
            Err(err) => return Err(err),
        };
        if !message
            .keywords
            .iter()
            .any(|keyword| keyword.as_str().eq_ignore_ascii_case(DRAFT_KEYWORD))
        {
            return Ok(None);
        }
        let rfc822 = self.get_message_body(id).await.unwrap_or_else(|_| Bytes::new());
        let (body_html, body_markdown) = parse_body_fields(&rfc822);
        Ok(Some(DraftMessage {
            id: message.id,
            to: message.to,
            cc: message.cc,
            bcc: message.bcc,
            subject: message.subject,
            body_html,
            body_markdown,
        }))
    }

    pub(crate) async fn update_draft_cached(
        &self,
        id: &BackendMsgId,
        draft: DraftPayload,
    ) -> Result<()> {
        let existing = self.get_draft_cached(id).await?;
        if existing.is_none() {
            return Err(hail_backend::Error::NotFound {
                kind: "draft",
                id: id.as_str().to_owned(),
            }
            .into());
        }
        let rfc822 = render_rfc822(&draft, None);
        let raw = RawMessage {
            id: id.clone(),
            thread_id: None,
            rfc822: Bytes::from(rfc822),
            keywords: vec![Keyword::new(DRAFT_KEYWORD)],
            envelope: Some(hail_backend::Envelope {
                mail_from: draft.from.clone(),
                rcpt_to: draft.to.clone(),
            }),
            received_at_epoch_secs: Some(Utc::now().timestamp()),
            size_bytes: None,
            blob_refs: Vec::new(),
            attachments: Vec::new(),
            metadata: draft_metadata(&draft),
        };
        crate::readthrough::upsert_raw_metadata(self.db(), self.account_id(), raw).await?;
        Ok(())
    }

    pub(crate) async fn submit_composed_cached(
        &self,
        payload: OutboundPayload,
    ) -> Result<ComposeSubmission> {
        let submission_id = self.backend().send(&payload.rfc822, &payload.envelope).await?;
        Ok(ComposeSubmission {
            message_id: generated_message_id("sent"),
            submission_id,
        })
    }

    pub(crate) async fn reply_context_cached(
        &self,
        thread_id: &str,
    ) -> Result<Option<ReplyContext>> {
        let thread = self.get_thread(thread_id).await?;
        let Some(last) = thread.messages.into_iter().max_by_key(|message| message.received_at)
        else {
            return Ok(None);
        };
        Ok(Some(ReplyContext {
            to: if last.from.is_empty() { Vec::new() } else { vec![last.from] },
            subject: reply_subject(&last.subject),
            in_reply_to: Vec::new(),
            references: Vec::new(),
        }))
    }

    pub(crate) async fn list_attachments_cached(&self, limit: usize) -> Result<Vec<CachedAttachment>> {
        let page = self
            .backend()
            .list_message_ids(&Query::all(), &PageRequest::first(limit_as_u32(limit.saturating_mul(4))))
            .await?;
        let mut out = Vec::new();
        for id in page.items {
            if out.len() >= limit {
                break;
            }
            let raw = self.backend().get_message(&id).await?;
            crate::readthrough::upsert_raw_metadata(self.db(), self.account_id(), raw.clone()).await?;
            let cached = crate::readthrough::cached_message_from_raw(raw.clone());
            let context = CachedAttachmentContext {
                thread_id: cached.thread_id.unwrap_or_else(|| cached.id.as_str().to_owned()),
                message_id: cached.id,
                subject: cached.subject,
                from: cached.from,
                received_at: cached.received_at,
                preview: cached.preview,
            };
            for meta in raw.attachments {
                if let Some(item) = CachedAttachment::from_meta(meta, context.clone()) {
                    out.push(item);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(out)
    }

    pub(crate) async fn screener_previews_cached(
        &self,
        senders: &[String],
        limit_per_sender: usize,
    ) -> Result<Vec<ScreenerSenderPreview>> {
        let page = self
            .backend()
            .list_message_ids(&Query::all(), &PageRequest::first(1_000))
            .await?;
        let wanted = senders
            .iter()
            .map(|sender| sender.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let mut by_sender = BTreeMap::<String, Vec<ScreenerMessage>>::new();
        for id in page.items {
            let raw = self.backend().get_message(&id).await?;
            let cached = crate::readthrough::cached_message_from_raw(raw.clone());
            let sender = cached.from.to_ascii_lowercase();
            if !wanted.contains(&sender) {
                continue;
            }
            crate::readthrough::upsert_raw_metadata(self.db(), self.account_id(), raw).await?;
            by_sender.entry(sender).or_default().push(ScreenerMessage {
                email_id: cached.id,
                subject: cached.subject,
                preview: cached.preview,
                from: cached.from,
                received_at: cached.received_at,
            });
        }
        Ok(senders
            .iter()
            .map(|sender| {
                let mut emails = by_sender
                    .remove(&sender.to_ascii_lowercase())
                    .unwrap_or_default();
                emails.sort_by_key(|email| std::cmp::Reverse(email.received_at));
                let message_count = emails.len();
                emails.truncate(limit_per_sender);
                ScreenerSenderPreview { sender: sender.clone(), message_count, emails }
            })
            .collect())
    }

    pub(crate) async fn apply_screener_backfill_cached(
        &self,
        sender: &str,
        decision: ScreenerDecision,
        classify_as: Option<Keyword>,
    ) -> Result<()> {
        let ids = self.message_ids_from_sender(sender).await?;
        let add = match decision {
            ScreenerDecision::Approve => vec![classify_as.ok_or_else(|| hail_backend::Error::InvalidRequest("missing screener classification".to_string()))?, Keyword::new(HAIL_SCREENED_KEYWORD)],
            ScreenerDecision::Deny => vec![Keyword::new(HAIL_SCREENED_KEYWORD), Keyword::new(TRASH_KEYWORD)],
        };
        let remove = match decision {
            ScreenerDecision::Approve => vec![Keyword::new(TRASH_KEYWORD), Keyword::new(JUNK_KEYWORD)],
            ScreenerDecision::Deny => MailClassification::ALL.into_iter().map(Keyword::from_classification).collect(),
        };
        for id in ids {
            self.mutate_keywords(MailTarget::Message(&id), &add, &remove).await?;
            if decision == ScreenerDecision::Deny {
                self.move_to_role(MailTarget::Message(&id), MailboxRole::Trash).await?;
            } else {
                self.move_to_role(MailTarget::Message(&id), MailboxRole::Inbox).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn undo_screener_deny_cached(&self, sender: &str, classify_as: Keyword) -> Result<()> {
        let ids = self.message_ids_from_sender(sender).await?;
        let stale = MailClassification::ALL
            .into_iter()
            .map(Keyword::from_classification)
            .filter(|keyword| keyword != &classify_as)
            .chain([Keyword::new(TRASH_KEYWORD), Keyword::new(JUNK_KEYWORD)])
            .collect::<Vec<_>>();
        for id in ids {
            self.mutate_keywords(MailTarget::Message(&id), &[classify_as.clone(), Keyword::new(HAIL_SCREENED_KEYWORD)], &stale).await?;
            self.move_to_role(MailTarget::Message(&id), MailboxRole::Inbox).await?;
        }
        Ok(())
    }

    pub(crate) async fn restore_mailboxes_cached(&self, snapshots: &[MailboxSnapshot]) -> Result<()> {
        for snapshot in snapshots {
            if snapshot.mailbox_ids.iter().any(|id| id.eq_ignore_ascii_case("trash")) {
                self.move_to_role(MailTarget::Message(&snapshot.message_id), MailboxRole::Trash).await?;
            } else {
                self.move_to_role(MailTarget::Message(&snapshot.message_id), MailboxRole::Inbox).await?;
            }
        }
        Ok(())
    }

    async fn message_ids_from_sender(&self, sender: &str) -> Result<Vec<BackendMsgId>> {
        let page = self
            .backend()
            .list_message_ids(&Query { scope: QueryScope::Search, text: Some(sender.to_owned()), mailbox_role: None, keywords: Vec::new(), newer_than_epoch_secs: None, older_than_epoch_secs: None }, &PageRequest::first(1_000))
            .await?;
        let mut ids = Vec::new();
        for id in page.items {
            let raw = self.backend().get_message(&id).await?;
            let cached = crate::readthrough::cached_message_from_raw(raw.clone());
            if cached.from.eq_ignore_ascii_case(sender) {
                crate::readthrough::upsert_raw_metadata(self.db(), self.account_id(), raw).await?;
                ids.push(cached.id);
            }
        }
        Ok(ids)
    }
}

fn generated_message_id(prefix: &str) -> BackendMsgId {
    BackendMsgId::new(format!("hail-{prefix}-{}", uuid::Uuid::new_v4()))
}

fn draft_metadata(draft: &DraftPayload) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert("subject".to_string(), draft.subject.clone());
    metadata.insert("preview".to_string(), draft.body_markdown.clone());
    metadata
}

fn render_rfc822(draft: &DraftPayload, reply: Option<&ReplyContext>) -> Vec<u8> {
    let mut headers = vec![
        format!("From: {}", draft.from),
        format!("To: {}", draft.to.join(", ")),
        format!("Subject: {}", sanitize_header(&draft.subject)),
        "MIME-Version: 1.0".to_string(),
    ];
    if !draft.cc.is_empty() {
        headers.push(format!("Cc: {}", draft.cc.join(", ")));
    }
    if let Some(reply) = reply {
        if let Some(id) = reply.in_reply_to.first() {
            headers.push(format!("In-Reply-To: {}", sanitize_header(id)));
        }
        if !reply.references.is_empty() {
            headers.push(format!("References: {}", sanitize_header(&reply.references.join(" "))));
        }
    }
    headers.push("Content-Type: text/html; charset=utf-8".to_string());
    headers.push(String::new());
    headers.push(draft.html.clone());
    headers.join("\r\n").into_bytes()
}

fn sanitize_header(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn parse_body_fields(rfc822: &[u8]) -> (String, String) {
    let Some(message) = mail_parser::MessageParser::default().parse(rfc822) else {
        let body = String::from_utf8_lossy(rfc822).into_owned();
        return (String::new(), body);
    };
    let html = (0..message.html_body_count())
        .filter_map(|index| message.body_html(index).map(|body| body.into_owned()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let text = (0..message.text_body_count())
        .filter_map(|index| message.body_text(index).map(|body| body.into_owned()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let markdown = if text.is_empty() { html_fragment_to_text(&html) } else { text };
    (html, markdown)
}

fn reply_subject(subject: &str) -> String {
    if subject.trim_start().get(..3).is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:")) {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    }
}

fn limit_as_u32(limit: usize) -> u32 {
    u32::try_from(limit).unwrap_or(u32::MAX).max(1)
}

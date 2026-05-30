use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hail_db::app_events::insert_app_event;
use sqlx::{SqliteConnection, SqlitePool};
use thiserror::Error;

use crate::app_events::WorkerAppEvent;
use crate::rfc822_import::{
    ImportedRfc822Message, Rfc822ImportError, Rfc822ImportRequest, Rfc822Importer,
};
use crate::screener::{EmailEnvelope, JmapOps, RouteError, RouteOutcome, route_email};

#[derive(Debug, Error)]
pub enum RoutedRfc822ImportError {
    #[error(transparent)]
    Import(#[from] Rfc822ImportError),
    #[error(transparent)]
    Route(#[from] RouteError),
    #[error("database error during routed import: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutedImportedRfc822Message {
    pub imported: ImportedRfc822Message,
    pub route_outcome: Option<RouteOutcome>,
}

#[async_trait]
pub trait Rfc822ImportRouter: Send + Sync {
    async fn route_imported_rfc822(
        &self,
        conn: &mut SqliteConnection,
        user_id: i64,
        imported: &ImportedRfc822Message,
        request: &Rfc822ImportRequest,
    ) -> Result<RouteOutcome, RouteError>;
}

pub struct ScreenerRfc822ImportRouter<'a> {
    jmap: &'a dyn JmapOps,
}

impl<'a> ScreenerRfc822ImportRouter<'a> {
    #[must_use]
    pub fn new(jmap: &'a dyn JmapOps) -> Self {
        Self { jmap }
    }
}

#[async_trait]
impl Rfc822ImportRouter for ScreenerRfc822ImportRouter<'_> {
    async fn route_imported_rfc822(
        &self,
        conn: &mut SqliteConnection,
        user_id: i64,
        imported: &ImportedRfc822Message,
        request: &Rfc822ImportRequest,
    ) -> Result<RouteOutcome, RouteError> {
        let Some(env) = email_envelope_from_import(imported, request) else {
            return Ok(RouteOutcome::AlreadyScreened);
        };
        route_email(conn, self.jmap, user_id, &env).await
    }
}

#[allow(dead_code)]
#[async_trait]
pub trait RoutedRfc822Importer: Send + Sync {
    async fn import_and_route_rfc822(
        &self,
        db: &SqlitePool,
        user_id: i64,
        request: Rfc822ImportRequest,
    ) -> Result<RoutedImportedRfc822Message, RoutedRfc822ImportError>;
}

pub struct RoutingRfc822Importer<'a, I, R> {
    importer: &'a I,
    router: &'a R,
}

impl<'a, I, R> RoutingRfc822Importer<'a, I, R> {
    #[must_use]
    pub fn new(importer: &'a I, router: &'a R) -> Self {
        Self { importer, router }
    }
}

#[async_trait]
impl<I, R> RoutedRfc822Importer for RoutingRfc822Importer<'_, I, R>
where
    I: Rfc822Importer,
    R: Rfc822ImportRouter,
{
    async fn import_and_route_rfc822(
        &self,
        db: &SqlitePool,
        user_id: i64,
        request: Rfc822ImportRequest,
    ) -> Result<RoutedImportedRfc822Message, RoutedRfc822ImportError> {
        let imported = self.import_rfc822_only(request.clone()).await?;
        let outcome = self
            .route_imported_rfc822(db, user_id, &imported, &request)
            .await?;
        Ok(RoutedImportedRfc822Message {
            imported,
            route_outcome: Some(outcome),
        })
    }
}

impl<I, R> RoutingRfc822Importer<'_, I, R>
where
    I: Rfc822Importer,
    R: Rfc822ImportRouter,
{
    pub async fn import_rfc822_only(
        &self,
        request: Rfc822ImportRequest,
    ) -> Result<ImportedRfc822Message, Rfc822ImportError> {
        self.importer.import_rfc822(request).await
    }

    pub async fn route_imported_rfc822(
        &self,
        db: &SqlitePool,
        user_id: i64,
        imported: &ImportedRfc822Message,
        request: &Rfc822ImportRequest,
    ) -> Result<RouteOutcome, RoutedRfc822ImportError> {
        let mut conn = db.acquire().await?;
        let outcome = self
            .router
            .route_imported_rfc822(conn.as_mut(), user_id, imported, request)
            .await?;
        publish_route_event(db, user_id, &outcome).await?;
        Ok(outcome)
    }
}

fn email_envelope_from_import(
    imported: &ImportedRfc822Message,
    request: &Rfc822ImportRequest,
) -> Option<EmailEnvelope> {
    let from = first_header_value(&request.raw_rfc822, "From")?;
    let from = crate::screener::normalize_sender(&from);
    if from.is_empty() {
        return None;
    }

    Some(EmailEnvelope {
        id: imported.jmap_email_id.clone(),
        thread_id: imported.jmap_thread_id.clone().unwrap_or_default(),
        from,
        subject: first_header_value(&request.raw_rfc822, "Subject").unwrap_or_default(),
        preview: None,
        raw_rfc822: Some(request.raw_rfc822.clone()),
        to: header_values(&request.raw_rfc822, "To"),
        cc: header_values(&request.raw_rfc822, "Cc"),
        mailbox_ids: imported.jmap_mailbox_ids.clone(),
        keywords: request.keywords.clone(),
        received_at: request
            .received_at
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0)),
        size: u32::try_from(request.raw_rfc822.len()).ok(),
    })
}

fn header_values(raw_rfc822: &[u8], header_name: &str) -> Vec<String> {
    all_header_values(raw_rfc822, header_name)
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(normalize_header_address)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalize_header_address(value: &str) -> String {
    let trimmed = value.trim();
    if let Some((_, rest)) = trimmed.rsplit_once('<') {
        if let Some((addr, _)) = rest.split_once('>') {
            return addr.trim().to_ascii_lowercase();
        }
    }
    trimmed.to_ascii_lowercase()
}

fn first_header_value(raw_rfc822: &[u8], header_name: &str) -> Option<String> {
    all_header_values(raw_rfc822, header_name)
        .into_iter()
        .next()
}

fn all_header_values(raw_rfc822: &[u8], header_name: &str) -> Vec<String> {
    let headers = String::from_utf8_lossy(raw_rfc822);
    let header_block = headers
        .split("\r\n\r\n")
        .next()
        .and_then(|part| part.split("\n\n").next())
        .unwrap_or(headers.as_ref());
    let unfolded = unfold_headers(header_block);
    unfolded
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(header_name)
                .then(|| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .collect()
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

async fn publish_route_event(
    db: &SqlitePool,
    user_id: i64,
    outcome: &RouteOutcome,
) -> Result<(), sqlx::Error> {
    let event = match outcome {
        RouteOutcome::Classified { classification } => match classification {
            hail_core::MailClassification::Imbox => WorkerAppEvent::ImboxNew,
            hail_core::MailClassification::Feed => WorkerAppEvent::FeedNew,
            hail_core::MailClassification::Papertrail => WorkerAppEvent::PapertrailNew,
        },
        RouteOutcome::ScreenerPending { .. } => WorkerAppEvent::ScreenerPending,
        RouteOutcome::SpeakeasyBypass => WorkerAppEvent::ImboxNew,
        RouteOutcome::Trashed | RouteOutcome::Spam | RouteOutcome::AlreadyScreened => {
            WorkerAppEvent::ThreadUpdated
        }
    };
    insert_app_event(db, Some(user_id), event.event_type(), "{}").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_folded_headers_for_route_envelope() {
        let raw = b"From: Sender <SENDER@example.com>\r\nSubject: hello\r\n world\r\n\r\nBody";
        let imported = ImportedRfc822Message {
            jmap_email_id: "email-1".to_string(),
            jmap_thread_id: Some("thread-1".to_string()),
            jmap_mailbox_ids: vec!["inbox".to_string()],
            rfc822_message_ids: Vec::new(),
            duplicate: false,
        };
        let request = Rfc822ImportRequest::into_mailbox(raw.to_vec(), "inbox");
        let env = email_envelope_from_import(&imported, &request).expect("envelope");

        assert_eq!(env.from, "sender@example.com");
        assert_eq!(env.subject, "hello world");
        assert_eq!(env.size, Some(raw.len() as u32));
    }
}

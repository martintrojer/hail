use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{SqliteConnection, SqlitePool};
use thiserror::Error;

use crate::rfc822_import::{
    ImportedRfc822Message, Rfc822ImportError, Rfc822ImportRequest, Rfc822Importer,
};
use hail_core::screener::normalize_sender;

#[derive(Debug, Error)]
pub enum RoutedRfc822ImportError {
    #[error(transparent)]
    Import(#[from] Rfc822ImportError),
    #[error("route error: {0}")]
    Route(String),
    #[error("database error during routed import: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutedImportedRfc822Message {
    pub imported: ImportedRfc822Message,
    pub route_outcome: Option<()>,
}

#[async_trait]
pub trait Rfc822ImportRouter: Send + Sync {
    async fn route_imported_rfc822(
        &self,
        conn: &mut SqliteConnection,
        user_id: i64,
        imported: &ImportedRfc822Message,
        request: &Rfc822ImportRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn publish_route_event(
        &self,
        _db: &SqlitePool,
        _user_id: i64,
    ) -> Result<(), sqlx::Error> {
        Ok(())
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
        self.route_imported_rfc822(db, user_id, &imported, &request)
            .await?;
        Ok(RoutedImportedRfc822Message {
            imported,
            route_outcome: None,
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
    ) -> Result<(), RoutedRfc822ImportError> {
        let mut conn = db.acquire().await?;
        self.router
            .route_imported_rfc822(conn.as_mut(), user_id, imported, request)
            .await
            .map_err(|err| RoutedRfc822ImportError::Route(err.to_string()))?;
        self.router.publish_route_event(db, user_id).await?;
        Ok(())
    }
}

pub fn email_envelope_from_import(
    imported: &ImportedRfc822Message,
    request: &Rfc822ImportRequest,
) -> Option<ImportedEmailEnvelope> {
    let from = first_header_value(&request.raw_rfc822, "From")?;
    let from = normalize_sender(&from);
    if from.is_empty() {
        return None;
    }

    Some(ImportedEmailEnvelope {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedEmailEnvelope {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub preview: Option<String>,
    pub raw_rfc822: Option<Vec<u8>>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub mailbox_ids: Vec<String>,
    pub keywords: Vec<String>,
    pub received_at: Option<DateTime<Utc>>,
    pub size: Option<u32>,
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

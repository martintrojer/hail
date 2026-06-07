use async_trait::async_trait;
use hail_db::app_events::insert_app_event;
use sqlx::{SqliteConnection, SqlitePool};

use crate::provider_import_routing::{ImportedEmailEnvelope, Rfc822ImportRouter};
use crate::rfc822_import::{ImportedRfc822Message, Rfc822ImportRequest};
use crate::screener::{EmailEnvelope, JmapOps, RouteOutcome, route_email};

pub struct ScreenerRfc822ImportRouter<'a> {
    jmap: &'a dyn JmapOps,
    last_outcome: tokio::sync::Mutex<Option<RouteOutcome>>,
}

impl<'a> ScreenerRfc822ImportRouter<'a> {
    #[must_use]
    pub fn new(jmap: &'a dyn JmapOps) -> Self {
        Self {
            jmap,
            last_outcome: tokio::sync::Mutex::new(None),
        }
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
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(env) =
            crate::provider_import_routing::email_envelope_from_import(imported, request)
        else {
            *self.last_outcome.lock().await = Some(RouteOutcome::AlreadyScreened);
            return Ok(());
        };
        let outcome = route_email(conn, self.jmap, user_id, &email_envelope(env))
            .await
            .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)?;
        *self.last_outcome.lock().await = Some(outcome);
        Ok(())
    }

    async fn publish_route_event(&self, db: &SqlitePool, user_id: i64) -> Result<(), sqlx::Error> {
        let outcome = self.last_outcome.lock().await.clone();
        let event = match outcome.as_ref() {
            Some(RouteOutcome::Classified { classification }) => match classification {
                hail_core::MailClassification::Imbox => crate::app_events::WorkerAppEvent::ImboxNew,
                hail_core::MailClassification::Feed => crate::app_events::WorkerAppEvent::FeedNew,
                hail_core::MailClassification::Papertrail => {
                    crate::app_events::WorkerAppEvent::PapertrailNew
                }
            },
            Some(RouteOutcome::ScreenerPending { .. }) => {
                crate::app_events::WorkerAppEvent::ScreenerPending
            }
            Some(RouteOutcome::SpeakeasyBypass) => crate::app_events::WorkerAppEvent::ImboxNew,
            Some(RouteOutcome::Trashed | RouteOutcome::Spam | RouteOutcome::AlreadyScreened)
            | None => crate::app_events::WorkerAppEvent::ThreadUpdated,
        };
        insert_app_event(db, Some(user_id), event.event_type(), "{}")
            .await
            .map(|_| ())
    }
}

fn email_envelope(env: ImportedEmailEnvelope) -> EmailEnvelope {
    EmailEnvelope {
        id: env.id,
        thread_id: env.thread_id,
        from: env.from,
        subject: env.subject,
        preview: env.preview,
        raw_rfc822: env.raw_rfc822,
        to: env.to,
        cc: env.cc,
        mailbox_ids: env.mailbox_ids,
        keywords: env.keywords,
        received_at: env.received_at,
        size: env.size,
    }
}

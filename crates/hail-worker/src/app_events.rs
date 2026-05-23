//! Durable app-event publishing helpers for worker-originated UI events.
//!
//! `hail-worker` cannot reach `hail-api`'s in-process WebSocket broadcast bus.
//! Instead, it writes coarse product invalidation events to the shared SQLite
//! outbox; hail-api polls and rebroadcasts those rows to connected browser
//! clients.

use anyhow::{Context, Result};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerAppEvent {
    ImboxNew,
    FeedNew,
    PapertrailNew,
    ScreenerPending,
    ThreadUpdated,
    BubbleFired,
    SendCompleted,
    SendFailed,
}

impl WorkerAppEvent {
    #[must_use]
    pub fn event_type(self) -> &'static str {
        match self {
            Self::ImboxNew => "imbox.new",
            Self::FeedNew => "feed.new",
            Self::PapertrailNew => "papertrail.new",
            Self::ScreenerPending => "screener.pending",
            Self::ThreadUpdated => "thread.updated",
            Self::BubbleFired => "bubble.fired",
            Self::SendCompleted => "send.completed",
            Self::SendFailed => "send.failed",
        }
    }
}

pub async fn publish_app_event(
    db: &SqlitePool,
    user_id: i64,
    event: WorkerAppEvent,
) -> Result<i64> {
    hail_db::app_events::insert_app_event(db, Some(user_id), event.event_type(), "{}")
        .await
        .with_context(|| format!("publish app event {}", event.event_type()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_event_types_match_websocket_contract() {
        assert_eq!(WorkerAppEvent::ImboxNew.event_type(), "imbox.new");
        assert_eq!(WorkerAppEvent::FeedNew.event_type(), "feed.new");
        assert_eq!(WorkerAppEvent::PapertrailNew.event_type(), "papertrail.new");
        assert_eq!(
            WorkerAppEvent::ScreenerPending.event_type(),
            "screener.pending"
        );
        assert_eq!(WorkerAppEvent::ThreadUpdated.event_type(), "thread.updated");
        assert_eq!(WorkerAppEvent::BubbleFired.event_type(), "bubble.fired");
        assert_eq!(WorkerAppEvent::SendCompleted.event_type(), "send.completed");
        assert_eq!(WorkerAppEvent::SendFailed.event_type(), "send.failed");
    }
}

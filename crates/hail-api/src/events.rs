//! Application events for the WebSocket multiplexer plus the durable
//! worker-to-API bridge.
//!
//! `hail-api` still uses a process-local Tokio broadcast channel for connected
//! `/api/ws` clients, but worker-originated events now cross the process
//! boundary through SQLite: `hail-worker` appends rows to `app_events`, and the
//! API process polls rows newer than its startup cursor and rebroadcasts them.
//! This keeps the self-hosted v1 stack single-file/single-host friendly: no
//! Redis, Postgres, NATS, or extra inbound worker port.

use chrono::{DateTime, Utc};
use hail_db::app_events::StoredAppEvent;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};
use tokio_util::sync::CancellationToken;

const DB_BRIDGE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const DB_BRIDGE_BATCH_LIMIT: i64 = 100;

/// App-level event delivered to `/api/ws` clients.
///
/// The externally visible JSON shape is a tagged object with a stable
/// `type` field, for example `{ "type": "imbox.new" }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppEvent {
    /// Server heartbeat emitted by each WebSocket connection every 30s.
    #[serde(rename = "heartbeat")]
    Heartbeat { at: DateTime<Utc> },
    #[serde(rename = "imbox.new")]
    ImboxNew,
    #[serde(rename = "feed.new")]
    FeedNew,
    #[serde(rename = "papertrail.new")]
    PapertrailNew,
    #[serde(rename = "screener.pending")]
    ScreenerPending,
    #[serde(rename = "thread.updated")]
    ThreadUpdated,
    #[serde(rename = "thread.removed")]
    ThreadRemoved,
    #[serde(rename = "bubble.fired")]
    BubbleFired,
    #[serde(rename = "send.completed")]
    SendCompleted,
    #[serde(rename = "send.failed")]
    SendFailed,
}

impl AppEvent {
    #[must_use]
    pub fn from_type(event_type: &str) -> Option<Self> {
        match event_type {
            "imbox.new" => Some(Self::ImboxNew),
            "feed.new" => Some(Self::FeedNew),
            "papertrail.new" => Some(Self::PapertrailNew),
            "screener.pending" => Some(Self::ScreenerPending),
            "thread.updated" => Some(Self::ThreadUpdated),
            "thread.removed" => Some(Self::ThreadRemoved),
            "bubble.fired" => Some(Self::BubbleFired),
            "send.completed" => Some(Self::SendCompleted),
            "send.failed" => Some(Self::SendFailed),
            _ => None,
        }
    }
}

/// Internal event with optional user scope. The scope is not serialized to the
/// browser; it is used by the WebSocket handler to avoid waking unrelated tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEventEnvelope {
    pub user_id: Option<i64>,
    pub event: AppEvent,
}

/// Cloneable in-process broadcast bus for app events.
#[derive(Clone, Debug)]
pub struct AppEventBus {
    sender: broadcast::Sender<AppEventEnvelope>,
}

impl AppEventBus {
    const DEFAULT_CAPACITY: usize = 256;

    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEventEnvelope> {
        self.sender.subscribe()
    }

    pub fn publish(
        &self,
        event: AppEvent,
    ) -> Result<usize, broadcast::error::SendError<AppEventEnvelope>> {
        self.sender.send(AppEventEnvelope {
            user_id: None,
            event,
        })
    }

    pub fn publish_for_user(
        &self,
        user_id: i64,
        event: AppEvent,
    ) -> Result<usize, broadcast::error::SendError<AppEventEnvelope>> {
        self.sender.send(AppEventEnvelope {
            user_id: Some(user_id),
            event,
        })
    }
}

impl Default for AppEventBus {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

/// Spawn the SQLite polling bridge used by the production API process.
pub fn spawn_db_event_bridge(
    db: SqlitePool,
    bus: AppEventBus,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = run_db_event_bridge(db, bus, cancel, DB_BRIDGE_POLL_INTERVAL).await {
            tracing::error!(error = %err, "app event bridge stopped with error");
        }
    })
}

/// Poll the durable app_events outbox and rebroadcast rows to local WS clients.
///
/// On startup the bridge advances to the current max id. That deliberately
/// avoids replaying stale invalidation hints after API restart; future worker
/// rows are delivered at-least-once while the API process is alive.
pub async fn run_db_event_bridge(
    db: SqlitePool,
    bus: AppEventBus,
    cancel: CancellationToken,
    poll_interval: Duration,
) -> Result<(), sqlx::Error> {
    let mut last_seen_id = hail_db::app_events::latest_app_event_id(&db).await?;
    let mut ticker = time::interval(poll_interval);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = ticker.tick() => {
                last_seen_id = poll_once(&db, &bus, last_seen_id).await?;
            }
        }
    }

    Ok(())
}

async fn poll_once(
    db: &SqlitePool,
    bus: &AppEventBus,
    mut last_seen_id: i64,
) -> Result<i64, sqlx::Error> {
    loop {
        let rows =
            hail_db::app_events::fetch_app_events_after(db, last_seen_id, DB_BRIDGE_BATCH_LIMIT)
                .await?;
        let is_full_batch = i64::try_from(rows.len()).unwrap_or(0) == DB_BRIDGE_BATCH_LIMIT;
        if rows.is_empty() {
            return Ok(last_seen_id);
        }

        for row in rows {
            last_seen_id = row.id;
            publish_stored_event(bus, row);
        }

        if !is_full_batch {
            return Ok(last_seen_id);
        }
    }
}

fn publish_stored_event(bus: &AppEventBus, row: StoredAppEvent) {
    let Some(event) = AppEvent::from_type(&row.event_type) else {
        tracing::warn!(app_event_id = row.id, event_type = %row.event_type, "ignoring unknown app event type");
        return;
    };

    let sent = match row.user_id {
        Some(user_id) => bus.publish_for_user(user_id, event),
        None => bus.publish(event),
    };
    if sent.is_err() {
        tracing::debug!(
            app_event_id = row.id,
            "no websocket receivers for app event"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_event_from_type_accepts_durable_event_types() {
        assert_eq!(AppEvent::from_type("imbox.new"), Some(AppEvent::ImboxNew));
        assert_eq!(AppEvent::from_type("feed.new"), Some(AppEvent::FeedNew));
        assert_eq!(
            AppEvent::from_type("papertrail.new"),
            Some(AppEvent::PapertrailNew)
        );
        assert_eq!(
            AppEvent::from_type("screener.pending"),
            Some(AppEvent::ScreenerPending)
        );
        assert_eq!(
            AppEvent::from_type("thread.updated"),
            Some(AppEvent::ThreadUpdated)
        );
        assert_eq!(
            AppEvent::from_type("thread.removed"),
            Some(AppEvent::ThreadRemoved)
        );
        assert_eq!(
            AppEvent::from_type("bubble.fired"),
            Some(AppEvent::BubbleFired)
        );
        assert_eq!(
            AppEvent::from_type("send.completed"),
            Some(AppEvent::SendCompleted)
        );
        assert_eq!(
            AppEvent::from_type("send.failed"),
            Some(AppEvent::SendFailed)
        );
        assert_eq!(AppEvent::from_type("heartbeat"), None);
    }
}

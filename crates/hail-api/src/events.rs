//! In-process application event bus for the WebSocket multiplexer.
//!
//! This is deliberately process-local for now: `hail-api` can broadcast
//! events to connected browsers, but `hail-worker` runs in a separate
//! process and cannot publish here yet. TODO(worker-to-api-bus): replace
//! or bridge this with a cross-process channel (for example SQLite-backed
//! notifications, Redis/NATS, or a Stalwart/JMAP event fan-out) so worker
//! events reach API WebSocket clients without co-locating the binaries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

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
    #[serde(rename = "screener.pending")]
    ScreenerPending,
    #[serde(rename = "thread.updated")]
    ThreadUpdated,
    #[serde(rename = "bubble.fired")]
    BubbleFired,
    #[serde(rename = "send.completed")]
    SendCompleted,
    #[serde(rename = "send.failed")]
    SendFailed,
}

/// Cloneable in-process broadcast bus for app events.
#[derive(Clone, Debug)]
pub struct AppEventBus {
    sender: broadcast::Sender<AppEvent>,
}

impl AppEventBus {
    const DEFAULT_CAPACITY: usize = 256;

    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    /// Internal sender exposed for future worker/API bridging and for
    /// API handlers that need to emit events after local state changes.
    pub fn sender(&self) -> broadcast::Sender<AppEvent> {
        self.sender.clone()
    }

    pub fn publish(&self, event: AppEvent) -> Result<usize, broadcast::error::SendError<AppEvent>> {
        self.sender.send(event)
    }
}

impl Default for AppEventBus {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

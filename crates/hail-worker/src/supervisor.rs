//! Placeholder supervisor task.
//!
//! In the final design (design.md §8.1) the supervisor will own the set of
//! per-user EventSource tasks and the scheduler. For now it's a heartbeat
//! that proves the runtime is wired up and the cancellation token is honored.
//!
//! The tick cadence is read from `HAIL_TICK_SECS` (default 30). It is kept
//! deliberately *outside* the TOML config: it's a debug knob for smoke tests
//! and ops sessions, not persistent operator configuration.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::state::AppState;

/// Default tick cadence; matches design.md §8.1 item 4 (scheduler 60s) cut
/// in half so heartbeat is visible without spamming.
const DEFAULT_TICK_SECS: u64 = 30;

/// Resolve the tick cadence from `HAIL_TICK_SECS`, with a sane default.
fn tick_secs() -> u64 {
    env::var("HAIL_TICK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICK_SECS)
}

/// Run the supervisor loop until `cancel` is triggered.
///
/// Each iteration races a `sleep(tick)` against `cancel.cancelled()`, so a
/// shutdown signal preempts the sleep instead of waiting up to a full tick.
pub async fn run(_state: Arc<AppState>, cancel: CancellationToken) -> Result<()> {
    let secs = tick_secs();
    let tick = Duration::from_secs(secs);
    info!(tick_secs = secs, "supervisor: starting");

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!("supervisor: cancellation received, exiting");
                return Ok(());
            }
            _ = sleep(tick) => {
                info!("supervisor tick");
            }
        }
    }
}

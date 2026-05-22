//! Placeholder supervisor task.
//!
//! In the final design (design.md §8.1) the supervisor will own the set of
//! per-user EventSource tasks and the scheduler. For now it's a heartbeat
//! that proves the runtime is wired up and the cancellation token is honored.
//!
//! The tick cadence comes from `AppState.config.tick_secs` (env
//! `HAIL_TICK_SECS`, default 30). Smoke tests set it low so they don't have
//! to wait a full tick to see liveness.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::state::AppState;

/// Run the supervisor loop until `cancel` is triggered.
///
/// Each iteration races a `sleep(tick)` against `cancel.cancelled()`, so a
/// shutdown signal preempts the sleep instead of waiting up to a full tick.
pub async fn run(state: Arc<AppState>, cancel: CancellationToken) -> Result<()> {
    let tick = Duration::from_secs(state.config.tick_secs);
    info!(tick_secs = state.config.tick_secs, "supervisor: starting");

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

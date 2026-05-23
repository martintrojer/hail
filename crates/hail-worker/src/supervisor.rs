//! Top-level supervisor: drive per-user EventSource tasks.
//!
//! Implements design.md §8.1 / §8.2's "one tokio task per active user"
//! shape. Every `HAIL_TICK_SECS` seconds we re-query the set of active
//! users and reconcile:
//!   - new active user  -> spawn `run_user_supervisor`
//!   - removed user     -> cancel the supervisor's token
//!   - existing user    -> leave alone (do not restart)
//!
//! The set of running supervisors lives in a single `HashMap` owned by
//! this task, so there's no cross-task synchronization beyond the
//! `JoinSet` that holds the per-user `JoinHandle`s. On shutdown we
//! cancel the parent token and `join_all`.

use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::reconcile::{LiveThreadVerifier, process_reconciliation};
use crate::scheduler::{LiveBubbleJmapOps, process_due_bubble_ups};
use crate::state::AppState;
use crate::user::run_user_supervisor;

/// Default tick cadence. Design.md §8.1 item 4 cites 60s; the env
/// knob exists for smoke tests (the task contract bakes `HAIL_TICK_SECS=1`
/// into the SIGINT verification).
const DEFAULT_TICK_SECS: u64 = 60;

fn tick_secs() -> u64 {
    env::var("HAIL_TICK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TICK_SECS)
}

/// Nightly reconciliation must not inherit short smoke-test ticks. Keep this
/// coarse unless explicitly overridden for dev/tests.
const DEFAULT_RECONCILE_EVERY_SECS: u64 = 24 * 60 * 60;

fn reconcile_every_secs() -> u64 {
    env::var("HAIL_RECONCILE_EVERY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_RECONCILE_EVERY_SECS)
}

/// Run the top-level supervisor loop until `cancel` is triggered.
pub async fn run(state: Arc<AppState>, cancel: CancellationToken) -> Result<()> {
    let secs = tick_secs();
    let tick = Duration::from_secs(secs);
    let reconcile_secs = reconcile_every_secs();
    info!(
        tick_secs = secs,
        reconcile_every_secs = reconcile_secs,
        "supervisor: starting"
    );

    let mut running: HashMap<i64, CancellationToken> = HashMap::new();
    let mut tasks: JoinSet<()> = JoinSet::new();
    let bubble_jmap = LiveBubbleJmapOps::new(
        state.db.clone(),
        state.config.stalwart.jmap_url.clone(),
        state.token_decryptor.clone(),
    );
    let thread_verifier = LiveThreadVerifier::new(
        state.db.clone(),
        state.config.stalwart.jmap_url.clone(),
        state.token_decryptor.clone(),
    );
    let mut next_reconcile_at = chrono::Utc::now();

    // Reconcile immediately so the first tick doesn't burn a full
    // cadence before users come online. Also run scheduled jobs once
    // so overdue bubble-ups fire promptly after worker restart.
    reconcile(&state, &cancel, &mut running, &mut tasks).await;
    run_scheduler_tick(&state, &bubble_jmap).await;
    run_reconciliation_if_due(&state, &thread_verifier, &mut next_reconcile_at, reconcile_secs)
        .await;

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!("supervisor: cancellation received");
                break;
            }
            _ = sleep(tick) => {
                reconcile(&state, &cancel, &mut running, &mut tasks).await;
                run_scheduler_tick(&state, &bubble_jmap).await;
                run_reconciliation_if_due(&state, &thread_verifier, &mut next_reconcile_at, reconcile_secs)
                    .await;
            }
            // Reap any finished per-user tasks so the JoinSet doesn't
            // grow unbounded across user churn.
            Some(res) = tasks.join_next() => {
                if let Err(e) = res {
                    warn!(error = %e, "per-user supervisor task panicked");
                }
            }
        }
    }

    // Cancel all per-user supervisors. The per-user supervisors hold
    // a top-level `select!` against their cancel token, so dropping
    // their futures via cancel propagation tears down any in-flight
    // reqwest connection immediately (via `Drop`).
    for (_uid, token) in running.drain() {
        token.cancel();
    }

    // Bounded drain (5s per task contract). After the deadline we
    // `abort_all` so a stuck `.await` outside any `select!` can't
    // block process exit. join_next never returns errors for
    // aborted tasks beyond JoinError::is_cancelled, which we ignore.
    let drain = async {
        while let Some(res) = tasks.join_next().await {
            match res {
                Ok(()) => {}
                Err(e) if e.is_cancelled() => {}
                Err(e) => warn!(error = %e, "per-user task error on shutdown"),
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(5), drain).await.is_err() {
        warn!("supervisor: per-user tasks did not finish within 5s, aborting");
        tasks.abort_all();
        // Drain JoinErrors from the abort so we don't leak handles.
        while let Some(res) = tasks.join_next().await {
            if let Err(e) = res {
                if !e.is_cancelled() {
                    warn!(error = %e, "per-user task error after abort");
                }
            }
        }
    }

    info!("supervisor: shutdown complete");
    Ok(())
}

async fn run_scheduler_tick(state: &AppState, bubble_jmap: &LiveBubbleJmapOps) {
    match process_due_bubble_ups(&state.db, bubble_jmap, chrono::Utc::now()).await {
        Ok(fired) if fired > 0 => info!(fired, "scheduler: bubble-ups processed"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "scheduler: bubble-up tick failed"),
    }
}

async fn run_reconciliation_if_due(
    state: &AppState,
    verifier: &LiveThreadVerifier,
    next_run_at: &mut chrono::DateTime<chrono::Utc>,
    every_secs: u64,
) {
    let now = chrono::Utc::now();
    if now < *next_run_at {
        return;
    }

    let interval = chrono::Duration::seconds(every_secs.max(1) as i64);
    *next_run_at = now + interval;

    match process_reconciliation(&state.db, verifier, now).await {
        Ok(report) => info!(
            users_checked = report.users_checked,
            thread_ids_checked = report.thread_ids_checked,
            stack_positions_checked = report.stack_positions_checked,
            stack_positions_deleted = report.stack_positions_deleted,
            bubble_ups_checked = report.bubble_ups_checked,
            bubble_ups_deleted = report.bubble_ups_deleted,
            "reconciliation: sidecar thread refs processed"
        ),
        Err(e) => warn!(error = %e, "reconciliation: tick failed"),
    }
}

/// Reconcile `running` against the current set of active users in
/// the DB: spawn newcomers, cancel departures. Errors during the DB
/// query are logged but don't abort the supervisor.
async fn reconcile(
    state: &Arc<AppState>,
    parent_cancel: &CancellationToken,
    running: &mut HashMap<i64, CancellationToken>,
    tasks: &mut JoinSet<()>,
) {
    let active = match active_user_ids(&state.db).await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(error = %e, "supervisor: active-user query failed");
            return;
        }
    };

    // Spawn for any user we're not already running.
    for uid in &active {
        if running.contains_key(uid) {
            continue;
        }
        let child = parent_cancel.child_token();
        let task_state = Arc::clone(state);
        let task_cancel = child.clone();
        let user_id = *uid;
        info!(user_id, "supervisor: spawning per-user task");
        tasks.spawn(async move {
            // Per-user panic = log + exit *that* task only. The
            // top-level supervisor reads the JoinError below.
            match run_user_supervisor(user_id, task_state, task_cancel).await {
                Ok(()) => {}
                Err(e) => warn!(user_id, error = %e, "per-user supervisor exited with error"),
            }
        });
        running.insert(user_id, child);
    }

    // Cancel any user no longer active.
    let removed: Vec<i64> = running
        .keys()
        .copied()
        .filter(|uid| !active.contains(uid))
        .collect();
    for uid in removed {
        if let Some(token) = running.remove(&uid) {
            info!(user_id = uid, "supervisor: cancelling per-user task (user no longer active)");
            token.cancel();
        }
    }
}

/// "Active user" = at least one session row that hasn't expired yet.
/// Matches §6.2's `sessions.expires_at`.
async fn active_user_ids(db: &SqlitePool) -> Result<Vec<i64>> {
    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT DISTINCT user_id FROM sessions WHERE expires_at > ?",
    )
    .bind(now)
    .fetch_all(db)
    .await
    .context("select active users")?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

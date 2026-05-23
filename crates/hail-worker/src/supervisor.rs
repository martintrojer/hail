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
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::FutureExt;
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::reconcile::{LiveThreadVerifier, process_reconciliation};
use crate::scheduler::live::{LiveBubbleJmapOps, LiveSendSubmitter};
use crate::scheduler::{process_due_bubble_ups, process_due_scheduled_sends};
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

    let mut running: HashMap<i64, RunningUserTask> = HashMap::new();
    let mut tasks: JoinSet<(i64, u64)> = JoinSet::new();
    let mut next_run_id = 0;
    let bubble_jmap = LiveBubbleJmapOps::new(
        state.db.clone(),
        state.config.stalwart.jmap_url.clone(),
        state.token_decryptor.clone(),
    );
    let send_submitter = LiveSendSubmitter::new(
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
    // so overdue bubble-ups and scheduled sends fire promptly after worker restart.
    reconcile(&state, &cancel, &mut running, &mut tasks, &mut next_run_id).await;
    run_scheduler_tick(&state, &bubble_jmap, &send_submitter).await;
    run_reconciliation_if_due(
        &state,
        &thread_verifier,
        &mut next_reconcile_at,
        reconcile_secs,
    )
    .await;

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!("supervisor: cancellation received");
                break;
            }
            _ = sleep(tick) => {
                reap_finished_user_tasks(&mut running, &mut tasks);
                reconcile(
                    &state,
                    &cancel,
                    &mut running,
                    &mut tasks,
                    &mut next_run_id,
                )
                .await;
                run_scheduler_tick(&state, &bubble_jmap, &send_submitter).await;
                run_reconciliation_if_due(&state, &thread_verifier, &mut next_reconcile_at, reconcile_secs)
                    .await;
            }
            // Reap any finished per-user tasks so the JoinSet doesn't
            // grow unbounded across user churn.
            Some(res) = tasks.join_next() => {
                handle_finished_user_task(res, &mut running);
            }
        }
    }

    // Cancel all per-user supervisors. The per-user supervisors hold
    // a top-level `select!` against their cancel token, so dropping
    // their futures via cancel propagation tears down any in-flight
    // reqwest connection immediately (via `Drop`).
    for (_uid, task) in running.drain() {
        task.cancel.cancel();
    }

    // Bounded drain (5s per task contract). After the deadline we
    // `abort_all` so a stuck `.await` outside any `select!` can't
    // block process exit. join_next never returns errors for
    // aborted tasks beyond JoinError::is_cancelled, which we ignore.
    let drain = async {
        while let Some(res) = tasks.join_next().await {
            match res {
                Ok((_user_id, _run_id)) => {}
                Err(e) if e.is_cancelled() => {}
                Err(e) => warn!(error = %e, "per-user task error on shutdown"),
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .is_err()
    {
        warn!("supervisor: per-user tasks did not finish within 5s, aborting");
        tasks.abort_all();
        // Drain JoinErrors from the abort so we don't leak handles.
        while let Some(res) = tasks.join_next().await {
            if let Err(e) = res
                && !e.is_cancelled()
            {
                warn!(error = %e, "per-user task error after abort");
            }
        }
    }

    info!("supervisor: shutdown complete");
    Ok(())
}

async fn run_scheduler_tick(
    state: &AppState,
    bubble_jmap: &LiveBubbleJmapOps,
    send_submitter: &LiveSendSubmitter,
) {
    let now = chrono::Utc::now();
    match process_due_bubble_ups(&state.db, bubble_jmap, now).await {
        Ok(fired) if fired > 0 => info!(fired, "scheduler: bubble-ups processed"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "scheduler: bubble-up tick failed"),
    }

    match process_due_scheduled_sends(&state.db, send_submitter, now).await {
        Ok(sent) if sent > 0 => info!(sent, "scheduler: scheduled sends processed"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "scheduler: scheduled-send tick failed"),
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

struct RunningUserTask {
    cancel: CancellationToken,
    run_id: u64,
}

/// Reap all already-finished per-user tasks without blocking the supervisor
/// loop. A completed task means that user's old run is no longer actually
/// running, so it must be removed from the bookkeeping map before the next
/// active-user reconciliation decides whether to spawn it again.
fn reap_finished_user_tasks(
    running: &mut HashMap<i64, RunningUserTask>,
    tasks: &mut JoinSet<(i64, u64)>,
) {
    while let Some(res) = tasks.try_join_next() {
        handle_finished_user_task(res, running);
    }
}

fn handle_finished_user_task(
    res: std::result::Result<(i64, u64), tokio::task::JoinError>,
    running: &mut HashMap<i64, RunningUserTask>,
) {
    match res {
        Ok((user_id, run_id)) => {
            if running
                .get(&user_id)
                .is_some_and(|task| task.run_id == run_id)
            {
                running.remove(&user_id);
                info!(user_id, run_id, "supervisor: per-user task finished");
            }
        }
        Err(e) if e.is_cancelled() => {}
        Err(e) => warn!(error = %e, "per-user supervisor task join failed"),
    }
}

/// Reconcile `running` against the current set of active users in
/// the DB: spawn newcomers, cancel departures. Errors during the DB
/// query are logged but don't abort the supervisor.
async fn reconcile(
    state: &Arc<AppState>,
    parent_cancel: &CancellationToken,
    running: &mut HashMap<i64, RunningUserTask>,
    tasks: &mut JoinSet<(i64, u64)>,
    next_run_id: &mut u64,
) {
    let active = match active_user_ids(&state.db).await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(error = %e, "supervisor: active-user query failed");
            return;
        }
    };

    reconcile_active_users(
        &active,
        parent_cancel,
        running,
        tasks,
        next_run_id,
        |user_id, run_id, child, tasks| {
            let task_state = Arc::clone(state);
            spawn_user_task(tasks, user_id, run_id, task_state, child);
        },
    );
}

fn reconcile_active_users(
    active: &[i64],
    parent_cancel: &CancellationToken,
    running: &mut HashMap<i64, RunningUserTask>,
    tasks: &mut JoinSet<(i64, u64)>,
    next_run_id: &mut u64,
    mut spawn: impl FnMut(i64, u64, CancellationToken, &mut JoinSet<(i64, u64)>),
) {
    // Spawn for any user we're not already running.
    for uid in active {
        if running.contains_key(uid) {
            continue;
        }
        let child = parent_cancel.child_token();
        let user_id = *uid;
        *next_run_id = next_run_id.wrapping_add(1);
        let run_id = *next_run_id;
        info!(user_id, run_id, "supervisor: spawning per-user task");
        spawn(user_id, run_id, child.clone(), tasks);
        running.insert(
            user_id,
            RunningUserTask {
                cancel: child,
                run_id,
            },
        );
    }

    // Cancel any user no longer active.
    let removed: Vec<i64> = running
        .keys()
        .copied()
        .filter(|uid| !active.contains(uid))
        .collect();
    for uid in removed {
        if let Some(task) = running.remove(&uid) {
            info!(
                user_id = uid,
                run_id = task.run_id,
                "supervisor: cancelling per-user task (user no longer active)"
            );
            task.cancel.cancel();
        }
    }
}

fn spawn_user_task(
    tasks: &mut JoinSet<(i64, u64)>,
    user_id: i64,
    run_id: u64,
    task_state: Arc<AppState>,
    task_cancel: CancellationToken,
) {
    tasks.spawn(async move {
        // Per-user panic = log + exit *that* task only. Catching
        // unwind inside the task lets the top-level supervisor still
        // learn which user/run finished and clear `running` for retry.
        match AssertUnwindSafe(run_user_supervisor(user_id, task_state, task_cancel))
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(user_id, error = %e, "per-user supervisor exited with error"),
            Err(_) => warn!(user_id, "per-user supervisor task panicked"),
        }
        (user_id, run_id)
    });
}

/// "Active user" = at least one session row that hasn't expired yet.
/// Matches §6.2's `sessions.expires_at`.
async fn active_user_ids(db: &SqlitePool) -> Result<Vec<i64>> {
    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<(i64,)> =
        sqlx::query_as("SELECT DISTINCT user_id FROM sessions WHERE expires_at > ?")
            .bind(now)
            .fetch_all(db)
            .await
            .context("select active users")?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reap_finished_user_task_removes_running_entry() {
        let parent_cancel = CancellationToken::new();
        let mut running = HashMap::new();
        let mut tasks = JoinSet::new();
        let mut next_run_id = 0;

        reconcile_active_users(
            &[42],
            &parent_cancel,
            &mut running,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, _cancel, tasks| {
                tasks.spawn(async move { (user_id, run_id) });
            },
        );
        assert!(running.contains_key(&42));

        tokio::task::yield_now().await;
        reap_finished_user_tasks(&mut running, &mut tasks);

        assert!(!running.contains_key(&42));
    }

    #[tokio::test]
    async fn active_user_respawns_after_finished_task_is_reaped() {
        let parent_cancel = CancellationToken::new();
        let mut running = HashMap::new();
        let mut tasks = JoinSet::new();
        let mut next_run_id = 0;
        let mut spawned = Vec::new();

        reconcile_active_users(
            &[7],
            &parent_cancel,
            &mut running,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, _cancel, tasks| {
                spawned.push((user_id, run_id));
                tasks.spawn(async move { (user_id, run_id) });
            },
        );
        tokio::task::yield_now().await;
        reap_finished_user_tasks(&mut running, &mut tasks);

        reconcile_active_users(
            &[7],
            &parent_cancel,
            &mut running,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, _cancel, tasks| {
                spawned.push((user_id, run_id));
                tasks.spawn(async move { (user_id, run_id) });
            },
        );

        assert_eq!(spawned, vec![(7, 1), (7, 2)]);
        assert_eq!(running.get(&7).map(|task| task.run_id), Some(2));
    }

    #[tokio::test]
    async fn inactive_user_cancels_running_task() {
        let parent_cancel = CancellationToken::new();
        let mut running = HashMap::new();
        let mut tasks = JoinSet::new();
        let mut next_run_id = 0;
        let mut child = None;

        reconcile_active_users(
            &[99],
            &parent_cancel,
            &mut running,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, cancel, tasks| {
                child = Some(cancel.clone());
                tasks.spawn(async move {
                    cancel.cancelled().await;
                    (user_id, run_id)
                });
            },
        );
        reconcile_active_users(
            &[],
            &parent_cancel,
            &mut running,
            &mut tasks,
            &mut next_run_id,
            |_user_id, _run_id, _cancel, _tasks| unreachable!("no active users to spawn"),
        );

        assert!(running.is_empty());
        assert!(child.expect("child token").is_cancelled());
    }
}

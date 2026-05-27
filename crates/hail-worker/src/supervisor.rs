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
use std::future::Future;
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

use crate::provider_sync_scheduler::live::LiveProviderSyncRunner;
use crate::provider_sync_scheduler::{ProviderSyncSchedulerOptions, process_provider_sync_tick};
use crate::reconcile::{LiveThreadVerifier, process_reconciliation};
use crate::scheduler::live::{LiveBubbleJmapOps, LiveSendSubmitter};
use crate::scheduler::{
    DEFAULT_TRASH_RETENTION_DAYS, process_due_bubble_ups, process_due_scheduled_sends,
    process_spam_purge, process_trash_purge,
};
use crate::state::AppState;
use crate::user::{UserSupervisorExit, run_user_supervisor};

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

fn trash_retention_days() -> u16 {
    env::var("HAIL_TRASH_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_TRASH_RETENTION_DAYS)
}

/// Trash purge is safe to run hourly; avoid doing account-wide Trash scans on
/// every short dev/smoke tick.
const TRASH_PURGE_EVERY_SECS: i64 = 60 * 60;

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
    let mut terminal_users = TerminalUsers::default();
    let mut tasks: JoinSet<UserTaskExit> = JoinSet::new();
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
    let provider_sync_runner = LiveProviderSyncRunner::new(
        state.db.clone(),
        &state.config.secrets.server_key,
        state.token_decryptor.clone(),
        state.config.provider_import.gmail.oauth_client_id.clone(),
        state
            .config
            .provider_import
            .gmail
            .oauth_client_secret
            .clone(),
        state.config.provider_import.gmail.oauth_token_url.clone(),
        state.config.provider_import.gmail.api_base_url.clone(),
        state.config.stalwart.jmap_url.clone(),
        state
            .config
            .provider_import
            .gmail
            .initial_import_max_messages,
    )?;
    let trash_retention_days = trash_retention_days();
    let mut next_trash_purge_at = chrono::Utc::now();
    let mut next_spam_purge_at = chrono::Utc::now();
    let mut next_reconcile_at = chrono::Utc::now();

    // Reconcile immediately so the first tick doesn't burn a full
    // cadence before users come online. Also run scheduled jobs once
    // so overdue bubble-ups and scheduled sends fire promptly after worker restart.
    if reconcile(
        &state,
        &cancel,
        &mut running,
        &mut terminal_users,
        &mut tasks,
        &mut next_run_id,
    )
    .await
        && run_scheduler_tick(
            &state,
            &bubble_jmap,
            &send_submitter,
            &provider_sync_runner,
            &mut next_trash_purge_at,
            &mut next_spam_purge_at,
            trash_retention_days,
            &cancel,
        )
        .await
    {
        run_reconciliation_if_due(
            &state,
            &thread_verifier,
            &mut next_reconcile_at,
            reconcile_secs,
            &cancel,
        )
        .await;
    }

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!("supervisor: cancellation received");
                break;
            }
            _ = sleep(tick) => {
                reap_finished_user_tasks(&mut running, &mut tasks, &mut terminal_users);
                if !reconcile(
                    &state,
                    &cancel,
                    &mut running,
                    &mut terminal_users,
                    &mut tasks,
                    &mut next_run_id,
                )
                .await
                {
                    break;
                }
                if !run_scheduler_tick(
                    &state,
                    &bubble_jmap,
                    &send_submitter,
                    &provider_sync_runner,
                    &mut next_trash_purge_at,
                    &mut next_spam_purge_at,
                    trash_retention_days,
                    &cancel,
                ).await {
                    break;
                }
                if !run_reconciliation_if_due(
                    &state,
                    &thread_verifier,
                    &mut next_reconcile_at,
                    reconcile_secs,
                    &cancel,
                )
                .await
                {
                    break;
                }
            }
            // Reap any finished per-user tasks so the JoinSet doesn't
            // grow unbounded across user churn.
            Some(res) = tasks.join_next() => {
                handle_finished_user_task(res, &mut running, &mut terminal_users);
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
                Ok(_exit) => {}
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
    provider_sync_runner: &LiveProviderSyncRunner,
    next_trash_purge_at: &mut chrono::DateTime<chrono::Utc>,
    next_spam_purge_at: &mut chrono::DateTime<chrono::Utc>,
    trash_retention_days: u16,
    cancel: &CancellationToken,
) -> bool {
    let now = chrono::Utc::now();
    match cancel_or_complete(cancel, process_due_bubble_ups(&state.db, bubble_jmap, now)).await {
        Some(Ok(fired)) if fired > 0 => info!(fired, "scheduler: bubble-ups processed"),
        Some(Ok(_)) => {}
        Some(Err(e)) => warn!(error = %e, "scheduler: bubble-up tick failed"),
        None => {
            info!("scheduler: bubble-up tick cancelled");
            return false;
        }
    }

    match cancel_or_complete(
        cancel,
        process_due_scheduled_sends(&state.db, send_submitter, now),
    )
    .await
    {
        Some(Ok(sent)) if sent > 0 => info!(sent, "scheduler: scheduled sends processed"),
        Some(Ok(_)) => {}
        Some(Err(e)) => warn!(error = %e, "scheduler: scheduled-send tick failed"),
        None => {
            info!("scheduler: scheduled-send tick cancelled");
            return false;
        }
    }

    match cancel_or_complete(
        cancel,
        process_provider_sync_tick(
            &state.db,
            provider_sync_runner,
            now,
            ProviderSyncSchedulerOptions::default(),
            cancel,
        ),
    )
    .await
    {
        Some(Ok(provider)) if provider.succeeded > 0 || provider.failed > 0 => info!(
            considered = provider.considered,
            initial_runs = provider.initial_runs,
            incremental_runs = provider.incremental_runs,
            succeeded = provider.succeeded,
            failed = provider.failed,
            "scheduler: provider sync processed"
        ),
        Some(Ok(_)) => {}
        Some(Err(e)) => warn!(error = %e, "scheduler: provider sync tick failed"),
        None => {
            info!("scheduler: provider sync tick cancelled");
            return false;
        }
    }

    if now >= *next_trash_purge_at {
        *next_trash_purge_at = now + chrono::Duration::seconds(TRASH_PURGE_EVERY_SECS);
        match cancel_or_complete(
            cancel,
            process_trash_purge(&state.db, bubble_jmap, trash_retention_days, now),
        )
        .await
        {
            Some(Ok(purged)) if purged > 0 => info!(purged, "scheduler: trash purge processed"),
            Some(Ok(_)) => {}
            Some(Err(e)) => warn!(error = %e, "scheduler: trash purge tick failed"),
            None => {
                info!("scheduler: trash purge tick cancelled");
                return false;
            }
        }
    }

    if now >= *next_spam_purge_at {
        *next_spam_purge_at = now + chrono::Duration::seconds(TRASH_PURGE_EVERY_SECS);
        match cancel_or_complete(cancel, process_spam_purge(&state.db, bubble_jmap, now)).await {
            Some(Ok(purged)) if purged > 0 => info!(purged, "scheduler: spam purge processed"),
            Some(Ok(_)) => {}
            Some(Err(e)) => warn!(error = %e, "scheduler: spam purge tick failed"),
            None => {
                info!("scheduler: spam purge tick cancelled");
                return false;
            }
        }
    }

    true
}

async fn run_reconciliation_if_due(
    state: &AppState,
    verifier: &LiveThreadVerifier,
    next_run_at: &mut chrono::DateTime<chrono::Utc>,
    every_secs: u64,
    cancel: &CancellationToken,
) -> bool {
    let now = chrono::Utc::now();
    if now < *next_run_at {
        return true;
    }

    let interval = chrono::Duration::seconds(every_secs.max(1) as i64);
    *next_run_at = now + interval;

    match cancel_or_complete(cancel, process_reconciliation(&state.db, verifier, now)).await {
        Some(Ok(report)) => info!(
            users_checked = report.users_checked,
            thread_ids_checked = report.thread_ids_checked,
            stack_positions_checked = report.stack_positions_checked,
            stack_positions_deleted = report.stack_positions_deleted,
            bubble_ups_checked = report.bubble_ups_checked,
            bubble_ups_deleted = report.bubble_ups_deleted,
            "reconciliation: sidecar thread refs processed"
        ),
        Some(Err(e)) => warn!(error = %e, "reconciliation: tick failed"),
        None => {
            info!("reconciliation: tick cancelled");
            return false;
        }
    }

    true
}

async fn cancel_or_complete<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        output = future => Some(output),
    }
}

struct UserTaskExit {
    user_id: i64,
    run_id: u64,
    outcome: UserSupervisorExit,
}

struct RunningUserTask {
    cancel: CancellationToken,
    run_id: u64,
}

struct TerminalUsers(std::collections::HashSet<i64>);

impl TerminalUsers {
    fn contains(&self, user_id: i64) -> bool {
        self.0.contains(&user_id)
    }

    fn mark(&mut self, user_id: i64) {
        self.0.insert(user_id);
    }

    fn remove(&mut self, user_id: i64) {
        self.0.remove(&user_id);
    }

    fn retain_active(&mut self, active: &[i64]) {
        self.0.retain(|user_id| active.contains(user_id));
    }
}

impl Default for TerminalUsers {
    fn default() -> Self {
        Self(std::collections::HashSet::new())
    }
}

/// Reap all already-finished per-user tasks without blocking the supervisor
/// loop. A completed task means that user's old run is no longer actually
/// running, so it must be removed from the bookkeeping map before the next
/// active-user reconciliation decides whether to spawn it again.
fn reap_finished_user_tasks(
    running: &mut HashMap<i64, RunningUserTask>,
    tasks: &mut JoinSet<UserTaskExit>,
    terminal_users: &mut TerminalUsers,
) {
    while let Some(res) = tasks.try_join_next() {
        handle_finished_user_task(res, running, terminal_users);
    }
}

fn handle_finished_user_task(
    res: std::result::Result<UserTaskExit, tokio::task::JoinError>,
    running: &mut HashMap<i64, RunningUserTask>,
    terminal_users: &mut TerminalUsers,
) {
    match res {
        Ok(UserTaskExit {
            user_id,
            run_id,
            outcome,
        }) => {
            if running
                .get(&user_id)
                .is_some_and(|task| task.run_id == run_id)
            {
                running.remove(&user_id);
                match outcome {
                    UserSupervisorExit::Terminal => {
                        terminal_users.mark(user_id);
                        warn!(
                            user_id,
                            run_id, "supervisor: per-user task exited terminally"
                        );
                    }
                    UserSupervisorExit::Retryable => {
                        terminal_users.remove(user_id);
                        info!(user_id, run_id, "supervisor: per-user task finished");
                    }
                }
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
    terminal_users: &mut TerminalUsers,
    tasks: &mut JoinSet<UserTaskExit>,
    next_run_id: &mut u64,
) -> bool {
    let active = match cancel_or_complete(parent_cancel, active_user_ids(&state.db)).await {
        Some(Ok(ids)) => ids,
        Some(Err(e)) => {
            warn!(error = %e, "supervisor: active-user query failed");
            return true;
        }
        None => {
            info!("supervisor: active-user reconciliation cancelled");
            return false;
        }
    };

    reconcile_active_users(
        &active,
        parent_cancel,
        running,
        terminal_users,
        tasks,
        next_run_id,
        |user_id, run_id, child, tasks| {
            let task_state = Arc::clone(state);
            spawn_user_task(tasks, user_id, run_id, task_state, child);
        },
    );

    true
}

fn reconcile_active_users(
    active: &[i64],
    parent_cancel: &CancellationToken,
    running: &mut HashMap<i64, RunningUserTask>,
    terminal_users: &mut TerminalUsers,
    tasks: &mut JoinSet<UserTaskExit>,
    next_run_id: &mut u64,
    mut spawn: impl FnMut(i64, u64, CancellationToken, &mut JoinSet<UserTaskExit>),
) {
    terminal_users.retain_active(active);

    // Spawn for any user we're not already running.
    for uid in active {
        if running.contains_key(uid) || terminal_users.contains(*uid) {
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
    tasks: &mut JoinSet<UserTaskExit>,
    user_id: i64,
    run_id: u64,
    task_state: Arc<AppState>,
    task_cancel: CancellationToken,
) {
    tasks.spawn(async move {
        // Per-user panic = log + exit *that* task only. Catching
        // unwind inside the task lets the top-level supervisor still
        // learn which user/run finished and clear `running` for retry.
        let outcome = match AssertUnwindSafe(run_user_supervisor(user_id, task_state, task_cancel))
            .catch_unwind()
            .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(e)) => {
                warn!(user_id, error = %e, "per-user supervisor exited with error");
                UserSupervisorExit::Retryable
            }
            Err(_) => {
                warn!(user_id, "per-user supervisor task panicked");
                UserSupervisorExit::Retryable
            }
        };
        UserTaskExit {
            user_id,
            run_id,
            outcome,
        }
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
    async fn cancel_or_complete_returns_none_when_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();

        let result = tokio::time::timeout(
            Duration::from_millis(50),
            cancel_or_complete(&cancel, std::future::pending::<()>()),
        )
        .await
        .expect("cancel branch should complete promptly");

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn cancel_or_complete_returns_completed_output() {
        let cancel = CancellationToken::new();

        let result = cancel_or_complete(&cancel, async { 7 }).await;

        assert_eq!(result, Some(7));
    }

    #[tokio::test]
    async fn reap_finished_user_task_removes_running_entry() {
        let parent_cancel = CancellationToken::new();
        let mut running = HashMap::new();
        let mut tasks = JoinSet::new();
        let mut next_run_id = 0;

        let mut terminal_users = TerminalUsers::default();
        reconcile_active_users(
            &[42],
            &parent_cancel,
            &mut running,
            &mut terminal_users,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, _cancel, tasks| {
                tasks.spawn(async move {
                    UserTaskExit {
                        user_id,
                        run_id,
                        outcome: UserSupervisorExit::Retryable,
                    }
                });
            },
        );
        assert!(running.contains_key(&42));

        tokio::task::yield_now().await;
        reap_finished_user_tasks(&mut running, &mut tasks, &mut terminal_users);

        assert!(!running.contains_key(&42));
    }

    #[tokio::test]
    async fn active_user_respawns_after_finished_task_is_reaped() {
        let parent_cancel = CancellationToken::new();
        let mut running = HashMap::new();
        let mut tasks = JoinSet::new();
        let mut next_run_id = 0;
        let mut spawned = Vec::new();

        let mut terminal_users = TerminalUsers::default();
        reconcile_active_users(
            &[7],
            &parent_cancel,
            &mut running,
            &mut terminal_users,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, _cancel, tasks| {
                spawned.push((user_id, run_id));
                tasks.spawn(async move {
                    UserTaskExit {
                        user_id,
                        run_id,
                        outcome: UserSupervisorExit::Retryable,
                    }
                });
            },
        );
        tokio::task::yield_now().await;
        reap_finished_user_tasks(&mut running, &mut tasks, &mut terminal_users);

        reconcile_active_users(
            &[7],
            &parent_cancel,
            &mut running,
            &mut terminal_users,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, _cancel, tasks| {
                spawned.push((user_id, run_id));
                tasks.spawn(async move {
                    UserTaskExit {
                        user_id,
                        run_id,
                        outcome: UserSupervisorExit::Retryable,
                    }
                });
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

        let mut terminal_users = TerminalUsers::default();
        reconcile_active_users(
            &[99],
            &parent_cancel,
            &mut running,
            &mut terminal_users,
            &mut tasks,
            &mut next_run_id,
            |user_id, run_id, cancel, tasks| {
                child = Some(cancel.clone());
                tasks.spawn(async move {
                    cancel.cancelled().await;
                    UserTaskExit {
                        user_id,
                        run_id,
                        outcome: UserSupervisorExit::Retryable,
                    }
                });
            },
        );
        reconcile_active_users(
            &[],
            &parent_cancel,
            &mut running,
            &mut terminal_users,
            &mut tasks,
            &mut next_run_id,
            |_user_id, _run_id, _cancel, _tasks| unreachable!("no active users to spawn"),
        );

        assert!(running.is_empty());
        assert!(child.expect("child token").is_cancelled());
    }
}

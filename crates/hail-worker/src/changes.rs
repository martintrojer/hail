//! Per-user JMAP `Email/changes` consumer.
//!
//! See design.md §8.1 item 2 ("Inbound routing") and §6.2 (the
//! `jmap_state` table). This module is intentionally split from
//! `user.rs` so the change-diff -> envelope-fetch -> log -> persist
//! pipeline is unit-testable against a fake JMAP backend without
//! standing up a real `Client`.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::app_events::{WorkerAppEvent, publish_app_event};
use crate::screener::{self, JmapOps, RouteOutcome, route_email};

/// JMAP TypeStates that hail-worker tracks per user. Mirrors
/// design.md §6.2 (`jmap_state.type_state` column) and §8.1 item 1
/// (subscribed EventSource types).
pub const TRACKED_TYPE_STATES: &[&str] = &["Email", "EmailDelivery", "Mailbox", "EmailSubmission"];

/// Minimal envelope of an `Email/get` row. Carries exactly the
/// properties named in the task contract (Id, ThreadId, ReceivedAt,
/// From, Subject, Preview, Keywords, MailboxIds, Size). NO bodies,
/// NO attachments — logging hygiene (design.md §10.1).
///
/// `received_at` and `preview` are populated but not surfaced in the
/// log line yet; screener-routing reads them.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct EmailEnvelope {
    pub id: String,
    pub thread_id: Option<String>,
    pub received_at: Option<i64>,
    pub from: Vec<(Option<String>, String)>,
    pub subject: Option<String>,
    pub preview: Option<String>,
    pub keywords: Vec<String>,
    pub mailbox_ids: Vec<String>,
    pub size: usize,
}

/// Outcome of an `Email/changes` round-trip: the new state cursor
/// plus the set of envelopes we resolved for created/updated ids.
#[derive(Debug, Clone, Default)]
pub struct EmailChanges {
    pub new_state: String,
    pub created: Vec<EmailEnvelope>,
    pub updated: Vec<EmailEnvelope>,
    pub destroyed: Vec<String>,
}

impl EmailChanges {
    /// True if no ids changed in either direction. Empty rounds still
    /// advance the cursor (the server can hand us a new state even
    /// when nothing user-visible changed). Used by tests; downstream
    /// screener-routing will likely use it for fast-paths too.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.updated.is_empty() && self.destroyed.is_empty()
    }

    #[must_use]
    pub fn total_changes(&self) -> usize {
        self.created.len() + self.updated.len() + self.destroyed.len()
    }
}

/// Backend that turns "give me changes since `cursor`" into resolved
/// envelopes. The real impl wraps `jmap_client::client::Client`; tests
/// drive an in-memory fake. The trait is intentionally narrow — the
/// supervisor never touches the JMAP request builders directly, all
/// shaping lives in the impl.
#[async_trait]
pub trait JmapChangeFetcher: Send + Sync {
    /// Fetch the current state token without replaying history. Used
    /// only when no `jmap_state` row exists yet for a first-run user.
    #[allow(dead_code)]
    async fn current_state(&self, type_state: &str) -> Result<String>;

    /// Fetch the diff for `type_state` since `since_cursor`, then
    /// resolve created+updated ids to envelopes via `Email/get`. The
    /// `since_cursor` is the opaque state token stored in
    /// `jmap_state.state`; an empty string means "first run, fetch
    /// current state without diff".
    async fn fetch(&self, type_state: &str, since_cursor: &str) -> Result<EmailChanges>;
}

/// Top-level entry point. For each TypeState that the EventSource
/// signalled as changed, fetch the diff + envelopes, log each
/// envelope, then UPSERT the new cursor into `jmap_state`.
///
/// `changed_types`: which TypeStates we should walk this round. On
/// the supervisor's initial sync (before any push event) we walk
/// every entry in [`TRACKED_TYPE_STATES`]. On a live push event we
/// walk only what JMAP told us changed.
#[cfg_attr(test, allow(dead_code))]
pub async fn handle_changes(
    db: &SqlitePool,
    user_id: i64,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &dyn JmapOps,
    changed_types: &BTreeSet<String>,
) -> Result<usize> {
    handle_changes_with_mode(
        db,
        user_id,
        fetcher,
        jmap_ops,
        changed_types,
        ErrorMode::LogAndContinue,
    )
    .await
}

/// Strict variant used by startup/reconnect catch-up. Any TypeState fetch
/// failure aborts the replay so the per-user supervisor backs off instead of
/// opening EventSource with an unreplayed persisted cursor.
pub async fn handle_changes_strict(
    db: &SqlitePool,
    user_id: i64,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &dyn JmapOps,
    changed_types: &BTreeSet<String>,
) -> Result<usize> {
    handle_changes_with_mode(
        db,
        user_id,
        fetcher,
        jmap_ops,
        changed_types,
        ErrorMode::ReturnError,
    )
    .await
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Debug, Clone, Copy)]
enum ErrorMode {
    LogAndContinue,
    ReturnError,
}

async fn handle_changes_with_mode(
    db: &SqlitePool,
    user_id: i64,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &dyn JmapOps,
    changed_types: &BTreeSet<String>,
    error_mode: ErrorMode,
) -> Result<usize> {
    let mut applied = 0usize;
    for type_state in changed_types {
        if !TRACKED_TYPE_STATES.contains(&type_state.as_str()) {
            // EventSource can deliver types we don't care about
            // (Identity, Core, ...). Skip rather than error.
            continue;
        }

        let cursor = load_cursor(db, user_id, type_state).await?;
        let changes = match fetcher.fetch(type_state, &cursor).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    user_id,
                    type_state = %type_state,
                    error = %e,
                    "Email/changes round failed; will retry on next event"
                );
                match error_mode {
                    ErrorMode::LogAndContinue => continue,
                    ErrorMode::ReturnError => {
                        return Err(e)
                            .with_context(|| format!("fetch {type_state} changes during catchup"));
                    }
                }
            }
        };

        route_envelopes(db, user_id, type_state, jmap_ops, &changes).await?;
        applied += changes.total_changes();

        if changes.new_state.is_empty() {
            // Defensive: a fetcher that returns an empty new_state
            // would silently re-fetch the same window forever. Log
            // and skip the UPSERT.
            warn!(
                user_id,
                type_state = %type_state,
                "fetcher returned empty new_state; cursor not advanced"
            );
            continue;
        }

        upsert_cursor(db, user_id, type_state, &changes.new_state).await?;
    }
    Ok(applied)
}

/// Load the stored cursor for `(user_id, type_state)`, or empty
/// string if the row doesn't exist yet (first run).
pub async fn load_cursor(db: &SqlitePool, user_id: i64, type_state: &str) -> Result<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT state FROM jmap_state WHERE user_id = ? AND type_state = ?")
            .bind(user_id)
            .bind(type_state)
            .fetch_optional(db)
            .await
            .context("select jmap_state")?;
    Ok(row.map(|(s,)| s).unwrap_or_default())
}

/// UPSERT the cursor. Updates `updated_at` to now on every write so
/// ops can spot stuck supervisors at a glance.
pub async fn upsert_cursor(
    db: &SqlitePool,
    user_id: i64,
    type_state: &str,
    new_state: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO jmap_state (user_id, type_state, state, updated_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(user_id, type_state) DO UPDATE SET \
           state = excluded.state, \
           updated_at = excluded.updated_at",
    )
    .bind(user_id)
    .bind(type_state)
    .bind(new_state)
    .bind(now)
    .execute(db)
    .await
    .context("upsert jmap_state")?;
    Ok(())
}

/// Route created/updated Email envelopes through Screener rules. Destroyed ids
/// are logged in a single line — we don't have envelopes for them.
///
/// Logging hygiene (design.md §10.1): INFO logs include ids and route outcomes
/// only. Subjects and full envelopes are debug-only material, not emitted here.
async fn route_envelopes(
    db: &SqlitePool,
    user_id: i64,
    type_state: &str,
    jmap_ops: &dyn JmapOps,
    changes: &EmailChanges,
) -> Result<()> {
    if type_state == "Email" {
        let mut conn = db.acquire().await.context("acquire sqlite connection")?;
        for env in changes.created.iter().chain(changes.updated.iter()) {
            let route_env = match route_envelope_from_change(env) {
                Some(route_env) => route_env,
                None => {
                    warn!(
                        user_id,
                        email_id = %env.id,
                        "skipping route for email without sender"
                    );
                    continue;
                }
            };
            match route_email(conn.as_mut(), jmap_ops, user_id, &route_env).await {
                Ok(outcome) => {
                    if let Some(event) = app_event_for_route_outcome(&outcome) {
                        if let Err(err) = publish_app_event(db, user_id, event).await {
                            warn!(
                                user_id,
                                email_id = %route_env.id,
                                event_type = %event.event_type(),
                                error = %err,
                                "failed to publish routed mail app event"
                            );
                        }
                    }
                    info!(
                        user_id,
                        email_id = %route_env.id,
                        outcome = ?outcome,
                        "routed"
                    );
                }
                Err(e) => {
                    warn!(
                        user_id,
                        email_id = %route_env.id,
                        error = %e,
                        "route_email failed; cursor not advanced so change will retry"
                    );
                    return Err(e)
                        .with_context(|| format!("route email {} through screener", route_env.id));
                }
            }
        }
    }

    if !changes.destroyed.is_empty() {
        if let Err(err) = publish_app_event(db, user_id, WorkerAppEvent::ThreadUpdated).await {
            warn!(
                user_id,
                type_state = %type_state,
                error = %err,
                "failed to publish destroyed ids app event"
            );
        }
        info!(
            user_id,
            type_state = %type_state,
            destroyed = ?changes.destroyed,
            "jmap destroyed ids"
        );
    }
    Ok(())
}

fn app_event_for_route_outcome(outcome: &RouteOutcome) -> Option<WorkerAppEvent> {
    match outcome {
        RouteOutcome::Classified { classification } => Some(match classification {
            hail_core::MailClassification::Imbox => WorkerAppEvent::ImboxNew,
            hail_core::MailClassification::Feed => WorkerAppEvent::FeedNew,
            hail_core::MailClassification::Papertrail => WorkerAppEvent::PapertrailNew,
        }),
        RouteOutcome::ScreenerPending { .. } => Some(WorkerAppEvent::ScreenerPending),
        RouteOutcome::SpeakeasyBypass => Some(WorkerAppEvent::ImboxNew),
        RouteOutcome::Trashed => Some(WorkerAppEvent::ThreadUpdated),
        RouteOutcome::Spam => Some(WorkerAppEvent::ThreadUpdated),
        RouteOutcome::AlreadyScreened => Some(WorkerAppEvent::ThreadUpdated),
    }
}

fn route_envelope_from_change(env: &EmailEnvelope) -> Option<screener::EmailEnvelope> {
    // Skip drafts — they are user-created, not incoming mail.
    if env.keywords.iter().any(|kw| kw == "$draft") {
        return None;
    }
    let from = env
        .from
        .first()
        .map(|(_, addr)| screener::normalize_sender(addr))?;
    if from.is_empty() {
        return None;
    }
    Some(screener::EmailEnvelope {
        id: env.id.clone(),
        thread_id: env.thread_id.clone().unwrap_or_default(),
        from,
        subject: env.subject.clone().unwrap_or_default(),
        preview: env.preview.clone(),
        raw_rfc822: None,
        mailbox_ids: env.mailbox_ids.clone(),
        keywords: env.keywords.clone(),
        received_at: env
            .received_at
            .and_then(|ts| chrono::DateTime::<Utc>::from_timestamp(ts, 0)),
        size: u32::try_from(env.size).ok(),
    })
}

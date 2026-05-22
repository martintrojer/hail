//! Per-user JMAP `Email/changes` consumer.
//!
//! See design.md §8.1 item 2 ("Inbound routing") and §6.2 (the
//! `jmap_state` table). This module is intentionally split from
//! `user.rs` so the change-diff -> envelope-fetch -> log -> persist
//! pipeline is unit-testable against a fake JMAP backend without
//! standing up a real `Client`.
//!
//! Screener routing logic is downstream (`screener-routing` task) and
//! NOT done here. For this task we log each envelope structured at
//! INFO and persist the new cursor.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::{info, warn};

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
}

/// Backend that turns "give me changes since `cursor`" into resolved
/// envelopes. The real impl wraps `jmap_client::client::Client`; tests
/// drive an in-memory fake. The trait is intentionally narrow — the
/// supervisor never touches the JMAP request builders directly, all
/// shaping lives in the impl.
#[async_trait]
pub trait JmapChangeFetcher: Send + Sync {
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
pub async fn handle_changes(
    db: &SqlitePool,
    user_id: i64,
    fetcher: &dyn JmapChangeFetcher,
    changed_types: &BTreeSet<String>,
) -> Result<()> {
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
                // Surface but don't abort the whole round — other
                // type_states may still succeed.
                warn!(
                    user_id,
                    type_state = %type_state,
                    error = %e,
                    "Email/changes round failed; will retry on next event"
                );
                continue;
            }
        };

        log_envelopes(user_id, type_state, &changes);

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
    Ok(())
}

/// Load the stored cursor for `(user_id, type_state)`, or empty
/// string if the row doesn't exist yet (first run).
pub async fn load_cursor(db: &SqlitePool, user_id: i64, type_state: &str) -> Result<String> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT state FROM jmap_state WHERE user_id = ? AND type_state = ?",
    )
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

/// Emit one INFO line per created/updated envelope. Destroyed ids
/// are logged in a single line — we don't have envelopes for them.
///
/// Logging hygiene (design.md §10.1): no bodies, no decrypted tokens,
/// preview is already a server-side excerpt and is safe to log.
fn log_envelopes(user_id: i64, type_state: &str, changes: &EmailChanges) {
    for env in changes.created.iter().chain(changes.updated.iter()) {
        let from = env
            .from
            .iter()
            .map(|(_, addr)| addr.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let mailboxes = env.mailbox_ids.join(",");
        let keywords = env.keywords.join(",");
        info!(
            user_id,
            type_state = %type_state,
            email_id = %env.id,
            thread_id = env.thread_id.as_deref().unwrap_or(""),
            from = %from,
            subject = env.subject.as_deref().unwrap_or(""),
            mailbox_ids = %mailboxes,
            keywords = %keywords,
            size = env.size,
            "jmap change envelope"
        );
    }
    if !changes.destroyed.is_empty() {
        info!(
            user_id,
            type_state = %type_state,
            destroyed = ?changes.destroyed,
            "jmap destroyed ids"
        );
    }
}

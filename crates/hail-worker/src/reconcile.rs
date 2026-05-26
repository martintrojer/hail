//! Nightly sidecar reconciliation for JMAP thread references.
//!
//! This job implements design.md §6 drift mitigation and §8.1's nightly
//! reconciliation pass. The sidecar is allowed to cache product state that
//! points at JMAP objects; JMAP remains source of truth. When a referenced
//! thread no longer exists, we prune the sidecar row so future UI/scheduler
//! work does not act on an orphan.
//!
//! Pending `bubble_ups` are deleted rather than marked fired. A deleted target
//! thread means there is no user-visible reminder left to deliver; stamping
//! `fired_at` would incorrectly imply the reminder fired successfully.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqlitePool, Transaction};

/// Verifies which JMAP Thread ids still exist for one hail user.
#[async_trait]
pub trait ThreadVerifier: Send + Sync {
    async fn existing_threads(&self, user_id: i64, ids: &[String]) -> Result<VerificationOutcome>;
}

/// Structured reconciliation counters logged by the supervisor and returned to
/// tests/callers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub users_checked: usize,
    pub users_unverifiable: usize,
    pub thread_ids_checked: usize,
    pub stack_positions_checked: usize,
    pub stack_positions_deleted: u64,
    pub bubble_ups_checked: usize,
    pub bubble_ups_deleted: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ThreadRef {
    table: SidecarTable,
    user_id: i64,
    thread_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SidecarTable {
    StackPositions,
    PendingBubbleUps,
}

#[derive(Debug)]
pub enum VerificationOutcome {
    Verified(HashSet<String>),
    Unverifiable(String),
}

/// Reconcile sidecar rows that point at missing JMAP Thread objects.
///
/// Scope: `stack_positions.thread_id` and pending `bubble_ups.thread_id`
/// (`fired_at IS NULL`). v1 has no `clips` table, so clips are intentionally
/// out of scope.
pub async fn process_reconciliation(
    db: &SqlitePool,
    verifier: &dyn ThreadVerifier,
    now: DateTime<Utc>,
) -> Result<ReconcileReport> {
    let _ = now;
    let refs = load_thread_refs(db).await?;
    let mut by_user: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    let mut report = ReconcileReport::default();

    for r in &refs {
        match r.table {
            SidecarTable::StackPositions => report.stack_positions_checked += 1,
            SidecarTable::PendingBubbleUps => report.bubble_ups_checked += 1,
        }
        by_user
            .entry(r.user_id)
            .or_default()
            .insert(r.thread_id.clone());
    }

    report.users_checked = by_user.len();
    report.thread_ids_checked = by_user.values().map(BTreeSet::len).sum();

    let mut missing_by_user: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (user_id, ids_set) in by_user {
        let ids: Vec<String> = ids_set.into_iter().collect();
        let outcome = verifier
            .existing_threads(user_id, &ids)
            .await
            .with_context(|| format!("verify JMAP threads for user {user_id}"))?;
        let existing = match outcome {
            VerificationOutcome::Verified(existing) => existing,
            VerificationOutcome::Unverifiable(reason) => {
                report.users_unverifiable += 1;
                tracing::warn!(
                    user_id,
                    reason = %reason,
                    "reconciliation skipped unverifiable user"
                );
                continue;
            }
        };
        let missing: Vec<String> = ids
            .into_iter()
            .filter(|id| !existing.contains(id))
            .collect();
        if !missing.is_empty() {
            missing_by_user.insert(user_id, missing);
        }
    }

    if missing_by_user.is_empty() {
        return Ok(report);
    }

    let mut tx = db
        .begin()
        .await
        .context("begin reconciliation transaction")?;
    for (user_id, missing_ids) in missing_by_user {
        for thread_id in missing_ids {
            report.stack_positions_deleted += delete_stack_positions(&mut tx, user_id, &thread_id)
                .await
                .with_context(|| {
                    format!("delete orphan stack_positions user={user_id} thread={thread_id}")
                })?;
            report.bubble_ups_deleted += delete_pending_bubble_ups(&mut tx, user_id, &thread_id)
                .await
                .with_context(|| {
                    format!("delete orphan pending bubble_ups user={user_id} thread={thread_id}")
                })?;
        }
    }
    tx.commit()
        .await
        .context("commit reconciliation transaction")?;

    Ok(report)
}

async fn load_thread_refs(db: &SqlitePool) -> Result<Vec<ThreadRef>> {
    let mut refs = Vec::new();

    let stack_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT user_id, thread_id FROM stack_positions ORDER BY user_id, thread_id",
    )
    .fetch_all(db)
    .await
    .context("select stack_positions thread refs")?;
    refs.extend(
        stack_rows
            .into_iter()
            .map(|(user_id, thread_id)| ThreadRef {
                table: SidecarTable::StackPositions,
                user_id,
                thread_id,
            }),
    );

    let bubble_rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT user_id, thread_id FROM bubble_ups WHERE fired_at IS NULL ORDER BY user_id, thread_id",
    )
    .fetch_all(db)
    .await
    .context("select pending bubble_ups thread refs")?;
    refs.extend(
        bubble_rows
            .into_iter()
            .map(|(user_id, thread_id)| ThreadRef {
                table: SidecarTable::PendingBubbleUps,
                user_id,
                thread_id,
            }),
    );

    Ok(refs)
}

async fn delete_stack_positions(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: i64,
    thread_id: &str,
) -> Result<u64> {
    let result = sqlx::query("DELETE FROM stack_positions WHERE user_id = ? AND thread_id = ?")
        .bind(user_id)
        .bind(thread_id)
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected())
}

async fn delete_pending_bubble_ups(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: i64,
    thread_id: &str,
) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM bubble_ups WHERE user_id = ? AND thread_id = ? AND fired_at IS NULL",
    )
    .bind(user_id)
    .bind(thread_id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

mod live {
    use std::collections::HashSet;
    use std::sync::Arc;

    use anyhow::{Context, Result};
    use async_trait::async_trait;
    use hail_jmap::jmap_client::core::response::ThreadGetResponse;
    use secrecy::SecretString;
    use sqlx::SqlitePool;

    use super::{ThreadVerifier, VerificationOutcome};
    use crate::crypto::TokenDecryptor;

    /// Live JMAP-backed verifier for nightly reconciliation.
    pub struct LiveThreadVerifier {
        db: SqlitePool,
        jmap_url: String,
        token_decryptor: Arc<dyn TokenDecryptor>,
    }

    impl LiveThreadVerifier {
        #[must_use]
        pub fn new(
            db: SqlitePool,
            jmap_url: String,
            token_decryptor: Arc<dyn TokenDecryptor>,
        ) -> Self {
            Self {
                db,
                jmap_url,
                token_decryptor,
            }
        }

        async fn latest_active_token(&self, user_id: i64) -> Result<SecretString> {
            crate::jmap_helpers::latest_active_token(&self.db, &self.token_decryptor, user_id).await
        }
    }

    #[async_trait]
    impl ThreadVerifier for LiveThreadVerifier {
        async fn existing_threads(
            &self,
            user_id: i64,
            ids: &[String],
        ) -> Result<VerificationOutcome> {
            if ids.is_empty() {
                return Ok(VerificationOutcome::Verified(HashSet::new()));
            }

            let token = match self.latest_active_token(user_id).await {
                Ok(token) => token,
                Err(err) => return Ok(VerificationOutcome::Unverifiable(err.to_string())),
            };
            let session = hail_jmap::login_bearer(&self.jmap_url, token)
                .await
                .with_context(|| format!("JMAP login for user {user_id}"))?;

            let mut request = session.client().build();
            request.get_thread().ids(ids.iter().cloned());
            let mut response = request
                .send_single::<ThreadGetResponse>()
                .await
                .with_context(|| format!("Thread/get batch for user {user_id}"))?;

            let existing = response
                .take_list()
                .into_iter()
                .map(|thread| thread.id().to_string())
                .collect();
            Ok(VerificationOutcome::Verified(existing))
        }
    }
}

#[allow(unused_imports)]
pub use live::LiveThreadVerifier;

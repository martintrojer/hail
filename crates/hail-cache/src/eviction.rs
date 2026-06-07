//! Body/blob eviction for bounded cache policy.
//!
//! Eviction intentionally clears cached body/blob references while preserving
//! message metadata rows so list views, routing state, and cache misses keep
//! working. Pinned messages are never evicted.

use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use hail_blob_store::{BlobStore, SweepStats};
use hail_core::MailCacheMode;
use sqlx::{Row, SqlitePool};

use crate::{CachePolicy, Result};

/// Summary returned by one cache eviction pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvictionStats {
    /// Account whose policy was evaluated.
    pub account_id: i64,
    /// Metadata rows with cached body blobs that were considered.
    pub considered: usize,
    /// Message bodies whose cache references were cleared.
    pub evicted_bodies: usize,
    /// Sum of `messages.size_bytes` for evicted body rows.
    pub bytes_unreferenced: u64,
    /// Blob-store mark/sweep result after message references were cleared.
    pub sweep: SweepStats,
}

#[derive(Debug, Clone)]
struct Candidate {
    id: i64,
    size_bytes: u64,
    internal_date: i64,
}

/// Evict cached message bodies for one account under its cache policy.
///
/// `off` and `full` policies are no-ops. `bounded` policies clear
/// `messages.body_blob_id` and `messages.body_text` for unpinned body rows
/// that violate any configured cap:
///
/// - older than `keep_days`;
/// - beyond `keep_max_msgs` in least-recently-used order;
/// - beyond `keep_max_bytes` when retaining newest-accessed rows up to budget.
///
/// The union of those sets is evicted, making the most restrictive cap win.
/// Metadata rows are kept. After references are cleared, the blob store is
/// swept so orphaned body blobs can be removed.
pub async fn evict_account_bodies(
    db: &SqlitePool,
    blob_store: &dyn BlobStore,
    account_id: i64,
    policy: &CachePolicy,
) -> Result<EvictionStats> {
    let mut stats = EvictionStats {
        account_id,
        ..EvictionStats::default()
    };

    if policy.mode != MailCacheMode::Bounded {
        return Ok(stats);
    }

    let candidates = load_candidates(db, account_id).await?;
    stats.considered = candidates.len();
    if candidates.is_empty() {
        return Ok(stats);
    }

    let now_epoch_secs = Utc::now().timestamp();
    let cutoff_epoch_secs = now_epoch_secs.saturating_sub(
        Duration::days(i64::from(policy.keep_days))
            .num_seconds()
            .max(0),
    );
    let mut evict_ids = BTreeSet::new();

    if policy.keep_days > 0 {
        evict_ids.extend(
            candidates
                .iter()
                .filter(|candidate| candidate.internal_date < cutoff_epoch_secs)
                .map(|candidate| candidate.id),
        );
    }

    let keep_max_msgs = usize::try_from(policy.keep_max_msgs).unwrap_or(usize::MAX);
    if candidates.len() > keep_max_msgs {
        evict_ids.extend(
            candidates
                .iter()
                .take(candidates.len() - keep_max_msgs)
                .map(|candidate| candidate.id),
        );
    }

    let mut retained_bytes = 0_u64;
    for candidate in candidates.iter().rev() {
        match retained_bytes.checked_add(candidate.size_bytes) {
            Some(next) if next <= policy.keep_max_bytes => retained_bytes = next,
            _ => {
                evict_ids.insert(candidate.id);
            }
        }
    }

    if evict_ids.is_empty() {
        return Ok(stats);
    }

    let evicted_ids = evict_ids.into_iter().collect::<Vec<_>>();
    stats.bytes_unreferenced = candidates
        .iter()
        .filter(|candidate| evicted_ids.contains(&candidate.id))
        .map(|candidate| candidate.size_bytes)
        .sum();
    stats.evicted_bodies = clear_body_refs(db, &evicted_ids).await?;
    stats.sweep = blob_store.sweep_unreferenced(db).await?;
    Ok(stats)
}

/// Load `cache_policy` for all accounts that should be swept.
pub async fn load_account_policies(db: &SqlitePool) -> Result<Vec<(i64, CachePolicy)>> {
    let rows = sqlx::query(
        "SELECT account_id, mode, keep_days, keep_max_msgs, keep_max_bytes, backfill \
         FROM cache_policy ORDER BY account_id",
    )
    .fetch_all(db)
    .await?;

    rows.into_iter()
        .map(|row| {
            let mode = parse_mode(row.get::<String, _>("mode"));
            let backfill = parse_backfill(row.get::<String, _>("backfill"));
            Ok((
                row.get("account_id"),
                CachePolicy::new(
                    mode,
                    optional_i64_to_u32(row.get("keep_days")),
                    optional_i64_to_u64(row.get("keep_max_msgs")),
                    optional_i64_to_u64(row.get("keep_max_bytes")),
                    backfill,
                ),
            ))
        })
        .collect()
}

/// Refresh `messages.pinned` from hail-side state that is currently represented
/// in SQLite sidecar tables.
///
/// The helper covers scheduled sends, set-aside/reply-later stacks, bubble-ups,
/// thread notes, and screener-pending senders. Draft pinning is represented by
/// the `$draft` keyword in cached message keywords. Pins are recomputed from
/// these sources so rows are also unpinned when a message leaves the final pin
/// source (for example when a pending Screener sender is approved or denied).
pub async fn refresh_pinned_messages(db: &SqlitePool) -> Result<u64> {
    let updated = sqlx::query(REFRESH_PINNED_MESSAGES_SQL)
        .execute(db)
        .await?
        .rows_affected();
    Ok(updated)
}

/// Refresh `messages.pinned` using an existing SQLite connection.
pub async fn refresh_pinned_messages_conn(conn: &mut sqlx::SqliteConnection) -> Result<u64> {
    let updated = sqlx::query(REFRESH_PINNED_MESSAGES_SQL)
        .execute(conn)
        .await?
        .rows_affected();
    Ok(updated)
}

const REFRESH_PINNED_MESSAGES_SQL: &str = "UPDATE messages \
     SET pinned = CASE WHEN (\
       EXISTS (SELECT 1 FROM message_keywords mk WHERE mk.message_id = messages.id AND mk.keyword = '$draft') \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN scheduled_sends ss ON ss.user_id = ma.user_id WHERE ma.id = messages.account_id AND ss.draft_email_id = messages.backend_msg_id AND ss.status IN ('pending', 'processing')) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN stack_positions sp ON sp.user_id = ma.user_id WHERE ma.id = messages.account_id AND sp.thread_id = messages.thread_id) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN bubble_ups bu ON bu.user_id = ma.user_id WHERE ma.id = messages.account_id AND bu.thread_id = messages.thread_id AND bu.fired_at IS NULL) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN thread_notes tn ON tn.user_id = ma.user_id WHERE ma.id = messages.account_id AND (tn.thread_id = messages.thread_id OR tn.email_id = messages.backend_msg_id)) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN screener_rules sr ON sr.user_id = ma.user_id WHERE ma.id = messages.account_id AND sr.decision = 'pending' AND lower(trim(sr.sender_address)) = lower(trim(messages.from_addr)))\
     ) THEN 1 ELSE 0 END \
     WHERE pinned != CASE WHEN (\
       EXISTS (SELECT 1 FROM message_keywords mk WHERE mk.message_id = messages.id AND mk.keyword = '$draft') \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN scheduled_sends ss ON ss.user_id = ma.user_id WHERE ma.id = messages.account_id AND ss.draft_email_id = messages.backend_msg_id AND ss.status IN ('pending', 'processing')) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN stack_positions sp ON sp.user_id = ma.user_id WHERE ma.id = messages.account_id AND sp.thread_id = messages.thread_id) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN bubble_ups bu ON bu.user_id = ma.user_id WHERE ma.id = messages.account_id AND bu.thread_id = messages.thread_id AND bu.fired_at IS NULL) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN thread_notes tn ON tn.user_id = ma.user_id WHERE ma.id = messages.account_id AND (tn.thread_id = messages.thread_id OR tn.email_id = messages.backend_msg_id)) \
       OR EXISTS (SELECT 1 FROM mail_accounts ma JOIN screener_rules sr ON sr.user_id = ma.user_id WHERE ma.id = messages.account_id AND sr.decision = 'pending' AND lower(trim(sr.sender_address)) = lower(trim(messages.from_addr)))\
     ) THEN 1 ELSE 0 END";

async fn load_candidates(db: &SqlitePool, account_id: i64) -> Result<Vec<Candidate>> {
    let rows = sqlx::query(
        "SELECT id, size_bytes, internal_date \
         FROM messages \
         WHERE account_id = ?1 AND pinned = 0 AND body_blob_id IS NOT NULL \
         ORDER BY datetime(accessed_at) ASC, id ASC",
    )
    .bind(account_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Candidate {
            id: row.get("id"),
            size_bytes: nonnegative_i64_to_u64(row.get("size_bytes")),
            internal_date: row.get("internal_date"),
        })
        .collect())
}

async fn clear_body_refs(db: &SqlitePool, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "UPDATE messages SET body_blob_id = NULL, body_text = NULL WHERE id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let affected = builder.build().execute(db).await?.rows_affected();
    Ok(usize::try_from(affected).unwrap_or(usize::MAX))
}

fn parse_mode(value: String) -> MailCacheMode {
    match value.as_str() {
        "off" => MailCacheMode::Off,
        "bounded" => MailCacheMode::Bounded,
        "full" => MailCacheMode::Full,
        _ => MailCacheMode::Bounded,
    }
}

fn parse_backfill(value: String) -> hail_core::MailBackfill {
    match value.as_str() {
        "incremental" => hail_core::MailBackfill::Incremental,
        _ => hail_core::MailBackfill::Off,
    }
}

fn optional_i64_to_u32(value: Option<i64>) -> u32 {
    value
        .and_then(|value| u32::try_from(value.max(0)).ok())
        .unwrap_or(u32::MAX)
}

fn optional_i64_to_u64(value: Option<i64>) -> u64 {
    value.map(nonnegative_i64_to_u64).unwrap_or(u64::MAX)
}

fn nonnegative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

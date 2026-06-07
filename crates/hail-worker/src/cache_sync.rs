//! Worker sync loop that feeds backend-neutral changes into hail-cache.
//!
//! Concrete providers hide their native mechanics behind `MailBackend`:
//! JMAP can use EventSource internally, Gmail can use periodic history polling,
//! and this module only applies common `Change` values to `CachedMail`.

use std::future::Future;
use std::time::Duration;

use futures_util::StreamExt;
use hail_backend::{MailBackend, SyncCursor};
use hail_blob_store::BlobStore;
use hail_cache::{CachePolicy, CachedMail};
use sqlx::{Row, SqlitePool};
use thiserror::Error;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const DEFAULT_SYNC_POLL_SECS: u64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSyncOptions {
    pub poll_interval: Duration,
}

impl Default for CacheSyncOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(DEFAULT_SYNC_POLL_SECS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAccount {
    pub account_id: i64,
    pub user_id: i64,
    pub cursor: Option<String>,
    pub policy: CachePolicy,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheSyncSummary {
    pub accounts_considered: usize,
    pub changes_applied: usize,
    pub cancelled: bool,
}

#[derive(Debug, Error)]
pub enum CacheSyncError {
    #[error("database error during cache sync: {0}")]
    Database(#[from] sqlx::Error),
    #[error("cache error during backend sync: {0}")]
    Cache(#[from] hail_cache::CacheError),
    #[error("backend error during cache sync: {0}")]
    Backend(#[from] hail_backend::Error),
}

pub async fn run_cache_sync_once<B, Fut>(
    db: &SqlitePool,
    blob_store: std::sync::Arc<dyn BlobStore>,
    accounts: Vec<SyncAccount>,
    mut backend_factory: B,
    cancel: &CancellationToken,
) -> Result<CacheSyncSummary, CacheSyncError>
where
    B: FnMut(i64) -> Fut,
    Fut: Future<Output = Option<Box<dyn MailBackend + Send + Sync>>>,
{
    let mut summary = CacheSyncSummary {
        accounts_considered: accounts.len(),
        ..CacheSyncSummary::default()
    };

    for account in accounts {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let Some(backend) = cancel_or_complete(cancel, backend_factory(account.account_id))
            .await
            .flatten()
        else {
            warn!(
                account_id = account.account_id,
                user_id = account.user_id,
                "cache sync backend unavailable; skipping account"
            );
            continue;
        };
        let cursor = SyncCursor::new(account.cursor.unwrap_or_default());
        let (changes, next_cursor) =
            match cancel_or_complete(cancel, backend.poll_changes(&cursor)).await {
                Some(Ok(result)) => result,
                Some(Err(err)) => return Err(CacheSyncError::Backend(err)),
                None => {
                    summary.cancelled = true;
                    break;
                }
            };
        let applied = changes.len();
        let cache = CachedMail::with_account_id(
            db.clone(),
            std::sync::Arc::clone(&blob_store),
            backend,
            account.policy,
            account.account_id,
        );
        if cancel_or_complete(cancel, cache.apply_changes(changes))
            .await
            .is_none()
        {
            summary.cancelled = true;
            break;
        }
        persist_cursor(db, account.account_id, next_cursor.as_str()).await?;
        summary.changes_applied += applied;
    }

    Ok(summary)
}

#[allow(dead_code)]
pub async fn run_cache_watch_loop(
    db: SqlitePool,
    blob_store: std::sync::Arc<dyn BlobStore>,
    account: SyncAccount,
    backend: Box<dyn MailBackend + Send + Sync>,
    cancel: CancellationToken,
) -> Result<usize, CacheSyncError> {
    let mut stream = backend.watch_changes().await;
    let cache =
        CachedMail::with_account_id(db, blob_store, backend, account.policy, account.account_id);
    let mut applied = 0;

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            change = stream.next() => {
                let Some(change) = change else { break };
                match cancel_or_complete(&cancel, cache.apply_change(change)).await {
                    Some(Ok(())) => applied += 1,
                    Some(Err(err)) => return Err(CacheSyncError::Cache(err)),
                    None => break,
                }
            }
        }
    }

    Ok(applied)
}

pub async fn run_cache_sync_poll_loop<B, Fut>(
    db: SqlitePool,
    blob_store: std::sync::Arc<dyn BlobStore>,
    mut backend_factory: B,
    options: CacheSyncOptions,
    cancel: CancellationToken,
) -> Result<(), CacheSyncError>
where
    B: FnMut(i64) -> Fut,
    Fut: Future<Output = Option<Box<dyn MailBackend + Send + Sync>>>,
{
    let mut ticks = interval(options.poll_interval.max(Duration::from_secs(1)));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = ticks.tick() => {
                let accounts = load_active_sync_accounts(&db).await?;
                let summary = run_cache_sync_once(
                    &db,
                    std::sync::Arc::clone(&blob_store),
                    accounts,
                    &mut backend_factory,
                    &cancel,
                )
                .await?;
                if summary.changes_applied > 0 {
                    info!(
                        accounts = summary.accounts_considered,
                        changes = summary.changes_applied,
                        "cache sync poll applied backend changes"
                    );
                }
                if summary.cancelled {
                    break;
                }
            }
        }
    }

    Ok(())
}

pub async fn load_active_sync_accounts(db: &SqlitePool) -> Result<Vec<SyncAccount>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT ma.id, ma.user_id, ma.last_profile_history_id, \
                COALESCE(cp.mode, ?1) AS mode, \
                COALESCE(cp.keep_days, ?2) AS keep_days, \
                COALESCE(cp.keep_max_msgs, ?3) AS keep_max_msgs, \
                COALESCE(cp.keep_max_bytes, ?4) AS keep_max_bytes, \
                COALESCE(cp.backfill, ?5) AS backfill \
         FROM mail_accounts ma \
         LEFT JOIN cache_policy cp ON cp.account_id = ma.id \
         WHERE ma.sync_status = 'active' \
         ORDER BY ma.id",
    )
    .bind(mode_to_str(hail_core::MailCacheMode::Bounded))
    .bind(90_i64)
    .bind(50_000_i64)
    .bind(5_i64 * 1024 * 1024 * 1024)
    .bind(backfill_to_str(hail_core::MailBackfill::Off))
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| SyncAccount {
            account_id: row.get("id"),
            user_id: row.get("user_id"),
            cursor: row.get("last_profile_history_id"),
            policy: CachePolicy::new(
                parse_mode(row.get::<String, _>("mode").as_str()),
                row.get::<i64, _>("keep_days") as u32,
                row.get::<i64, _>("keep_max_msgs") as u64,
                row.get::<i64, _>("keep_max_bytes") as u64,
                parse_backfill(row.get::<String, _>("backfill").as_str()),
            ),
        })
        .collect())
}

async fn persist_cursor(db: &SqlitePool, account_id: i64, cursor: &str) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE mail_accounts \
         SET last_profile_history_id = ?1, profile_synced_at = ?2, last_sync_succeeded_at = ?2, \
             last_error_class = NULL, last_error_message = NULL, updated_at = ?2 \
         WHERE id = ?3",
    )
    .bind(cursor)
    .bind(now)
    .bind(account_id)
    .execute(db)
    .await?;
    Ok(())
}

fn parse_mode(value: &str) -> hail_core::MailCacheMode {
    match value {
        "off" => hail_core::MailCacheMode::Off,
        "full" => hail_core::MailCacheMode::Full,
        _ => hail_core::MailCacheMode::Bounded,
    }
}

fn parse_backfill(value: &str) -> hail_core::MailBackfill {
    match value {
        "incremental" => hail_core::MailBackfill::Incremental,
        _ => hail_core::MailBackfill::Off,
    }
}

fn mode_to_str(mode: hail_core::MailCacheMode) -> &'static str {
    match mode {
        hail_core::MailCacheMode::Off => "off",
        hail_core::MailCacheMode::Bounded => "bounded",
        hail_core::MailCacheMode::Full => "full",
    }
}

fn backfill_to_str(backfill: hail_core::MailBackfill) -> &'static str {
    match backfill {
        hail_core::MailBackfill::Off => "off",
        hail_core::MailBackfill::Incremental => "incremental",
    }
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

use std::future::Future;
use std::time::Duration;

use hail_blob_store::{BlobStore, FilesystemBlobStore};
use hail_cache::{
    EvictionStats, evict_account_bodies, load_account_policies, refresh_pinned_messages,
};
use sqlx::SqlitePool;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub const DEFAULT_CACHE_EVICTION_SWEEP_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEvictionSweeperOptions {
    pub interval: Duration,
}

impl Default for CacheEvictionSweeperOptions {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_CACHE_EVICTION_SWEEP_SECS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CacheEvictionSweepSummary {
    pub accounts_considered: usize,
    pub accounts_swept: usize,
    pub evicted_bodies: usize,
    pub bytes_unreferenced: u64,
    pub blobs_deleted: usize,
    pub blob_bytes_deleted: u64,
    pub pin_rows_refreshed: u64,
}

pub async fn run_cache_eviction_sweep_once(
    db: &SqlitePool,
    blob_store: &dyn BlobStore,
) -> hail_cache::Result<CacheEvictionSweepSummary> {
    let pin_rows_refreshed = refresh_pinned_messages(db).await?;
    let policies = load_account_policies(db).await?;
    let mut summary = CacheEvictionSweepSummary {
        accounts_considered: policies.len(),
        pin_rows_refreshed,
        ..CacheEvictionSweepSummary::default()
    };

    for (account_id, policy) in policies {
        let stats = evict_account_bodies(db, blob_store, account_id, &policy).await?;
        accumulate(&mut summary, &stats);
    }

    Ok(summary)
}

pub async fn run_cache_eviction_sweeper(
    db: SqlitePool,
    blob_store: FilesystemBlobStore,
    options: CacheEvictionSweeperOptions,
    cancel: CancellationToken,
) -> hail_cache::Result<()> {
    let mut ticks = interval(options.interval.max(Duration::from_secs(1)));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = ticks.tick() => {
                match cancel_or_complete(
                    &cancel,
                    run_cache_eviction_sweep_once(&db, &blob_store),
                )
                .await
                {
                    Some(Ok(summary)) => {
                        if summary.evicted_bodies > 0 || summary.blobs_deleted > 0 {
                            info!(
                                accounts = summary.accounts_swept,
                                evicted_bodies = summary.evicted_bodies,
                                bytes_unreferenced = summary.bytes_unreferenced,
                                blobs_deleted = summary.blobs_deleted,
                                blob_bytes_deleted = summary.blob_bytes_deleted,
                                "cache eviction sweep processed"
                            );
                        }
                    }
                    Some(Err(err)) => warn!(error = %err, "cache eviction sweep failed"),
                    None => break,
                }
            }
        }
    }

    Ok(())
}

fn accumulate(summary: &mut CacheEvictionSweepSummary, stats: &EvictionStats) {
    if stats.considered > 0 || stats.evicted_bodies > 0 || stats.sweep.scanned > 0 {
        summary.accounts_swept += 1;
    }
    summary.evicted_bodies += stats.evicted_bodies;
    summary.bytes_unreferenced = summary
        .bytes_unreferenced
        .saturating_add(stats.bytes_unreferenced);
    summary.blobs_deleted += stats.sweep.deleted;
    summary.blob_bytes_deleted = summary
        .blob_bytes_deleted
        .saturating_add(stats.sweep.bytes_deleted);
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

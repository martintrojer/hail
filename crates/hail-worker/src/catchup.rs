//! Startup/reconnect catch-up replay for per-user JMAP cursors.
//!
//! On every supervisor start and reconnect, we must replay persisted
//! `jmap_state` cursors before opening the live EventSource stream.
//! Missing rows are first-run users: seed the current server state via
//! cheap */get calls and do not replay historical mail.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::changes::{JmapChangeFetcher, TRACKED_TYPE_STATES, handle_changes, upsert_cursor};
use crate::screener::JmapOps;

pub async fn catchup_user(
    db: &SqlitePool,
    user_id: i64,
    fetcher: &dyn JmapChangeFetcher,
    jmap_ops: &dyn JmapOps,
    cancel: CancellationToken,
) -> Result<()> {
    for type_state in TRACKED_TYPE_STATES {
        if cancel.is_cancelled() {
            return Ok(());
        }

        info!(user_id, type_state = ?type_state, "catchup: starting");
        if cursor_exists(db, user_id, type_state).await? {
            let mut types = BTreeSet::new();
            types.insert((*type_state).to_string());
            let changes = handle_changes(db, user_id, fetcher, jmap_ops, &types).await?;
            info!(user_id, type_state = ?type_state, changes, "catchup: applied");
        } else {
            let state = fetcher.current_state(type_state).await?;
            upsert_cursor(db, user_id, type_state, &state).await?;
            info!(user_id, type_state = ?type_state, changes = 0usize, "catchup: applied");
        }
    }

    info!(user_id, "catchup: complete");
    Ok(())
}

async fn cursor_exists(db: &SqlitePool, user_id: i64, type_state: &str) -> Result<bool> {
    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM jmap_state WHERE user_id = ? AND type_state = ?",
    )
    .bind(user_id)
    .bind(type_state)
    .fetch_optional(db)
    .await
    .context("select jmap_state existence")?;
    Ok(exists.is_some())
}

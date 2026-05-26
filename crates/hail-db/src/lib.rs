//! SQLite persistence layer and migrations for hail.
//!
//! See `design.md` §5 DD-7 and §6.2 for context on why SQLite (WAL + Litestream)
//! is the sidecar store and what schema the baseline migration establishes.

pub mod app_events;

use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

/// Errors surfaced by the hail-db crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying SQLx error (connection, query, etc.).
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    /// Error returned by `sqlx::migrate!()` while applying migrations.
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Open a SQLite pool against `url` with the pragmas hail expects:
/// `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`,
/// `busy_timeout=5000`. Accepts any URL `SqliteConnectOptions::from_str`
/// understands, including `sqlite::memory:` and `sqlite://path/hail.db`.
///
/// (In-memory databases silently keep their default journal mode; the WAL
/// request is harmless there.)
pub async fn connect(url: &str) -> Result<SqlitePool, Error> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5000));

    let pool = SqlitePoolOptions::new().connect_with(opts).await?;
    Ok(pool)
}

/// Apply all embedded migrations from `crates/hail-db/migrations/`.
pub async fn migrate(pool: &SqlitePool) -> Result<(), Error> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

/// Mark a thread as seen by a user, updating `seen_at` when it was already seen.
pub async fn mark_thread_seen(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO thread_seen (user_id, thread_id) VALUES (?, ?) \
         ON CONFLICT DO UPDATE SET seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )
    .bind(user_id)
    .bind(thread_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear stack placement rows for a thread.
pub async fn clear_thread_stack_positions(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM stack_positions WHERE user_id = ? AND thread_id = ?")
        .bind(user_id)
        .bind(thread_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Clear sidecar state that should not survive moving a thread between views.
///
/// Fired Bubble Ups are retained as history; only pending reminders are removed.
pub async fn clear_thread_sidecar_state(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: &str,
) -> Result<(), sqlx::Error> {
    clear_thread_stack_positions(pool, user_id, thread_id).await?;
    sqlx::query(
        "DELETE FROM bubble_ups WHERE user_id = ? AND thread_id = ? AND fired_at IS NULL",
    )
    .bind(user_id)
    .bind(thread_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return whether a user has seen a thread.
pub async fn is_thread_seen(
    pool: &SqlitePool,
    user_id: i64,
    thread_id: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM thread_seen WHERE user_id = ? AND thread_id = ?",
    )
    .bind(user_id)
    .bind(thread_id)
    .fetch_one(pool)
    .await?;
    Ok(row > 0)
}

/// Return all thread ids a user has seen for batch filtering.
pub async fn seen_thread_ids(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows =
        sqlx::query_scalar::<_, String>("SELECT thread_id FROM thread_seen WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// Return all thread ids for Bubble Ups that have fired for a user.
pub async fn fired_bubble_up_thread_ids(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT thread_id FROM bubble_ups WHERE user_id = ? AND fired_at IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

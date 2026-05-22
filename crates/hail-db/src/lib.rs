//! SQLite persistence layer and migrations for hail.
//!
//! See `design.md` §5 DD-7 and §6.2 for context on why SQLite (WAL + Litestream)
//! is the sidecar store and what schema the baseline migration establishes.

use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::SqlitePool;

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

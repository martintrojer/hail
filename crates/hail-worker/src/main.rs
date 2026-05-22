//! `hail-worker` — tokio process for JMAP push consumption and scheduled jobs.
//!
//! This is the *skeleton*: runtime bootstrap, logging, sqlx pool, migrations,
//! a placeholder supervisor task, and graceful shutdown on SIGINT/SIGTERM.
//! JMAP EventSource subscriptions and screener routing arrive in follow-up
//! tasks (`jmap-eventsource`, `screener-routing`).

mod config;
mod state;
mod supervisor;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::signal;
use tokio::signal::unix::{SignalKind, signal as unix_signal};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env();
    info!(
        database_url = %config.database_url,
        stalwart_url = %config.stalwart_url,
        tick_secs = config.tick_secs,
        "hail-worker starting"
    );

    let db = hail_db::connect(&config.database_url)
        .await
        .with_context(|| format!("opening sqlite pool at {}", config.database_url))?;
    hail_db::migrate(&db)
        .await
        .context("running hail-db migrations")?;
    info!("db ready");

    let state = Arc::new(AppState { db, config });
    let cancel = CancellationToken::new();

    // Placeholder supervisor — real subscription/scheduler logic lands later.
    let supervisor_handle = {
        let state = state.clone();
        let cancel = cancel.child_token();
        tokio::spawn(async move {
            if let Err(e) = supervisor::run(state, cancel).await {
                error!(error = %e, "supervisor exited with error");
            }
        })
    };

    wait_for_shutdown().await?;
    info!("shutdown signal received");
    cancel.cancel();
    info!("shutting down");

    // Let the supervisor unwind. The cancel token guarantees this returns
    // promptly; we surface a join error but don't fail the process.
    if let Err(e) = supervisor_handle.await {
        error!(error = %e, "supervisor join failed");
    }

    // Explicit pool close so in-flight writes flush before exit.
    state.db.close().await;
    drop(state);

    Ok(())
}

/// Initialise `tracing` with an env-driven filter and a compact text formatter.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("hail_worker=info,info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

/// Block until SIGINT (Ctrl-C) or SIGTERM arrives. Returns on the first signal.
///
/// We use `tokio::signal::ctrl_c()` for SIGINT — it composes cleanly with
/// `tokio::select!` and works whether the process is foregrounded under a
/// terminal or backgrounded under a supervisor. SIGTERM is wired via the
/// unix-specific stream because `ctrl_c` only catches SIGINT.
async fn wait_for_shutdown() -> Result<()> {
    let mut sigterm = unix_signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    tokio::select! {
        res = signal::ctrl_c() => {
            res.context("install SIGINT handler")?;
            info!("received SIGINT");
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM");
        }
    }
    Ok(())
}

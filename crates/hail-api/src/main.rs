//! `hail-api` — Axum HTTP server.
//!
//! The router is built in `hail_api::build_router` so integration tests
//! can exercise the exact same stack without binding a TCP port; this
//! shell takes care of:
//!   1. tracing setup,
//!   2. loading the unified `hail-core` config + parsing the server key,
//!   3. opening the SQLite pool and running migrations,
//!   4. wiring `AppState` + the in-memory login rate limiter,
//!   5. serving with eager-installed SIGINT/SIGTERM shutdown handlers.
//!
//! Mirrors `hail-worker` deliberately so both binaries behave the same
//! under systemd / Docker.

use std::sync::Arc;

use anyhow::{Context, Result};
use hail_api::middleware::rate_limit::IpRateLimiter;
use hail_api::state::AppState;
use hail_core::Config;
use tokio::net::TcpListener;
use tokio::signal::unix::{Signal, SignalKind, signal as unix_signal};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::load().context("loading hail config (TOML + env)")?;
    // NB: we deliberately do NOT log `config.secrets` or the parsed key.
    info!(
        database_url = %config.database_url,
        stalwart_url = %config.stalwart.jmap_url,
        bind = %config.server.bind,
        "hail-api starting"
    );

    // Parse the server key once at startup; failures here are fatal —
    // the API cannot encrypt/decrypt session tokens without it.
    let server_key = hail_core::parse_server_key(&config.secrets.server_key)
        .context("parsing secrets.server_key")?;

    let db = hail_db::connect(&config.database_url)
        .await
        .with_context(|| format!("opening sqlite pool at {}", config.database_url))?;
    hail_db::migrate(&db)
        .await
        .context("running hail-db migrations")?;
    info!("db ready");

    let bind = config.server.bind.clone();
    let state = AppState {
        db,
        config,
        server_key: Arc::new(server_key),
        login_limiter: Arc::new(IpRateLimiter::default()),
    };

    let router = hail_api::build_router(state.clone(), false);

    // Install signal handlers EAGERLY — before we bind the listener or
    // enter the serve loop. `tokio::signal::unix::signal` registers a
    // per-SignalKind dispatcher; if we leave the install lazy (inside
    // the `select!` future) and a signal arrives before that future is
    // first polled, the default disposition runs and the process dies
    // hard (exit 143 for SIGTERM). Hoisting the install closes that
    // window.
    let mut sigint = unix_signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    let mut sigterm = unix_signal(SignalKind::terminate()).context("install SIGTERM handler")?;

    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding HTTP listener at {bind}"))?;
    let local = listener.local_addr().ok();
    info!(?local, "hail-api listening");

    // We use `into_make_service_with_connect_info` so the login handler
    // can read the peer's `SocketAddr` for the per-IP rate limiter.
    let make_svc = router.into_make_service_with_connect_info::<std::net::SocketAddr>();

    // Shutdown shape: race `axum::serve` against the signal streams in a
    // `tokio::select!`. We deliberately do NOT use
    // `with_graceful_shutdown` here — it waits for every open connection
    // (including idle HTTP/1.1 keep-alives) to close before resolving,
    // which wedges shutdown indefinitely under realistic clients.
    tokio::select! {
        res = axum::serve(listener, make_svc) => {
            res.context("axum serve loop")?;
        }
        _ = wait_for_shutdown(&mut sigint, &mut sigterm) => {
            info!("shutting down");
        }
    }

    state.db.close().await;
    info!("hail-api stopped");
    Ok(())
}

/// Initialise `tracing` with an env-driven filter and a compact text
/// formatter. Matches the shape used in `hail-worker` so both binaries
/// produce identically-structured logs under `RUST_LOG`.
fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("hail_api=info,info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

/// Block until SIGINT or SIGTERM arrives. Takes pre-installed streams
/// by `&mut` so the handler registration happens at startup.
async fn wait_for_shutdown(sigint: &mut Signal, sigterm: &mut Signal) {
    tokio::select! {
        _ = sigint.recv() => {
            info!("received SIGINT");
        }
        _ = sigterm.recv() => {
            info!("received SIGTERM");
        }
    }
}

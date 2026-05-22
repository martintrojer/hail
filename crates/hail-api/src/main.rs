//! `hail-api` — Axum HTTP server.
//!
//! Scope of this build: a runnable skeleton that
//!   1. boots a tokio runtime + tracing,
//!   2. loads the unified `hail-core` config,
//!   3. connects to the SQLite sidecar and runs migrations
//!      (idempotent under sqlx — `hail-worker` does the same),
//!   4. serves `/healthz`, `/readyz`, and `/api/openapi.json`,
//!   5. (optionally, behind `--features dev-docs`) serves a Redoc UI at
//!      `/api/docs`,
//!   6. shuts down cleanly on SIGINT / SIGTERM.
//!
//! Real domain endpoints (auth, views, verbs, WS, admin) arrive in
//! follow-up tasks. Shutdown / tracing / db plumbing mirrors
//! `hail-worker` so both binaries behave the same under systemd.

mod openapi;
mod routes;
mod state;

use anyhow::{Context, Result};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use hail_core::Config;
use tokio::net::TcpListener;
use tokio::signal::unix::{Signal, SignalKind, signal as unix_signal};
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::openapi::ApiDoc;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::load().context("loading hail config (TOML + env)")?;
    info!(
        database_url = %config.database_url,
        stalwart_url = %config.stalwart.jmap_url,
        bind = %config.server.bind,
        "hail-api starting"
    );

    let db = hail_db::connect(&config.database_url)
        .await
        .with_context(|| format!("opening sqlite pool at {}", config.database_url))?;
    hail_db::migrate(&db)
        .await
        .context("running hail-db migrations")?;
    info!("db ready");

    let bind = config.server.bind.clone();
    let state = AppState { db, config };

    // Build the unified router. `utoipa-axum`'s `OpenApiRouter` collects
    // both the axum routes and their `#[utoipa::path]` metadata; we
    // `split_for_parts` at the end to mount the static `/api/openapi.json`
    // and `/api/docs` endpoints alongside the live handlers.
    let api_router: OpenApiRouter<AppState> =
        OpenApiRouter::with_openapi(ApiDoc::openapi()).merge(routes::health::router());

    let (router, api) = api_router.with_state(state.clone()).split_for_parts();

    // Pre-serialize the OpenAPI doc once: the spec doesn't change at
    // runtime, so handing axum a `&'static String` via `move` is
    // simpler (and faster) than re-serializing on every request.
    let openapi_json = serde_json::to_string(&api).context("serializing OpenAPI spec to JSON")?;

    #[cfg_attr(not(feature = "dev-docs"), allow(unused_mut))]
    let mut router = router
        .route(
            "/api/openapi.json",
            get(move || {
                let body = openapi_json.clone();
                async move {
                    (
                        StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        body,
                    )
                }
            }),
        )
        .layer(TraceLayer::new_for_http());

    // Mount the Redoc UI only when the `dev-docs` feature is on. This
    // keeps the production binary free of the Redoc bundle and any
    // accidental information-leak surface. Operators that want the UI
    // can build with `--features dev-docs`.
    #[cfg(feature = "dev-docs")]
    {
        use utoipa_redoc::{Redoc, Servable};
        router = router.merge(Redoc::with_url("/api/docs", api));
    }
    #[cfg(not(feature = "dev-docs"))]
    let _ = api;

    // Install signal handlers EAGERLY — before we bind the listener or
    // enter the serve loop. `tokio::signal::unix::signal` registers a
    // per-SignalKind dispatcher; if we leave the install lazy (inside
    // the `select!` future) and a signal arrives before that future is
    // first polled, the default disposition runs and the process dies
    // hard (exit 143 for SIGTERM). Hoisting the install closes that
    // window.
    let mut sigint = unix_signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    let mut sigterm =
        unix_signal(SignalKind::terminate()).context("install SIGTERM handler")?;

    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding HTTP listener at {bind}"))?;
    let local = listener.local_addr().ok();
    info!(?local, "hail-api listening");

    // Shutdown shape: race `axum::serve` against the signal streams in
    // a `tokio::select!`. We deliberately do NOT use
    // `with_graceful_shutdown` here — it waits for every open connection
    // (including idle HTTP/1.1 keep-alives held by clients like curl)
    // to close before resolving, which wedges shutdown indefinitely
    // under realistic clients. For this skeleton there is no in-flight
    // state worth draining: a follow-up task can layer in a bounded
    // grace period once we actually have streaming responses (WS, SSE).
    tokio::select! {
        res = axum::serve(listener, router) => {
            res.context("axum serve loop")?;
        }
        _ = wait_for_shutdown(&mut sigint, &mut sigterm) => {
            info!("shutting down");
        }
    }

    // Explicit pool close so in-flight writes flush before exit.
    state.db.close().await;
    info!("hail-api stopped");
    Ok(())
}

/// Tiny placeholder JSON 404 handler so unknown paths don't fall through
/// to axum's default plaintext body. Currently unused (no fallback
/// route registered yet); kept around so the auth/view tasks have a
/// shared error shape to bolt onto.
#[allow(dead_code)]
async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "not_found" })),
    )
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
/// by `&mut` so the handler registration happens at startup (see
/// `main` for rationale).
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

//! Health endpoints surfaced under design.md §7.5.
//!
//! - `GET /healthz` — liveness. Returns 204 unconditionally; the process
//!   being able to handle the request *is* the signal.
//! - `GET /readyz`  — readiness. Runs a trivial `SELECT 1` round-trip
//!   against the SQLite pool, returning 200 OK on success and 503 Service
//!   Unavailable otherwise. This is the first place we'll extend later
//!   (the JMAP session check called out in §7.5 lands in a follow-up
//!   task, gated on the `jmap-eventsource` work).
//!
//! Both handlers are tagged `system` so they group together in the
//! generated OpenAPI/Redoc output.

use axum::extract::State;
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// OpenAPI tag for operational endpoints. Kept as a `const` so the
/// `ApiDoc` derive and the per-handler attributes stay in sync.
pub const TAG: &str = "system";

/// Build the subrouter that owns the two demo health endpoints. Mounted
/// at the API root by `main.rs`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(healthz))
        .routes(routes!(readyz))
}

/// Liveness probe.
///
/// Returns 204 No Content if the process is up enough to accept HTTP.
/// Intended for container orchestrators (Kubernetes `livenessProbe`,
/// Docker `HEALTHCHECK`, etc.) — a failing liveness check should
/// restart the container.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = TAG,
    responses(
        (status = 204, description = "Process is alive."),
    ),
)]
async fn healthz() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Readiness probe.
///
/// Returns 200 OK once the SQLite pool answers `SELECT 1`, otherwise
/// 503 Service Unavailable. Designed for `readinessProbe` style
/// gating: a failing readiness check should remove the instance from
/// load-balancer rotation but *not* restart it.
///
/// Per design.md §7.5 the production check will also verify the JMAP
/// session; that's wired in by the `jmap-eventsource` task.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = TAG,
    responses(
        (status = 200, description = "All dependencies reachable."),
        (status = 503, description = "A dependency is unhealthy."),
    ),
)]
async fn readyz(State(state): State<AppState>) -> StatusCode {
    // `SELECT 1` is the canonical sqlx liveness query. We deliberately
    // use `fetch_one` against a scalar so we exercise the round-trip
    // rather than just acquiring a connection.
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => StatusCode::OK,
        Err(err) => {
            tracing::warn!(error = %err, "readyz: database probe failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

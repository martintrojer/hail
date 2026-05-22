//! OpenAPI document root.
//!
//! Per design.md §7.6, `hail-api` is the source of truth for the API
//! contract: it emits an OpenAPI 3.1 schema at `/api/openapi.json`,
//! which the webapp consumes via `openapi-typescript` to generate TS
//! types. Keeping a single `ApiDoc` here means future feature work
//! (auth, views, verbs, admin) just adds tags / `nest()`s the
//! relevant `OpenApiRouter` — no manual spec bookkeeping.

use utoipa::OpenApi;

use crate::routes::health;

/// Root OpenAPI document. The list of `paths(...)` is intentionally
/// *empty*: every path is registered by the `utoipa_axum::routes!`
/// macro at mount time, so this struct only needs to carry top-level
/// `info`, `tags`, and (later) security schemes.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "hail",
        version = env!("CARGO_PKG_VERSION"),
        description = "Task-oriented HTTP+JSON API for hail \
                       (self-hosted hey.com-style email front-end \
                       on top of Stalwart/JMAP). \
                       See docs/design.md §7 for the verb/view model.",
        license(
            name = "AGPL-3.0-or-later",
        ),
    ),
    tags(
        (name = health::TAG, description = "Liveness, readiness, and operational probes."),
    ),
)]
pub struct ApiDoc;

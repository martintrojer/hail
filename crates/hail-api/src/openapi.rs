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
        (name = crate::routes::invites::TAG, description = "Public invite preview and acceptance endpoints."),
        (name = "setup", description = "First-run setup wizard endpoints."),
        (name = crate::routes::labels::TAG, description = "Local label management endpoints."),
        (name = crate::routes::admin_stats::TAG, description = "Administrator-only user, domain, and system status endpoints."),
        (name = crate::routes::attachments::TAG, description = "Attachment listing and download endpoints."),
        (name = crate::routes::blobs::TAG, description = "JMAP blob upload endpoints."),
        (name = crate::routes::capabilities::TAG, description = "Runtime mail backend and cache feature flags."),
        (name = crate::routes::compose::TAG, description = "Compose, reply, and scheduled-send creation."),
        (name = crate::routes::contacts::TAG, description = "Contact notes and contact detail views."),
        (name = crate::routes::drafts::TAG, description = "Draft autosave create/update endpoints."),
        (name = crate::routes::pile::TAG, description = "Saved thread piles such as Set Aside and Reply Later."),
        (name = crate::routes::provider_accounts::TAG, description = "Provider import account OAuth and disconnect endpoints."),
        (name = crate::routes::provider_sync::TAG, description = "Provider import sync status and manual trigger endpoints."),
        (name = crate::routes::screener::TAG, description = "Screener pending sender view and decisions."),
        (name = crate::routes::threads::TAG, description = "Thread mutation verbs."),
        (name = crate::routes::undo::TAG, description = "Short-lived undo token execution."),
        (name = crate::routes::users::TAG, description = "Current user preferences."),
        (name = crate::routes::views::TAG, description = "Mail list views and unified search."),
        (name = crate::routes::workflows::TAG, description = "Workflow/mail rule CRUD endpoints."),
    ),
)]
pub struct ApiDoc;

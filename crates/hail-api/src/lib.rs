//! `hail-api` shared library surface.
//!
//! The binary in `main.rs` is a thin shell that:
//!   1. loads config + opens the DB,
//!   2. builds the router via [`build_router`],
//!   3. serves it with graceful shutdown.
//!
//! Putting [`build_router`] in a library lets the integration tests in
//! `tests/auth.rs` exercise the exact same router stack without binding
//! a real TCP listener — they hand-craft requests via `tower::ServiceExt`.

pub mod audit;
pub mod events;
pub mod middleware;
pub mod openapi;
pub mod routes;
pub mod state;

use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::{Router, extract::Request};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::openapi::ApiDoc;
use crate::state::AppState;

/// Build the full Axum router for `hail-api`.
///
/// Layout (security boundary called out per design.md §10):
///
/// * **Public** (no auth middleware): `/healthz`, `/readyz`,
///   `/api/openapi.json`, `POST /api/auth/login`, `POST /api/auth/logout`.
///   Logout is public because a stale cookie should still be able to
///   clear itself, but its handler still requires `X-Hail-Request: 1`
///   because it is a mutating endpoint that accepts ambient cookies.
///
/// * **Protected** (auth middleware + CSRF on mutations): everything else
///   under `/api/*`, including `GET /api/auth/me` and any downstream
///   verbs/views added by later tasks. The middleware reads the
///   `hail_session` cookie, decrypts the JMAP token, attaches an
///   `AuthUser`, and 401s otherwise.
///
/// `include_test_stubs` mounts `POST /api/threads/test` (a 204-on-reach
/// stub) behind the auth middleware — used exclusively by `tests/auth.rs`
/// to verify the CSRF gate.
pub fn build_router(state: AppState, include_test_stubs: bool) -> Router {
    build_api_router(state, include_test_stubs, resolve_webapp_dir)
}

fn build_api_router(
    state: AppState,
    include_test_stubs: bool,
    webapp_dir: impl FnOnce(&AppState) -> PathBuf,
) -> Router {
    // OpenAPI-tracked public routes (health) are mounted directly. Protected
    // route specs are merged into the document below, but their routers are
    // mounted only in the auth-wrapped `protected` subtree.
    let api_router: OpenApiRouter<AppState> = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(routes::health::router())
        .merge(routes::invites::openapi_router());
    let (open_router, mut api) = api_router.with_state(state.clone()).split_for_parts();
    for protected_api in [
        routes::admin_stats::router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::attachments::router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::blobs::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::compose::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::contacts::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::drafts::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::labels::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::notes::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::pile::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::provider_accounts::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::provider_sync::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::screener::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::speakeasy::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::threads::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::threads_view::router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::undo::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::views::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
        routes::workflows::openapi_router()
            .with_state::<AppState>(state.clone())
            .split_for_parts()
            .1,
    ] {
        api.merge(protected_api);
    }

    // Serialize once. The OpenAPI doc is immutable at runtime.
    let openapi_json = serde_json::to_string(&api).expect("OpenAPI doc must serialize to JSON");

    let openapi_route = get(move || {
        let body = openapi_json.clone();
        async move {
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
        }
    });

    // Protected half — wrapped in the auth middleware. We layer the
    // middleware on a sub-Router and merge it in so the public routes are
    // structurally exempt from auth (no risk of an ordering mistake
    // accidentally protecting `/healthz` or unprotecting `/api/...`).
    // `mut` only when the test-stub merge below is compiled in; in
    // release builds the binding is never reassigned.
    #[cfg_attr(not(feature = "__test-stubs"), allow(unused_mut))]
    let mut protected: Router<AppState> = routes::auth::protected_router()
        .merge(Router::from(routes::admin_stats::router()))
        .merge(Router::from(routes::attachments::router()))
        .merge(routes::admin_domains::router())
        .merge(routes::admin_users::router())
        .merge(routes::blobs::router())
        .merge(routes::compose::router())
        .merge(routes::contacts::router())
        .merge(routes::drafts::router())
        .merge(routes::labels::router())
        .merge(routes::notes::router())
        .merge(routes::pile::router())
        .merge(routes::provider_accounts::router_with_client(Arc::new(
            routes::provider_accounts::LiveGmailOAuthClient::from_config(&state.config),
        )))
        .merge(routes::provider_sync::router())
        .merge(routes::screener::router())
        .merge(routes::speakeasy::router())
        .merge(routes::threads::router())
        .merge(Router::from(routes::threads_view::router()))
        .merge(routes::undo::router())
        .merge(routes::views::router())
        .merge(routes::workflows::router())
        .merge(routes::ws::router());

    #[cfg(feature = "__test-stubs")]
    if include_test_stubs {
        protected = protected.merge(routes::test_stub::router());
    }
    #[cfg(not(feature = "__test-stubs"))]
    {
        let _ = include_test_stubs;
    }

    let protected = protected.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        middleware::auth::require_auth,
    ));

    let router = Router::new()
        // Public routes first — these MUST NOT carry the auth middleware.
        .merge(open_router) // /healthz, /readyz
        .route("/api/openapi.json", openapi_route)
        .merge(routes::auth::public_router())
        .merge(routes::setup::router())
        // Protected routes, behind the auth middleware layer.
        .merge(protected)
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http());

    mount_spa_fallback(router, &webapp_dir(&state)).layer(axum::middleware::from_fn_with_state(
        state,
        middleware::security_headers::add_security_headers,
    ))
}

fn resolve_webapp_dir(state: &AppState) -> PathBuf {
    state
        .config
        .server
        .webapp_dir
        .clone()
        .or_else(|| std::env::var_os("HAIL_WEBAPP_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/srv/hail/webapp"))
}

fn mount_spa_fallback(router: Router, webapp_dir: &Path) -> Router {
    if !webapp_dir.exists() {
        tracing::warn!(path = %webapp_dir.display(), "webapp dir missing; SPA static serving disabled");
        return router.fallback(api_only_not_found);
    }

    router
        .fallback_service(
            // `not_found_service` forces the fallback response status back to
            // 404 in tower-http 0.6; history-mode SPA routes need `200 OK`, so
            // use the same ServeFile fallback without SetStatus wrapping.
            ServeDir::new(webapp_dir).fallback(ServeFile::new(webapp_dir.join("index.html"))),
        )
        .layer(axum::middleware::from_fn(normalize_javascript_content_type))
}

async fn normalize_javascript_content_type(req: Request, next: Next) -> Response {
    let is_js = req.uri().path().ends_with(".js");
    let mut response = next.run(req).await;
    if is_js && response.status().is_success() {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript"),
        );
    }
    response
}

async fn api_only_not_found(uri: Uri) -> StatusCode {
    if !uri.path().starts_with("/api/") {
        tracing::debug!(path = %uri.path(), "SPA static serving disabled; returning 404");
    }
    StatusCode::NOT_FOUND
}

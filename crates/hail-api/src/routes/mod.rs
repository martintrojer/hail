//! HTTP route modules. Each module exposes a function that returns an
//! `OpenApiRouter<AppState>` so handlers self-register their OpenAPI spec
//! via `#[utoipa::path]` when mounted (see `utoipa_axum::router`).

pub mod health;

//! HTTP route modules. Each module exposes a function that returns an
//! `OpenApiRouter<AppState>` (for OpenAPI-tracked routes) or a plain
//! `Router<AppState>` (for routes we don't want in the public spec —
//! login/logout/me).

pub mod admin_domains;
pub mod admin_stats;
pub mod admin_users;
pub mod attachments;
pub mod auth;
pub mod blobs;
pub mod compose;
pub mod contacts;
pub mod drafts;
pub mod health;
pub mod invites;
pub mod jmap_helpers;
pub mod labels;
pub mod management_http;
pub mod notes;
pub mod outbound;
pub mod pile;
pub mod provider_accounts;
pub mod provider_sync;
pub mod response;
pub mod screener;
pub mod setup;
pub mod threads;
pub mod threads_view;
pub mod undo;
pub mod validation;
pub mod views;
pub mod workflows;
pub mod ws;

// `test_stub` is gated behind the `__test-stubs` feature so it never
// ships in a release binary. `tests/auth.rs` enables this feature via
// its `dev-dependencies` entry; nothing else does.
#[cfg(feature = "__test-stubs")]
pub mod test_stub;

//! HTTP route modules. Each module exposes a function that returns an
//! `OpenApiRouter<AppState>` (for OpenAPI-tracked routes) or a plain
//! `Router<AppState>` (for routes we don't want in the public spec —
//! login/logout/me).

pub mod auth;
pub mod health;

// `test_stub` is gated behind the `__test-stubs` feature so it never
// ships in a release binary. `tests/auth.rs` enables this feature via
// its `dev-dependencies` entry; nothing else does.
#[cfg(feature = "__test-stubs")]
pub mod test_stub;

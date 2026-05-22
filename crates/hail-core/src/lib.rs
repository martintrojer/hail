//! Domain types shared between `hail-api` and `hail-worker`.
//!
//! For now this crate is mostly the unified configuration loader; future
//! tasks will park shared API types and error enums here as the binaries
//! grow.

pub mod config;

pub use config::{AdminConfig, Config, ConfigError, SecretsConfig, ServerConfig, StalwartConfig};

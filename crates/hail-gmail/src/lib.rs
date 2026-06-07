//! Gmail backend implementation and Gmail provider-import primitives.

pub mod gmail_client;
pub mod gmail_historical_import;
pub mod gmail_incremental_sync;
pub mod gmail_initial_sync;
pub mod gmail_outbound_smtp;
pub mod provider_import_routing;
pub mod rfc822_import;

mod backend;

pub use backend::GmailBackend;

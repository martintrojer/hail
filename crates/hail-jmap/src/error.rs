//! Error types for JMAP session setup.

/// Errors returned while establishing a JMAP session.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to connect to the JMAP endpoint.
    #[error("failed to connect to JMAP server")]
    Connect(#[source] jmap_client::Error),

    /// Failed to authenticate with the JMAP endpoint.
    #[error("failed to authenticate with JMAP server")]
    Auth(#[source] jmap_client::Error),

    /// The authenticated JMAP session did not advertise a primary Mail account.
    #[error("JMAP session is missing a primary mail account")]
    MissingPrimaryAccount,
}

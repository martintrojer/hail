//! Errors surfaced by the cache facade.

/// Errors surfaced by `CachedMail`.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The facade method is reserved for a downstream cache implementation task.
    #[error("cache operation is not implemented yet: {operation}")]
    NotImplemented { operation: &'static str },

    /// SQLite access failed.
    #[error(transparent)]
    Db(#[from] sqlx::Error),

    /// Blob store access failed.
    #[error(transparent)]
    Blob(#[from] hail_blob_store::BlobStoreError),

    /// The selected upstream backend failed.
    #[error(transparent)]
    Backend(#[from] hail_backend::Error),
}

use fetchx_models::DownloadId;
use thiserror::Error;

/// Errors returned by database initialization and repository operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("SQLite migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("invalid SQLite database URL: {0}")]
    InvalidDatabaseUrl(sqlx::Error),
    #[error("download {0} was not found")]
    NotFound(DownloadId),
    #[error("invalid persisted download field '{field}': {reason}")]
    InvalidPersistedData { field: &'static str, reason: String },
    #[error("download field '{field}' exceeds SQLite's signed integer range")]
    IntegerOutOfRange { field: &'static str },
    #[error("destination path cannot be represented as UTF-8")]
    NonUtf8Destination,
}

impl StorageError {
    pub(crate) fn invalid_data(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidPersistedData {
            field,
            reason: reason.into(),
        }
    }
}

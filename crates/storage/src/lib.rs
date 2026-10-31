#![forbid(unsafe_code)]

mod database;
mod error;
mod export;
mod maintenance;
mod repository;
mod sqlite;

pub use database::Database;
pub use error::StorageError;
pub use export::{DatabaseExport, EXPORT_FORMAT_VERSION, ExportResult, ImportResult};
pub use maintenance::{
    BackupResult, CURRENT_SCHEMA_VERSION, CheckStatus, DatabaseCheck, DatabaseSchemaState,
    MIN_SUPPORTED_SCHEMA_VERSION, SchemaMigrationStatus,
};
pub use repository::{
    DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
    TorrentSourceType,
};
pub use sqlite::{
    SqliteConfigRepository, SqliteDownloadRepository, SqliteTorrentMetadataRepository,
};

#[cfg(test)]
mod tests;

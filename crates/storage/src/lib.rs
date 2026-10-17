#![forbid(unsafe_code)]

mod database;
mod error;
mod repository;
mod sqlite;

pub use database::Database;
pub use error::StorageError;
pub use repository::{
    DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
    TorrentSourceType,
};
pub use sqlite::{
    SqliteConfigRepository, SqliteDownloadRepository, SqliteTorrentMetadataRepository,
};

#[cfg(test)]
mod tests;

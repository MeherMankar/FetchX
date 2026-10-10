#![forbid(unsafe_code)]

mod database;
mod error;
mod repository;
mod sqlite;

pub use database::Database;
pub use error::StorageError;
pub use repository::DownloadRepository;
pub use sqlite::{SqliteConfigRepository, SqliteDownloadRepository};

#[cfg(test)]
mod tests;

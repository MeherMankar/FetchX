use std::{borrow::Cow, path::Path, str::FromStr, sync::LazyLock, time::Duration};

use sqlx::{
    SqlitePool,
    migrate::{Migration, MigrationType, Migrator},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{SqliteDownloadRepository, StorageError};

static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| Migrator {
    migrations: Cow::Owned(vec![Migration::new(
        1,
        Cow::Borrowed("create_downloads"),
        MigrationType::Simple,
        Cow::Borrowed(include_str!(
            "../../../migrations/0001_create_downloads.sql"
        )),
        false,
    )]),
    ..Migrator::DEFAULT
});

/// Owns the SQLite connection pool and initializes the schema on connection.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Connects to the caller-selected SQLite URL and applies embedded migrations.
    ///
    /// In-memory databases use one pooled connection so every operation sees the same
    /// database. File-backed databases use a small async pool.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let mut options = SqliteConnectOptions::from_str(database_url)
            .map_err(StorageError::InvalidDatabaseUrl)?;
        let is_in_memory = options.get_filename() == Path::new(":memory:");
        options = options
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(if is_in_memory { 1 } else { 5 })
            .connect_with(options)
            .await?;
        let database = Self { pool };
        database.migrate().await?;
        Ok(database)
    }

    /// Applies pending embedded migrations. Safe to call after `connect`.
    pub async fn migrate(&self) -> Result<(), StorageError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// Returns a repository for download records.
    #[must_use]
    pub fn downloads(&self) -> SqliteDownloadRepository {
        SqliteDownloadRepository::new(self.pool.clone())
    }

    #[cfg(test)]
    pub(crate) const fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

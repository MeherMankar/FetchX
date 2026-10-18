use std::{borrow::Cow, path::Path, str::FromStr, sync::LazyLock, time::Duration};

use sqlx::{
    SqlitePool,
    migrate::{Migration, MigrationType, Migrator},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{
    SqliteConfigRepository, SqliteDownloadRepository, SqliteTorrentMetadataRepository, StorageError,
};

static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| Migrator {
    migrations: Cow::Owned(vec![
        Migration::new(
            1,
            Cow::Borrowed("create_downloads"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0001_create_downloads.sql"
            )),
            false,
        ),
        Migration::new(
            2,
            Cow::Borrowed("add_queue_order"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!("../../../migrations/0002_add_queue_order.sql")),
            false,
        ),
        Migration::new(
            3,
            Cow::Borrowed("create_download_schedules"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0003_create_download_schedules.sql"
            )),
            false,
        ),
        Migration::new(
            4,
            Cow::Borrowed("create_engine_config"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0004_create_engine_config.sql"
            )),
            false,
        ),
        Migration::new(
            5,
            Cow::Borrowed("add_ytdlp_format"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0005_add_ytdlp_format.sql"
            )),
            false,
        ),
        Migration::new(
            6,
            Cow::Borrowed("add_hls_engine_kind"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0006_add_hls_engine_kind.sql"
            )),
            false,
        ),
        Migration::new(
            7,
            Cow::Borrowed("torrent_metadata"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0007_torrent_metadata.sql"
            )),
            false,
        ),
        Migration::new(
            8,
            Cow::Borrowed("magnet_metadata"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!("../../../migrations/0008_magnet_metadata.sql")),
            false,
        ),
        Migration::new(
            9,
            Cow::Borrowed("native_torrent_engine"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0009_native_torrent_engine.sql"
            )),
            false,
        ),
    ]),
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

        Self::connect_with_options(options, is_in_memory).await
    }

    /// Connects to a file-backed database without interpreting the path as a URL.
    ///
    /// This preserves legal path characters such as percent signs and question marks.
    pub async fn connect_path(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        Self::connect_with_options(options, false).await
    }

    async fn connect_with_options(
        options: SqliteConnectOptions,
        is_in_memory: bool,
    ) -> Result<Self, StorageError> {
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

    /// Returns a repository for application configuration key-value entries.
    #[must_use]
    pub fn config(&self) -> SqliteConfigRepository {
        SqliteConfigRepository::new(self.pool.clone())
    }

    /// Returns a repository for durable torrent metadata rows.
    #[must_use]
    pub fn torrent_metadata(&self) -> SqliteTorrentMetadataRepository {
        SqliteTorrentMetadataRepository::new(self.pool.clone())
    }

    #[cfg(test)]
    pub(crate) const fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

use std::{borrow::Cow, path::Path, str::FromStr, sync::LazyLock, time::Duration};

use fetchx_models::{
    DiagnosticComponent, DiagnosticCounter, DiagnosticEvent, DiagnosticLevel, global_diagnostics,
};
use sqlx::{
    SqlitePool,
    migrate::{Migration, MigrationType, Migrator},
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{
    SqliteConfigRepository, SqliteDownloadRepository, SqliteTorrentMetadataRepository, StorageError,
};

pub(crate) static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| Migrator {
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
        Migration::new(
            10,
            Cow::Borrowed("add_ftp_sftp_metalink_engines"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0010_add_ftp_sftp_metalink_engines.sql"
            )),
            false,
        ),
        Migration::new(
            11,
            Cow::Borrowed("allow_merged_ytdlp_format_selectors"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../migrations/0011_allow_merged_ytdlp_format_selectors.sql"
            )),
            false,
        ),
    ]),
    ..Migrator::DEFAULT
});

/// Owns the SQLite connection pool and initializes the schema on connection.
#[derive(Clone)]
pub struct Database {
    pub(crate) pool: SqlitePool,
    pub(crate) path: Option<std::path::PathBuf>,
}

impl Database {
    #[cfg(test)]
    pub(crate) fn migration_count() -> usize {
        MIGRATOR.migrations.len()
    }

    /// Connects to the caller-selected SQLite URL and applies embedded migrations.
    ///
    /// In-memory databases use one pooled connection so every operation sees the same
    /// database. File-backed databases use a small async pool.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let mut options = SqliteConnectOptions::from_str(database_url)
            .map_err(StorageError::InvalidDatabaseUrl)?;
        let is_in_memory = options.get_filename() == Path::new(":memory:");
        let path = (!is_in_memory).then(|| options.get_filename().to_path_buf());
        options = options
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        Self::connect_with_options(options, is_in_memory, path).await
    }

    /// Connects to a file-backed database without interpreting the path as a URL.
    ///
    /// This preserves legal path characters such as percent signs and question marks.
    pub async fn connect_path(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let database_path = path.as_ref().to_path_buf();
        let options = SqliteConnectOptions::new()
            .filename(&database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        Self::connect_with_options(options, false, Some(database_path)).await
    }

    /// Opens an existing file without creating it or applying migrations.
    pub async fn open_existing_path(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_path_with_options(path, false).await
    }

    /// Opens an existing file read-only without applying migrations.
    pub async fn open_read_only_path(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_path_with_options(path, true).await
    }

    async fn open_path_with_options(
        path: impl AsRef<Path>,
        read_only: bool,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .read_only(read_only)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Ok(Self {
            pool,
            path: Some(path.to_path_buf()),
        })
    }

    async fn connect_with_options(
        options: SqliteConnectOptions,
        is_in_memory: bool,
        path: Option<std::path::PathBuf>,
    ) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(if is_in_memory { 1 } else { 5 })
            .connect_with(options)
            .await?;
        let database = Self { pool, path };
        database.migrate().await?;
        Ok(database)
    }

    /// Applies pending embedded migrations. Safe to call after `connect`.
    pub async fn migrate(&self) -> Result<(), StorageError> {
        let diagnostics = global_diagnostics();
        diagnostics.increment(DiagnosticCounter::StorageMigrationAttempts);
        diagnostics.record(DiagnosticEvent::new(
            DiagnosticComponent::Storage,
            "migration_started",
            DiagnosticLevel::Info,
        ));
        let result = async {
            let recovery_snapshot = crate::maintenance::prepare_migration(self).await?;
            if let Err(error) = MIGRATOR.run(&self.pool).await {
                if let Some(path) = recovery_snapshot {
                    return Err(StorageError::MigrationRecovery {
                        message: error.to_string(),
                        backup_path: path.display().to_string(),
                    });
                }
                return Err(StorageError::Migration(error));
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            diagnostics.increment(DiagnosticCounter::StorageMigrationFailures);
            diagnostics.record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Storage,
                    "migration_failed",
                    DiagnosticLevel::Error,
                )
                .with_error_code("STORAGE_MIGRATION_FAILED"),
            );
            tracing::warn!(
                error_code = "STORAGE_MIGRATION_FAILED",
                "database migration failed"
            );
        } else {
            diagnostics.record(DiagnosticEvent::new(
                DiagnosticComponent::Storage,
                "migration_completed",
                DiagnosticLevel::Info,
            ));
            tracing::info!("database migration completed");
        }
        result
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

    /// Closes all database connections and waits for active operations to finish.
    pub async fn close(self) {
        self.pool.close().await;
    }

    #[cfg(test)]
    pub(crate) const fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

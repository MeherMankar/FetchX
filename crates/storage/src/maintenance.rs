use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use fetchx_models::{
    DiagnosticComponent, DiagnosticCounter, DiagnosticEvent, DiagnosticLevel, global_diagnostics,
};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::{Database, StorageError, database::MIGRATOR};

pub const MIN_SUPPORTED_SCHEMA_VERSION: i64 = 1;
pub const CURRENT_SCHEMA_VERSION: i64 = 11;
static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const REQUIRED_TABLES: &[&str] = &[
    "downloads",
    "download_schedules",
    "app_config",
    "torrent_metadata",
];
const REQUIRED_INDEXES: &[&str] = &[
    "downloads_created_at_id",
    "downloads_queue_order",
    "download_schedules_enabled_time",
    "torrent_metadata_gid",
    "torrent_metadata_seed_state",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DatabaseSchemaState {
    UpToDate,
    MigrationRequired,
    MigrationInProgress,
    MigrationFailed,
    UnsupportedFutureVersion,
    Corrupted,
    EmptyDatabase,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckStatus {
    Ok,
    Failed,
    NotChecked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SchemaMigrationStatus {
    pub state: DatabaseSchemaState,
    pub current_schema_version: i64,
    pub database_schema_version: Option<i64>,
    pub minimum_supported_schema_version: i64,
    pub maximum_supported_schema_version: i64,
    pub migration_path: Vec<i64>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DatabaseCheck {
    pub database_path: Option<String>,
    pub database_size_bytes: Option<u64>,
    pub schema: SchemaMigrationStatus,
    pub sqlite_integrity: CheckStatus,
    pub sqlite_integrity_detail: Vec<String>,
    pub foreign_keys: CheckStatus,
    pub foreign_key_violations: u64,
    pub required_tables: CheckStatus,
    pub missing_tables: Vec<String>,
    pub required_indexes: CheckStatus,
    pub missing_indexes: Vec<String>,
    pub table_counts: BTreeMap<String, u64>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl DatabaseCheck {
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.schema.state == DatabaseSchemaState::UpToDate
            && self.sqlite_integrity == CheckStatus::Ok
            && self.foreign_keys == CheckStatus::Ok
            && self.required_tables == CheckStatus::Ok
            && self.required_indexes == CheckStatus::Ok
            && self.errors.is_empty()
    }

    fn empty(path: &Path) -> Self {
        Self {
            database_path: Some(path.display().to_string()),
            database_size_bytes: None,
            schema: SchemaMigrationStatus {
                state: DatabaseSchemaState::EmptyDatabase,
                current_schema_version: CURRENT_SCHEMA_VERSION,
                database_schema_version: None,
                minimum_supported_schema_version: MIN_SUPPORTED_SCHEMA_VERSION,
                maximum_supported_schema_version: CURRENT_SCHEMA_VERSION,
                migration_path: (MIN_SUPPORTED_SCHEMA_VERSION..=CURRENT_SCHEMA_VERSION).collect(),
                detail: Some("database file does not exist".to_owned()),
            },
            sqlite_integrity: CheckStatus::NotChecked,
            sqlite_integrity_detail: Vec::new(),
            foreign_keys: CheckStatus::NotChecked,
            foreign_key_violations: 0,
            required_tables: CheckStatus::NotChecked,
            missing_tables: Vec::new(),
            required_indexes: CheckStatus::NotChecked,
            missing_indexes: Vec::new(),
            table_counts: BTreeMap::new(),
            errors: Vec::new(),
            warnings: vec!["database file does not exist".to_owned()],
        }
    }

    fn open_failed(path: &Path, error: &str) -> Self {
        let mut check = Self::empty(path);
        check.schema.state = DatabaseSchemaState::Corrupted;
        check.schema.detail = Some("database could not be opened".to_owned());
        check
            .errors
            .push(format!("database could not be opened: {error}"));
        check.warnings.clear();
        check
    }
}

impl Database {
    /// Performs diagnostics without applying migrations or changing persisted state.
    pub async fn check(&self) -> Result<DatabaseCheck, StorageError> {
        let schema = inspect_schema(&self.pool).await;
        let mut errors = Vec::new();
        let (sqlite_integrity, sqlite_integrity_detail) = match integrity_check(&self.pool).await {
            Ok(messages)
                if !messages.is_empty() && messages.iter().all(|message| message == "ok") =>
            {
                (CheckStatus::Ok, messages)
            }
            Ok(messages) => {
                errors.extend(messages.iter().filter(|message| *message != "ok").cloned());
                (CheckStatus::Failed, messages)
            }
            Err(error) => {
                errors.push(format!("SQLite integrity check failed: {error}"));
                (CheckStatus::Failed, vec![error])
            }
        };
        let (foreign_keys, foreign_key_violations) = match sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&self.pool)
            .await
        {
            Ok(rows) => (
                if rows.is_empty() {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Failed
                },
                rows.len() as u64,
            ),
            Err(error) => {
                errors.push(format!("foreign-key check failed: {error}"));
                (CheckStatus::Failed, 0)
            }
        };
        if sqlite_integrity == CheckStatus::Failed || foreign_keys == CheckStatus::Failed {
            global_diagnostics().increment(DiagnosticCounter::StorageIntegrityFailures);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Storage,
                    "integrity_check_failed",
                    DiagnosticLevel::Error,
                )
                .with_error_code("STORAGE_INTEGRITY_FAILED"),
            );
        }
        let tables = list_schema_objects(&self.pool, "table").await;
        let indexes = list_schema_objects(&self.pool, "index").await;
        let missing_tables = match &tables {
            Ok(objects) => REQUIRED_TABLES
                .iter()
                .filter(|name| !objects.iter().any(|object| object == *name))
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
            Err(error) => {
                errors.push(format!("required-table check failed: {error}"));
                REQUIRED_TABLES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect()
            }
        };
        let missing_indexes = match &indexes {
            Ok(objects) => REQUIRED_INDEXES
                .iter()
                .filter(|name| !objects.iter().any(|object| object == *name))
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
            Err(error) => {
                errors.push(format!("required-index check failed: {error}"));
                REQUIRED_INDEXES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect()
            }
        };
        let (table_counts, count_errors) = collect_table_counts(&self.pool, &tables).await;
        errors.extend(count_errors);
        let (database_path, database_size_bytes) = match &self.path {
            Some(path) => (
                Some(path.display().to_string()),
                std::fs::metadata(path).ok().map(|metadata| metadata.len()),
            ),
            None => (None, None),
        };
        let mut warnings = Vec::new();
        if schema.state == DatabaseSchemaState::MigrationRequired {
            warnings.push("database requires schema migrations".to_owned());
        }
        if schema.state == DatabaseSchemaState::UnsupportedFutureVersion {
            errors.push("database schema is newer than this application supports".to_owned());
        }
        if schema.state == DatabaseSchemaState::Corrupted {
            errors.push(
                schema
                    .detail
                    .clone()
                    .unwrap_or_else(|| "migration history is inconsistent".to_owned()),
            );
        }

        Ok(DatabaseCheck {
            database_path,
            database_size_bytes,
            schema,
            sqlite_integrity,
            sqlite_integrity_detail,
            foreign_keys,
            foreign_key_violations,
            required_tables: if missing_tables.is_empty() {
                CheckStatus::Ok
            } else {
                CheckStatus::Failed
            },
            missing_tables,
            required_indexes: if missing_indexes.is_empty() {
                CheckStatus::Ok
            } else {
                CheckStatus::Failed
            },
            missing_indexes,
            table_counts,
            errors,
            warnings,
        })
    }

    /// Read-only database check. Missing or unopenable files become diagnostics.
    pub async fn check_path(path: impl AsRef<Path>) -> DatabaseCheck {
        let path = path.as_ref();
        match std::fs::metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return DatabaseCheck::empty(path);
            }
            Err(error) => return DatabaseCheck::open_failed(path, &error.to_string()),
            Ok(_) => {}
        }
        match Self::open_read_only_path(path).await {
            Ok(database) => database
                .check()
                .await
                .unwrap_or_else(|error| DatabaseCheck::open_failed(path, &error.to_string())),
            Err(error) => DatabaseCheck::open_failed(path, &error.to_string()),
        }
    }

    /// Creates a consistent SQLite snapshot and validates it before returning.
    pub async fn backup_to(
        &self,
        destination: impl AsRef<Path>,
    ) -> Result<BackupResult, StorageError> {
        let result = self.backup_to_inner(destination.as_ref()).await;
        if result.is_err() {
            global_diagnostics().increment(DiagnosticCounter::StorageBackupFailures);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Storage,
                    "backup_failed",
                    DiagnosticLevel::Error,
                )
                .with_error_code("STORAGE_BACKUP_FAILED"),
            );
            tracing::warn!(
                error_code = "STORAGE_BACKUP_FAILED",
                "database backup failed"
            );
        } else {
            global_diagnostics().record(DiagnosticEvent::new(
                DiagnosticComponent::Storage,
                "backup_completed",
                DiagnosticLevel::Info,
            ));
            tracing::info!("database backup completed");
        }
        result
    }

    async fn backup_to_inner(&self, destination: &Path) -> Result<BackupResult, StorageError> {
        if destination.exists() {
            return Err(StorageError::Maintenance(
                "backup destination already exists".to_owned(),
            ));
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err(StorageError::Maintenance(
                "backup destination parent directory does not exist".to_owned(),
            ));
        }
        if let (Some(database_path), Ok(destination_path)) =
            (&self.path, std::fs::canonicalize(parent))
        {
            let destination_path =
                destination_path.join(destination.file_name().ok_or_else(|| {
                    StorageError::Maintenance("invalid backup destination".to_owned())
                })?);
            if std::fs::canonicalize(database_path).ok().as_ref() == Some(&destination_path) {
                return Err(StorageError::Maintenance(
                    "backup destination must differ from the live database".to_owned(),
                ));
            }
        }
        create_snapshot(&self.pool, destination).await?;
        validate_snapshot(destination).await?;
        let metadata = std::fs::metadata(destination)
            .map_err(|error| StorageError::Maintenance(error.to_string()))?;
        let schema = inspect_snapshot_schema(destination).await?;
        Ok(BackupResult {
            path: destination.to_path_buf(),
            size_bytes: metadata.len(),
            schema_version: schema.database_schema_version,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BackupResult {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub schema_version: Option<i64>,
}

pub(crate) async fn prepare_migration(
    database: &Database,
) -> Result<Option<PathBuf>, StorageError> {
    let status = inspect_schema(&database.pool).await;
    match status.state {
        DatabaseSchemaState::UpToDate => return Ok(None),
        DatabaseSchemaState::MigrationRequired | DatabaseSchemaState::EmptyDatabase => {}
        DatabaseSchemaState::UnsupportedFutureVersion
        | DatabaseSchemaState::MigrationInProgress
        | DatabaseSchemaState::MigrationFailed
        | DatabaseSchemaState::Corrupted => {
            return Err(StorageError::UnsupportedSchema(
                status
                    .detail
                    .unwrap_or_else(|| format!("{:?}", status.state)),
            ));
        }
    }
    let Some(database_path) = database.path.as_deref() else {
        return Ok(None);
    };
    let has_existing_data = std::fs::metadata(database_path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false);
    if !has_existing_data || status.state == DatabaseSchemaState::EmptyDatabase {
        return Ok(None);
    }
    let snapshot = recovery_snapshot_path(database_path)?;
    create_snapshot(&database.pool, &snapshot).await?;
    validate_snapshot(&snapshot).await?;
    Ok(Some(snapshot))
}

fn recovery_snapshot_path(database_path: &Path) -> Result<PathBuf, StorageError> {
    let filename = database_path
        .file_name()
        .ok_or_else(|| StorageError::RecoverySnapshot("database path has no filename".into()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| StorageError::RecoverySnapshot(error.to_string()))?
        .as_millis();
    for attempt in 0..100_u64 {
        let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed) + attempt;
        let mut name = filename.to_os_string();
        name.push(format!(".pre-migration-{timestamp}-{sequence}.bak"));
        let candidate = database_path.with_file_name(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(StorageError::RecoverySnapshot(
        "could not choose a unique snapshot filename".to_owned(),
    ))
}

async fn create_snapshot(pool: &SqlitePool, destination: &Path) -> Result<(), StorageError> {
    let destination = destination
        .to_str()
        .ok_or(StorageError::NonUtf8Destination)?;
    let quoted = destination.replace('\'', "''");
    let statement = format!("VACUUM INTO '{quoted}'");
    sqlx::query(&statement)
        .execute(pool)
        .await
        .map_err(|error| StorageError::RecoverySnapshot(error.to_string()))?;
    Ok(())
}

async fn validate_snapshot(path: &Path) -> Result<(), StorageError> {
    let database = Database::open_read_only_path(path).await.map_err(|error| {
        StorageError::BackupValidation(format!("snapshot cannot be opened: {error}"))
    })?;
    let integrity = integrity_check(&database.pool)
        .await
        .map_err(StorageError::BackupValidation)?;
    if integrity.iter().any(|message| message != "ok") {
        return Err(StorageError::BackupValidation(integrity.join("; ")));
    }
    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&database.pool)
        .await
        .map_err(|error| StorageError::BackupValidation(error.to_string()))?;
    if !foreign_key_violations.is_empty() {
        return Err(StorageError::BackupValidation(format!(
            "snapshot contains {} foreign-key violation(s)",
            foreign_key_violations.len()
        )));
    }
    let schema = inspect_schema(&database.pool).await;
    if matches!(
        schema.state,
        DatabaseSchemaState::Corrupted
            | DatabaseSchemaState::UnsupportedFutureVersion
            | DatabaseSchemaState::MigrationFailed
            | DatabaseSchemaState::MigrationInProgress
    ) {
        return Err(StorageError::BackupValidation(
            schema
                .detail
                .unwrap_or_else(|| "snapshot migration history is invalid".to_owned()),
        ));
    }
    Ok(())
}

async fn inspect_snapshot_schema(path: &Path) -> Result<SchemaMigrationStatus, StorageError> {
    let database = Database::open_read_only_path(path).await?;
    Ok(inspect_schema(&database.pool).await)
}

async fn inspect_schema(pool: &SqlitePool) -> SchemaMigrationStatus {
    let default = |state, version, detail| SchemaMigrationStatus {
        state,
        current_schema_version: CURRENT_SCHEMA_VERSION,
        database_schema_version: version,
        minimum_supported_schema_version: MIN_SUPPORTED_SCHEMA_VERSION,
        maximum_supported_schema_version: CURRENT_SCHEMA_VERSION,
        migration_path: Vec::new(),
        detail,
    };
    let table_exists = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    {
        Ok(count) => count != 0,
        Err(error) => {
            return default(
                DatabaseSchemaState::Corrupted,
                None,
                Some(error.to_string()),
            );
        }
    };
    if !table_exists {
        let user_tables = list_schema_objects(pool, "table").await;
        return match user_tables {
            Ok(tables) if tables.iter().all(|table| table == "sqlite_sequence") => default(
                DatabaseSchemaState::EmptyDatabase,
                None,
                Some("database has not been initialized".to_owned()),
            ),
            Ok(_) => default(
                DatabaseSchemaState::Corrupted,
                None,
                Some("application tables exist without SQLx migration history".to_owned()),
            ),
            Err(error) => default(DatabaseSchemaState::Corrupted, None, Some(error)),
        };
    }
    let rows = match sqlx::query(
        "SELECT version, description, success, checksum
         FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return default(
                DatabaseSchemaState::Corrupted,
                None,
                Some(error.to_string()),
            );
        }
    };
    let mut previous_version = 0_i64;
    for row in &rows {
        let version: i64 = row.get("version");
        let description: String = row.get("description");
        let success: bool = row.get("success");
        let checksum: Vec<u8> = row.get("checksum");
        if !success {
            return default(
                DatabaseSchemaState::MigrationFailed,
                Some(version),
                Some(format!("migration {version} is marked unsuccessful")),
            );
        }
        if version > CURRENT_SCHEMA_VERSION {
            return default(
                DatabaseSchemaState::UnsupportedFutureVersion,
                Some(version),
                Some(format!(
                    "database version {version} exceeds supported version {CURRENT_SCHEMA_VERSION}"
                )),
            );
        }
        if version != previous_version + 1 {
            return default(
                DatabaseSchemaState::Corrupted,
                Some(version),
                Some(format!(
                    "migration history skips version {}",
                    previous_version + 1
                )),
            );
        }
        let Some(migration) = MIGRATOR
            .iter()
            .find(|migration| migration.version == version)
        else {
            return default(
                DatabaseSchemaState::Corrupted,
                Some(version),
                Some(format!("migration {version} is unavailable in this build")),
            );
        };
        if migration.description.as_ref() != description || migration.checksum.as_ref() != checksum
        {
            return default(
                DatabaseSchemaState::Corrupted,
                Some(version),
                Some(format!(
                    "migration {version} checksum or description changed"
                )),
            );
        }
        previous_version = version;
    }
    if previous_version < MIN_SUPPORTED_SCHEMA_VERSION {
        let mut status = default(
            DatabaseSchemaState::MigrationRequired,
            Some(previous_version),
            None,
        );
        status.migration_path = (previous_version + 1..=CURRENT_SCHEMA_VERSION).collect();
        return status;
    }
    if previous_version < CURRENT_SCHEMA_VERSION {
        let mut status = default(
            DatabaseSchemaState::MigrationRequired,
            Some(previous_version),
            None,
        );
        status.migration_path = (previous_version + 1..=CURRENT_SCHEMA_VERSION).collect();
        return status;
    }
    default(DatabaseSchemaState::UpToDate, Some(previous_version), None)
}

async fn integrity_check(pool: &SqlitePool) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())
}

async fn list_schema_objects(pool: &SqlitePool, object_type: &str) -> Result<Vec<String>, String> {
    let query = match object_type {
        "table" => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        }
        "index" => {
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%'"
        }
        _ => return Err("unsupported SQLite object type".to_owned()),
    };
    sqlx::query_scalar::<_, String>(query)
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())
}

async fn collect_table_counts(
    pool: &SqlitePool,
    tables: &Result<Vec<String>, String>,
) -> (BTreeMap<String, u64>, Vec<String>) {
    let mut counts = BTreeMap::new();
    let mut errors = Vec::new();
    let Ok(tables) = tables else {
        return (counts, errors);
    };
    for table in REQUIRED_TABLES {
        if tables.iter().any(|name| name == table) {
            let query = match *table {
                "downloads" => "SELECT COUNT(*) FROM downloads",
                "download_schedules" => "SELECT COUNT(*) FROM download_schedules",
                "app_config" => "SELECT COUNT(*) FROM app_config",
                "torrent_metadata" => "SELECT COUNT(*) FROM torrent_metadata",
                _ => continue,
            };
            match sqlx::query_scalar::<_, i64>(query).fetch_one(pool).await {
                Ok(count) => {
                    counts.insert((*table).to_owned(), count.max(0) as u64);
                }
                Err(error) => errors.push(format!("could not count {table}: {error}")),
            }
        }
    }
    (counts, errors)
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, error::Error};

    use sqlx::{
        migrate::Migrator,
        sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    };

    use super::*;
    use crate::{Database, DatabaseSchemaState};

    async fn create_version_10_database(path: &Path) -> Result<(), Box<dyn Error>> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let migrator = Migrator {
            migrations: Cow::Owned(MIGRATOR.iter().take(10).cloned().collect()),
            ..Migrator::DEFAULT
        };
        migrator.run(&pool).await?;
        pool.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn current_schema_health_check_is_read_only_and_complete() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("fetchx.db");
        let database = Database::connect_path(&path).await?;
        let check = database.check().await?;
        assert_eq!(check.schema.state, DatabaseSchemaState::UpToDate);
        assert_eq!(
            check.schema.database_schema_version,
            Some(CURRENT_SCHEMA_VERSION)
        );
        assert!(check.is_healthy(), "{check:?}");
        assert_eq!(check.table_counts.get("downloads"), Some(&0));
        Ok(())
    }

    #[tokio::test]
    async fn missing_and_corrupt_databases_are_reported_without_recreation()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let missing = directory.path().join("missing.db");
        let missing_check = Database::check_path(&missing).await;
        assert_eq!(
            missing_check.schema.state,
            DatabaseSchemaState::EmptyDatabase
        );
        assert!(!missing.exists());

        let corrupt = directory.path().join("corrupt.db");
        std::fs::write(&corrupt, b"not a sqlite database")?;
        let corrupt_check = Database::check_path(&corrupt).await;
        assert_eq!(corrupt_check.schema.state, DatabaseSchemaState::Corrupted);
        assert_eq!(std::fs::read(&corrupt)?, b"not a sqlite database");
        Ok(())
    }

    #[tokio::test]
    async fn database_with_application_tables_but_no_migration_history_is_rejected()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("wrong-schema.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await?;
        sqlx::query("CREATE TABLE downloads (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await?;
        pool.close().await;

        let check = Database::check_path(&path).await;
        assert_eq!(check.schema.state, DatabaseSchemaState::Corrupted);
        assert!(
            check
                .schema
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("without SQLx migration history"))
        );
        assert!(Database::connect_path(&path).await.is_err());
        assert_eq!(
            Database::check_path(&path).await.schema.state,
            DatabaseSchemaState::Corrupted
        );
        Ok(())
    }

    #[tokio::test]
    async fn inconsistent_migration_history_is_rejected_without_rewriting_database()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("inconsistent.db");
        let database = Database::connect_path(&path).await?;
        sqlx::query("UPDATE _sqlx_migrations SET description = 'tampered' WHERE version = 10")
            .execute(&database.pool)
            .await?;
        database.pool.close().await;

        let check = Database::check_path(&path).await;
        assert_eq!(check.schema.state, DatabaseSchemaState::Corrupted);
        let error = match Database::connect_path(&path).await {
            Ok(_) => panic!("tampered migration history should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("unsupported"));
        let database = Database::open_existing_path(&path).await?;
        let description: String =
            sqlx::query_scalar("SELECT description FROM _sqlx_migrations WHERE version = 10")
                .fetch_one(&database.pool)
                .await?;
        assert_eq!(description, "tampered");
        Ok(())
    }

    #[tokio::test]
    async fn backup_uses_sqlite_snapshot_and_refuses_overwrite() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("fetchx.db");
        let backup_path = directory.path().join("fetchx.db.bak");
        let database = Database::connect_path(&path).await?;
        sqlx::query("INSERT INTO app_config (key, value) VALUES ('safe-test', '1')")
            .execute(&database.pool)
            .await?;

        let backup = database.backup_to(&backup_path).await?;
        assert!(backup.size_bytes > 0);
        assert_eq!(backup.schema_version, Some(CURRENT_SCHEMA_VERSION));
        let check = Database::check_path(&backup_path).await;
        assert!(check.is_healthy(), "{check:?}");
        assert_eq!(check.table_counts.get("app_config"), Some(&1));

        assert!(database.backup_to(&backup_path).await.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT value FROM app_config WHERE key = 'safe-test'")
                .fetch_one(&database.pool)
                .await?,
            "1"
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_backup_leaves_source_healthy_and_does_not_create_destination()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.db");
        let database = Database::connect_path(&path).await?;
        let missing_parent = directory.path().join("missing").join("backup.db");

        assert!(database.backup_to(&missing_parent).await.is_err());
        assert!(!missing_parent.exists());
        assert!(database.check().await?.is_healthy());
        assert!(Database::check_path(&path).await.is_healthy());
        Ok(())
    }

    #[tokio::test]
    async fn migration_from_supported_old_version_creates_recovery_snapshot()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("old.db");
        create_version_10_database(&path).await?;
        let database = Database::connect_path(&path).await?;
        assert_eq!(
            database.check().await?.schema.database_schema_version,
            Some(CURRENT_SCHEMA_VERSION)
        );
        let snapshots = std::fs::read_dir(directory.path())?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("pre-migration")
            })
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 1);
        let snapshot_check = Database::check_path(snapshots[0].path()).await;
        assert_eq!(
            snapshot_check.schema.database_schema_version,
            Some(CURRENT_SCHEMA_VERSION - 1)
        );
        assert_eq!(
            snapshot_check.schema.state,
            DatabaseSchemaState::MigrationRequired
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_migration_preserves_recovery_snapshot_and_original_file()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("old.db");
        create_version_10_database(&path).await?;
        let setup = Database::open_existing_path(&path).await?;
        sqlx::query("CREATE TABLE downloads_new (id TEXT)")
            .execute(&setup.pool)
            .await?;
        setup.pool.close().await;

        let error = Database::connect_path(&path)
            .await
            .err()
            .ok_or("expected migration to fail")?;
        assert!(error.to_string().contains("recovery snapshot"));
        let snapshots = std::fs::read_dir(directory.path())?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("pre-migration")
            })
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 1);
        let snapshot_check = Database::check_path(snapshots[0].path()).await;
        assert_eq!(
            snapshot_check.schema.database_schema_version,
            Some(CURRENT_SCHEMA_VERSION - 1)
        );
        let original_check = Database::check_path(&path).await;
        assert_eq!(
            original_check.schema.database_schema_version,
            Some(CURRENT_SCHEMA_VERSION - 1)
        );
        assert!(path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn unsupported_future_schema_is_not_migrated() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("future.db");
        let database = Database::connect_path(&path).await?;
        sqlx::query("UPDATE _sqlx_migrations SET version = 99 WHERE version = 11")
            .execute(&database.pool)
            .await?;
        database.pool.close().await;

        let check = Database::check_path(&path).await;
        assert_eq!(
            check.schema.state,
            DatabaseSchemaState::UnsupportedFutureVersion
        );
        let error = Database::connect_path(&path)
            .await
            .err()
            .ok_or("expected refusal")?;
        assert!(error.to_string().contains("unsupported"));
        assert_eq!(
            Database::check_path(&path)
                .await
                .schema
                .database_schema_version,
            Some(99)
        );
        Ok(())
    }
}

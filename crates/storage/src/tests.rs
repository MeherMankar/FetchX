use std::{error::Error, path::PathBuf};

use mydownloader_models::{
    DownloadError, DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadProgress,
    DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
};

use crate::{Database, DownloadRepository, StorageError};

fn item(id: DownloadId, with_error: bool) -> Result<DownloadItem, Box<dyn Error>> {
    let source = DownloadSource::new("https://example.test/archive.zip", EngineKind::DirectHttp)?;
    let filename = FileName::new("archive.zip")?;
    let progress = DownloadProgress::new(1536, Some(4096), Some(512), Some(5))?;
    let error = with_error
        .then(|| DownloadError::new(DownloadErrorCode::Engine, "engine stopped"))
        .transpose()?;

    Ok(DownloadItem::from_parts(
        id,
        source,
        filename,
        PathBuf::from("downloads/archive.zip"),
        DownloadStatus::Paused,
        DownloadPriority::High,
        progress,
        Timestamp::from_unix_millis(1_725_000_000_123),
        Timestamp::from_unix_millis(1_725_000_000_456),
        error,
    )?)
}

#[tokio::test]
async fn connect_runs_migrations_and_migrate_is_idempotent() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    database.migrate().await?;

    let table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'downloads'",
    )
    .fetch_one(database.pool())
    .await?;
    assert_eq!(table_count, 1);

    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(database.pool())
            .await?;
    assert_eq!(migration_count, 1);

    Ok(())
}

#[tokio::test]
async fn repository_round_trips_and_performs_crud() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();

    let first = item(DownloadId::new(), true)?;
    let second = item(DownloadId::new(), false)?;
    repository.create(&first).await?;
    repository.create(&second).await?;

    let retrieved = repository
        .get(first.id())
        .await?
        .ok_or("created download was not found")?;
    assert_eq!(retrieved, first);
    assert_eq!(
        retrieved.error().map(DownloadError::code),
        Some(DownloadErrorCode::Engine)
    );
    assert_eq!(
        retrieved.error().map(DownloadError::message),
        Some("engine stopped")
    );
    assert_eq!(retrieved.progress().downloaded_bytes(), 1536);
    assert_eq!(retrieved.progress().total_bytes(), Some(4096));
    assert_eq!(retrieved.progress().speed_bytes_per_second(), Some(512));
    assert_eq!(retrieved.progress().estimated_remaining_seconds(), Some(5));
    assert_eq!(retrieved.created_at().as_unix_millis(), 1_725_000_000_123);
    assert_eq!(retrieved.updated_at().as_unix_millis(), 1_725_000_000_456);
    assert_eq!(retrieved.id(), first.id());

    let listed = repository.list().await?;
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|download| download == &first));
    assert!(listed.iter().any(|download| download == &second));

    let updated = DownloadItem::from_parts(
        first.id(),
        DownloadSource::new("https://example.test/archive.zip", EngineKind::Aria2)?,
        FileName::new("renamed.zip")?,
        PathBuf::from("downloads/renamed.zip"),
        DownloadStatus::Completed,
        DownloadPriority::Low,
        DownloadProgress::new(4096, Some(4096), None, Some(0))?,
        first.created_at(),
        Timestamp::from_unix_millis(1_725_000_001_000),
        None,
    )?;
    repository.update(&updated).await?;
    assert_eq!(repository.get(first.id()).await?, Some(updated));

    assert!(repository.delete(second.id()).await?);
    assert!(!repository.delete(second.id()).await?);
    assert_eq!(repository.get(second.id()).await?, None);
    assert_eq!(repository.list().await?.len(), 1);

    Ok(())
}

#[tokio::test]
async fn update_reports_missing_items() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let missing = item(DownloadId::new(), false)?;

    assert!(matches!(
        repository.update(&missing).await,
        Err(StorageError::NotFound(id)) if id == missing.id()
    ));
    Ok(())
}

#[tokio::test]
async fn timestamp_conditional_update_rejects_stale_snapshots() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let original = item(DownloadId::new(), false)?;
    repository.create(&original).await?;

    let newest_timestamp = Timestamp::from_unix_millis(original.updated_at().as_unix_millis() + 1);
    let mut newest = original.clone();
    newest.set_progress_at(
        DownloadProgress::new(2048, Some(4096), None, None)?,
        newest_timestamp,
    )?;
    assert!(
        repository
            .update_if_status_and_timestamp_current(
                &newest,
                original.status(),
                original.updated_at(),
            )
            .await?
    );

    let mut stale = original.clone();
    stale.set_progress_at(
        DownloadProgress::new(1600, Some(4096), None, None)?,
        Timestamp::from_unix_millis(newest_timestamp.as_unix_millis() + 1),
    )?;
    assert!(
        !repository
            .update_if_status_and_timestamp_current(
                &stale,
                original.status(),
                original.updated_at(),
            )
            .await?
    );
    assert_eq!(repository.get(original.id()).await?, Some(newest));
    Ok(())
}

#[tokio::test]
async fn schema_rejects_invalid_enum_values() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let download = item(DownloadId::new(), false)?;
    repository.create(&download).await?;

    let update = sqlx::query("UPDATE downloads SET status = 'future_status' WHERE id = ?")
        .bind(download.id().to_string())
        .execute(database.pool())
        .await;

    assert!(matches!(update, Err(sqlx::Error::Database(_))));
    assert_eq!(repository.get(download.id()).await?, Some(download));
    Ok(())
}

#[tokio::test]
async fn values_outside_sqlite_integer_range_are_rejected() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let download = DownloadItem::from_parts(
        DownloadId::new(),
        DownloadSource::new("https://example.test/file", EngineKind::DirectHttp)?,
        FileName::new("file")?,
        PathBuf::from("file"),
        DownloadStatus::Queued,
        DownloadPriority::Normal,
        DownloadProgress::new((i64::MAX as u64) + 1, None, None, None)?,
        Timestamp::from_unix_millis(0),
        Timestamp::from_unix_millis(0),
        None,
    )?;

    assert!(matches!(
        repository.create(&download).await,
        Err(StorageError::IntegerOutOfRange {
            field: "downloaded_bytes"
        })
    ));
    Ok(())
}

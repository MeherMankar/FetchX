use std::sync::atomic::{AtomicU64, Ordering};
use std::{error::Error, path::PathBuf};

use fetchx_models::{
    DownloadError, DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadProgress,
    DownloadSchedule, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
};

use crate::{Database, DownloadRepository, StorageError};

static TEMP_DATABASE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    assert_eq!(migration_count, 5);

    Ok(())
}

#[tokio::test]
async fn connect_path_preserves_special_path_characters() -> Result<(), Box<dyn Error>> {
    let special_characters = if cfg!(windows) { "# %20" } else { "? # %20" };
    let path = std::env::temp_dir().join(format!(
        "FetchX-path-{}-{}.db",
        std::process::id(),
        special_characters
    ));
    {
        let database = Database::connect_path(&path).await?;
        assert!(database.downloads().list().await?.is_empty());
        database.pool().close().await;
    }
    std::fs::remove_file(path)?;
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
async fn repository_persists_selected_ytdlp_format() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let id = DownloadId::new();
    let mut download = DownloadItem::new(
        id,
        DownloadSource::new("https://example.test/video", EngineKind::YtDlp)?
            .with_yt_dlp_format("137")?,
        FileName::new("video.mp4")?,
        PathBuf::from("downloads/video.mp4"),
        Timestamp::now()?,
    );
    repository.create(&download).await?;
    assert_eq!(
        repository
            .get(id)
            .await?
            .ok_or("selected-format download was not found")?
            .source()
            .yt_dlp_format(),
        Some("137")
    );

    let now = Timestamp::now()?;
    download.set_priority_at(DownloadPriority::High, now)?;
    repository.update(&download).await?;
    assert_eq!(
        repository
            .get(id)
            .await?
            .ok_or("updated selected-format download was not found")?
            .source()
            .yt_dlp_format(),
        Some("137")
    );
    Ok(())
}

#[tokio::test]
async fn queue_order_and_priority_updates_preserve_download_progress() -> Result<(), Box<dyn Error>>
{
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let first = item(DownloadId::new(), false)?;
    let second = item(DownloadId::new(), false)?;
    repository.create(&first).await?;
    repository.create(&second).await?;

    let ordered = repository.list().await?;
    assert_eq!(
        ordered.iter().map(DownloadItem::id).collect::<Vec<_>>(),
        [first.id(), second.id()]
    );
    repository.move_to_queue_tail(first.id()).await?;

    let reordered = repository.list().await?;
    assert_eq!(
        reordered.iter().map(DownloadItem::id).collect::<Vec<_>>(),
        [second.id(), first.id()]
    );

    let original = repository
        .get(first.id())
        .await?
        .ok_or("download was not found")?;
    assert!(
        repository
            .set_priority(first.id(), DownloadPriority::Low)
            .await?
    );
    let updated = repository
        .get(first.id())
        .await?
        .ok_or("download was not found")?;
    assert_eq!(updated.priority(), DownloadPriority::Low);
    assert_eq!(updated.progress(), original.progress());
    assert_eq!(updated.updated_at(), original.updated_at());

    Ok(())
}

#[tokio::test]
async fn schedules_round_trip_update_and_delete() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let download = item(DownloadId::new(), false)?;
    repository.create(&download).await?;
    let schedule = DownloadSchedule::new(
        download.id(),
        Timestamp::from_unix_millis(1_800_000_000_000),
        true,
        Timestamp::from_unix_millis(1_700_000_000_000),
    );
    repository.save_schedule(&schedule).await?;
    assert_eq!(
        repository.get_schedule(download.id()).await?,
        Some(schedule)
    );
    assert_eq!(repository.list_schedules().await?, [schedule]);

    let mut disabled = schedule;
    disabled.set_enabled(false, Timestamp::from_unix_millis(1_700_000_000_001))?;
    repository.save_schedule(&disabled).await?;
    assert_eq!(
        repository.get_schedule(download.id()).await?,
        Some(disabled)
    );
    assert!(repository.delete_schedule(download.id()).await?);
    assert!(!repository.delete_schedule(download.id()).await?);
    assert!(repository.list_schedules().await?.is_empty());
    assert_eq!(repository.get(download.id()).await?, Some(download));
    Ok(())
}

#[tokio::test]
async fn schedules_survive_database_reopen_and_deleted_schedules_stay_deleted()
-> Result<(), Box<dyn Error>> {
    let path = std::env::temp_dir().join(format!(
        "FetchX-schedules-{}-{}.db",
        std::process::id(),
        TEMP_DATABASE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let scheduled_item;
    let unscheduled_item;
    let expected_schedule;
    {
        let database = Database::connect(&url).await?;
        let repository = database.downloads();
        scheduled_item = item(DownloadId::new(), false)?;
        unscheduled_item = item(DownloadId::new(), false)?;
        repository.create(&scheduled_item).await?;
        repository.create(&unscheduled_item).await?;
        expected_schedule = DownloadSchedule::new(
            scheduled_item.id(),
            Timestamp::from_unix_millis(1_800_000_000_000),
            false,
            Timestamp::from_unix_millis(1_700_000_000_000),
        );
        repository.save_schedule(&expected_schedule).await?;
    }
    {
        let database = Database::connect(&url).await?;
        let repository = database.downloads();
        assert_eq!(
            repository.get_schedule(scheduled_item.id()).await?,
            Some(expected_schedule)
        );
        assert_eq!(repository.get_schedule(unscheduled_item.id()).await?, None);
        assert!(repository.delete_schedule(scheduled_item.id()).await?);
    }
    {
        let database = Database::connect(&url).await?;
        let repository = database.downloads();
        assert_eq!(repository.get_schedule(scheduled_item.id()).await?, None);
        assert_eq!(
            repository.get(scheduled_item.id()).await?,
            Some(scheduled_item)
        );
        assert_eq!(
            repository.get(unscheduled_item.id()).await?,
            Some(unscheduled_item)
        );
        database.pool().close().await;
    }
    std::fs::remove_file(path)?;
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

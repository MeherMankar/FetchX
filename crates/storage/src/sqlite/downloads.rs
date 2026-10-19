use std::path::PathBuf;

use fetchx_models::{
    DownloadError, DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadProgress,
    DownloadSchedule, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::{DownloadRepository, StorageError};

/// SQLite-backed implementation of [`DownloadRepository`].
#[derive(Clone)]
pub struct SqliteDownloadRepository {
    pool: SqlitePool,
}

impl SqliteDownloadRepository {
    pub(crate) const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl DownloadRepository for SqliteDownloadRepository {
    async fn create(&self, item: &DownloadItem) -> Result<(), StorageError> {
        let destination = item
            .destination()
            .to_str()
            .ok_or(StorageError::NonUtf8Destination)?;
        let progress = item.progress();
        let error = item.error();

        sqlx::query(
            "INSERT INTO downloads (
                id, source_url, engine_kind, yt_dlp_format, filename, destination, status, priority,
                downloaded_bytes, total_bytes, speed_bytes_per_second,
                estimated_remaining_seconds, created_at, updated_at, error_code, error_message,
                queue_order
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                (SELECT COALESCE(MAX(queue_order), 0) + 1 FROM downloads))",
        )
        .bind(item.id().to_string())
        .bind(item.source().url())
        .bind(engine_to_str(item.source().engine()))
        .bind(item.source().yt_dlp_format())
        .bind(item.filename().as_str())
        .bind(destination)
        .bind(status_to_str(item.status()))
        .bind(priority_to_str(item.priority()))
        .bind(to_sqlite_integer(
            "downloaded_bytes",
            progress.downloaded_bytes(),
        )?)
        .bind(option_to_sqlite_integer(
            "total_bytes",
            progress.total_bytes(),
        )?)
        .bind(option_to_sqlite_integer(
            "speed_bytes_per_second",
            progress.speed_bytes_per_second(),
        )?)
        .bind(option_to_sqlite_integer(
            "estimated_remaining_seconds",
            progress.estimated_remaining_seconds(),
        )?)
        .bind(to_sqlite_integer(
            "created_at",
            item.created_at().as_unix_millis(),
        )?)
        .bind(to_sqlite_integer(
            "updated_at",
            item.updated_at().as_unix_millis(),
        )?)
        .bind(error.map(|value| error_code_to_str(value.code())))
        .bind(error.map(DownloadError::message))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: DownloadId) -> Result<Option<DownloadItem>, StorageError> {
        let row = sqlx::query(
            "SELECT id, source_url, engine_kind, yt_dlp_format, filename, destination, status, priority,
                    downloaded_bytes, total_bytes, speed_bytes_per_second,
                    estimated_remaining_seconds, created_at, updated_at, error_code, error_message
             FROM downloads
             WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(row_to_item).transpose()
    }

    async fn list(&self) -> Result<Vec<DownloadItem>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, source_url, engine_kind, yt_dlp_format, filename, destination, status, priority,
                    downloaded_bytes, total_bytes, speed_bytes_per_second,
                    estimated_remaining_seconds, created_at, updated_at, error_code, error_message
             FROM downloads
             ORDER BY queue_order, id",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_item).collect()
    }

    async fn move_to_queue_tail(&self, id: DownloadId) -> Result<(), StorageError> {
        let result = sqlx::query(
            "UPDATE downloads
             SET queue_order = (SELECT COALESCE(MAX(queue_order), 0) + 1 FROM downloads)
             WHERE id = ?",
        )
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(id));
        }
        Ok(())
    }

    async fn set_priority(
        &self,
        id: DownloadId,
        priority: DownloadPriority,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query("UPDATE downloads SET priority = ? WHERE id = ?")
            .bind(priority_to_str(priority))
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn save_schedule(&self, schedule: &DownloadSchedule) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO download_schedules (
                download_id, scheduled_at_ms, enabled, created_at_ms, updated_at_ms
             ) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(download_id) DO UPDATE SET
                scheduled_at_ms = excluded.scheduled_at_ms,
                enabled = excluded.enabled,
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(schedule.download_id().to_string())
        .bind(to_sqlite_integer(
            "scheduled_at_ms",
            schedule.scheduled_at().as_unix_millis(),
        )?)
        .bind(i64::from(schedule.enabled()))
        .bind(to_sqlite_integer(
            "created_at_ms",
            schedule.created_at().as_unix_millis(),
        )?)
        .bind(to_sqlite_integer(
            "updated_at_ms",
            schedule.updated_at().as_unix_millis(),
        )?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_schedule(&self, id: DownloadId) -> Result<Option<DownloadSchedule>, StorageError> {
        let row = sqlx::query(
            "SELECT download_id, scheduled_at_ms, enabled, created_at_ms, updated_at_ms
             FROM download_schedules
             WHERE download_id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_schedule).transpose()
    }

    async fn list_schedules(&self) -> Result<Vec<DownloadSchedule>, StorageError> {
        let rows = sqlx::query(
            "SELECT download_id, scheduled_at_ms, enabled, created_at_ms, updated_at_ms
             FROM download_schedules
             ORDER BY scheduled_at_ms, download_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_schedule).collect()
    }

    async fn delete_schedule(&self, id: DownloadId) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM download_schedules WHERE download_id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn update(&self, item: &DownloadItem) -> Result<(), StorageError> {
        let destination = item
            .destination()
            .to_str()
            .ok_or(StorageError::NonUtf8Destination)?;
        let progress = item.progress();
        let error = item.error();

        let result = sqlx::query(
            "UPDATE downloads SET
                source_url = ?, engine_kind = ?, yt_dlp_format = ?, filename = ?, destination = ?, status = ?,
                priority = ?, downloaded_bytes = ?, total_bytes = ?,
                speed_bytes_per_second = ?, estimated_remaining_seconds = ?,
                created_at = ?, updated_at = ?, error_code = ?, error_message = ?
             WHERE id = ?",
        )
        .bind(item.source().url())
        .bind(engine_to_str(item.source().engine()))
        .bind(item.source().yt_dlp_format())
        .bind(item.filename().as_str())
        .bind(destination)
        .bind(status_to_str(item.status()))
        .bind(priority_to_str(item.priority()))
        .bind(to_sqlite_integer(
            "downloaded_bytes",
            progress.downloaded_bytes(),
        )?)
        .bind(option_to_sqlite_integer(
            "total_bytes",
            progress.total_bytes(),
        )?)
        .bind(option_to_sqlite_integer(
            "speed_bytes_per_second",
            progress.speed_bytes_per_second(),
        )?)
        .bind(option_to_sqlite_integer(
            "estimated_remaining_seconds",
            progress.estimated_remaining_seconds(),
        )?)
        .bind(to_sqlite_integer(
            "created_at",
            item.created_at().as_unix_millis(),
        )?)
        .bind(to_sqlite_integer(
            "updated_at",
            item.updated_at().as_unix_millis(),
        )?)
        .bind(error.map(|value| error_code_to_str(value.code())))
        .bind(error.map(DownloadError::message))
        .bind(item.id().to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(item.id()));
        }

        Ok(())
    }

    async fn update_if_status_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
    ) -> Result<bool, StorageError> {
        conditional_update(&self.pool, item, expected_status, None).await
    }

    async fn update_if_status_and_timestamp_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
        expected_updated_at: Timestamp,
    ) -> Result<bool, StorageError> {
        conditional_update(&self.pool, item, expected_status, Some(expected_updated_at)).await
    }

    async fn delete(&self, id: DownloadId) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM downloads WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() != 0)
    }
}

async fn conditional_update(
    pool: &SqlitePool,
    item: &DownloadItem,
    expected_status: DownloadStatus,
    expected_updated_at: Option<Timestamp>,
) -> Result<bool, StorageError> {
    let destination = item
        .destination()
        .to_str()
        .ok_or(StorageError::NonUtf8Destination)?;
    let progress = item.progress();
    let error = item.error();
    let expected_updated_at = expected_updated_at
        .map(|timestamp| to_sqlite_integer("updated_at", timestamp.as_unix_millis()))
        .transpose()?;
    let result = sqlx::query(
        "UPDATE downloads SET
            source_url = ?, engine_kind = ?, yt_dlp_format = ?, filename = ?, destination = ?, status = ?,
            priority = ?, downloaded_bytes = ?, total_bytes = ?,
            speed_bytes_per_second = ?, estimated_remaining_seconds = ?,
            created_at = ?, updated_at = ?, error_code = ?, error_message = ?
         WHERE id = ? AND status = ? AND (? IS NULL OR updated_at = ?)",
    )
    .bind(item.source().url())
    .bind(engine_to_str(item.source().engine()))
    .bind(item.source().yt_dlp_format())
    .bind(item.filename().as_str())
    .bind(destination)
    .bind(status_to_str(item.status()))
    .bind(priority_to_str(item.priority()))
    .bind(to_sqlite_integer(
        "downloaded_bytes",
        progress.downloaded_bytes(),
    )?)
    .bind(option_to_sqlite_integer(
        "total_bytes",
        progress.total_bytes(),
    )?)
    .bind(option_to_sqlite_integer(
        "speed_bytes_per_second",
        progress.speed_bytes_per_second(),
    )?)
    .bind(option_to_sqlite_integer(
        "estimated_remaining_seconds",
        progress.estimated_remaining_seconds(),
    )?)
    .bind(to_sqlite_integer(
        "created_at",
        item.created_at().as_unix_millis(),
    )?)
    .bind(to_sqlite_integer(
        "updated_at",
        item.updated_at().as_unix_millis(),
    )?)
    .bind(error.map(|value| error_code_to_str(value.code())))
    .bind(error.map(DownloadError::message))
    .bind(item.id().to_string())
    .bind(status_to_str(expected_status))
    .bind(expected_updated_at)
    .bind(expected_updated_at)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

fn row_to_item(row: SqliteRow) -> Result<DownloadItem, StorageError> {
    let id_text: String = row.try_get("id")?;
    let id = DownloadId::parse(&id_text)
        .map_err(|error| StorageError::invalid_data("id", error.to_string()))?;
    if id.to_string() != id_text {
        return Err(StorageError::invalid_data(
            "id",
            "download ID is not in canonical string form",
        ));
    }

    let source_url: String = row.try_get("source_url")?;
    let engine_text: String = row.try_get("engine_kind")?;
    let engine = engine_from_str(&engine_text)?;
    let yt_dlp_format: Option<String> = row.try_get("yt_dlp_format")?;
    let source = DownloadSource::new(source_url, engine)
        .and_then(|source| match yt_dlp_format {
            Some(format_id) => source.with_yt_dlp_format(format_id),
            None => Ok(source),
        })
        .map_err(|error| StorageError::invalid_data("source_url", error.to_string()))?;

    let filename_text: String = row.try_get("filename")?;
    let filename = FileName::new(filename_text)
        .map_err(|error| StorageError::invalid_data("filename", error.to_string()))?;

    let status_text: String = row.try_get("status")?;
    let status = status_from_str(&status_text)?;
    let priority_text: String = row.try_get("priority")?;
    let priority = priority_from_str(&priority_text)?;

    let downloaded_bytes =
        from_sqlite_integer("downloaded_bytes", row.try_get("downloaded_bytes")?)?;
    let total_bytes = optional_from_sqlite_integer("total_bytes", row.try_get("total_bytes")?)?;
    let speed_bytes_per_second = optional_from_sqlite_integer(
        "speed_bytes_per_second",
        row.try_get("speed_bytes_per_second")?,
    )?;
    let estimated_remaining_seconds = optional_from_sqlite_integer(
        "estimated_remaining_seconds",
        row.try_get("estimated_remaining_seconds")?,
    )?;
    let progress = DownloadProgress::new(
        downloaded_bytes,
        total_bytes,
        speed_bytes_per_second,
        estimated_remaining_seconds,
    )
    .map_err(|error| StorageError::invalid_data("progress", error.to_string()))?;

    let created_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "created_at",
        row.try_get("created_at")?,
    )?);
    let updated_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "updated_at",
        row.try_get("updated_at")?,
    )?);

    let error_code: Option<String> = row.try_get("error_code")?;
    let error_message: Option<String> = row.try_get("error_message")?;
    let error =
        match (error_code, error_message) {
            (None, None) => None,
            (Some(code), Some(message)) => {
                let code = error_code_from_str(&code)?;
                Some(DownloadError::new(code, message).map_err(|error| {
                    StorageError::invalid_data("error_message", error.to_string())
                })?)
            }
            _ => {
                return Err(StorageError::invalid_data(
                    "error",
                    "error code and message must both be null or both be present",
                ));
            }
        };

    let destination: String = row.try_get("destination")?;
    DownloadItem::from_parts(
        id,
        source,
        filename,
        PathBuf::from(destination),
        status,
        priority,
        progress,
        created_at,
        updated_at,
        error,
    )
    .map_err(|error| StorageError::invalid_data("updated_at", error.to_string()))
}

fn row_to_schedule(row: SqliteRow) -> Result<DownloadSchedule, StorageError> {
    let id_text: String = row.try_get("download_id")?;
    let id = DownloadId::parse(&id_text)
        .map_err(|error| StorageError::invalid_data("download_id", error.to_string()))?;
    if id.to_string() != id_text {
        return Err(StorageError::invalid_data(
            "download_id",
            "download ID is not in canonical string form",
        ));
    }
    let enabled: i64 = row.try_get("enabled")?;
    let enabled = match enabled {
        0 => false,
        1 => true,
        _ => {
            return Err(StorageError::invalid_data(
                "enabled",
                "schedule enabled flag must be zero or one",
            ));
        }
    };
    let scheduled_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "scheduled_at_ms",
        row.try_get("scheduled_at_ms")?,
    )?);
    let created_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "created_at_ms",
        row.try_get("created_at_ms")?,
    )?);
    let updated_at = Timestamp::from_unix_millis(from_sqlite_integer(
        "updated_at_ms",
        row.try_get("updated_at_ms")?,
    )?);
    DownloadSchedule::from_parts(id, scheduled_at, enabled, created_at, updated_at)
        .map_err(|error| StorageError::invalid_data("updated_at_ms", error.to_string()))
}

fn engine_to_str(value: EngineKind) -> &'static str {
    match value {
        EngineKind::DirectHttp => "direct_http",
        EngineKind::Aria2 => "aria2",
        EngineKind::YtDlp => "yt_dlp",
        EngineKind::Hls => "hls",
        EngineKind::NativeTorrent => "native_torrent",
        EngineKind::Ftp => "ftp",
        EngineKind::Ftps => "ftps",
        EngineKind::Sftp => "sftp",
        EngineKind::Metalink => "metalink",
    }
}

fn engine_from_str(value: &str) -> Result<EngineKind, StorageError> {
    match value {
        "direct_http" => Ok(EngineKind::DirectHttp),
        "aria2" => Ok(EngineKind::Aria2),
        "yt_dlp" => Ok(EngineKind::YtDlp),
        "hls" => Ok(EngineKind::Hls),
        "native_torrent" => Ok(EngineKind::NativeTorrent),
        "ftp" => Ok(EngineKind::Ftp),
        "ftps" => Ok(EngineKind::Ftps),
        "sftp" => Ok(EngineKind::Sftp),
        "metalink" => Ok(EngineKind::Metalink),
        _ => Err(StorageError::invalid_data(
            "engine_kind",
            "unknown engine kind",
        )),
    }
}

fn status_to_str(value: DownloadStatus) -> &'static str {
    match value {
        DownloadStatus::Queued => "queued",
        DownloadStatus::Preparing => "preparing",
        DownloadStatus::Downloading => "downloading",
        DownloadStatus::Paused => "paused",
        DownloadStatus::Completed => "completed",
        DownloadStatus::Failed => "failed",
        DownloadStatus::Cancelled => "cancelled",
        DownloadStatus::MetadataAcquiring => "metadata_acquiring",
        DownloadStatus::Seeding => "seeding",
        DownloadStatus::Recovering => "recovering",
        DownloadStatus::Stopped => "stopped",
    }
}

fn status_from_str(value: &str) -> Result<DownloadStatus, StorageError> {
    match value {
        "queued" => Ok(DownloadStatus::Queued),
        "preparing" => Ok(DownloadStatus::Preparing),
        "downloading" => Ok(DownloadStatus::Downloading),
        "paused" => Ok(DownloadStatus::Paused),
        "completed" => Ok(DownloadStatus::Completed),
        "failed" => Ok(DownloadStatus::Failed),
        "cancelled" => Ok(DownloadStatus::Cancelled),
        "metadata_acquiring" => Ok(DownloadStatus::MetadataAcquiring),
        "seeding" => Ok(DownloadStatus::Seeding),
        "recovering" => Ok(DownloadStatus::Recovering),
        "stopped" => Ok(DownloadStatus::Stopped),
        _ => Err(StorageError::invalid_data(
            "status",
            "unknown download status",
        )),
    }
}

fn priority_to_str(value: DownloadPriority) -> &'static str {
    match value {
        DownloadPriority::Low => "low",
        DownloadPriority::Normal => "normal",
        DownloadPriority::High => "high",
    }
}

fn priority_from_str(value: &str) -> Result<DownloadPriority, StorageError> {
    match value {
        "low" => Ok(DownloadPriority::Low),
        "normal" => Ok(DownloadPriority::Normal),
        "high" => Ok(DownloadPriority::High),
        _ => Err(StorageError::invalid_data(
            "priority",
            "unknown download priority",
        )),
    }
}

fn error_code_to_str(value: DownloadErrorCode) -> &'static str {
    match value {
        DownloadErrorCode::Source => "source",
        DownloadErrorCode::Destination => "destination",
        DownloadErrorCode::Engine => "engine",
        DownloadErrorCode::Internal => "internal",
    }
}

fn error_code_from_str(value: &str) -> Result<DownloadErrorCode, StorageError> {
    match value {
        "source" => Ok(DownloadErrorCode::Source),
        "destination" => Ok(DownloadErrorCode::Destination),
        "engine" => Ok(DownloadErrorCode::Engine),
        "internal" => Ok(DownloadErrorCode::Internal),
        _ => Err(StorageError::invalid_data(
            "error_code",
            "unknown download error code",
        )),
    }
}

fn to_sqlite_integer(field: &'static str, value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::IntegerOutOfRange { field })
}

fn option_to_sqlite_integer(
    field: &'static str,
    value: Option<u64>,
) -> Result<Option<i64>, StorageError> {
    value
        .map(|value| to_sqlite_integer(field, value))
        .transpose()
}

fn from_sqlite_integer(field: &'static str, value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::invalid_data(field, "negative integer"))
}

fn optional_from_sqlite_integer(
    field: &'static str,
    value: Option<i64>,
) -> Result<Option<u64>, StorageError> {
    value
        .map(|value| from_sqlite_integer(field, value))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::{engine_from_str, error_code_from_str, priority_from_str, status_from_str};
    use crate::StorageError;

    #[test]
    fn unknown_enum_values_are_rejected() {
        assert!(matches!(
            engine_from_str("unknown"),
            Err(StorageError::InvalidPersistedData {
                field: "engine_kind",
                ..
            })
        ));
        assert!(matches!(
            status_from_str("unknown"),
            Err(StorageError::InvalidPersistedData {
                field: "status",
                ..
            })
        ));
        assert!(matches!(
            priority_from_str("unknown"),
            Err(StorageError::InvalidPersistedData {
                field: "priority",
                ..
            })
        ));
        assert!(matches!(
            error_code_from_str("unknown"),
            Err(StorageError::InvalidPersistedData {
                field: "error_code",
                ..
            })
        ));
    }
}

/// Configuration passed from Android to initialise CoreServices.
/// Maps to the `dictionary CoreConfig` in android_bridge.udl.
pub struct CoreConfig {
    pub database_path: String,
    pub downloads_dir: String,
    pub runtime_dir: String,
    pub yt_dlp_executable_path: Option<String>,
    pub aria2_executable_path: Option<String>,
}

/// Mirrors `DownloadStatus` across the FFI boundary.
/// Maps to the `enum DownloadStatusDto` in android_bridge.udl.
pub enum DownloadStatusDto {
    Queued,
    Preparing,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
    MetadataAcquiring,
    Seeding,
    Recovering,
    Stopped,
}

/// Mirrors `DownloadPriority` across the FFI boundary.
/// Only three variants — CRITICAL is not present in the Rust domain model.
/// Maps to the `enum DownloadPriorityDto` in android_bridge.udl.
pub enum DownloadPriorityDto {
    Low,
    Normal,
    High,
}

/// Progress snapshot with `-1` sentinels replacing `Option<u64>` fields.
/// Maps to the `dictionary DownloadProgressDto` in android_bridge.udl.
pub struct DownloadProgressDto {
    pub downloaded_bytes: u64,
    /// `-1` means unknown.
    pub total_bytes: i64,
    /// `-1` means unknown.
    pub speed_bytes_per_second: i64,
    /// `-1` means unknown.
    pub estimated_remaining_seconds: i64,
}

/// Flat representation of a download item for cross-language transfer.
/// Maps to the `dictionary DownloadInfoDto` in android_bridge.udl.
pub struct DownloadInfoDto {
    pub id: String,
    pub url: String,
    pub engine: String,
    pub filename: String,
    pub destination: String,
    pub status: DownloadStatusDto,
    pub priority: DownloadPriorityDto,
    pub progress: DownloadProgressDto,
    pub created_at_millis: i64,
    pub updated_at_millis: i64,
    pub error_message: Option<String>,
}

pub struct DownloadRuntimeStatusDto {
    pub id: String,
    pub status: DownloadStatusDto,
    pub progress: DownloadRuntimeProgressDto,
    pub updated_at: u64,
}

pub struct DownloadRuntimeProgressDto {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: Option<u64>,
    pub estimated_remaining_seconds: Option<u64>,
    pub percentage_basis_points: Option<u16>,
}

pub struct DuplicateDownloadMatchDto {
    pub id: String,
    pub filename: String,
    pub status: DownloadStatusDto,
    pub updated_at: u64,
}

pub struct YtDlpFormatDto {
    pub format_id: String,
    pub label: String,
    pub extension: Option<String>,
    pub title: Option<String>,
    pub platform: Option<String>,
}

pub struct TorrentTelemetryDto {
    pub download_speed_bps: Option<u64>,
    pub upload_speed_bps: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub peer_count: Option<u64>,
    pub completed_peer_count: Option<u64>,
    pub seed_count: Option<u64>,
    pub share_ratio: Option<f64>,
    pub seeding: Option<bool>,
    pub tracker_uris: Option<Vec<String>>,
}

pub struct TorrentFileDto {
    pub index: u32,
    pub path: String,
    pub size_bytes: u64,
}

pub struct TorrentMetadataDto {
    pub name: String,
    pub info_hash: String,
    pub total_size_bytes: u64,
    pub files: Vec<TorrentFileDto>,
    pub trackers: Vec<String>,
    pub piece_length: u64,
    pub piece_count: u64,
}

pub struct TorrentDownloadMetadataDto {
    pub metadata_state: String,
    pub metadata: Option<TorrentMetadataDto>,
    pub selected_file_indexes: Vec<u32>,
    pub selected_total_bytes: u64,
    pub selection_confirmed: bool,
}

/// Event emitted by the Rust backend, forwarded to Kotlin listeners.
/// Maps to the `[Enum] interface DownloadEventDto` in android_bridge.udl.
pub enum DownloadEventDto {
    Created { item: DownloadInfoDto },
    Updated { item: DownloadInfoDto },
    Removed { id: String },
}

// ── Conversion helpers ────────────────────────────────────────────────────

use fetchx_models::{
    DownloadItem, DownloadPriority, DownloadProgress, DownloadRuntimeStatus, DownloadStatus,
    DuplicateDownloadMatch, EngineKind,
};
use fetchx_queue::DownloadEvent;

/// Converts a `DownloadStatus` to its DTO counterpart.
pub fn status_to_dto(s: DownloadStatus) -> DownloadStatusDto {
    match s {
        DownloadStatus::Queued => DownloadStatusDto::Queued,
        DownloadStatus::Preparing => DownloadStatusDto::Preparing,
        DownloadStatus::Downloading => DownloadStatusDto::Downloading,
        DownloadStatus::Paused => DownloadStatusDto::Paused,
        DownloadStatus::Completed => DownloadStatusDto::Completed,
        DownloadStatus::Failed => DownloadStatusDto::Failed,
        DownloadStatus::Cancelled => DownloadStatusDto::Cancelled,
        DownloadStatus::MetadataAcquiring => DownloadStatusDto::MetadataAcquiring,
        DownloadStatus::Seeding => DownloadStatusDto::Seeding,
        DownloadStatus::Recovering => DownloadStatusDto::Recovering,
        DownloadStatus::Stopped => DownloadStatusDto::Stopped,
    }
}

/// Converts a `DownloadPriority` to its DTO counterpart.
pub fn priority_to_dto(p: DownloadPriority) -> DownloadPriorityDto {
    match p {
        DownloadPriority::Low => DownloadPriorityDto::Low,
        DownloadPriority::Normal => DownloadPriorityDto::Normal,
        DownloadPriority::High => DownloadPriorityDto::High,
    }
}

/// Converts a DTO priority back to a model priority.
/// The DTO has no CRITICAL variant; High maps to High.
pub fn priority_from_dto(p: DownloadPriorityDto) -> DownloadPriority {
    match p {
        DownloadPriorityDto::Low => DownloadPriority::Low,
        DownloadPriorityDto::Normal => DownloadPriority::Normal,
        DownloadPriorityDto::High => DownloadPriority::High,
    }
}

/// Converts `DownloadProgress` to the flat DTO using `-1` sentinels for unknown fields.
pub fn progress_to_dto(p: &DownloadProgress) -> DownloadProgressDto {
    let total_bytes = p
        .total_bytes()
        .map(|v| i64::try_from(v).unwrap_or(i64::MAX))
        .unwrap_or(-1);
    let speed_bytes_per_second = p
        .speed_bytes_per_second()
        .map(|v| i64::try_from(v).unwrap_or(i64::MAX))
        .unwrap_or(-1);
    let estimated_remaining_seconds = p
        .estimated_remaining_seconds()
        .map(|v| i64::try_from(v).unwrap_or(i64::MAX))
        .unwrap_or(-1);

    DownloadProgressDto {
        downloaded_bytes: p.downloaded_bytes(),
        total_bytes,
        speed_bytes_per_second,
        estimated_remaining_seconds,
    }
}

/// Converts a `DownloadItem` to the flat `DownloadInfoDto`.
pub fn item_to_dto(item: &DownloadItem) -> DownloadInfoDto {
    let engine_str = match item.source().engine() {
        EngineKind::DirectHttp => "direct_http",
        EngineKind::Aria2 => "aria2",
        EngineKind::YtDlp => "yt_dlp",
        EngineKind::Hls => "hls",
        EngineKind::NativeTorrent => "native_torrent",
        EngineKind::Ftp => "ftp",
        EngineKind::Ftps => "ftps",
        EngineKind::Sftp => "sftp",
        EngineKind::Metalink => "metalink",
    };
    let error_message = item.error().map(|e| e.message().to_string());

    DownloadInfoDto {
        id: item.id().to_string(),
        url: item.source().url().to_string(),
        engine: engine_str.to_string(),
        filename: item.filename().as_str().to_string(),
        destination: item.destination().to_string_lossy().into_owned(),
        status: status_to_dto(item.status()),
        priority: priority_to_dto(item.priority()),
        progress: progress_to_dto(item.progress()),
        created_at_millis: i64::try_from(item.created_at().as_unix_millis()).unwrap_or(i64::MAX),
        updated_at_millis: i64::try_from(item.updated_at().as_unix_millis()).unwrap_or(i64::MAX),
        error_message,
    }
}

pub fn runtime_status_to_dto(status: &DownloadRuntimeStatus) -> DownloadRuntimeStatusDto {
    let progress = status.progress();
    DownloadRuntimeStatusDto {
        id: status.id().to_string(),
        status: status_to_dto(status.status()),
        progress: DownloadRuntimeProgressDto {
            downloaded_bytes: progress.downloaded_bytes(),
            total_bytes: progress.total_bytes(),
            speed_bytes_per_second: progress.speed_bytes_per_second(),
            estimated_remaining_seconds: progress.estimated_remaining_seconds(),
            percentage_basis_points: progress.percentage_basis_points(),
        },
        updated_at: status.updated_at().as_unix_millis(),
    }
}

pub fn duplicate_match_to_dto(item: &DuplicateDownloadMatch) -> DuplicateDownloadMatchDto {
    DuplicateDownloadMatchDto {
        id: item.id().to_string(),
        filename: item.filename().to_owned(),
        status: status_to_dto(item.status()),
        updated_at: item.updated_at().as_unix_millis(),
    }
}

/// Converts a `DownloadEvent` to the UDL-defined `DownloadEventDto`.
pub fn event_to_dto(e: DownloadEvent) -> DownloadEventDto {
    match e {
        DownloadEvent::Created { item, .. } => DownloadEventDto::Created {
            item: item_to_dto(&item),
        },
        DownloadEvent::Updated { item, .. } => DownloadEventDto::Updated {
            item: item_to_dto(&item),
        },
        DownloadEvent::Removed { id, .. } => DownloadEventDto::Removed { id: id.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fetchx_models::{DownloadId, DownloadSource, EngineKind, FileName, Timestamp};
    use std::path::PathBuf;

    #[test]
    fn runtime_status_dto_uses_shared_progress_shape() -> Result<(), Box<dyn std::error::Error>> {
        let progress = DownloadProgress::new(50, Some(100), Some(25), Some(2))?;
        let item = DownloadItem::from_parts(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file.bin", EngineKind::DirectHttp)?,
            FileName::new("file.bin")?,
            PathBuf::from("file.bin"),
            DownloadStatus::Downloading,
            DownloadPriority::default(),
            progress,
            Timestamp::from_unix_millis(10),
            Timestamp::from_unix_millis(20),
            None,
        )?;

        let dto = runtime_status_to_dto(&DownloadRuntimeStatus::from(&item));

        assert_eq!(dto.progress.downloaded_bytes, 50);
        assert_eq!(dto.progress.total_bytes, Some(100));
        assert_eq!(dto.progress.speed_bytes_per_second, Some(25));
        assert_eq!(dto.progress.estimated_remaining_seconds, Some(2));
        assert_eq!(dto.progress.percentage_basis_points, Some(5_000));
        assert_eq!(dto.updated_at, 20);
        Ok(())
    }
}

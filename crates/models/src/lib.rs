#![forbid(unsafe_code)]

mod download;
mod engine;
mod error;
mod ids;
mod priority;
mod progress;
mod resources;
mod schedule;
mod timestamp;
mod torrent;

pub use download::{
    DownloadItem, DownloadItemError, DownloadSource, DownloadSourceError, DownloadStatus, FileName,
    FileNameError, MagnetInfo, parse_magnet_uri,
};
pub use engine::EngineKind;
pub use error::{DownloadError, DownloadErrorCode, DownloadErrorMessageError};
pub use ids::{DownloadId, DownloadIdParseError};
pub use priority::DownloadPriority;
pub use progress::{DownloadProgress, DownloadProgressError};
pub use resources::{
    EngineBandwidthLimits, ResourcePolicy, ResourcePolicyError, ResourcePressure, ResourceProfile,
};
pub use schedule::{DownloadSchedule, DownloadScheduleError};
pub use timestamp::{Timestamp, TimestampError};
pub use torrent::{
    TorrentFile, TorrentFileSelection, TorrentMetadata, TorrentMetadataError, TorrentTelemetry,
    parse_torrent_metainfo,
};

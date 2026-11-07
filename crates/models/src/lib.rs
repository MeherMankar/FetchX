#![forbid(unsafe_code)]

mod diagnostics;
mod download;
mod engine;
mod error;
mod ids;
mod priority;
mod progress;
mod resources;
mod runtime;
mod schedule;
mod timestamp;
mod torrent;

pub use diagnostics::{
    ApplicationDiagnostics, DEFAULT_DIAGNOSTIC_EVENT_LIMIT, DIAGNOSTICS_SCHEMA_VERSION,
    DiagnosticComponent, DiagnosticComponentStatus, DiagnosticCounter, DiagnosticEvent,
    DiagnosticHealth, DiagnosticLevel, DiagnosticMetrics, DiagnosticsRecorder, DiagnosticsSnapshot,
    EngineDiagnostics, PluginDiagnostics, QueueDiagnostics, RpcDiagnostics, StorageDiagnostics,
    global_diagnostics, redact_sensitive_text, sanitize_url,
};
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
pub use runtime::DownloadRuntimeStatus;
pub use schedule::{DownloadSchedule, DownloadScheduleError};
pub use timestamp::{Timestamp, TimestampError};
pub use torrent::{
    TorrentDownloadMetadata, TorrentFile, TorrentFileSelection, TorrentMetadata,
    TorrentMetadataError, TorrentTelemetry, parse_torrent_metainfo,
};

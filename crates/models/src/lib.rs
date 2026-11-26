#![forbid(unsafe_code)]

mod credentials;
mod diagnostics;
mod download;
mod duplicate;
mod engine;
mod error;
mod folder_categories;
mod ids;
mod post_download;
mod post_queue_action;
mod priority;
mod progress;
mod proxy;
mod queue_definition;
mod resources;
mod runtime;
mod schedule;
mod timestamp;
mod torrent;

pub use credentials::{CredentialKey, HttpCredentials};
pub use diagnostics::{
    ApplicationDiagnostics, DEFAULT_DIAGNOSTIC_EVENT_LIMIT, DIAGNOSTICS_SCHEMA_VERSION,
    DiagnosticComponent, DiagnosticComponentStatus, DiagnosticCounter, DiagnosticEvent,
    DiagnosticHealth, DiagnosticLevel, DiagnosticMetrics, DiagnosticsRecorder, DiagnosticsSnapshot,
    EngineDiagnostics, PluginDiagnostics, QueueDiagnostics, RpcDiagnostics, StorageDiagnostics,
    global_diagnostics, redact_sensitive_text, sanitize_url,
};
pub use download::{
    DownloadItem, DownloadItemError, DownloadSource, DownloadSourceError, DownloadStatus, FileName,
    FileNameError, MagnetInfo, YtDlpAudioFormat, parse_magnet_uri,
};
pub use duplicate::DuplicateDownloadMatch;
pub use engine::EngineKind;
pub use error::{DownloadError, DownloadErrorCode, DownloadErrorMessageError};
pub use folder_categories::{CategoryRule, FolderCategoriesConfig};
pub use ids::{DownloadId, DownloadIdParseError};
pub use post_download::PostDownloadAction;
pub use post_queue_action::PostQueueAction;
pub use priority::DownloadPriority;
pub use progress::{DownloadProgress, DownloadProgressError};
pub use proxy::{DesktopProxyConfig, ProxyKind};
pub use queue_definition::{QueueDefinition, QueueProfile};
pub use resources::{
    BandwidthWindow, EngineBandwidthLimits, ResourcePolicy, ResourcePolicyError, ResourcePressure,
    ResourceProfile, WeeklyBandwidthSchedule, WeeklyQueueSchedule, WeeklyQueueScheduleError,
};
pub use runtime::DownloadRuntimeStatus;
pub use schedule::{DownloadSchedule, DownloadScheduleError};
pub use timestamp::{Timestamp, TimestampError};
pub use torrent::{
    TorrentDownloadMetadata, TorrentFile, TorrentFileSelection, TorrentMetadata,
    TorrentMetadataError, TorrentTelemetry, parse_torrent_metainfo,
};

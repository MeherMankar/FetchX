#![forbid(unsafe_code)]

mod adaptive;
mod aria2;
mod bandwidth;
mod dispatcher;
mod engine;
mod error;
mod ffmpeg;
mod ftp;
mod hls;
mod http;
mod media_extractor_adapter;
mod metalink;
mod native_torrent;
mod retry;
mod segments;
mod sftp;
mod supervisor;
mod ytdlp;

pub use adaptive::AdaptiveConfig;
pub use aria2::{
    Aria2Client, Aria2ClientConfig, Aria2ClientError, Aria2DownloadStatus, Aria2Engine, Aria2Gid,
    Aria2GlobalStat, Aria2RecoveryState, Aria2Status, Aria2Version,
};
pub use bandwidth::{BandwidthControl, BandwidthLimiter};
pub use dispatcher::{EngineDispatcher, is_dash_url};
pub use engine::{DownloadEngine, ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand};
pub use error::DownloadEngineError;
pub use ffmpeg::{
    FfmpegConfig, FfmpegError, FfmpegFuture, FfmpegProcessExit, FfmpegProcessRunner,
    FfmpegProcessSpec, FfmpegProcessor, FrameRate, MediaContainer, MediaExecutable, MediaInfo,
    MediaOperation, MediaProcessOutput, MediaProcessRequest, MediaProgress, MediaStream,
    MediaStreamKind, TranscodePreset,
};
pub use ftp::{FtpEngine, FtpEngineConfig};
pub use hls::{HlsEngine, HlsEngineConfig, is_hls_url};
#[cfg(feature = "phase30-queue-fault-injection")]
pub use http::inject_next_write_failure;
pub use http::{
    DirectHttpEngine, HttpEngineConfig, save_initial_context_headers, validate_http_source_url,
};
pub use media_extractor_adapter::{YtDlpProcessRunnerAdapter, parse_yt_dlp_json_to_raw_formats};
pub use metalink::{MetalinkEngine, MetalinkFile, MetalinkMirror};
pub use native_torrent::{NativeTorrentConfig, NativeTorrentEngine};
pub use sftp::{KnownHostsPolicy, SftpEngine, SftpEngineConfig, SftpKnownHost};
pub use supervisor::{
    Aria2ProcessLauncher, Aria2Supervisor, Aria2SupervisorConfig, Aria2SupervisorError,
    Aria2SupervisorState, ProcessExit, SupervisedProcess,
};
pub use ytdlp::{
    YtDlpConfig, YtDlpEngine, YtDlpError, YtDlpFormat, YtDlpProcessExit, YtDlpProcessRunner,
    YtDlpProgress, YtDlpRunnerEvent, YtDlpRunnerFuture, parse_format_output,
};

#[cfg(test)]
mod tests;

#![forbid(unsafe_code)]

mod adaptive;
mod aria2;
mod dispatcher;
mod engine;
mod error;
mod ffmpeg;
mod hls;
mod http;
mod retry;
mod segments;
mod supervisor;
mod ytdlp;

pub use adaptive::AdaptiveConfig;
pub use aria2::{
    Aria2Client, Aria2ClientConfig, Aria2ClientError, Aria2DownloadStatus, Aria2Engine, Aria2Gid,
    Aria2Status, Aria2Version,
};
pub use dispatcher::EngineDispatcher;
pub use engine::{DownloadEngine, ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand};
pub use error::DownloadEngineError;
pub use ffmpeg::{
    FfmpegConfig, FfmpegError, FfmpegFuture, FfmpegProcessExit, FfmpegProcessRunner,
    FfmpegProcessSpec, FfmpegProcessor, FrameRate, MediaContainer, MediaExecutable, MediaInfo,
    MediaOperation, MediaProcessOutput, MediaProcessRequest, MediaProgress, MediaStream,
    MediaStreamKind, TranscodePreset,
};
pub use hls::{HlsEngine, HlsEngineConfig, is_hls_url};
pub use http::{DirectHttpEngine, HttpEngineConfig, validate_http_source_url};
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

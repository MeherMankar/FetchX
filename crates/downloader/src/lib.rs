#![forbid(unsafe_code)]

mod adaptive;
mod aria2;
mod dispatcher;
mod engine;
mod error;
mod ffmpeg;
mod http;
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
pub use http::{DirectHttpEngine, HttpEngineConfig};
pub use supervisor::{
    Aria2ProcessLauncher, Aria2Supervisor, Aria2SupervisorConfig, Aria2SupervisorError,
    Aria2SupervisorState, ProcessExit, SupervisedProcess,
};
pub use ytdlp::{
    YtDlpConfig, YtDlpEngine, YtDlpError, YtDlpProcessExit, YtDlpProcessRunner, YtDlpProgress,
    YtDlpRunnerEvent, YtDlpRunnerFuture,
};

#[cfg(test)]
mod tests;

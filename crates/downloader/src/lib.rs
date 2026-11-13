#![forbid(unsafe_code)]

mod adaptive;
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
mod native_media;
mod native_mp4;
mod native_torrent;
mod retry;
mod segments;
mod sftp;
mod ytdlp;

pub use adaptive::AdaptiveConfig;
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
    DirectHttpEngine, HttpEngineConfig, remove_http_multi_source_set, save_http_multi_source_set,
    save_initial_context_headers, validate_http_source_url,
};
pub use media_extractor_adapter::{YtDlpProcessRunnerAdapter, parse_yt_dlp_json_to_raw_formats};
pub use metalink::{MetalinkEngine, MetalinkFile, MetalinkMirror};
pub use native_media::{NativeMediaError, NativeMediaProcessor};
pub use native_mp4::{NativeMp4Error, NativeMp4Info, NativeMp4Processor, NativeMp4Track};
pub use native_torrent::{NativeTorrentConfig, NativeTorrentEngine};
pub use sftp::{KnownHostsPolicy, SftpEngine, SftpEngineConfig, SftpKnownHost};
pub use ytdlp::{
    YtDlpConfig, YtDlpEngine, YtDlpError, YtDlpFormat, YtDlpProcessExit, YtDlpProcessRunner,
    YtDlpProgress, YtDlpRunnerEvent, YtDlpRunnerFuture, parse_format_output,
};

#[cfg(test)]
mod tests;

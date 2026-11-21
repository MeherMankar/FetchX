#![forbid(unsafe_code)]

mod adaptive;
mod bandwidth;
mod dispatcher;
mod drm_router;
mod drm_session;
mod engine;
mod error;
mod ffmpeg;
mod ftp;
mod hls;
mod http;
mod http_auth;
mod media_extractor_adapter;
mod metalink;
mod native_media;
mod native_mp4;
mod native_oxideav;
mod native_torrent;
mod native_video;
mod native_ytdlp;
mod retry;
mod segments;
mod sftp;
mod widevine_provider;
mod ytdlp;

pub use adaptive::AdaptiveConfig;
pub use bandwidth::{BandwidthControl, BandwidthLimiter};
pub use dispatcher::{EngineDispatcher, is_dash_url};
pub use drm_router::{DrmSystem, detect_drm_system};
pub use engine::{DownloadEngine, ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand};
pub use error::DownloadEngineError;
pub use ffmpeg::{
    FfmpegConfig, FfmpegError, FfmpegProcessor, FrameRate, MediaContainer, MediaInfo,
    MediaOperation, MediaProcessOutput, MediaProcessRequest, MediaStream, MediaStreamKind,
    MetadataOverride, TranscodePreset, write_metadata,
};
pub use ftp::{FtpEngine, FtpEngineConfig};
pub use hls::{HlsEngine, HlsEngineConfig, is_hls_url};

/// A request sent to the DRM plugin process via plugin-manager.
#[derive(Clone, Debug)]
pub struct DrmPluginRequest {
    pub pssh_b64: String,
    pub license_url: String,
    pub drm_system: String,
}

/// A response from the DRM plugin process.
#[derive(Clone, Debug)]
pub struct DrmPluginResponse {
    pub keys: Vec<DrmKeyPair>,
}

/// A single decryption key pair from the plugin.
#[derive(Clone, Debug)]
pub struct DrmKeyPair {
    pub kid: String,
    pub key: String,
}
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
pub use native_oxideav::{NativeOxideavError, NativeOxideavProcessor};
pub use native_torrent::{NativeTorrentConfig, NativeTorrentEngine};
pub use native_video::{NativeVideoError, NativeVideoProcessor, YuvFrame};
pub use native_ytdlp::NativeYtDlpProcessRunner;
pub use sftp::{KnownHostsPolicy, SftpEngine, SftpEngineConfig, SftpKnownHost};
pub use widevine_provider::build_widevine_provider;
pub use ytdlp::{
    YtDlpConfig, YtDlpEngine, YtDlpError, YtDlpFormat, YtDlpProcessExit, YtDlpProcessRunner,
    YtDlpProgress, YtDlpRunnerEvent, YtDlpRunnerFuture, parse_format_output,
};

#[cfg(test)]
mod tests;

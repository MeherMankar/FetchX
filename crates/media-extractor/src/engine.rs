//! Format-to-engine mapping.

use fetchx_models::EngineKind;

use crate::models::{MediaFormat, MediaProtocol};

/// Maps a [`MediaFormat`]'s protocol and URL to the [`EngineKind`] that should
/// download it.
///
/// The `format_id` is inspected to detect the `+bestaudio` merge pattern, which
/// requires yt-dlp's mux/merge step even if the protocol would otherwise allow
/// a direct HTTP download.
///
/// This function is pure and stateless.
pub fn format_to_engine_kind(format: &MediaFormat) -> EngineKind {
    // Any format whose id contains '+' requires ffmpeg muxing — must go through
    // YtDlp regardless of protocol.
    if format.id.contains('+') {
        return EngineKind::YtDlp;
    }
    // Empty URL means the direct stream URL is unknown at extraction time.
    if format.url.is_empty() {
        return EngineKind::YtDlp;
    }
    match &format.protocol {
        MediaProtocol::Http => EngineKind::DirectHttp,
        MediaProtocol::Hls => EngineKind::Hls,
        // DASH: no native FetchX engine yet; fall back to yt-dlp.
        MediaProtocol::Dash => EngineKind::YtDlp,
        // Explicit yt-dlp protocol token.
        MediaProtocol::YtDlp => EngineKind::YtDlp,
        // Unknown protocol: safe default is yt-dlp.
        MediaProtocol::Other(_) => EngineKind::YtDlp,
    }
}

impl MediaFormat {
    /// Returns the [`EngineKind`] that should be used to download this format.
    ///
    /// This is a convenience wrapper around [`format_to_engine_kind`].
    pub fn engine_kind(&self) -> EngineKind {
        format_to_engine_kind(self)
    }
}

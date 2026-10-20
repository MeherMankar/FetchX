//! [`MediaInfo`], [`MediaFormat`], and [`MediaProtocol`] model types.

/// Structured information about a media item returned by a [`crate::MediaExtractor`].
#[derive(Clone, Debug)]
pub struct MediaInfo {
    /// Human-readable title of the media item (e.g. video title).
    pub title: String,

    /// Duration in seconds. `None` if unavailable (e.g. live streams).
    pub duration: Option<f64>,

    /// URL of a representative thumbnail image, if available.
    pub thumbnail_url: Option<String>,

    /// The canonical URL of the page that was extracted.
    pub webpage_url: String,

    /// Name of the extractor that produced this result (e.g. "Youtube", "Vimeo").
    /// Populated from yt-dlp's `extractor_key` field.
    pub extractor_name: String,

    /// All available downloadable formats, in the order returned by the extractor.
    /// At least one format is always present (enforced by [`crate::YtDlpExtractor`]).
    pub formats: Vec<MediaFormat>,
}

/// A single downloadable format within a [`MediaInfo`].
#[derive(Clone, Debug)]
pub struct MediaFormat {
    /// Opaque format identifier used to re-select this format for download.
    /// For yt-dlp formats this is the `format_id` (possibly with `+bestaudio`
    /// suffix as produced by `parse_format_output`).
    pub id: String,

    /// File extension without the leading dot (e.g. `"mp4"`, `"webm"`).
    pub ext: String,

    /// Protocol used to fetch this format. Drives [`crate::format_to_engine_kind`].
    pub protocol: MediaProtocol,

    /// Direct URL to the media stream. For yt-dlp formats this is the signed
    /// CDN URL embedded in the JSON (`url` field). Empty string means the URL
    /// is not available at extraction time — the download must go through
    /// [`fetchx_models::EngineKind::YtDlp`].
    pub url: String,

    /// Human-readable label for display in a format picker (e.g. "1080p + audio - mp4 - 250.3 MB").
    /// Downstream code must not attempt to parse this.
    pub label: String,

    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,

    /// Bitrate in bits per second.
    pub bitrate: Option<u64>,

    /// File size in bytes; may be approximate.
    pub filesize: Option<u64>,

    /// Extra headers that must be sent with requests for this format's URL.
    ///
    /// # Security
    /// These headers are extracted from yt-dlp's `http_headers` JSON field.
    /// Before forwarding to the download engine the caller MUST apply the same
    /// name/value validation rules (character whitelist, blocklist for
    /// `Authorization`, `Cookie`, etc.) to prevent auth tokens leaking to
    /// unrelated domains. See the security design doc for details.
    pub required_headers: Vec<(String, String)>,
}

impl MediaInfo {
    /// Remove any [`MediaFormat`] whose URL fails [`crate::validation::is_safe_media_url`],
    /// and strip blocked headers from all remaining formats.
    ///
    /// Formats with an empty URL are kept — those route to
    /// [`fetchx_models::EngineKind::YtDlp`] and the URL is never used for
    /// direct HTTP fetches.
    ///
    /// Returns the number of formats removed.
    ///
    /// # When to call
    /// Call this before returning a [`MediaInfo`] from any [`crate::MediaExtractor`]
    /// implementation to guarantee that the safety invariant holds regardless of
    /// which extractor produced the result.
    pub fn sanitise_format_urls(&mut self) -> usize {
        use crate::security::filter_safe_headers;
        use crate::validation::is_safe_media_url;

        let before = self.formats.len();
        self.formats.retain(|f| {
            // Empty URLs are kept; they fall back to the YtDlp engine.
            if f.url.is_empty() {
                return true;
            }
            match is_safe_media_url(&f.url) {
                Ok(()) => true,
                Err(_) => {
                    tracing::warn!(
                        format_id = %f.id,
                        "sanitise_format_urls: dropping format with unsafe URL"
                    );
                    false
                }
            }
        });
        // Filter blocked headers on every surviving format.
        for fmt in &mut self.formats {
            let headers = std::mem::take(&mut fmt.required_headers);
            fmt.required_headers = filter_safe_headers(&headers);
        }
        before - self.formats.len()
    }
}

/// The transport protocol for a [`MediaFormat`].
///
/// `#[non_exhaustive]` ensures downstream match arms include a wildcard, which
/// is important because yt-dlp can return protocol strings not anticipated here.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MediaProtocol {
    /// Plain HTTPS/HTTP direct download.
    /// Maps to [`fetchx_models::EngineKind::DirectHttp`].
    Http,

    /// HLS (HTTP Live Streaming) manifest.
    /// Maps to [`fetchx_models::EngineKind::Hls`].
    Hls,

    /// DASH (MPEG-DASH) manifest.
    /// No dedicated FetchX engine yet; routes to [`fetchx_models::EngineKind::YtDlp`].
    Dash,

    /// Format requires yt-dlp to fetch (mux, merge, or proprietary protocol).
    /// Maps to [`fetchx_models::EngineKind::YtDlp`].
    YtDlp,

    /// Any other protocol string from yt-dlp (e.g. "mms", "rtmp").
    /// Routes to [`fetchx_models::EngineKind::YtDlp`] as the safe fallback.
    Other(String),
}

impl MediaProtocol {
    /// Parse a yt-dlp JSON `protocol` string into a [`MediaProtocol`].
    ///
    /// Mapping:
    /// - `"https"` or `"http"` → [`MediaProtocol::Http`]
    /// - `"m3u8"` or `"m3u8_native"` → [`MediaProtocol::Hls`]
    /// - `"http_dash_segments"` → [`MediaProtocol::Dash`]
    /// - `"mhtml"` → [`MediaProtocol::YtDlp`]
    /// - absent/null/anything else → [`MediaProtocol::YtDlp`] (safe default)
    pub fn from_yt_dlp_str(s: &str) -> Self {
        match s {
            "https" | "http" => Self::Http,
            "m3u8" | "m3u8_native" => Self::Hls,
            "http_dash_segments" => Self::Dash,
            "mhtml" => Self::YtDlp,
            other => Self::Other(other.to_owned()),
        }
    }
}

//! [`MediaExtractor`] trait, [`ExtractOptions`], and [`ExtractionError`].

use std::{fmt, time::Duration};

use async_trait::async_trait;

use crate::models::MediaInfo;

/// A source-agnostic media format extractor.
///
/// Implementations must be cheaply shareable behind an `Arc`. They must not
/// perform any download; they only resolve a page URL to a structured format
/// list.
#[async_trait]
pub trait MediaExtractor: Send + Sync {
    /// Returns `true` if this extractor can attempt to handle the given URL.
    ///
    /// This is a lightweight syntactic check only — it must not make network
    /// requests.
    fn can_handle(&self, url: &str) -> bool;

    /// Extracts the available media formats for the given URL.
    ///
    /// On success returns a fully populated [`MediaInfo`]. On failure returns
    /// an [`ExtractionError`] describing the failure category.
    async fn extract(&self, url: &str, opts: &ExtractOptions)
    -> Result<MediaInfo, ExtractionError>;
}

/// Options forwarded to the extractor backend.
///
/// # Security
/// `cookies` are never embedded in logs; the yt-dlp adapter writes them to a
/// temporary cookie file and the native YouTube extractor filters them for
/// its player requests. `headers` are validated before being forwarded.
#[derive(Clone, Default)]
pub struct ExtractOptions {
    /// Netscape-format cookie string. Maximum 65,536 bytes. The yt-dlp adapter
    /// writes it to a temporary file; the native YouTube path sends only
    /// eligible YouTube-scoped cookies. Never embedded in logs or arguments.
    pub cookies: Option<String>,

    /// User-provided YouTube PO tokens, scoped to a player client and token
    /// context. Tokens are never acquired automatically or included in logs.
    pub youtube_po_tokens: Vec<YoutubePoToken>,

    /// Extra HTTP headers forwarded to the extractor.
    /// Each name must match `[A-Za-z0-9\-]+` (max 128 bytes).
    /// Each value must be printable ASCII, max 4,096 bytes.
    /// Invalid headers are dropped silently (with a `tracing::warn!`).
    pub headers: Vec<(String, String)>,

    /// Request timeout. Defaults to 30 seconds when `None`.
    pub timeout: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum YoutubePlayerClient {
    TvSamsung,
    VisionOs,
    Web,
}

impl YoutubePlayerClient {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TvSamsung => "tv_samsung",
            Self::VisionOs => "visionos",
            Self::Web => "web",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum YoutubePoTokenContext {
    Player,
    Gvs,
}

impl YoutubePoTokenContext {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Gvs => "gvs",
        }
    }
}

#[derive(Clone)]
pub struct YoutubePoToken {
    pub client: YoutubePlayerClient,
    pub context: YoutubePoTokenContext,
    pub token: String,
}

impl fmt::Debug for YoutubePoToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("YoutubePoToken")
            .field("client", &self.client)
            .field("context", &self.context)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for ExtractOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtractOptions")
            .field("cookies", &self.cookies.as_ref().map(|_| "[REDACTED]"))
            .field("youtube_po_tokens", &self.youtube_po_tokens)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Errors produced by an extractor or the registry.
#[derive(Debug, thiserror::Error)]
pub enum ExtractionError {
    /// The URL failed [`crate::is_safe_media_url`] validation.
    #[error("URL is not a safe media URL: {reason}")]
    UnsafeUrl { reason: &'static str },

    /// No extractor in the registry can handle this URL.
    #[error("no extractor is available for this URL")]
    UnsupportedUrl,

    /// YouTube requires an access state the caller did not provide.
    #[error("YouTube requires additional access data: {reason}")]
    AuthenticationRequired { reason: &'static str },

    /// Extraction timed out.
    #[error("extraction timed out")]
    Timeout,

    /// The extractor returned data but no usable formats were present.
    #[error("no usable formats found")]
    NoFormats,

    /// The underlying tool or service returned an error.
    #[error("extractor backend failed: {message}")]
    BackendError { message: String },

    /// The output from the backend was malformed or could not be parsed.
    #[error("extractor output was invalid")]
    InvalidOutput,
}

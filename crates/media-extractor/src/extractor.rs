//! [`MediaExtractor`] trait, [`ExtractOptions`], and [`ExtractionError`].

use std::time::Duration;

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
/// `cookies` is written to a temp file and passed via `--cookies`; it is never
/// embedded in argument strings or log lines. `headers` are validated before
/// being forwarded.
#[derive(Clone, Debug, Default)]
pub struct ExtractOptions {
    /// Netscape-format cookie string. Maximum 65,536 bytes. Written to a temp
    /// file; never embedded in process arguments or logs.
    pub cookies: Option<String>,

    /// Extra HTTP headers forwarded to the extractor.
    /// Each name must match `[A-Za-z0-9\-]+` (max 128 bytes).
    /// Each value must be printable ASCII, max 4,096 bytes.
    /// Invalid headers are dropped silently (with a `tracing::warn!`).
    pub headers: Vec<(String, String)>,

    /// Request timeout. Defaults to 30 seconds when `None`.
    pub timeout: Option<Duration>,
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

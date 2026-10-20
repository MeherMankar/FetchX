//! [`YtDlpExtractor`] — wraps the existing yt-dlp process runner.
//!
//! # Desktop/CLI only
//! This extractor spawns yt-dlp as a subprocess and is **not Android-safe**.
//! On Android, SELinux prevents executing binaries from writable storage paths.
//! The Android path uses Chaquopy (CPython in-process) via `AndroidYtDlpRuntime`.
//!
//! # Dependency note
//! `fetchx-media-extractor` does **not** depend on `fetchx-downloader` at the
//! `Cargo.toml` level to keep the crate lightweight and independently testable.
//! The production integration point is:
//!
//! ```text
//! fetchx-downloader → (creates YtDlpExtractor, passing its Arc<dyn YtDlpProcessRunner>)
//!                                    ↓
//!                      fetchx-media-extractor (holds a type-erased runner via the trait below)
//! ```
//!
//! A caller that owns an `Arc<dyn YtDlpProcessRunner>` from `fetchx-downloader`
//! constructs a [`YtDlpExtractor`] via [`YtDlpExtractor::new`] and registers it
//! in an [`crate::ExtractorRegistry`]. The runner is hidden behind the
//! [`ProcessRunner`] trait defined in this module so this crate can compile
//! without depending on `fetchx-downloader`.
//!
//! To wire up the real runner implement [`ProcessRunner`] on a newtype wrapper
//! around `Arc<dyn fetchx_downloader::YtDlpProcessRunner>` in the integrating
//! crate.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    models::{MediaFormat, MediaInfo, MediaProtocol},
    security::{CookieTempFile, filter_safe_headers, validate_extracted_url},
};

/// Options forwarded to [`ProcessRunner::discover_formats`].
///
/// Decouples flag-passing (cookies, headers) from the trait signature used
/// in tests, allowing the mock implementations to simply ignore extra options.
#[derive(Default)]
pub struct RunnerOptions<'a> {
    /// Path to a Netscape-format cookie file. Passed as `--cookies <path>`.
    pub cookie_file: Option<&'a std::path::Path>,
    /// Validated headers to forward. Each becomes `--add-headers "Name:Value"`.
    pub extra_headers: &'a [(String, String)],
}

/// Minimal process-runner interface used by [`YtDlpExtractor`].
///
/// Implement this on a newtype in the integration crate that owns the real
/// `fetchx_downloader::YtDlpProcessRunner`, then pass it to
/// [`YtDlpExtractor::new`].
pub trait ProcessRunner: Send + Sync {
    /// Discover formats for `url` and return a list of [`RawFormat`] entries.
    ///
    /// Implementors call yt-dlp with `--dump-single-json` (or the equivalent
    /// Android in-process call) and parse the result into [`RawFormat`]s.
    ///
    /// Returns `Err(RunnerError::Unsupported)` if the runtime does not support
    /// format discovery (e.g. a stub in unit tests).
    fn discover_formats<'a>(
        &'a self,
        executable: &'a std::path::Path,
        url: &'a str,
        timeout: Duration,
        options: &'a RunnerOptions<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<RawFormat>, RunnerError>> + Send + 'a>,
    >;
}

/// Raw format data returned by a [`ProcessRunner`].
///
/// All fields are optional because yt-dlp JSON output is highly variable across
/// sites and format types.
#[derive(Clone, Debug, Default)]
pub struct RawFormat {
    pub format_id: String,
    pub ext: Option<String>,
    pub protocol: Option<String>,
    pub url: Option<String>,
    pub label: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub bitrate: Option<u64>,
    pub filesize: Option<u64>,
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail_url: Option<String>,
    pub webpage_url: Option<String>,
    pub extractor_name: Option<String>,
    pub required_headers: Vec<(String, String)>,
}

/// Errors from a [`ProcessRunner`].
#[derive(Debug)]
pub enum RunnerError {
    /// This runtime does not support format discovery.
    Unsupported,
    /// The runner timed out.
    Timeout,
    /// The runner's output could not be parsed.
    InvalidOutput,
    /// The runner failed with a message.
    Backend(String),
}

impl From<RunnerError> for ExtractionError {
    fn from(e: RunnerError) -> Self {
        match e {
            RunnerError::Unsupported => ExtractionError::UnsupportedUrl,
            RunnerError::Timeout => ExtractionError::Timeout,
            RunnerError::InvalidOutput => ExtractionError::InvalidOutput,
            RunnerError::Backend(msg) => ExtractionError::BackendError { message: msg },
        }
    }
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A [`crate::MediaExtractor`] that delegates to the existing yt-dlp process runner.
///
/// # Android safety
/// **Desktop/CLI only.** See module-level doc for the Android restriction.
pub struct YtDlpExtractor {
    runner: Arc<dyn ProcessRunner>,
    executable: PathBuf,
    timeout: Duration,
}

impl YtDlpExtractor {
    /// Create a new extractor.
    ///
    /// `runner` must be an implementation of [`ProcessRunner`] that wraps the
    /// real `fetchx_downloader::YtDlpProcessRunner` (or a test stub).
    pub fn new(runner: Arc<dyn ProcessRunner>, executable: PathBuf) -> Self {
        Self {
            runner,
            executable,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the extraction timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl MediaExtractor for YtDlpExtractor {
    /// Returns `true` for any `http://` or `https://` URL.
    ///
    /// `YtDlpExtractor` acts as the universal fallback; it defers actual URL
    /// supportability to yt-dlp itself.
    fn can_handle(&self, url: &str) -> bool {
        matches!(
            url.split_once("://").map(|(s, _)| s),
            Some("https" | "http")
        )
    }

    async fn extract(
        &self,
        url: &str,
        opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        // ── Cookies: write to RAII temp file if provided ──────────────────
        let cookie_guard: Option<CookieTempFile> = if let Some(ref cookie_str) = opts.cookies {
            let guard =
                CookieTempFile::write(cookie_str).map_err(|e| ExtractionError::BackendError {
                    message: format!("failed to create cookie temp file: {e}"),
                })?;
            Some(guard)
        } else {
            None
        };

        // ── Headers: validate and filter ─────────────────────────────────
        let safe_headers: Vec<(String, String)> = filter_safe_headers(&opts.headers);

        let runner_opts = RunnerOptions {
            cookie_file: cookie_guard.as_ref().map(|g| g.path()),
            extra_headers: &safe_headers,
        };

        let raw_formats = self
            .runner
            .discover_formats(&self.executable, url, self.timeout, &runner_opts)
            .await
            .map_err(ExtractionError::from)?;

        // Cookie temp file is dropped here (RAII cleanup).
        drop(cookie_guard);

        if raw_formats.is_empty() {
            return Err(ExtractionError::NoFormats);
        }

        // Pull MediaInfo top-level fields from the first format entry that has them.
        let first = &raw_formats[0];
        let title = first.title.clone().unwrap_or_default();
        let duration = first.duration;
        let thumbnail_url = first.thumbnail_url.clone();
        let webpage_url = first.webpage_url.clone().unwrap_or_else(|| url.to_owned());
        let extractor_name = first.extractor_name.clone().unwrap_or_default();

        let mut formats: Vec<MediaFormat> = raw_formats
            .into_iter()
            .filter_map(|r| {
                // Validate extracted URL (SSRF guard). Empty URLs are kept.
                let url_str = r.url.clone().unwrap_or_default();
                if let Err(e) = validate_extracted_url(&url_str) {
                    tracing::warn!(
                        format_id = %r.format_id,
                        reason = %e,
                        "dropping format with unsafe extracted URL"
                    );
                    return None;
                }

                let protocol = r
                    .protocol
                    .as_deref()
                    .map(MediaProtocol::from_yt_dlp_str)
                    .unwrap_or(MediaProtocol::YtDlp);

                // Build the label: use the provided label or construct a fallback.
                let label = r.label.unwrap_or_else(|| {
                    let mut parts = Vec::new();
                    if let Some(h) = r.height {
                        parts.push(format!("{h}p"));
                    }
                    if let Some(ref ext) = r.ext {
                        parts.push(ext.clone());
                    }
                    if let Some(fs) = r.filesize {
                        let mb = fs as f64 / 1_048_576.0;
                        parts.push(format!("{mb:.1} MB"));
                    }
                    if parts.is_empty() {
                        r.format_id.clone()
                    } else {
                        parts.join(" - ")
                    }
                });

                Some(MediaFormat {
                    id: r.format_id,
                    ext: r.ext.unwrap_or_default(),
                    protocol,
                    url: url_str,
                    label,
                    video_codec: r.video_codec,
                    audio_codec: r.audio_codec,
                    width: r.width,
                    height: r.height,
                    fps: r.fps,
                    bitrate: r.bitrate,
                    filesize: r.filesize,
                    required_headers: r.required_headers,
                })
            })
            .collect();

        if formats.is_empty() {
            return Err(ExtractionError::NoFormats);
        }

        // Filter blocked headers from all formats.
        for fmt in &mut formats {
            let headers = std::mem::take(&mut fmt.required_headers);
            fmt.required_headers = filter_safe_headers(&headers);
        }

        Ok(MediaInfo {
            title,
            duration,
            thumbnail_url,
            webpage_url,
            extractor_name,
            formats,
        })
    }
}

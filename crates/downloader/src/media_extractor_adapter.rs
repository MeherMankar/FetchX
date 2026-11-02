//! Adapter that bridges `fetchx-downloader`'s yt-dlp infrastructure to the
//! `fetchx-media-extractor` [`ProcessRunner`] trait.
//!
//! # Dependency direction
//! `fetchx-downloader` → `fetchx-media-extractor` (safe; no cycle).
//! `fetchx-media-extractor` never depends on `fetchx-downloader`.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use fetchx_media_extractor::ytdlp::{ProcessRunner, RawFormat, RunnerError, RunnerOptions};
use serde_json::Value;
use tokio::{io::AsyncReadExt, time::timeout};
use tracing::warn;

use crate::ytdlp::{YtDlpError, YtDlpProcessRunner};

/// Header names that must never be forwarded from yt-dlp JSON output.
///
/// Kept in sync with `BLOCKED_HEADER_NAMES` in `fetchx-media-extractor::security`.
const BLOCKED_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "www-authenticate",
    "x-forwarded-for",
    "x-real-ip",
];

/// Maximum bytes of yt-dlp `--dump-single-json` output accepted.
const MAX_FORMAT_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

/// Adapter that implements `fetchx_media_extractor::ProcessRunner` by
/// either delegating to a wrapped [`YtDlpProcessRunner`] (e.g. Android's
/// Chaquopy in-process runner) or by spawning yt-dlp directly when no
/// runner is available (desktop / CLI subprocess path).
///
/// # Subprocess path
/// When `runner` is `None`, the adapter spawns yt-dlp and parses the richer
/// `--dump-single-json` field set (`url`, `protocol`, codec fields, etc.) that
/// `TokioYtDlpRunner::discover_formats` intentionally discards.
///
/// # Delegation path
/// When `runner` is `Some`, the adapter calls `runner.discover_formats` and
/// converts the returned `Vec<YtDlpFormat>` display DTOs into `Vec<RawFormat>`
/// pipeline DTOs.  This enables the Android Chaquopy path without spawning a
/// subprocess (which SELinux blocks on Android).
pub struct YtDlpProcessRunnerAdapter {
    /// The executable path used both as the fallback subprocess command and
    /// as the `executable` argument forwarded to the wrapped runner.
    executable: PathBuf,
    /// Wrapped runner, present when an in-process runner is available (Android).
    runner: Option<Arc<dyn YtDlpProcessRunner>>,
}

impl YtDlpProcessRunnerAdapter {
    /// Create an adapter that delegates to `runner.discover_formats`.
    ///
    /// When `discover_formats` is called the adapter will call
    /// `runner.discover_formats(executable, url, timeout)` and convert the
    /// resulting `Vec<YtDlpFormat>` to `Vec<RawFormat>`.
    pub fn new(runner: Arc<dyn YtDlpProcessRunner>, executable: PathBuf) -> Self {
        Self {
            executable,
            runner: Some(runner),
        }
    }

    /// Create an adapter from an executable path only (no runner reference).
    ///
    /// `discover_formats` will spawn yt-dlp as a subprocess.  Use this when
    /// the runner is not accessible (e.g. the desktop / CLI path where
    /// `YtDlpEngine` was initialized with its default internal runner).
    pub fn from_executable(executable: PathBuf) -> Self {
        Self {
            executable,
            runner: None,
        }
    }

    /// Delegate discovery to the wrapped runner and convert the display DTOs
    /// (`Vec<YtDlpFormat>`) into pipeline DTOs (`Vec<RawFormat>`).
    async fn discover_via_runner(
        runner: &dyn YtDlpProcessRunner,
        executable: &Path,
        url: &str,
        command_timeout: Duration,
    ) -> Result<Vec<RawFormat>, RunnerError> {
        let yt_dlp_formats = runner
            .discover_formats(executable, url, command_timeout)
            .await
            .map_err(|e| match e {
                crate::ytdlp::YtDlpError::FormatDiscoveryTimeout => RunnerError::Timeout,
                crate::ytdlp::YtDlpError::FormatDiscoveryUnsupported => RunnerError::Unsupported,
                crate::ytdlp::YtDlpError::InvalidFormatOutput
                | crate::ytdlp::YtDlpError::FormatOutputTooLarge => RunnerError::InvalidOutput,
                other => RunnerError::Backend(other.to_string()),
            })?;

        let raw_formats = yt_dlp_formats
            .into_iter()
            .map(
                |crate::ytdlp::YtDlpFormat {
                     format_id,
                     label,
                     extension,
                     file_size_bytes,
                     title,
                     platform,
                 }| RawFormat {
                    format_id,
                    ext: extension,
                    label: Some(label),
                    filesize: file_size_bytes,
                    title,
                    extractor_name: platform,
                    // Fields not available from the display DTO.
                    url: None,
                    protocol: None,
                    video_codec: None,
                    audio_codec: None,
                    width: None,
                    height: None,
                    fps: None,
                    bitrate: None,
                    duration: None,
                    thumbnail_url: None,
                    webpage_url: None,
                    required_headers: Vec::new(),
                },
            )
            .collect();

        Ok(raw_formats)
    }
}

impl ProcessRunner for YtDlpProcessRunnerAdapter {
    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        url: &'a str,
        command_timeout: Duration,
        options: &'a RunnerOptions<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<RawFormat>, RunnerError>> + Send + 'a>>
    {
        // If a wrapped runner is present, delegate to it (e.g. Android Chaquopy).
        if let Some(ref runner) = self.runner {
            let runner = Arc::clone(runner);
            let executable = self.executable.clone();
            let url = url.to_owned();
            return Box::pin(async move {
                Self::discover_via_runner(runner.as_ref(), &executable, &url, command_timeout).await
            });
        }

        // Subprocess path: spawn yt-dlp directly.
        let executable = self.executable.clone();
        let url = url.to_owned();
        let cookie_file = options.cookie_file.map(|p| p.to_path_buf());
        let extra_headers: Vec<(String, String)> = options.extra_headers.to_vec();

        Box::pin(async move {
            let mut args: Vec<OsString> = vec![
                "--ignore-config".into(),
                "--no-plugin-dirs".into(),
                "--no-warnings".into(),
                "--quiet".into(),
                "--skip-download".into(),
                "--no-playlist".into(),
                "--dump-single-json".into(),
            ];

            if let Some(ref path) = cookie_file {
                args.push("--cookies".into());
                args.push(path.as_os_str().into());
            }

            for (name, value) in &extra_headers {
                args.push("--add-headers".into());
                // yt-dlp expects "Name:Value" as a single argument.
                let mut header = OsString::from(name);
                header.push(":");
                header.push(value);
                args.push(header);
            }

            args.push("--".into());
            args.push(url.as_str().into());

            let mut command = tokio::process::Command::new(&executable);
            command
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            // Suppress the console window that would otherwise flash on Windows.
            #[cfg(windows)]
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW

            let mut child = command
                .spawn()
                .map_err(|_| RunnerError::Backend("failed to spawn yt-dlp".into()))?;

            let result = timeout(command_timeout, async {
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| RunnerError::Backend("no stdout".into()))?;
                let mut bytes = Vec::new();
                stdout
                    .take((MAX_FORMAT_OUTPUT_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|e: io::Error| RunnerError::Backend(e.to_string()))?;
                if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
                    return Err(RunnerError::InvalidOutput);
                }
                let status = child
                    .wait()
                    .await
                    .map_err(|e| RunnerError::Backend(e.to_string()))?;
                if !status.success() {
                    return Err(RunnerError::Backend(format!(
                        "yt-dlp exited with status {:?}",
                        status.code()
                    )));
                }
                parse_yt_dlp_json_to_raw_formats(&bytes)
                    .map_err(|e: YtDlpError| map_ytdlp_error_to_runner_error(&e))
            })
            .await;

            // child has kill_on_drop(true); it is dropped here if timeout fires.
            match result {
                Ok(result) => result,
                Err(_elapsed) => Err(RunnerError::Timeout),
            }
        })
    }
}

/// Parse yt-dlp `--dump-single-json` bytes into [`RawFormat`]s.
///
/// Reads the richer field set (`url`, `protocol`, `vcodec`, `acodec`, etc.)
/// that `parse_format_output` intentionally discards.
///
/// # Security
/// - Filters out DRM-protected formats (`has_drm: true`).
/// - Caps at `MAX_DISCOVERED_FORMATS` (100).
/// - Drops HTTP headers whose names are in the blocklist (`authorization`, `cookie`, etc.).
pub fn parse_yt_dlp_json_to_raw_formats(bytes: &[u8]) -> Result<Vec<RawFormat>, YtDlpError> {
    if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
        return Err(YtDlpError::FormatOutputTooLarge);
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| YtDlpError::InvalidFormatOutput)?;

    let formats_arr = value
        .get("formats")
        .and_then(Value::as_array)
        .ok_or(YtDlpError::InvalidFormatOutput)?;

    // Top-level fields inherited into every format.
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let duration = value.get("duration").and_then(Value::as_f64);
    let thumbnail_url = value
        .get("thumbnail")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let webpage_url = value
        .get("webpage_url")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let extractor_name = value
        .get("extractor_key")
        .or_else(|| value.get("extractor"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut result = Vec::new();
    for fmt in formats_arr {
        // Skip DRM-protected formats.
        if fmt.get("has_drm").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        let format_id = match fmt.get("format_id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => continue,
        };

        if format_id.is_empty() || format_id.len() > 128 {
            continue;
        }

        let url = fmt.get("url").and_then(Value::as_str).map(str::to_owned);
        let protocol = fmt
            .get("protocol")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let ext = fmt.get("ext").and_then(Value::as_str).map(str::to_owned);
        let label = fmt
            .get("format")
            .or_else(|| fmt.get("format_note"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let video_codec = codec_field(fmt.get("vcodec"));
        let audio_codec = codec_field(fmt.get("acodec"));

        let width = fmt.get("width").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .and_then(|n| u32::try_from(n).ok())
        });
        let height = fmt.get("height").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .and_then(|n| u32::try_from(n).ok())
        });
        let fps = fmt.get("fps").and_then(Value::as_f64);

        // Bitrate: prefer tbr (total bit rate in kbps), fall back to vbr/abr.
        let bitrate = fmt
            .get("tbr")
            .or_else(|| fmt.get("vbr"))
            .or_else(|| fmt.get("abr"))
            .and_then(Value::as_f64)
            .map(|kbps| (kbps * 1000.0) as u64);

        let filesize = fmt
            .get("filesize")
            .and_then(Value::as_u64)
            .or_else(|| fmt.get("filesize_approx").and_then(Value::as_u64));

        // Parse and filter HTTP headers.
        let required_headers = parse_headers(fmt.get("http_headers"));

        result.push(RawFormat {
            format_id,
            ext,
            protocol,
            url,
            label,
            video_codec,
            audio_codec,
            width,
            height,
            fps,
            bitrate,
            filesize,
            title: title.clone(),
            duration,
            thumbnail_url: thumbnail_url.clone(),
            webpage_url: webpage_url.clone(),
            extractor_name: extractor_name.clone(),
            required_headers,
        });

        if result.len() >= 100 {
            break;
        }
    }

    Ok(result)
}

/// Extract a codec string, treating `"none"` and `""` as absent.
fn codec_field(value: Option<&Value>) -> Option<String> {
    match value.and_then(Value::as_str) {
        Some("none") | Some("") | None => None,
        Some(s) => Some(s.to_owned()),
    }
}

/// Parse yt-dlp's `http_headers` JSON object into `Vec<(String, String)>`,
/// dropping blocked headers and logging a warning (name only, never value).
fn parse_headers(value: Option<&Value>) -> Vec<(String, String)> {
    let obj = match value.and_then(Value::as_object) {
        Some(obj) => obj,
        None => return Vec::new(),
    };
    let mut result = Vec::new();
    for (name, val) in obj {
        let Some(val_str) = val.as_str() else {
            continue;
        };
        let name_lower = name.to_ascii_lowercase();
        if BLOCKED_HEADERS.contains(&name_lower.as_str()) {
            warn!(header_name = %name, "dropping blocked header from yt-dlp JSON output");
            continue;
        }
        result.push((name.clone(), val_str.to_owned()));
    }
    result
}

fn map_ytdlp_error_to_runner_error(e: &YtDlpError) -> RunnerError {
    match e {
        YtDlpError::FormatDiscoveryTimeout => RunnerError::Timeout,
        YtDlpError::FormatDiscoveryUnsupported => RunnerError::Unsupported,
        YtDlpError::InvalidFormatOutput | YtDlpError::FormatOutputTooLarge => {
            RunnerError::InvalidOutput
        }
        other => RunnerError::Backend(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_single_format(protocol: &str, url: &str) -> Vec<u8> {
        serde_json::json!({
            "title": "Test Video",
            "duration": 120.0,
            "thumbnail": "https://example.com/thumb.jpg",
            "webpage_url": "https://example.com/watch/1",
            "extractor_key": "Example",
            "formats": [{
                "format_id": "137",
                "ext": "mp4",
                "protocol": protocol,
                "url": url,
                "vcodec": "avc1.640028",
                "acodec": "mp4a.40.2",
                "width": 1920,
                "height": 1080,
                "fps": 30.0,
                "tbr": 5000.0,
                "filesize": 75_000_000_u64,
                "http_headers": {
                    "User-Agent": "Mozilla/5.0",
                    "Accept": "*/*"
                }
            }]
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn test_parse_https_format() {
        let bytes = fixture_single_format("https", "https://cdn.example.com/video.mp4");
        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        assert_eq!(formats.len(), 1);
        let f = &formats[0];
        assert_eq!(f.format_id, "137");
        assert_eq!(f.protocol.as_deref(), Some("https"));
        assert_eq!(f.url.as_deref(), Some("https://cdn.example.com/video.mp4"));
        assert_eq!(f.video_codec.as_deref(), Some("avc1.640028"));
        assert_eq!(f.audio_codec.as_deref(), Some("mp4a.40.2"));
        assert_eq!(f.width, Some(1920));
        assert_eq!(f.height, Some(1080));
        assert_eq!(f.fps, Some(30.0));
        assert_eq!(f.bitrate, Some(5_000_000));
        assert_eq!(f.filesize, Some(75_000_000));
        assert_eq!(f.title.as_deref(), Some("Test Video"));
        assert_eq!(f.duration, Some(120.0));
        assert_eq!(f.extractor_name.as_deref(), Some("Example"));
        // User-Agent and Accept headers should be present; none are blocked
        assert!(f.required_headers.iter().any(|(k, _)| k == "User-Agent"));
    }

    #[test]
    fn test_parse_hls_format() {
        let bytes = fixture_single_format("m3u8_native", "https://cdn.example.com/hls/1080p.m3u8");
        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        assert_eq!(formats[0].protocol.as_deref(), Some("m3u8_native"));
    }

    #[test]
    fn test_drm_formats_filtered_out() {
        let bytes = serde_json::json!({
            "title": "DRM Video",
            "formats": [{
                "format_id": "drm1",
                "ext": "mp4",
                "protocol": "https",
                "url": "https://cdn.example.com/drm.mp4",
                "has_drm": true,
                "vcodec": "avc1"
            }]
        })
        .to_string()
        .into_bytes();

        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        assert!(formats.is_empty());
    }

    #[test]
    fn test_blocked_headers_are_dropped() {
        let bytes = serde_json::json!({
            "title": "Test",
            "formats": [{
                "format_id": "1",
                "ext": "mp4",
                "protocol": "https",
                "url": "https://cdn.example.com/v.mp4",
                "vcodec": "avc1",
                "http_headers": {
                    "Authorization": "Bearer secret",
                    "Cookie": "session=abc",
                    "User-Agent": "test"
                }
            }]
        })
        .to_string()
        .into_bytes();

        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        let headers = &formats[0].required_headers;
        assert!(
            headers
                .iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")),
            "Authorization must be blocked"
        );
        assert!(
            headers
                .iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("cookie")),
            "Cookie must be blocked"
        );
        assert!(
            headers.iter().any(|(k, _)| k == "User-Agent"),
            "User-Agent must pass through"
        );
    }

    #[test]
    fn test_bitrate_converted_from_kbps() {
        let bytes = serde_json::json!({
            "title": "Test",
            "formats": [{
                "format_id": "1",
                "ext": "mp4",
                "protocol": "https",
                "url": "https://cdn.example.com/v.mp4",
                "vcodec": "avc1",
                "tbr": 2000.0
            }]
        })
        .to_string()
        .into_bytes();

        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        // 2000 kbps → 2_000_000 bps
        assert_eq!(formats[0].bitrate, Some(2_000_000));
    }

    #[test]
    fn test_none_codecs_treated_as_absent() {
        let bytes = serde_json::json!({
            "title": "Test",
            "formats": [{
                "format_id": "1",
                "ext": "mp4",
                "protocol": "https",
                "url": "https://cdn.example.com/v.mp4",
                "vcodec": "avc1",
                "acodec": "none"
            }]
        })
        .to_string()
        .into_bytes();

        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        assert_eq!(formats[0].video_codec.as_deref(), Some("avc1"));
        assert!(formats[0].audio_codec.is_none());
    }

    #[test]
    fn test_invalid_json_returns_error() {
        let result = parse_yt_dlp_json_to_raw_formats(b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_formats_array_returns_error() {
        let bytes = serde_json::json!({ "title": "no formats" })
            .to_string()
            .into_bytes();
        let result = parse_yt_dlp_json_to_raw_formats(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_output_too_large_returns_error() {
        let oversized = vec![b'x'; MAX_FORMAT_OUTPUT_BYTES + 1];
        let result = parse_yt_dlp_json_to_raw_formats(&oversized);
        assert!(result.is_err());
    }

    #[test]
    fn test_top_level_fields_inherited() {
        let bytes = fixture_single_format("https", "https://cdn.example.com/video.mp4");
        let formats = parse_yt_dlp_json_to_raw_formats(&bytes).unwrap();
        let f = &formats[0];
        assert_eq!(f.title.as_deref(), Some("Test Video"));
        assert_eq!(f.duration, Some(120.0));
        assert_eq!(
            f.thumbnail_url.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
        assert_eq!(
            f.webpage_url.as_deref(),
            Some("https://example.com/watch/1")
        );
        assert_eq!(f.extractor_name.as_deref(), Some("Example"));
    }
}

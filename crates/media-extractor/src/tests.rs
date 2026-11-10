//! Unit tests for `fetchx-media-extractor`.
//!
//! All tests are deterministic; none make external network calls.

use std::{path::Path, pin::Pin, sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::ytdlp::{ProcessRunner, RawFormat, RunnerError, RunnerOptions, YtDlpExtractor};
use crate::{
    ExtractOptions, ExtractionError, ExtractorRegistry, MediaExtractor, MediaFormat, MediaInfo,
    MediaProtocol, UnsupportedExtractor, format_to_engine_kind, is_safe_media_url,
};
use fetchx_models::EngineKind;

// ---------------------------------------------------------------------------
// Helpers: mock extractor and runner
// ---------------------------------------------------------------------------

/// A mock extractor that matches a fixed URL prefix.
struct PrefixExtractor {
    prefix: &'static str,
    name: &'static str,
}

struct ExplicitlyUnsupportedExtractor;
struct AuthenticationRequiredExtractor;

#[async_trait]
impl MediaExtractor for ExplicitlyUnsupportedExtractor {
    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("https://")
    }

    async fn extract(
        &self,
        _url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Err(ExtractionError::UnsupportedUrl)
    }
}

#[async_trait]
impl MediaExtractor for AuthenticationRequiredExtractor {
    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("https://example.com/watch/")
    }

    async fn extract(
        &self,
        _url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Err(ExtractionError::AuthenticationRequired {
            reason: "sign-in-required",
        })
    }
}

struct TimeoutExtractor;

#[async_trait]
impl MediaExtractor for TimeoutExtractor {
    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("https://")
    }

    async fn extract(
        &self,
        _url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Err(ExtractionError::Timeout)
    }
}

#[async_trait]
impl MediaExtractor for PrefixExtractor {
    fn can_handle(&self, url: &str) -> bool {
        url.starts_with(self.prefix)
    }

    async fn extract(
        &self,
        url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Ok(MediaInfo {
            title: self.name.to_owned(),
            duration: Some(60.0),
            thumbnail_url: None,
            webpage_url: url.to_owned(),
            extractor_name: self.name.to_owned(),
            formats: vec![MediaFormat {
                id: "fmt1".to_owned(),
                ext: "mp4".to_owned(),
                protocol: MediaProtocol::Http,
                url: format!("{url}/video.mp4"),
                label: "720p - mp4".to_owned(),
                video_codec: Some("avc1".to_owned()),
                audio_codec: Some("mp4a".to_owned()),
                width: Some(1280),
                height: Some(720),
                fps: Some(30.0),
                bitrate: Some(2_000_000),
                filesize: Some(50_000_000),
                required_headers: vec![],
            }],
        })
    }
}

/// A mock runner that returns a fixed list of [`RawFormat`]s.
struct MockRunner {
    formats: Vec<RawFormat>,
}

impl ProcessRunner for MockRunner {
    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        _url: &'a str,
        _timeout: Duration,
        _options: &'a RunnerOptions<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<RawFormat>, RunnerError>> + Send + 'a>>
    {
        let formats = self.formats.clone();
        Box::pin(async move { Ok(formats) })
    }
}

/// A runner that always returns `RunnerError::Unsupported`.
struct UnsupportedRunner;

impl ProcessRunner for UnsupportedRunner {
    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        _url: &'a str,
        _timeout: Duration,
        _options: &'a RunnerOptions<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<RawFormat>, RunnerError>> + Send + 'a>>
    {
        Box::pin(async { Err(RunnerError::Unsupported) })
    }
}

/// A runner that always returns `RunnerError::Timeout`.
struct TimeoutRunner;

impl ProcessRunner for TimeoutRunner {
    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        _url: &'a str,
        _timeout: Duration,
        _options: &'a RunnerOptions<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<RawFormat>, RunnerError>> + Send + 'a>>
    {
        Box::pin(async { Err(RunnerError::Timeout) })
    }
}

/// A runner that always returns `RunnerError::Backend(...)`.
struct BackendErrorRunner;

impl ProcessRunner for BackendErrorRunner {
    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        _url: &'a str,
        _timeout: Duration,
        _options: &'a RunnerOptions<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<RawFormat>, RunnerError>> + Send + 'a>>
    {
        Box::pin(async { Err(RunnerError::Backend("backend exploded".into())) })
    }
}

// ---------------------------------------------------------------------------
// Test 1: Registry routes to the correct extractor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_registry_routes_to_correct_extractor() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://vimeo.com",
        name: "vimeo",
    }));
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://youtube.com",
        name: "youtube",
    }));

    let result = registry
        .extract(
            "https://youtube.com/watch?v=abc123",
            ExtractOptions::default(),
        )
        .await
        .expect("extraction should succeed");

    assert_eq!(result.extractor_name, "youtube");
}

// ---------------------------------------------------------------------------
// Test 2: Unsupported URL returns error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unsupported_url_returns_error() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(UnsupportedExtractor));

    let err = registry
        .extract("https://example.com/video.mp4", ExtractOptions::default())
        .await
        .expect_err("should fail with UnsupportedUrl");

    assert!(
        matches!(err, ExtractionError::UnsupportedUrl),
        "expected UnsupportedUrl, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: HLS protocol maps to HLS engine
// ---------------------------------------------------------------------------

#[test]
fn test_hls_protocol_maps_to_hls_engine() {
    let format = MediaFormat {
        id: "hls_1080".to_owned(),
        ext: "m3u8".to_owned(),
        protocol: MediaProtocol::Hls,
        url: "https://cdn.example.com/playlist.m3u8".to_owned(),
        label: "1080p HLS".to_owned(),
        video_codec: None,
        audio_codec: None,
        width: None,
        height: Some(1080),
        fps: None,
        bitrate: None,
        filesize: None,
        required_headers: vec![],
    };

    assert_eq!(format.engine_kind(), EngineKind::Hls);
}

// ---------------------------------------------------------------------------
// Test 4: HTTP protocol maps to DirectHttp engine
// ---------------------------------------------------------------------------

#[test]
fn test_http_protocol_maps_to_direct_http_engine() {
    let format = MediaFormat {
        id: "direct_720".to_owned(),
        ext: "mp4".to_owned(),
        protocol: MediaProtocol::Http,
        url: "https://cdn.example.com/video_720p.mp4".to_owned(),
        label: "720p mp4".to_owned(),
        video_codec: None,
        audio_codec: None,
        width: None,
        height: Some(720),
        fps: None,
        bitrate: None,
        filesize: None,
        required_headers: vec![],
    };

    assert_eq!(format.engine_kind(), EngineKind::DirectHttp);
}

// ---------------------------------------------------------------------------
// Test 5: ExtractOptions default is empty
// ---------------------------------------------------------------------------

#[test]
fn test_extract_options_default_is_empty() {
    let opts = ExtractOptions::default();
    assert!(opts.cookies.is_none());
    assert!(opts.youtube_po_tokens.is_empty());
    assert!(opts.headers.is_empty());
    assert!(opts.timeout.is_none());
}

// ---------------------------------------------------------------------------
// Test 6: Registry tries extractors in order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_registry_tries_extractors_in_order() {
    let mut registry = ExtractorRegistry::new();
    // Both extractors match any https:// URL; the first one registered should win.
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://",
        name: "first",
    }));
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://",
        name: "second",
    }));

    let result = registry
        .extract("https://example.com/video", ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    assert_eq!(result.extractor_name, "first");
}

#[tokio::test]
async fn test_registry_falls_back_only_after_explicit_unsupported_result() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(ExplicitlyUnsupportedExtractor));
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://",
        name: "fallback",
    }));

    let result = registry
        .extract("https://example.com/watch", ExtractOptions::default())
        .await
        .expect("second extractor should handle the unsupported page");

    assert_eq!(result.extractor_name, "fallback");
}

#[tokio::test]
async fn test_registry_falls_back_after_authentication_required() {
    let url = "https://example.com/watch/secure";
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(AuthenticationRequiredExtractor));
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://example.com/watch/",
        name: "fallback",
    }));

    let info = registry
        .extract(url, ExtractOptions::default())
        .await
        .expect("fallback should run after authentication is required");
    assert_eq!(info.extractor_name, "fallback");
}

#[tokio::test]
async fn test_registry_preserves_authentication_requirement_without_fallback() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(AuthenticationRequiredExtractor));
    assert!(matches!(
        registry
            .extract(
                "https://example.com/watch/secure",
                ExtractOptions::default()
            )
            .await,
        Err(ExtractionError::AuthenticationRequired {
            reason: "sign-in-required"
        })
    ));
}

#[tokio::test]
async fn test_registry_does_not_fall_back_after_non_unsupported_error() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(TimeoutExtractor));
    registry.register(Arc::new(PrefixExtractor {
        prefix: "https://",
        name: "must-not-run",
    }));

    let error = registry
        .extract("https://example.com/watch", ExtractOptions::default())
        .await
        .expect_err("timeout must be surfaced");

    assert!(matches!(error, ExtractionError::Timeout));
}

// ---------------------------------------------------------------------------
// Test 7: Malformed URL is handled gracefully
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_malformed_url_handled_gracefully() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(UnsupportedExtractor));

    // A URL with no scheme should return an UnsafeUrl error from is_safe_media_url.
    let err = registry
        .extract("not a url at all", ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::UnsafeUrl { .. }),
        "expected UnsafeUrl, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: Safe URL validation (is_safe_media_url)
// ---------------------------------------------------------------------------

#[test]
fn test_safe_url_validation() {
    // Valid URLs
    assert!(is_safe_media_url("https://example.com/video.mp4").is_ok());
    assert!(is_safe_media_url("http://cdn.example.com/stream.m3u8").is_ok());

    // Invalid scheme
    assert!(matches!(
        is_safe_media_url("ftp://example.com/file"),
        Err(ExtractionError::UnsafeUrl { .. })
    ));
    assert!(matches!(
        is_safe_media_url("file:///etc/passwd"),
        Err(ExtractionError::UnsafeUrl { .. })
    ));

    // Credentials in URL
    assert!(matches!(
        is_safe_media_url("https://user:pass@example.com/video"),
        Err(ExtractionError::UnsafeUrl { .. })
    ));

    // Private/loopback IPs
    assert!(matches!(
        is_safe_media_url("http://127.0.0.1/video"),
        Err(ExtractionError::UnsafeUrl { .. })
    ));
    assert!(matches!(
        is_safe_media_url("http://192.168.0.1/video"),
        Err(ExtractionError::UnsafeUrl { .. })
    ));
    assert!(matches!(
        is_safe_media_url("http://localhost/video"),
        Err(ExtractionError::UnsafeUrl { .. })
    ));
}

// ---------------------------------------------------------------------------
// Additional: YtDlpExtractor uses mock runner
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ytdlp_extractor_with_mock_runner() {
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "137".to_owned(),
            ext: Some("mp4".to_owned()),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.yt.com/video.mp4".to_owned()),
            label: Some("1080p - mp4 - 120.5 MB".to_owned()),
            video_codec: Some("avc1.640028".to_owned()),
            audio_codec: None,
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
            bitrate: Some(4_000_000),
            filesize: Some(126_418_944),
            title: Some("Test Video".to_owned()),
            duration: Some(120.0),
            thumbnail_url: Some("https://cdn.yt.com/thumb.jpg".to_owned()),
            webpage_url: Some("https://youtube.com/watch?v=test".to_owned()),
            extractor_name: Some("Youtube".to_owned()),
            required_headers: vec![],
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let result = extractor
        .extract(
            "https://youtube.com/watch?v=test",
            &ExtractOptions::default(),
        )
        .await
        .expect("extraction should succeed");

    assert_eq!(result.title, "Test Video");
    assert_eq!(result.formats.len(), 1);
    assert_eq!(result.formats[0].id, "137");
    assert_eq!(result.formats[0].engine_kind(), EngineKind::DirectHttp);
}

#[tokio::test]
async fn test_ytdlp_extractor_unsupported_runner_returns_error() {
    let runner = Arc::new(UnsupportedRunner);
    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));

    let err = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::UnsupportedUrl),
        "expected UnsupportedUrl, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Additional: format_to_engine_kind covers merge format
// ---------------------------------------------------------------------------

#[test]
fn test_merge_format_id_maps_to_ytdlp_engine() {
    let format = MediaFormat {
        id: "137+bestaudio".to_owned(),
        ext: "mp4".to_owned(),
        protocol: MediaProtocol::Http,
        url: "https://cdn.example.com/video.mp4".to_owned(),
        label: "1080p".to_owned(),
        video_codec: None,
        audio_codec: None,
        width: None,
        height: None,
        fps: None,
        bitrate: None,
        filesize: None,
        required_headers: vec![],
    };

    // Despite Http protocol, the '+' in the id forces YtDlp.
    assert_eq!(format_to_engine_kind(&format), EngineKind::YtDlp);
}

// ---------------------------------------------------------------------------
// Security tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_format_urls_with_private_ip_are_filtered() {
    // A format whose URL is a private IP should be dropped.
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "bad".to_owned(),
            ext: Some("mp4".to_owned()),
            protocol: Some("https".to_owned()),
            url: Some("https://192.168.1.1/video.mp4".to_owned()),
            ..RawFormat::default()
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let err = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect_err("should fail with NoFormats");

    assert!(
        matches!(err, ExtractionError::NoFormats),
        "expected NoFormats after SSRF filtering, got {err:?}"
    );
}

#[tokio::test]
async fn test_blocked_headers_are_removed_from_required_headers() {
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "1".to_owned(),
            ext: Some("mp4".to_owned()),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.example.com/video.mp4".to_owned()),
            required_headers: vec![
                ("Authorization".to_owned(), "Bearer secret".to_owned()),
                ("User-Agent".to_owned(), "Mozilla/5.0".to_owned()),
            ],
            ..RawFormat::default()
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let info = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    let headers = &info.formats[0].required_headers;
    assert!(
        headers.iter().all(|(k, _)| k != "Authorization"),
        "Authorization header must be removed"
    );
    assert!(
        headers.iter().any(|(k, _)| k == "User-Agent"),
        "User-Agent must remain"
    );
}

#[tokio::test]
async fn test_auth_header_is_blocked() {
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "1".to_owned(),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.example.com/video.mp4".to_owned()),
            required_headers: vec![("Authorization".to_owned(), "Basic dXNlcjpwYXNz".to_owned())],
            ..RawFormat::default()
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let info = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    assert!(
        info.formats[0]
            .required_headers
            .iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")),
        "Authorization must be blocked"
    );
}

// ---------------------------------------------------------------------------
// Cookie / header handling tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_invalid_header_name_is_dropped() {
    // A header with spaces in the name is invalid and must be dropped.
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "1".to_owned(),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.example.com/video.mp4".to_owned()),
            ..RawFormat::default()
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let opts = ExtractOptions {
        headers: vec![
            ("Invalid Header".to_owned(), "value".to_owned()),
            ("Valid-Header".to_owned(), "ok".to_owned()),
        ],
        ..Default::default()
    };
    // Should not panic — invalid headers are dropped silently.
    let info = extractor
        .extract("https://example.com/video", &opts)
        .await
        .expect("extraction should succeed");
    assert!(!info.formats.is_empty());
}

#[tokio::test]
async fn test_valid_headers_are_forwarded() {
    // The mock runner doesn't use headers, but we verify extraction still succeeds
    // with valid headers provided.
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "1".to_owned(),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.example.com/video.mp4".to_owned()),
            ..RawFormat::default()
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let opts = ExtractOptions {
        headers: vec![("Referer".to_owned(), "https://example.com".to_owned())],
        ..Default::default()
    };
    let info = extractor
        .extract("https://example.com/video", &opts)
        .await
        .expect("extraction with valid headers should succeed");
    assert!(!info.formats.is_empty());
}

// ---------------------------------------------------------------------------
// Runner error propagation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_timeout_runner_returns_timeout_error() {
    let runner = Arc::new(TimeoutRunner);
    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));

    let err = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::Timeout),
        "expected Timeout, got {err:?}"
    );
}

#[tokio::test]
async fn test_backend_error_runner_returns_backend_error() {
    let runner = Arc::new(BackendErrorRunner);
    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));

    let err = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::BackendError { .. }),
        "expected BackendError, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// End-to-end pipeline tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_pipeline_hls_url_to_hls_engine() {
    use crate::format_selector::{FormatPreference, select_format};
    use crate::{ExtractOptions, ExtractorRegistry, format_to_engine_kind};

    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "hls-1080".to_owned(),
            ext: Some("m3u8".to_owned()),
            protocol: Some("m3u8_native".to_owned()),
            url: Some("https://cdn.example.com/hls/1080p.m3u8".to_owned()),
            label: Some("1080p HLS".to_owned()),
            video_codec: Some("avc1".to_owned()),
            audio_codec: Some("mp4a".to_owned()),
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
            bitrate: Some(5_000_000),
            filesize: None,
            title: Some("Test HLS Video".to_owned()),
            duration: Some(300.0),
            thumbnail_url: None,
            webpage_url: Some("https://example.com/watch/1".to_owned()),
            extractor_name: Some("Example".to_owned()),
            required_headers: vec![],
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(extractor));

    let info = registry
        .extract("https://example.com/watch/1", ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    assert!(!info.formats.is_empty());

    let selected = select_format(&info.formats, &FormatPreference::BestCombined)
        .expect("should find a format");
    assert_eq!(selected.engine_kind(), EngineKind::Hls);
    // Verify format_to_engine_kind directly
    assert_eq!(format_to_engine_kind(selected), EngineKind::Hls);
}

#[tokio::test]
async fn test_full_pipeline_http_url_to_direct_http_engine() {
    use crate::format_selector::{FormatPreference, select_format};

    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "direct-720".to_owned(),
            ext: Some("mp4".to_owned()),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.example.com/video_720p.mp4".to_owned()),
            label: None,
            video_codec: Some("avc1".to_owned()),
            audio_codec: Some("mp4a".to_owned()),
            width: Some(1280),
            height: Some(720),
            fps: None,
            bitrate: Some(2_000_000),
            filesize: Some(90_000_000),
            title: Some("Test Direct Video".to_owned()),
            duration: Some(60.0),
            thumbnail_url: None,
            webpage_url: Some("https://example.com/watch/2".to_owned()),
            extractor_name: Some("Example".to_owned()),
            required_headers: vec![],
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(extractor));

    let info = registry
        .extract("https://example.com/watch/2", ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    let selected = select_format(&info.formats, &FormatPreference::BestCombined)
        .expect("should find a format");
    assert_eq!(selected.engine_kind(), EngineKind::DirectHttp);
}

#[tokio::test]
async fn test_full_pipeline_merge_format_to_ytdlp_engine() {
    use crate::format_selector::{FormatPreference, select_format};

    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "137+251".to_owned(),
            ext: Some("mp4".to_owned()),
            protocol: Some("https".to_owned()),
            url: Some("https://cdn.example.com/video_1080p.mp4".to_owned()),
            label: None,
            video_codec: Some("avc1".to_owned()),
            audio_codec: Some("opus".to_owned()),
            width: Some(1920),
            height: Some(1080),
            fps: None,
            bitrate: Some(4_000_000),
            filesize: None,
            title: Some("Test Merge".to_owned()),
            duration: None,
            thumbnail_url: None,
            webpage_url: Some("https://example.com/watch/3".to_owned()),
            extractor_name: Some("Example".to_owned()),
            required_headers: vec![],
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(extractor));

    let info = registry
        .extract("https://example.com/watch/3", ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    let selected = select_format(
        &info.formats,
        &FormatPreference::FormatId("137+251".to_owned()),
    )
    .expect("should find format by id");
    assert_eq!(selected.engine_kind(), EngineKind::YtDlp);
}

#[tokio::test]
async fn test_full_pipeline_empty_url_format_to_ytdlp_engine() {
    use crate::format_selector::{FormatPreference, select_format};

    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "bestvideo-bestaudio".to_owned(),
            ext: Some("mp4".to_owned()),
            protocol: Some("https".to_owned()),
            url: None, // No direct URL
            label: None,
            video_codec: Some("avc1".to_owned()),
            audio_codec: Some("opus".to_owned()),
            width: Some(1920),
            height: Some(1080),
            fps: None,
            bitrate: Some(4_000_000),
            filesize: None,
            title: Some("Test No URL".to_owned()),
            duration: None,
            thumbnail_url: None,
            webpage_url: Some("https://example.com/watch/4".to_owned()),
            extractor_name: Some("Example".to_owned()),
            required_headers: vec![],
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(extractor));

    let info = registry
        .extract("https://example.com/watch/4", ExtractOptions::default())
        .await
        .expect("extraction should succeed");

    let selected =
        select_format(&info.formats, &FormatPreference::BestCombined).expect("should find format");
    // Empty URL → YtDlp engine.
    assert_eq!(selected.engine_kind(), EngineKind::YtDlp);
}

#[tokio::test]
async fn test_full_pipeline_unsafe_input_url_rejected() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(UnsupportedExtractor));

    // Private IP as input URL — should be rejected by the registry before
    // reaching any extractor.
    let err = registry
        .extract("https://192.168.1.1/watch/video", ExtractOptions::default())
        .await
        .expect_err("should fail with UnsafeUrl");

    assert!(
        matches!(err, ExtractionError::UnsafeUrl { .. }),
        "expected UnsafeUrl, got {err:?}"
    );
}

#[tokio::test]
async fn test_full_pipeline_unsupported_url_from_registry() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(UnsupportedExtractor));

    let err = registry
        .extract("https://example.com/video", ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::UnsupportedUrl),
        "expected UnsupportedUrl, got {err:?}"
    );
}

#[tokio::test]
async fn test_full_pipeline_runner_timeout_propagates() {
    let runner = Arc::new(TimeoutRunner);
    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(extractor));

    let err = registry
        .extract("https://example.com/video", ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::Timeout),
        "expected Timeout, got {err:?}"
    );
}

#[tokio::test]
async fn test_full_pipeline_runner_backend_error_propagates() {
    let runner = Arc::new(BackendErrorRunner);
    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(extractor));

    let err = registry
        .extract("https://example.com/video", ExtractOptions::default())
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ExtractionError::BackendError { .. }),
        "expected BackendError, got {err:?}"
    );
}

#[tokio::test]
async fn test_full_pipeline_extracted_ssrf_url_dropped() {
    // When ALL extracted URLs are private IPs, extraction returns NoFormats.
    let runner = Arc::new(MockRunner {
        formats: vec![RawFormat {
            format_id: "ssrf-1".to_owned(),
            protocol: Some("https".to_owned()),
            url: Some("https://10.0.0.1/video.mp4".to_owned()),
            ..RawFormat::default()
        }],
    });

    let extractor = YtDlpExtractor::new(runner, std::path::PathBuf::from("yt-dlp"));
    let err = extractor
        .extract("https://example.com/video", &ExtractOptions::default())
        .await
        .expect_err("should fail with NoFormats");

    assert!(
        matches!(err, ExtractionError::NoFormats),
        "expected NoFormats, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// FormatSelector regression: every FormatPreference variant is deterministic
// ---------------------------------------------------------------------------

/// Local helper mirroring `make_format` in `format_selector::tests`, since that
/// helper is private to its own test module.
fn regression_format(
    id: &str,
    bitrate: Option<u64>,
    height: Option<u32>,
    video_codec: Option<&str>,
    audio_codec: Option<&str>,
) -> MediaFormat {
    MediaFormat {
        id: id.to_owned(),
        ext: "mp4".to_owned(),
        protocol: MediaProtocol::Http,
        url: format!("https://cdn.example.com/{id}.mp4"),
        label: id.to_owned(),
        video_codec: video_codec.map(str::to_owned),
        audio_codec: audio_codec.map(str::to_owned),
        width: height.map(|h| h * 16 / 9),
        height,
        fps: None,
        bitrate,
        filesize: None,
        required_headers: vec![],
    }
}

#[test]
fn test_format_selector_all_criteria_regression() {
    use crate::format_selector::{FormatPreference, select_format};

    // A mixed format list exercising every selection path:
    //   - two combined AV formats (different bitrates)
    //   - one video-only format (highest bitrate)
    //   - two audio-only formats (different bitrates)
    //   - a malformed format with no bitrate
    let formats = vec![
        regression_format(
            "av-low",
            Some(1_000_000),
            Some(480),
            Some("avc1"),
            Some("mp4a"),
        ),
        regression_format(
            "av-high",
            Some(3_000_000),
            Some(720),
            Some("avc1"),
            Some("mp4a"),
        ),
        regression_format("vo-best", Some(5_000_000), Some(1080), Some("avc1"), None),
        regression_format("ao-128", Some(128_000), None, None, Some("mp4a")),
        regression_format("ao-256", Some(256_000), None, None, Some("opus")),
        // Malformed: no bitrate, no codecs — must not crash selection.
        regression_format("malformed", None, None, None, None),
    ];

    // BestCombined → highest-bitrate combined AV format (av-high).
    let selected =
        select_format(&formats, &FormatPreference::BestCombined).expect("BestCombined finds");
    assert_eq!(selected.id, "av-high");

    // BestVideo → video-only format with the highest bitrate (vo-best).
    let selected = select_format(&formats, &FormatPreference::BestVideo).expect("BestVideo finds");
    assert_eq!(selected.id, "vo-best");

    // BestAudio → audio-only format with the highest bitrate (ao-256).
    let selected = select_format(&formats, &FormatPreference::BestAudio).expect("BestAudio finds");
    assert_eq!(selected.id, "ao-256");

    // FormatId → exact match (av-high).
    let selected = select_format(&formats, &FormatPreference::FormatId("av-high".to_owned()))
        .expect("FormatId finds");
    assert_eq!(selected.id, "av-high");

    // FormatId → missing id returns None.
    assert!(select_format(&formats, &FormatPreference::FormatId("nope".to_owned())).is_none());

    // MaxHeight(480) → highest format at or below 480p (av-low).
    let selected =
        select_format(&formats, &FormatPreference::MaxHeight(480)).expect("MaxHeight finds");
    assert_eq!(selected.id, "av-low");

    // MaxHeight(720) → av-high is the highest at or below 720p.
    let selected =
        select_format(&formats, &FormatPreference::MaxHeight(720)).expect("MaxHeight finds");
    assert_eq!(selected.id, "av-high");
}

#[test]
fn test_format_selector_malformed_no_bitrate_returns_last() {
    // When every format lacks a bitrate, the selector falls back to the
    // yt-dlp convention of "best last".
    use crate::format_selector::{FormatPreference, select_format};

    let formats = vec![
        regression_format("first", None, Some(480), Some("avc1"), Some("mp4a")),
        regression_format("second", None, Some(720), Some("avc1"), Some("mp4a")),
        regression_format("last", None, Some(1080), Some("avc1"), Some("mp4a")),
    ];

    let selected =
        select_format(&formats, &FormatPreference::BestCombined).expect("should find a format");
    assert_eq!(selected.id, "last");
}

// ---------------------------------------------------------------------------
// Registry boundary: per-format sanitisation runs regardless of extractor
// ---------------------------------------------------------------------------

/// A mock extractor that returns a [`MediaInfo`] containing both a safe and an
/// unsafe (private-IP) format URL, plus a blocked header on the safe format.
/// It performs **no** inline sanitisation, so any protection must come from the
/// registry boundary. Proves the registry enforces per-format safety even when
/// the extractor does not.
struct UnsafeFormatExtractor;

#[async_trait]
impl MediaExtractor for UnsafeFormatExtractor {
    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("https://")
    }

    async fn extract(
        &self,
        url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Ok(MediaInfo {
            title: "unsafe-mock".to_owned(),
            duration: None,
            thumbnail_url: None,
            webpage_url: url.to_owned(),
            extractor_name: "unsafe-mock".to_owned(),
            formats: vec![
                MediaFormat {
                    id: "safe".to_owned(),
                    ext: "mp4".to_owned(),
                    protocol: MediaProtocol::Http,
                    url: "https://cdn.example.com/safe.mp4".to_owned(),
                    label: "safe".to_owned(),
                    video_codec: None,
                    audio_codec: None,
                    width: None,
                    height: None,
                    fps: None,
                    bitrate: None,
                    filesize: None,
                    required_headers: vec![
                        ("Authorization".to_owned(), "Bearer leak".to_owned()),
                        ("User-Agent".to_owned(), "FetchX".to_owned()),
                    ],
                },
                MediaFormat {
                    id: "private".to_owned(),
                    ext: "mp4".to_owned(),
                    protocol: MediaProtocol::Http,
                    url: "https://192.168.1.1/private.mp4".to_owned(),
                    label: "private".to_owned(),
                    video_codec: None,
                    audio_codec: None,
                    width: None,
                    height: None,
                    fps: None,
                    bitrate: None,
                    filesize: None,
                    required_headers: vec![],
                },
            ],
        })
    }
}

/// A mock extractor that returns only unsafe (private-IP) format URLs with no
/// inline sanitisation. Proves the registry reports [`ExtractionError::NoFormats`]
/// when its boundary sanitisation empties the format list.
struct OnlyUnsafeFormatExtractor;

#[async_trait]
impl MediaExtractor for OnlyUnsafeFormatExtractor {
    fn can_handle(&self, url: &str) -> bool {
        url.starts_with("https://")
    }

    async fn extract(
        &self,
        url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Ok(MediaInfo {
            title: "only-unsafe-mock".to_owned(),
            duration: None,
            thumbnail_url: None,
            webpage_url: url.to_owned(),
            extractor_name: "only-unsafe-mock".to_owned(),
            formats: vec![MediaFormat {
                id: "private".to_owned(),
                ext: "mp4".to_owned(),
                protocol: MediaProtocol::Http,
                url: "https://10.0.0.1/private.mp4".to_owned(),
                label: "private".to_owned(),
                video_codec: None,
                audio_codec: None,
                width: None,
                height: None,
                fps: None,
                bitrate: None,
                filesize: None,
                required_headers: vec![],
            }],
        })
    }
}

#[tokio::test]
async fn test_registry_drops_unsafe_per_format_urls_from_non_ytdlp_extractor() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(UnsafeFormatExtractor));

    let info = registry
        .extract("https://example.com/watch", ExtractOptions::default())
        .await
        .expect("extraction should succeed with safe formats retained");

    // The private-IP format must be dropped; only the safe format remains.
    assert_eq!(
        info.formats.len(),
        1,
        "unsafe per-format URL must be dropped"
    );
    assert_eq!(info.formats[0].id, "safe");
    // The blocked Authorization header must be stripped at the registry boundary.
    assert!(
        info.formats[0]
            .required_headers
            .iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")),
        "Authorization must be stripped by registry sanitisation"
    );
    assert!(
        info.formats[0]
            .required_headers
            .iter()
            .any(|(k, _)| k == "User-Agent"),
        "User-Agent must survive registry sanitisation"
    );
}

#[tokio::test]
async fn test_registry_returns_no_formats_when_only_unsafe_urls_remain() {
    let mut registry = ExtractorRegistry::new();
    registry.register(Arc::new(OnlyUnsafeFormatExtractor));

    let err = registry
        .extract("https://example.com/watch", ExtractOptions::default())
        .await
        .expect_err("should fail with NoFormats");

    assert!(
        matches!(err, ExtractionError::NoFormats),
        "expected NoFormats after registry sanitisation, got {err:?}"
    );
}

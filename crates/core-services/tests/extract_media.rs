//! Integration test: `CoreServices::extract_media` routes through the
//! `ExtractorRegistry` → `YtDlpExtractor` → `YtDlpProcessRunnerAdapter` →
//! injected `YtDlpProcessRunner` chain.
//!
//! Deterministic: the injected runner is a stub that never spawns yt-dlp.

use std::{
    ffi::OsString,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use fetchx_core_services::{CoreServices, CoreServicesConfig, EngineConfig, YtDlpConfig};
use fetchx_downloader::{
    YtDlpError, YtDlpFormat, YtDlpProcessExit, YtDlpProcessRunner, YtDlpRunnerEvent,
    YtDlpRunnerFuture,
};
use fetchx_media_extractor::{ExtractOptions, ExtractionError};
use tokio::sync::{mpsc, watch};

/// A stub `YtDlpProcessRunner` whose `discover_formats` always fails with
/// `YtDlpError::FormatDiscoveryUnsupported`, proving the error propagates
/// through the adapter → extractor → registry → `CoreServices::extract_media`.
struct StubDiscoveryRunner {
    discover_calls: Arc<AtomicUsize>,
    formats: Vec<YtDlpFormat>,
}

impl YtDlpProcessRunner for StubDiscoveryRunner {
    fn validate_version<'a>(
        &'a self,
        _executable: &'a Path,
        _timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
        Box::pin(async { Ok("test-version".to_owned()) })
    }

    fn run<'a>(
        &'a self,
        _executable: &'a Path,
        _arguments: &'a [OsString],
        _stop: watch::Receiver<bool>,
        _events: mpsc::Sender<YtDlpRunnerEvent>,
    ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
        Box::pin(async {
            Ok(YtDlpProcessExit {
                success: true,
                code: Some(0),
            })
        })
    }

    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        _url: &'a str,
        _timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
        self.discover_calls.fetch_add(1, Ordering::Relaxed);
        let formats = self.formats.clone();
        Box::pin(async move {
            if formats.is_empty() {
                Err(YtDlpError::FormatDiscoveryUnsupported)
            } else {
                Ok(formats)
            }
        })
    }
}

#[tokio::test]
async fn test_core_services_extract_media_calls_registry() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempfile::tempdir()?;

    let cfg = CoreServicesConfig {
        database_path: root.path().join("fetchx.db"),
        downloads_dir: root.path().join("downloads"),
        runtime_dir: root.path().join("runtime"),
        bundled_engines_dir: None,
        defer_queued_on_start: false,
    };

    // Enable yt-dlp so the YtDlpExtractor is registered in the registry.
    let engine_config = EngineConfig {
        yt_dlp: YtDlpConfig {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let discover_calls = Arc::new(AtomicUsize::new(0));
    let services = CoreServices::start_with_engine_config_and_ytdlp_runner(
        cfg,
        Some(engine_config),
        Some(Arc::new(StubDiscoveryRunner {
            discover_calls: Arc::clone(&discover_calls),
            formats: Vec::new(),
        })),
    )
    .await?;

    // The native YouTube extractor declines arbitrary caller headers before
    // making a request. The Generic extractor also declines YouTube URLs, so
    // the registry reaches the YtDlpExtractor backed by our stub runner.
    // `extract_media` must surface the legacy runner's error. The stub's
    // `discover_formats` returns `FormatDiscoveryUnsupported`, which the
    // adapter maps to `RunnerError::Unsupported` and the extractor maps to
    // `ExtractionError::UnsupportedUrl`.
    let result = services
        .extract_media(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            ExtractOptions {
                headers: vec![("X-Test-Context".to_owned(), "caller-provided".to_owned())],
                ..ExtractOptions::default()
            },
        )
        .await;

    let err = result.expect_err("extract_media should propagate the runner error");
    assert!(
        matches!(err, ExtractionError::UnsupportedUrl),
        "expected ExtractionError::UnsupportedUrl from the stub runner, got {err:?}"
    );
    for source in [
        "https://www.globalplayer.com/videos/2JsSZ7Gm2uP/",
        "https://www.ndr.de/ndraktuell28488-player.html",
        "https://www.tvp.pl/sess/tvplayer.php?object_id=51247504",
    ] {
        let result = services
            .extract_media(
                source,
                ExtractOptions {
                    cookies: Some("caller-provided".to_owned()),
                    ..ExtractOptions::default()
                },
            )
            .await;
        assert!(
            matches!(result, Err(ExtractionError::UnsupportedUrl)),
            "expected unsupported native result to reach fallback for {source}, got {result:?}"
        );
    }
    assert_eq!(discover_calls.load(Ordering::Relaxed), 4);

    services.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn yt_dlp_discovery_precedes_generic_fallback_and_preserves_qualities()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let cfg = CoreServicesConfig {
        database_path: root.path().join("fetchx.db"),
        downloads_dir: root.path().join("downloads"),
        runtime_dir: root.path().join("runtime"),
        bundled_engines_dir: None,
        defer_queued_on_start: false,
    };
    let expected_qualities = ["360p", "480p", "720p", "1080p"];
    let discover_calls = Arc::new(AtomicUsize::new(0));
    let services = CoreServices::start_with_engine_config_and_ytdlp_runner(
        cfg,
        Some(EngineConfig {
            yt_dlp: YtDlpConfig {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        }),
        Some(Arc::new(StubDiscoveryRunner {
            discover_calls: Arc::clone(&discover_calls),
            formats: expected_qualities
                .iter()
                .map(|quality| YtDlpFormat {
                    format_id: quality.to_string(),
                    label: quality.to_string(),
                    extension: Some("mp4".to_owned()),
                    file_size_bytes: None,
                    title: Some("Test video".to_owned()),
                    platform: Some("Test".to_owned()),
                })
                .collect(),
        })),
    )
    .await?;

    let info = services
        .extract_media(
            "https://example.invalid/video",
            ExtractOptions {
                timeout: Some(Duration::ZERO),
                ..ExtractOptions::default()
            },
        )
        .await?;
    let qualities: Vec<_> = info
        .formats
        .iter()
        .map(|format| format.label.as_str())
        .collect();
    assert_eq!(qualities, expected_qualities);
    assert_eq!(discover_calls.load(Ordering::Relaxed), 1);

    services.shutdown().await?;
    Ok(())
}

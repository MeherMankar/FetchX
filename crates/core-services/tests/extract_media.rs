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
struct UnsupportedDiscoveryRunner {
    discover_calls: Arc<AtomicUsize>,
}

impl YtDlpProcessRunner for UnsupportedDiscoveryRunner {
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
        Box::pin(async { Err(YtDlpError::FormatDiscoveryUnsupported) })
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
        Some(Arc::new(UnsupportedDiscoveryRunner {
            discover_calls: Arc::clone(&discover_calls),
        })),
    )
    .await?;

    // The native YouTube extractor defers pages that need request context. The
    // Generic extractor also declines YouTube URLs, so the registry reaches the
    // YtDlpExtractor backed by our stub runner. `extract_media` must surface the
    // legacy runner's error. The stub's
    // `discover_formats` returns `FormatDiscoveryUnsupported`, which the
    // adapter maps to `RunnerError::Unsupported` and the extractor maps to
    // `ExtractionError::UnsupportedUrl`.
    let result = services
        .extract_media(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            ExtractOptions {
                cookies: Some("example.com\tFALSE\t/\tFALSE\t0\ttest\tvalue\n".to_owned()),
                ..ExtractOptions::default()
            },
        )
        .await;

    let err = result.expect_err("extract_media should propagate the runner error");
    assert!(
        matches!(err, ExtractionError::UnsupportedUrl),
        "expected ExtractionError::UnsupportedUrl from the stub runner, got {err:?}"
    );
    assert_eq!(discover_calls.load(Ordering::Relaxed), 1);

    services.shutdown().await?;
    Ok(())
}

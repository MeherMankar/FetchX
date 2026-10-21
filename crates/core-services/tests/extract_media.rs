//! Integration test: `CoreServices::extract_media` routes through the
//! `ExtractorRegistry` → `YtDlpExtractor` → `YtDlpProcessRunnerAdapter` →
//! injected `YtDlpProcessRunner` chain.
//!
//! Deterministic: the injected runner is a stub that never spawns yt-dlp.

use std::{
    ffi::OsString,
    path::Path,
    sync::Arc,
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
struct UnsupportedDiscoveryRunner;

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
    };

    // Enable yt-dlp so the YtDlpExtractor is registered in the registry.
    let engine_config = EngineConfig {
        yt_dlp: YtDlpConfig {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let services = CoreServices::start_with_engine_config_and_ytdlp_runner(
        cfg,
        Some(engine_config),
        Some(Arc::new(UnsupportedDiscoveryRunner)),
    )
    .await?;

    // The registry is wired with the YtDlpExtractor backed by our stub runner.
    // `extract_media` must reach the runner and surface its error. The stub's
    // `discover_formats` returns `FormatDiscoveryUnsupported`, which the
    // adapter maps to `RunnerError::Unsupported` and the extractor maps to
    // `ExtractionError::UnsupportedUrl`.
    let result = services
        .extract_media("https://example.com/video", ExtractOptions::default())
        .await;

    let err = result.expect_err("extract_media should propagate the runner error");
    assert!(
        matches!(err, ExtractionError::UnsupportedUrl),
        "expected ExtractionError::UnsupportedUrl from the stub runner, got {err:?}"
    );

    services.shutdown().await?;
    Ok(())
}

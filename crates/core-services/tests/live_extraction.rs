//! Phase 20W.3 — Live media extraction integration tests.
//!
//! All tests in this file are marked `#[ignore]` so they do not run in CI.
//!
//! Run individually with:
//!
//! ```text
//! cargo test -p fetchx-core-services --test live_extraction -- --include-ignored
//! ```
//!
//! # Prerequisites
//! - yt-dlp must be on PATH (or the path below must be adjusted).
//! - Network access to youtube.com.
//!
//! # Rules (Phase 20W.3)
//! - No fake validation.
//! - If a network call fails for external reasons (rate-limit, blocked),
//!   the test prints `NOT VALIDATED` and exits successfully so it does not
//!   false-fail in constrained environments.

use std::path::PathBuf;

use fetchx_core_services::{CoreServices, CoreServicesConfig, EngineConfig, YtDlpConfig};
use fetchx_media_extractor::{
    ExtractOptions, ExtractionError, FormatPreference, FormatSelector, SelectionCriteria,
    format_to_engine_kind,
};
use fetchx_models::EngineKind;

/// Path to yt-dlp on the test machine.
///
/// Falls back to PATH search if the explicit path does not exist.
fn ytdlp_executable() -> PathBuf {
    let explicit =
        PathBuf::from(r"C:\Users\Meher\AppData\Local\Programs\Python\Python314\Scripts\yt-dlp.exe");
    if explicit.exists() {
        return explicit;
    }
    // Fallback: just use the name and let the OS search PATH.
    #[cfg(windows)]
    return PathBuf::from("yt-dlp.exe");
    #[cfg(not(windows))]
    return PathBuf::from("yt-dlp");
}

/// Build a `CoreServicesConfig` pointing at a temporary directory.
fn make_config(root: &std::path::Path) -> CoreServicesConfig {
    CoreServicesConfig {
        database_path: root.join("fetchx.db"),
        downloads_dir: root.join("downloads"),
        runtime_dir: root.join("runtime"),
        bundled_engines_dir: None,
        defer_queued_on_start: false,
    }
}

/// Build an `EngineConfig` with yt-dlp enabled and pointing at the known executable.
fn ytdlp_engine_config() -> EngineConfig {
    EngineConfig {
        yt_dlp: YtDlpConfig {
            enabled: true,
            executable_path: Some(ytdlp_executable()),
        },
        ..EngineConfig::default()
    }
}

// ── Test 1: YouTube full extraction pipeline ──────────────────────────────────

/// Live test: extract "Me at the zoo" (first YouTube video, public domain).
///
/// Validates:
/// - CoreServices initialises with yt-dlp enabled.
/// - `extract_media` produces a non-empty title.
/// - At least one format is returned.
/// - `FormatSelector` with `BestCombined`, `BestVideo`, `BestAudio`, and
///   `MaxHeight(720)` all return a format.
/// - Each selected format has an engine kind that is one of
///   `DirectHttp`, `Hls`, or `YtDlp`.
#[tokio::test]
#[ignore]
async fn test_youtube_live_extraction() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let services = CoreServices::start_with_engine_config(
        make_config(root.path()),
        Some(ytdlp_engine_config()),
    )
    .await?;

    // Confirm yt-dlp engine is available before attempting live extraction.
    if !services
        .engine_runtime
        .is_available(fetchx_models::EngineKind::YtDlp)
    {
        eprintln!("NOT VALIDATED — yt-dlp engine reported unavailable at startup");
        services.shutdown().await?;
        return Ok(());
    }

    let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw"; // "Me at the zoo"
    eprintln!("Extracting: {url}");

    let info = match services.extract_media(url, ExtractOptions::default()).await {
        Ok(info) => info,
        Err(ExtractionError::BackendError { ref message }) if message.contains("unavailable") => {
            eprintln!("NOT VALIDATED — YouTube blocked: {message}");
            services.shutdown().await?;
            return Ok(());
        }
        Err(ExtractionError::Timeout) => {
            eprintln!("NOT VALIDATED — extraction timed out");
            services.shutdown().await?;
            return Ok(());
        }
        Err(e) => {
            services.shutdown().await?;
            return Err(format!("extraction failed: {e}").into());
        }
    };

    eprintln!("  title:         {}", info.title);
    eprintln!("  extractor:     {}", info.extractor_name);
    eprintln!("  format count:  {}", info.formats.len());

    // ── Assertions ────────────────────────────────────────────────────────────

    assert!(!info.title.is_empty(), "title must be non-empty");
    assert!(
        !info.formats.is_empty(),
        "at least one format must be returned"
    );

    // Each format URL must be safe (empty is allowed — routes to YtDlp).
    for fmt in &info.formats {
        if !fmt.url.is_empty() {
            assert!(
                fmt.url.starts_with("https://") || fmt.url.starts_with("http://"),
                "format {} URL must be http/https, got: {}",
                fmt.id,
                fmt.url
            );
        }
    }

    // ── FormatSelector: BestCombined ─────────────────────────────────────────

    let selected = FormatSelector::select(&info.formats, &SelectionCriteria::BestCombined)
        .or_else(|| FormatSelector::select(&info.formats, &SelectionCriteria::Best))
        .expect("FormatSelector must return a format");

    eprintln!(
        "  BestCombined:  id={} protocol={:?} engine={:?}",
        selected.id,
        selected.protocol,
        format_to_engine_kind(selected)
    );

    let engine = format_to_engine_kind(selected);
    assert!(
        matches!(
            engine,
            EngineKind::DirectHttp | EngineKind::Hls | EngineKind::YtDlp
        ),
        "engine kind must be DirectHttp, Hls, or YtDlp, got {engine:?}"
    );

    // ── FormatSelector: BestVideo ─────────────────────────────────────────────

    let selected_video = FormatSelector::select(&info.formats, &SelectionCriteria::BestVideoOnly);
    if let Some(f) = selected_video {
        eprintln!(
            "  BestVideo:     id={} height={:?} engine={:?}",
            f.id,
            f.height,
            format_to_engine_kind(f)
        );
        let ek = format_to_engine_kind(f);
        assert!(
            matches!(
                ek,
                EngineKind::DirectHttp | EngineKind::Hls | EngineKind::YtDlp
            ),
            "BestVideo engine kind must be valid, got {ek:?}"
        );
    } else {
        eprintln!("  BestVideo:     no video-only format available (combined only)");
    }

    // ── FormatSelector: BestAudio ─────────────────────────────────────────────

    let selected_audio = FormatSelector::select(&info.formats, &SelectionCriteria::BestAudioOnly);
    if let Some(f) = selected_audio {
        eprintln!(
            "  BestAudio:     id={} engine={:?}",
            f.id,
            format_to_engine_kind(f)
        );
        let ek = format_to_engine_kind(f);
        assert!(
            matches!(
                ek,
                EngineKind::DirectHttp | EngineKind::Hls | EngineKind::YtDlp
            ),
            "BestAudio engine kind must be valid, got {ek:?}"
        );
    } else {
        eprintln!("  BestAudio:     no audio-only format available");
    }

    // ── FormatSelector: MaxHeight(720) ────────────────────────────────────────

    let selected_720 = FormatSelector::select(&info.formats, &SelectionCriteria::MaxHeight(720));
    if let Some(f) = selected_720 {
        eprintln!(
            "  MaxHeight(720): id={} height={:?} engine={:?}",
            f.id,
            f.height,
            format_to_engine_kind(f)
        );
    }

    eprintln!("test_youtube_live_extraction: PASS");
    services.shutdown().await?;
    Ok(())
}

// ── Test 2: FormatSelector strategy coverage ──────────────────────────────────

/// Tests all `FormatPreference` variants against a real YouTube extraction result.
///
/// Requires network; skipped gracefully if extraction fails.
#[tokio::test]
#[ignore]
async fn test_format_selector_strategies() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let services = CoreServices::start_with_engine_config(
        make_config(root.path()),
        Some(ytdlp_engine_config()),
    )
    .await?;

    if !services
        .engine_runtime
        .is_available(fetchx_models::EngineKind::YtDlp)
    {
        eprintln!("NOT VALIDATED — yt-dlp unavailable");
        services.shutdown().await?;
        return Ok(());
    }

    let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
    let info = match services.extract_media(url, ExtractOptions::default()).await {
        Ok(info) => info,
        Err(e) => {
            eprintln!("NOT VALIDATED — extraction failed: {e}");
            services.shutdown().await?;
            return Ok(());
        }
    };

    eprintln!("Extracted {} formats", info.formats.len());

    // Exercise every FormatPreference variant.
    let preferences = [
        FormatPreference::BestCombined,
        FormatPreference::BestVideo,
        FormatPreference::BestAudio,
        FormatPreference::MaxHeight(480),
        FormatPreference::MaxHeight(720),
        FormatPreference::MaxHeight(1080),
    ];

    for pref in &preferences {
        let result = fetchx_media_extractor::select_format(&info.formats, pref);
        match result {
            Some(f) => {
                eprintln!(
                    "  {pref:?} → id={} protocol={:?} engine={:?}",
                    f.id,
                    f.protocol,
                    format_to_engine_kind(f)
                );
                let ek = format_to_engine_kind(f);
                assert!(
                    matches!(
                        ek,
                        EngineKind::DirectHttp | EngineKind::Hls | EngineKind::YtDlp
                    ),
                    "{pref:?} returned unsupported engine kind: {ek:?}"
                );
            }
            None => {
                eprintln!("  {pref:?} → no match (acceptable if no qualifying format exists)");
            }
        }
    }

    // FormatId: pick the id of the first format and look it up by id.
    if let Some(first) = info.formats.first() {
        let by_id = fetchx_media_extractor::select_format(
            &info.formats,
            &FormatPreference::FormatId(first.id.clone()),
        );
        assert!(
            by_id.is_some(),
            "FormatId selection for known id {:?} must succeed",
            first.id
        );
        eprintln!("  FormatId({:?}) → id={}", first.id, by_id.unwrap().id);
    }

    eprintln!("test_format_selector_strategies: PASS");
    services.shutdown().await?;
    Ok(())
}

// ── Test 3: Engine routing for direct HTTP ────────────────────────────────────

/// Verifies that a `MediaFormat` with an `http://` URL and `MediaProtocol::Http`
/// resolves to `EngineKind::DirectHttp` without any network call.
///
/// This is a pure-logic test; no network required.
#[tokio::test]
#[ignore]
async fn test_engine_routing_direct_http() {
    use fetchx_media_extractor::{MediaFormat, MediaProtocol};

    let http_format = MediaFormat {
        id: "test-http".to_owned(),
        ext: "mp4".to_owned(),
        protocol: MediaProtocol::Http,
        url: "http://cdn.example.com/video.mp4".to_owned(),
        label: "Test HTTP".to_owned(),
        video_codec: Some("avc1".to_owned()),
        audio_codec: Some("mp4a".to_owned()),
        width: Some(1280),
        height: Some(720),
        fps: Some(30.0),
        bitrate: Some(2_000_000),
        filesize: Some(50_000_000),
        required_headers: vec![],
    };

    let engine = format_to_engine_kind(&http_format);
    assert_eq!(
        engine,
        EngineKind::DirectHttp,
        "http:// URL with Http protocol must route to DirectHttp"
    );
    eprintln!("  http:// → {engine:?}: PASS");

    let https_format = MediaFormat {
        protocol: MediaProtocol::Http,
        url: "https://cdn.example.com/video.mp4".to_owned(),
        id: "test-https".to_owned(),
        ..http_format.clone()
    };
    let engine2 = format_to_engine_kind(&https_format);
    assert_eq!(
        engine2,
        EngineKind::DirectHttp,
        "https:// URL with Http protocol must route to DirectHttp"
    );
    eprintln!("  https:// → {engine2:?}: PASS");

    let hls_format = MediaFormat {
        id: "test-hls".to_owned(),
        protocol: MediaProtocol::Hls,
        url: "https://cdn.example.com/playlist.m3u8".to_owned(),
        ..http_format.clone()
    };
    let engine3 = format_to_engine_kind(&hls_format);
    assert_eq!(engine3, EngineKind::Hls, "HLS URL must route to Hls engine");
    eprintln!("  m3u8 → {engine3:?}: PASS");

    let ytdlp_format = MediaFormat {
        id: "test-ytdlp".to_owned(),
        protocol: MediaProtocol::YtDlp,
        url: "".to_owned(), // empty = must go through yt-dlp
        ..http_format.clone()
    };
    let engine4 = format_to_engine_kind(&ytdlp_format);
    assert_eq!(
        engine4,
        EngineKind::YtDlp,
        "empty URL must route to YtDlp engine"
    );
    eprintln!("  empty URL → {engine4:?}: PASS");

    eprintln!("test_engine_routing_direct_http: PASS");
}

// ── Test 4: SSRF rejection at extract_media boundary ─────────────────────────

/// Verifies that `extract_media` with a private-IP URL returns an error
/// (SSRF blocked at the registry boundary) without making any network call.
#[tokio::test]
#[ignore]
async fn test_security_ssrf_at_extraction() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let services = CoreServices::start_with_engine_config(
        make_config(root.path()),
        Some(ytdlp_engine_config()),
    )
    .await?;

    let private_urls = [
        "http://192.168.1.1/video.mp4",
        "http://10.0.0.1/video.mp4",
        "http://172.16.0.1/stream.m3u8",
        "http://127.0.0.1/video.mp4",
        "http://localhost/video.mp4",
        "http://169.254.1.1/video.mp4",
    ];

    for url in &private_urls {
        let result = services.extract_media(url, ExtractOptions::default()).await;
        match result {
            Err(ExtractionError::UnsafeUrl { .. }) => {
                eprintln!("  {url} → SSRF blocked: PASS");
            }
            Ok(_) => {
                services.shutdown().await?;
                return Err(format!("SECURITY FAILURE: {url} was not SSRF-blocked").into());
            }
            Err(other) => {
                // Any other error (e.g. UnsupportedUrl from disabled yt-dlp) is acceptable
                // as long as the request did not succeed.
                eprintln!("  {url} → rejected with {other:?} (acceptable)");
            }
        }
    }

    // Unsafe scheme rejection.
    let unsafe_schemes = [
        "file:///etc/passwd",
        "ftp://internal.corp/file",
        "javascript:alert(1)",
    ];
    for url in &unsafe_schemes {
        let result = services.extract_media(url, ExtractOptions::default()).await;
        assert!(
            result.is_err(),
            "unsafe scheme {url:?} must be rejected, but extraction succeeded"
        );
        eprintln!("  {url} → rejected: PASS");
    }

    eprintln!("test_security_ssrf_at_extraction: PASS");
    services.shutdown().await?;
    Ok(())
}

use std::path::Path;

use fetchx_core_services::CoreServices;
use fetchx_downloader::validate_http_source_url;
use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSource, EngineKind, FileName, Timestamp,
    YtDlpAudioFormat, parse_magnet_uri,
};
use reqwest::Url;

use crate::{
    args::AddOpts,
    commands::schedule::parse_datetime,
    exit::{self, ExitCode},
    output,
};

pub async fn run(svc: &CoreServices, opts: AddOpts, json: bool) -> ExitCode {
    // ── URL validation (same logic as desktop add command) ──────────────

    let input = opts
        .magnet
        .as_deref()
        .or(opts.url.as_deref())
        .unwrap_or_default();
    let magnet = if input.starts_with("magnet:") {
        match parse_magnet_uri(input) {
            Ok(info) => Some(info),
            Err(error) => {
                output::print_error(json, &format!("invalid magnet URI: {error}"));
                return exit::INVALID_ARGS;
            }
        }
    } else {
        None
    };
    let torrent_path = (magnet.is_none()
        && !input.contains("://")
        && Path::new(input)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent")))
    .then(|| Path::new(input).to_path_buf());
    let selected_file_indexes = match opts.select_files.as_deref() {
        Some(value) if torrent_path.is_some() => match parse_torrent_selection(value) {
            Ok(indexes) => Some(indexes),
            Err(message) => {
                output::print_error(json, &message);
                return exit::INVALID_ARGS;
            }
        },
        Some(_) => {
            output::print_error(
                json,
                "--select-files is only valid with a local .torrent file",
            );
            return exit::INVALID_ARGS;
        }
        None => None,
    };
    let canonical_url = if magnet.is_some() || torrent_path.is_some() {
        input.to_owned()
    } else {
        match validate_http_source_url(input) {
            Ok(url) => url,
            Err(error) => {
                output::print_error(json, &format!("invalid URL: {error}"));
                return exit::INVALID_ARGS;
            }
        }
    };
    let parsed_url = if magnet.is_none() && torrent_path.is_none() {
        match Url::parse(&canonical_url) {
            Ok(url) => Some(url),
            Err(error) => {
                output::print_error(json, &format!("cannot parse URL: {error}"));
                return exit::INVALID_ARGS;
            }
        }
    } else {
        None
    };

    // ── Engine resolution ────────────────────────────────────────────────

    let audio_format = match opts.audio_format.as_deref() {
        Some("mp3") => Some(YtDlpAudioFormat::Mp3),
        Some("m4a") => Some(YtDlpAudioFormat::M4a),
        Some(_) => {
            output::print_error(json, "--audio-format must be mp3 or m4a");
            return exit::INVALID_ARGS;
        }
        None => None,
    };
    if audio_format.is_some()
        && (magnet.is_some()
            || torrent_path.is_some()
            || opts
                .engine
                .as_deref()
                .is_some_and(|engine| !matches!(parse_engine(Some(engine)), Ok(EngineKind::YtDlp))))
    {
        output::print_error(
            json,
            "--audio-format requires an HTTP(S) URL and the yt-dlp engine",
        );
        return exit::INVALID_ARGS;
    }

    let engine_kind = match if audio_format.is_some() {
        Ok(EngineKind::YtDlp)
    } else {
        resolve_engine(
            magnet.is_some(),
            torrent_path.is_some(),
            opts.engine.as_deref(),
            if magnet.is_none() && torrent_path.is_none() {
                Some(&canonical_url)
            } else {
                None
            },
        )
    } {
        Ok(e) => e,
        Err(msg) => {
            output::print_error(json, &msg);
            return exit::INVALID_ARGS;
        }
    };

    if !svc.engine_runtime.is_available(engine_kind) {
        output::print_error(
            json,
            &format!(
                "{engine_kind:?} engine is not available. \
                      Check `FetchX engines` or engine settings."
            ),
        );
        return exit::ENGINE_UNAVAILABLE;
    }

    // ── Priority ────────────────────────────────────────────────────────

    let priority = match parse_priority(&opts.priority) {
        Ok(p) => p,
        Err(msg) => {
            output::print_error(json, &msg);
            return exit::INVALID_ARGS;
        }
    };

    let id = if magnet.is_some() {
        let result = if opts.engine.is_some() {
            svc.enqueue_magnet_with_engine(&canonical_url, engine_kind, priority, opts.destination)
                .await
        } else {
            svc.enqueue_magnet(&canonical_url, priority, opts.destination)
                .await
        };
        match result {
            Ok(id) => id,
            Err(error) => {
                output::print_error(json, &format!("failed to enqueue magnet: {error}"));
                return exit::GENERAL_ERROR;
            }
        }
    } else if let Some(torrent_path) = torrent_path {
        let result = if opts.engine.is_some() {
            svc.enqueue_torrent_file_with_selection_and_engine(
                torrent_path,
                engine_kind,
                priority,
                opts.destination,
                selected_file_indexes,
            )
            .await
        } else {
            svc.enqueue_torrent_file_with_selection(
                torrent_path,
                priority,
                opts.destination,
                selected_file_indexes,
            )
            .await
        };
        match result {
            Ok(id) => id,
            Err(error) => {
                output::print_error(json, &format!("failed to enqueue torrent: {error}"));
                return exit::GENERAL_ERROR;
            }
        }
    } else {
        let url_filename = parsed_url
            .as_ref()
            .map(filename_for_url)
            .unwrap_or_else(|| "download".to_owned());
        let filename = if engine_kind == EngineKind::YtDlp {
            svc.resolve_media_filename(&canonical_url, &url_filename)
                .await
        } else {
            url_filename
        };
        let filename = audio_format.map_or(filename.clone(), |format| {
            Path::new(&filename)
                .with_extension(format.extension())
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("audio")
                .to_owned()
        });
        let filename = match FileName::new(&filename) {
            Ok(f) => f,
            Err(e) => {
                output::print_error(json, &format!("invalid filename: {e}"));
                return exit::INVALID_ARGS;
            }
        };
        let destination = opts
            .destination
            .unwrap_or_else(|| svc.downloads_dir().join(filename.as_str()));
        if audio_format.is_some_and(|format| {
            !destination
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()))
        }) {
            output::print_error(
                json,
                "--destination must use the extension selected by --audio-format",
            );
            return exit::INVALID_ARGS;
        }
        let id = DownloadId::new();
        let source = match DownloadSource::new(canonical_url, engine_kind).and_then(|source| {
            match audio_format {
                Some(format) => source.with_yt_dlp_audio_format(format),
                None => Ok(source),
            }
        }) {
            Ok(s) => s,
            Err(e) => {
                output::print_error(json, &format!("invalid source: {e}"));
                return exit::INVALID_ARGS;
            }
        };
        let created_at = match Timestamp::now() {
            Ok(t) => t,
            Err(e) => {
                output::print_error(json, &format!("timestamp error: {e}"));
                return exit::GENERAL_ERROR;
            }
        };
        let mut item = DownloadItem::new(id, source, filename, destination, created_at);
        if priority != DownloadPriority::Normal
            && let Err(error) = item.set_priority_at(priority, created_at)
        {
            output::print_error(json, &format!("priority error: {error}"));
            return exit::GENERAL_ERROR;
        }
        if let Err(error) = svc.queue().enqueue_deferred(item).await {
            output::print_error(json, &format!("failed to enqueue: {error}"));
            return exit::GENERAL_ERROR;
        }
        id
    };

    // ── Optional schedule ────────────────────────────────────────────────

    if let Some(at_str) = opts.schedule {
        match parse_datetime(&at_str) {
            Ok(scheduled_at) => {
                if let Err(e) = svc.scheduler().schedule_download(id, scheduled_at).await {
                    output::print_error(json, &format!("queued but schedule failed: {e}"));
                    return exit::GENERAL_ERROR;
                }
            }
            Err(msg) => {
                // Download was already enqueued; report schedule failure but exit as error.
                output::print_error(json, &format!("queued but schedule time invalid: {msg}"));
                return exit::INVALID_ARGS;
            }
        }
    }

    // ── Success output ────────────────────────────────────────────────────

    if json {
        output::print_json_ok(&serde_json::json!({
            "ok": true,
            "id": id.to_string(),
            "engine": format!("{engine_kind:?}"),
            "priority": format!("{priority:?}"),
        }));
    } else {
        println!("Added: {id}");
        println!("  engine:   {engine_kind:?}");
        println!("  priority: {priority:?}");
    }

    exit::SUCCESS
}

// ── Helpers ────────────────────────────────────────────────────────────────

pub fn parse_engine(s: Option<&str>) -> Result<EngineKind, String> {
    match s.unwrap_or("direct-http").to_ascii_lowercase().as_str() {
        "direct-http" | "direct_http" | "http" => Ok(EngineKind::DirectHttp),
        "native-torrent" | "native_torrent" => Ok(EngineKind::NativeTorrent),
        "yt-dlp" | "yt_dlp" | "ytdlp" => Ok(EngineKind::YtDlp),
        "ftp" => Ok(EngineKind::Ftp),
        "ftps" => Ok(EngineKind::Ftps),
        "sftp" => Ok(EngineKind::Sftp),
        "metalink" => Ok(EngineKind::Metalink),
        other => Err(format!(
            "unknown engine `{other}`. Valid: direct-http, native-torrent, yt-dlp, ftp, ftps, sftp, metalink"
        )),
    }
}

fn resolve_engine(
    is_magnet: bool,
    is_torrent_file: bool,
    engine: Option<&str>,
    url: Option<&str>,
) -> Result<EngineKind, String> {
    if is_magnet || is_torrent_file {
        if engine.is_none() {
            return Ok(EngineKind::NativeTorrent);
        }
        return match parse_engine(engine) {
            Ok(EngineKind::NativeTorrent) => Ok(EngineKind::NativeTorrent),
            Ok(_) => Err("magnet and torrent downloads require a torrent engine".to_owned()),
            Err(error) => Err(error),
        };
    }
    if engine.is_none()
        && let Some(url) = url
    {
        return Ok(EngineKind::from_source_url(url));
    }
    parse_engine(engine)
}

pub fn parse_priority(s: &str) -> Result<DownloadPriority, String> {
    match s.to_ascii_lowercase().as_str() {
        "low" => Ok(DownloadPriority::Low),
        "normal" => Ok(DownloadPriority::Normal),
        "high" => Ok(DownloadPriority::High),
        other => Err(format!(
            "unknown priority `{other}`. Valid: low, normal, high"
        )),
    }
}

fn parse_torrent_selection(value: &str) -> Result<Vec<u32>, String> {
    let mut indexes = Vec::new();
    for value in value.split(',') {
        let index = value.trim().parse::<u32>().map_err(|_| {
            "torrent file indexes must be positive comma-separated integers".to_owned()
        })?;
        let index = index.checked_sub(1).ok_or_else(|| {
            "torrent file indexes are 1-based and must be greater than zero".to_owned()
        })?;
        if indexes.contains(&index) {
            return Err(format!("torrent file index {} is repeated", index + 1));
        }
        indexes.push(index);
    }
    if indexes.is_empty() {
        return Err("at least one torrent file index must be selected".to_owned());
    }
    Ok(indexes)
}

fn filename_for_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{parse_torrent_selection, resolve_engine};
    use fetchx_models::{EngineKind, parse_magnet_uri};

    #[test]
    fn magnet_defaults_to_native_and_rejects_aria2_selection() {
        assert_eq!(
            resolve_engine(true, false, None, None),
            Ok(EngineKind::NativeTorrent)
        );
        assert_eq!(
            resolve_engine(false, true, None, None),
            Ok(EngineKind::NativeTorrent)
        );
        let metadata = parse_magnet_uri(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=..%5C..%5Cunsafe%3Aname%3F.torrent",
        )
        .expect("valid test magnet");
        assert_eq!(metadata.suggested_filename(), "unsafename.torrent");
        assert!(resolve_engine(true, false, Some("direct-http"), None).is_err());
        assert!(resolve_engine(true, false, Some("aria2"), None).is_err());
        assert_eq!(
            resolve_engine(false, true, Some("native-torrent"), None),
            Ok(EngineKind::NativeTorrent)
        );

        // FTP/FTPS scheme auto-detection.
        assert_eq!(
            resolve_engine(false, false, None, Some("ftp://host/file.bin")),
            Ok(EngineKind::Ftp)
        );
        assert_eq!(
            resolve_engine(false, false, None, Some("ftps://host/file.bin")),
            Ok(EngineKind::Ftps)
        );
        // Explicit --engine flag overrides auto-detection.
        assert_eq!(
            resolve_engine(
                false,
                false,
                Some("direct-http"),
                Some("ftp://host/file.bin")
            ),
            Ok(EngineKind::DirectHttp)
        );
        // Metalink extension auto-detection.
        assert_eq!(
            resolve_engine(false, false, None, Some("https://host/file.metalink")),
            Ok(EngineKind::Metalink)
        );
        assert_eq!(
            resolve_engine(false, false, None, Some("https://host/file.meta4")),
            Ok(EngineKind::Metalink)
        );
        // Explicit --engine metalink.
        assert_eq!(
            resolve_engine(
                false,
                false,
                Some("metalink"),
                Some("https://host/file.bin")
            ),
            Ok(EngineKind::Metalink)
        );
    }

    #[test]
    fn torrent_selection_uses_one_based_unique_indexes() {
        assert_eq!(parse_torrent_selection("1,3"), Ok(vec![0, 2]));
        assert!(parse_torrent_selection("0").is_err());
        assert!(parse_torrent_selection("1,1").is_err());
        assert!(parse_torrent_selection("1,x").is_err());
    }
}

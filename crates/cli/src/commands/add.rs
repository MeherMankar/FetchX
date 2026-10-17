use fetchx_core_services::CoreServices;
use fetchx_downloader::validate_http_source_url;
use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSource, EngineKind, FileName, Timestamp,
    parse_magnet_uri,
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

    let magnet = if opts.url.starts_with("magnet:") {
        match parse_magnet_uri(&opts.url) {
            Ok(info) => Some(info),
            Err(error) => {
                output::print_error(json, &format!("invalid magnet URI: {error}"));
                return exit::INVALID_ARGS;
            }
        }
    } else {
        None
    };
    let canonical_url = if magnet.is_some() {
        opts.url.clone()
    } else {
        match validate_http_source_url(&opts.url) {
            Ok(url) => url,
            Err(error) => {
                output::print_error(json, &format!("invalid URL: {error}"));
                return exit::INVALID_ARGS;
            }
        }
    };
    let parsed_url = if magnet.is_none() {
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

    let engine_kind = match resolve_engine(magnet.is_some(), opts.engine.as_deref()) {
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
        match svc
            .enqueue_magnet(&canonical_url, priority, opts.destination)
            .await
        {
            Ok(id) => id,
            Err(error) => {
                output::print_error(json, &format!("failed to enqueue magnet: {error}"));
                return exit::GENERAL_ERROR;
            }
        }
    } else {
        let filename = parsed_url
            .as_ref()
            .map(filename_for_url)
            .unwrap_or_else(|| "download".to_owned());
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
        let id = DownloadId::new();
        let source = match DownloadSource::new(canonical_url, engine_kind) {
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
        if let Err(error) = svc.queue().enqueue(item).await {
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
        "aria2" => Ok(EngineKind::Aria2),
        "yt-dlp" | "yt_dlp" | "ytdlp" => Ok(EngineKind::YtDlp),
        other => Err(format!(
            "unknown engine `{other}`. Valid: direct-http, aria2, yt-dlp"
        )),
    }
}

fn resolve_engine(is_magnet: bool, engine: Option<&str>) -> Result<EngineKind, String> {
    if is_magnet {
        if engine.is_none() {
            return Ok(EngineKind::Aria2);
        }
        return match parse_engine(engine) {
            Ok(EngineKind::Aria2) => Ok(EngineKind::Aria2),
            Ok(_) => Err("magnet downloads require the aria2 engine".to_owned()),
            Err(error) => Err(error),
        };
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

fn filename_for_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::resolve_engine;
    use fetchx_models::{EngineKind, parse_magnet_uri};

    #[test]
    fn magnet_defaults_to_aria2_and_suggests_a_confined_name() {
        assert_eq!(resolve_engine(true, None), Ok(EngineKind::Aria2));
        let metadata = parse_magnet_uri(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=..%5C..%5Cunsafe%3Aname%3F.torrent",
        )
        .expect("valid test magnet");
        assert_eq!(metadata.suggested_filename(), "unsafename.torrent");
        assert!(resolve_engine(true, Some("direct-http")).is_err());
    }
}

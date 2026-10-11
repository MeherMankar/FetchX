use mydownloader_core_services::CoreServices;
use mydownloader_downloader::validate_http_source_url;
use mydownloader_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSource, EngineKind, FileName, Timestamp,
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

    let canonical_url = match validate_http_source_url(&opts.url) {
        Ok(u) => u,
        Err(e) => {
            output::print_error(json, &format!("invalid URL: {e}"));
            return exit::INVALID_ARGS;
        }
    };

    let parsed_url = match Url::parse(&canonical_url) {
        Ok(u) => u,
        Err(e) => {
            output::print_error(json, &format!("cannot parse URL: {e}"));
            return exit::INVALID_ARGS;
        }
    };

    // ── Engine resolution ───────────────────────────────────────────────

    let engine_kind = match parse_engine(opts.engine.as_deref()) {
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
                      Check `mydownloader engines` or engine settings."
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

    // ── Build DownloadItem ──────────────────────────────────────────────

    let filename = filename_for_url(&parsed_url);
    let filename = match FileName::new(&filename) {
        Ok(f) => f,
        Err(e) => {
            output::print_error(json, &format!("invalid filename: {e}"));
            return exit::INVALID_ARGS;
        }
    };
    let destination = svc.downloads_dir().join(filename.as_str());
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

    // Apply non-default priority without touching the state machine.
    if priority != DownloadPriority::Normal {
        if let Err(e) = item.set_priority_at(priority, created_at) {
            output::print_error(json, &format!("priority error: {e}"));
            return exit::GENERAL_ERROR;
        }
    }

    // ── Enqueue ─────────────────────────────────────────────────────────

    if let Err(e) = svc.queue().enqueue(item).await {
        output::print_error(json, &format!("failed to enqueue: {e}"));
        return exit::GENERAL_ERROR;
    }

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

    // ── Success output ───────────────────────────────────────────────────

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

// ── Helpers ───────────────────────────────────────────────────────────────

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

//! Pause, resume, cancel, remove, retry — all delegate to QueueManager.

use fetchx_core_services::CoreServices;
use fetchx_downloader::validate_http_source_url;
use fetchx_models::{
    DownloadId, DownloadItem, DownloadSource, DownloadStatus, FileName, Timestamp,
};
use fetchx_queue::QueueError;
use reqwest::Url;

use crate::{
    args::QueueDirection,
    exit::{self, ExitCode},
    output,
};

// ── Pause ──────────────────────────────────────────────────────────────────

pub async fn pause(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let (id, _) = match resolve(&id_str, json) {
        Some(v) => v,
        None => return exit::INVALID_ARGS,
    };
    match svc.queue().pause(id).await {
        Ok(()) => success(json, id, "paused"),
        Err(e) => queue_error(json, &e),
    }
}

// ── Resume ────────────────────────────────────────────────────────────────

pub async fn resume(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let (id, _) = match resolve(&id_str, json) {
        Some(v) => v,
        None => return exit::INVALID_ARGS,
    };
    let result = async {
        // Short-lived CLI processes leave resumed work queued for a
        // long-running desktop/server worker instead of pausing it at exit.
        svc.queue().defer_download(id).await?;
        svc.queue().resume(id).await
    }
    .await;
    match result {
        Ok(()) => success(json, id, "resumed"),
        Err(e) => queue_error(json, &e),
    }
}

// ── Cancel ────────────────────────────────────────────────────────────────

pub async fn cancel(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let (id, _) = match resolve(&id_str, json) {
        Some(v) => v,
        None => return exit::INVALID_ARGS,
    };
    match svc.queue().cancel(id).await {
        Ok(()) => success(json, id, "cancelled"),
        Err(e) => queue_error(json, &e),
    }
}

// ── Remove ────────────────────────────────────────────────────────────────

pub async fn remove(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let (id, _) = match resolve(&id_str, json) {
        Some(v) => v,
        None => return exit::INVALID_ARGS,
    };
    match svc.queue().remove_download(id).await {
        Ok(()) => success(json, id, "removed"),
        Err(e) => queue_error(json, &e),
    }
}

// ── Retry ──────────────────────────────────────────────────────────────────

pub async fn retry(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let (id, item) = match resolve(&id_str, json) {
        Some(v) => v,
        None => return exit::INVALID_ARGS,
    };
    let Some(item) = item else {
        output::print_error(json, &format!("download {id} not found"));
        return exit::NOT_FOUND;
    };

    if !matches!(
        item.status(),
        DownloadStatus::Failed | DownloadStatus::Cancelled
    ) {
        output::print_error(
            json,
            &format!(
                "retry requires a failed or cancelled download (current: {:?})",
                item.status()
            ),
        );
        return exit::INVALID_STATE;
    }

    // Preserve original engine — same approach as desktop retry command.
    let original_engine = item.source().engine();
    if !svc.engine_runtime.is_available(original_engine) {
        output::print_error(
            json,
            &format!("{original_engine:?} engine is not available. Check `FetchX engines`."),
        );
        return exit::ENGINE_UNAVAILABLE;
    }

    let canonical_url = match validate_http_source_url(item.source().url()) {
        Ok(u) => u,
        Err(e) => {
            output::print_error(json, &format!("invalid URL in existing record: {e}"));
            return exit::INVALID_ARGS;
        }
    };
    let parsed_url = match Url::parse(&canonical_url) {
        Ok(u) => u,
        Err(e) => {
            output::print_error(json, &format!("{e}"));
            return exit::INVALID_ARGS;
        }
    };
    let filename = parsed_url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download");
    let filename = match FileName::new(filename) {
        Ok(f) => f,
        Err(e) => {
            output::print_error(json, &format!("{e}"));
            return exit::GENERAL_ERROR;
        }
    };
    let destination = svc.downloads_dir().join(filename.as_str());
    let new_id = DownloadId::new();
    let source = match DownloadSource::new(canonical_url, original_engine) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(json, &format!("{e}"));
            return exit::GENERAL_ERROR;
        }
    };
    let created_at = match Timestamp::now() {
        Ok(t) => t,
        Err(e) => {
            output::print_error(json, &format!("{e}"));
            return exit::GENERAL_ERROR;
        }
    };
    let new_item = DownloadItem::new(new_id, source, filename, destination, created_at);

    // Validate and construct the replacement before deleting the failed record.
    if let Err(e) = svc.queue().remove_download(id).await {
        output::print_error(json, &format!("could not remove old record: {e}"));
        return exit::GENERAL_ERROR;
    }

    if let Err(e) = svc.queue().enqueue_deferred(new_item).await {
        output::print_error(json, &format!("could not re-enqueue: {e}"));
        return exit::GENERAL_ERROR;
    }

    if json {
        output::print_json_ok(&serde_json::json!({
            "ok": true, "action": "retry",
            "old_id": id.to_string(), "new_id": new_id.to_string()
        }));
    } else {
        println!("Retried: new download {new_id}");
    }
    exit::SUCCESS
}

pub async fn move_in_queue(
    svc: &CoreServices,
    id_str: String,
    direction: QueueDirection,
    json: bool,
) -> ExitCode {
    let id = match super::parse_id(&id_str) {
        Ok(id) => id,
        Err(error) => {
            output::print_error(json, &error.to_string());
            return exit::INVALID_ARGS;
        }
    };
    let result = match direction {
        QueueDirection::Up => svc.queue().move_up(id).await,
        QueueDirection::Down => svc.queue().move_down(id).await,
    };
    match result {
        Ok(()) => {
            let direction = match direction {
                QueueDirection::Up => "up",
                QueueDirection::Down => "down",
            };
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true,
                    "id": id.to_string(),
                    "direction": direction,
                }));
            } else {
                println!("Moved download {id} {direction} in the queue.");
            }
            exit::SUCCESS
        }
        Err(error) => {
            output::print_error(json, &error.to_string());
            exit::GENERAL_ERROR
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Parse the ID and optionally fetch the item for commands that need it.
fn resolve(id_str: &str, json: bool) -> Option<(DownloadId, Option<DownloadItem>)> {
    match super::parse_id(id_str) {
        Ok(id) => Some((id, None)),
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            None
        }
    }
}

/// Overload: also fetch the item.
#[allow(dead_code)]
pub async fn resolve_with_item(
    svc: &CoreServices,
    id_str: &str,
    json: bool,
) -> Option<(DownloadId, DownloadItem)> {
    use fetchx_storage::DownloadRepository;
    let id = match super::parse_id(id_str) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return None;
        }
    };
    match svc.repository.get(id).await {
        Ok(Some(item)) => Some((id, item)),
        Ok(None) => {
            output::print_error(json, &format!("download {id} not found"));
            None
        }
        Err(e) => {
            output::print_error(json, &format!("storage error: {e}"));
            None
        }
    }
}

fn success(json: bool, id: DownloadId, action: &str) -> ExitCode {
    if json {
        output::print_json_ok(
            &serde_json::json!({ "ok": true, "action": action, "id": id.to_string() }),
        );
    } else {
        println!("{}: {id}", action.to_ascii_uppercase());
    }
    exit::SUCCESS
}

fn queue_error(json: bool, e: &QueueError) -> ExitCode {
    let (code, msg) = match e {
        QueueError::DownloadNotFound(_) => (exit::NOT_FOUND, format!("download not found: {e}")),
        QueueError::InvalidStatus { .. } | QueueError::UnsupportedEngineOperation { .. } => (
            exit::INVALID_STATE,
            format!("invalid state for this operation: {e}"),
        ),
        _ => (exit::GENERAL_ERROR, format!("operation failed: {e}")),
    };
    output::print_error(json, &msg);
    code
}

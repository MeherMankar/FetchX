//! Background clipboard monitor — polls the clipboard every ~1.5 s and
//! emits a Tauri event when new HTTP/HTTPS URLs are detected.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};

use fetchx_core_services::CoreServices;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::commands::AppState;
use crate::events::CLIPBOARD_URLS_DETECTED;

const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const SETTING_KEY: &str = "clipboard_monitor_enabled";

#[derive(Clone, Debug, Serialize)]
pub struct ClipboardUrlsEvent {
    pub urls: Vec<String>,
}

/// Spawns the clipboard monitor background task.
/// Only runs when clipboard_monitor_enabled is true in pp_config.
pub fn spawn_clipboard_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_hash: u64 = 0;

        loop {
            sleep(POLL_INTERVAL).await;

            // Check if enabled.
            let state = app.state::<AppState>();
            let enabled = match state.config_service.load().await {
                Ok(cfg) => cfg.clipboard_monitor_enabled,
                Err(_) => false,
            };
            if !enabled {
                continue;
            }

            // Poll clipboard text.
            let text = match tauri::async_runtime::spawn_blocking(|| {
                let mut clip = arboard::Clipboard::new().ok()?;
                clip.get_text().ok()
            })
            .await
            {
                Ok(Some(text)) => text,
                Ok(None) => continue,
                Err(_) => continue,
            };

            // Check if content changed.
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            let hash = hasher.finish();
            if hash == last_hash {
                continue;
            }
            last_hash = hash;

            // Extract URLs.
            let urls = extract_http_urls(&text);
            if urls.is_empty() {
                continue;
            }

            debug!(count = urls.len(), "clipboard URLs detected");
            let _ = app.emit(CLIPBOARD_URLS_DETECTED, ClipboardUrlsEvent { urls });
        }
    });
}

fn extract_http_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| {
            (word.starts_with("http://") || word.starts_with("https://"))
                && word.len() <= 8192
        })
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .take(20)
        .collect()
}

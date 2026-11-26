use std::sync::{Arc, atomic::Ordering};

use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_queue::{DownloadEvent, DownloadEventBus};
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tracing_subscriber::EnvFilter;

static NOTIFICATION_ID_COUNTER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

use crate::browser_ipc::BrowserIpcServer;
use crate::clipboard_monitor::spawn_clipboard_monitor;
use crate::commands::{self, AppState};
use crate::engine_commands;
use crate::events::DownloadUpdatedEvent;
use crate::media_processing::{self, MediaProcessingManager};

const SHUTDOWN_RUNNING: u8 = 0;
const SHUTDOWN_IN_PROGRESS: u8 = 1;
const SHUTDOWN_COMPLETE: u8 = 2;

impl AppState {
    async fn initialize(
        database_path: std::path::PathBuf,
        downloads_dir: std::path::PathBuf,
        bundled_engines_dir: std::path::PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime_dir = database_path
            .parent()
            .map(|p| p.join("runtime"))
            .unwrap_or_else(|| std::env::temp_dir().join("fetchx-runtime"));
        let services = CoreServices::start(CoreServicesConfig {
            database_path,
            downloads_dir,
            runtime_dir,
            bundled_engines_dir: Some(bundled_engines_dir),
            defer_queued_on_start: false,
        })
        .await?;
        Ok(AppState::from_core_services(services))
    }
}

/// Spawns a background task that reads from the event bus and forwards each
/// `DownloadEvent` to the Tauri frontend as a typed application event.
pub fn spawn_event_bridge(app: tauri::AppHandle, event_bus: &DownloadEventBus) {
    let mut rx = event_bus.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => forward_event(&app, event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        skipped = n,
                        "event bridge lagged; frontend may have missed events and should refresh"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        tracing::debug!("event bridge task exited cleanly");
    });
}

async fn forward_event(app: &tauri::AppHandle, event: DownloadEvent) {
    match event {
        DownloadEvent::Created { item, .. } | DownloadEvent::Updated { item, .. } => {
            let status = item.status();
            let fname = item.filename().as_str().to_owned();

            let config = app.state::<AppState>().config_service.load().await.ok();
            let post_download_action = config
                .as_ref()
                .map(|c| c.default_post_download_action.clone())
                .unwrap_or_default();

            if status == fetchx_models::DownloadStatus::Completed {
                emit_notification(app, &format!("Download complete: {fname}"), "");
                let dest = item.destination().to_owned();
                let parent_dir = dest.parent().map(|p| p.to_owned());
                let notif_id =
                    NOTIFICATION_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let show_result = app
                    .notification()
                    .builder()
                    .id(notif_id)
                    .title("Download complete")
                    .body(fname.clone())
                    .action_type_id("fetchx_download_complete")
                    .show();
                if let Err(error) = show_result {
                    tracing::warn!(%error, "could not show OS notification");
                }
                if let Some(dir) = parent_dir
                    && let Some(state) = app.try_state::<crate::PendingNotificationPaths>()
                {
                    state.insert(notif_id, dir);
                }
                // Execute the configured post-download action.
                let dest = item.destination().to_owned();
                match post_download_action {
                    fetchx_models::PostDownloadAction::Nothing => {}
                    fetchx_models::PostDownloadAction::OpenFile => {
                        if let Err(error) = open::that(&dest) {
                            tracing::warn!(%error, path = %dest.display(), "post-download: could not open file");
                        }
                    }
                    fetchx_models::PostDownloadAction::OpenFolder => {
                        let folder = dest.parent().unwrap_or(&dest);
                        if let Err(error) = open::that(folder) {
                            tracing::warn!(%error, path = %folder.display(), "post-download: could not open folder");
                        }
                    }
                    fetchx_models::PostDownloadAction::RunCommand { command } => {
                        let status_str = "completed";
                        let size_str = item
                            .progress()
                            .total_bytes()
                            .map(|b| b.to_string())
                            .unwrap_or_default();
                        let filename_str = dest
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let id_str = item.id().to_string();
                        let engine_str = format!("{:?}", item.source().engine())
                            .to_lowercase()
                            .replace(' ', "_");
                        let url_str = item.source().url().to_owned();
                        #[cfg(windows)]
                        let spawn_result = tokio::process::Command::new("cmd")
                            .args(["/C", &command])
                            .env("FETCHX_FILE", &dest)
                            .env("FETCHX_URL", &url_str)
                            .env("FETCHX_STATUS", status_str)
                            .env("FETCHX_SIZE", &size_str)
                            .env("FETCHX_FILENAME", &filename_str)
                            .env("FETCHX_ID", &id_str)
                            .env("FETCHX_ENGINE", &engine_str)
                            .spawn();
                        #[cfg(not(windows))]
                        let spawn_result = tokio::process::Command::new("sh")
                            .args(["-c", &command])
                            .env("FETCHX_FILE", &dest)
                            .env("FETCHX_URL", &url_str)
                            .env("FETCHX_STATUS", status_str)
                            .env("FETCHX_SIZE", &size_str)
                            .env("FETCHX_FILENAME", &filename_str)
                            .env("FETCHX_ID", &id_str)
                            .env("FETCHX_ENGINE", &engine_str)
                            .spawn();
                        if let Err(error) = spawn_result {
                            tracing::warn!(%error, %command, "post-download: could not spawn command");
                        }
                    }
                    fetchx_models::PostDownloadAction::ExtractArchive { delete_after } => {
                        let archive_path = dest.clone();
                        tauri::async_runtime::spawn(async move {
                            match extract_archive(&archive_path, delete_after).await {
                                Ok(()) => {
                                    tracing::info!(path = %archive_path.display(), "post-download: archive extracted");
                                }
                                Err(error) => {
                                    tracing::warn!(%error, path = %archive_path.display(), "post-download: archive extraction failed");
                                }
                            }
                        });
                    }
                }

                // Automatic folder categorisation
                let folder_categories = config
                    .as_ref()
                    .map(|c| c.folder_categories.clone())
                    .unwrap_or_default();
                if folder_categories.enabled {
                    let src = item.destination().to_owned();
                    if let Some(dest) = fetchx_core_services::folder_categories::categorise(
                        &src,
                        &folder_categories,
                    ) {
                        let result = async {
                            if let Some(parent) = dest.parent() {
                                tokio::fs::create_dir_all(parent).await?;
                            }
                            tokio::fs::rename(&src, &dest).await
                        }
                        .await;
                        match result {
                            Ok(()) => {
                                tracing::info!(from = %src.display(), to = %dest.display(), "folder categorisation: moved file")
                            }
                            Err(error) => {
                                tracing::warn!(%error, from = %src.display(), to = %dest.display(), "folder categorisation: could not move file")
                            }
                        }
                    }
                }
                // Health check: verify the file exists and has the expected size.
                let health_check_enabled = config
                    .as_ref()
                    .map(|c| c.health_check_on_complete)
                    .unwrap_or(true);
                if health_check_enabled {
                    let dest = item.destination().to_owned();
                    let total_bytes = item.progress().total_bytes();
                    match tokio::fs::metadata(&dest).await {
                        Err(e) => {
                            tracing::warn!(
                                path = %dest.display(),
                                error = %e,
                                "health-check: completed download file is missing"
                            );
                            let _ = app.emit(
                                "fetchx-notification",
                                serde_json::json!({
                                    "title": "Download health warning",
                                    "body": format!("File not found after download: {}", fname)
                                }),
                            );
                        }
                        Ok(meta) => {
                            if let Some(expected) = total_bytes {
                                let actual = meta.len();
                                // Warn if size differs by more than 1%.
                                let diff = actual.abs_diff(expected);
                                if expected > 0 && diff * 100 / expected > 1 {
                                    tracing::warn!(
                                        path = %dest.display(),
                                        expected_bytes = expected,
                                        actual_bytes = actual,
                                        "health-check: completed file size differs from expected"
                                    );
                                }
                            }
                        }
                    }
                }
                // Checksum auto-verification on completion.
                if health_check_enabled {
                    let dest = item.destination().to_owned();
                    let fname_for_checksum = fname.clone();
                    let app_for_checksum = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let sidecar = {
                            let mut s = dest.as_os_str().to_owned();
                            s.push(".fetchx.sha256");
                            std::path::PathBuf::from(s)
                        };
                        if sidecar.exists() {
                            match tokio::fs::read_to_string(&sidecar).await {
                                Ok(expected_hex) => {
                                    let expected_hex = expected_hex.trim().to_ascii_lowercase();
                                    match tokio::fs::read(&dest).await {
                                        Ok(bytes) => {
                                            let hash = sha256(&bytes);
                                            let actual_hex: String = hash.iter().fold(
                                                String::with_capacity(64),
                                                |mut s, b| {
                                                    use std::fmt::Write as _;
                                                    let _ = write!(s, "{b:02x}");
                                                    s
                                                },
                                            );
                                            if actual_hex != expected_hex {
                                                tracing::warn!(
                                                    path = %dest.display(),
                                                    expected = %expected_hex,
                                                    actual = %actual_hex,
                                                    "checksum: SHA-256 mismatch after download completed"
                                                );
                                                let _ = app_for_checksum.emit(
                                                    "fetchx-notification",
                                                    serde_json::json!({
                                                        "title": "Checksum mismatch",
                                                        "body": format!("SHA-256 did not match for: {fname_for_checksum}")
                                                    }),
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                path = %dest.display(),
                                                error = %e,
                                                "checksum: could not read file for verification"
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        path = %sidecar.display(),
                                        error = %e,
                                        "checksum: could not read sidecar file"
                                    );
                                }
                            }
                        }
                    });
                }
                // Virus scan on completion.
                let virus_scan_enabled = config
                    .as_ref()
                    .map(|c| c.virus_scan_on_complete)
                    .unwrap_or(false);
                if virus_scan_enabled {
                    let scan_path = item.destination().to_owned();
                    let app_for_scan = app.clone();
                    let fname_for_scan = fname.clone();
                    tauri::async_runtime::spawn(async move {
                        spawn_virus_scan(scan_path, fname_for_scan, app_for_scan).await;
                    });
                }
                // Mark-of-the-Web tagging on completion.
                let motw_enabled = config.as_ref().map(|c| c.mark_of_the_web).unwrap_or(true);
                if motw_enabled {
                    let motw_path = item.destination().to_owned();
                    let motw_url = item.source().url().to_owned();
                    tauri::async_runtime::spawn_blocking(move || {
                        mark_of_the_web(&motw_path, &motw_url);
                    });
                }
            } else if status == fetchx_models::DownloadStatus::Failed {
                let err = item
                    .error()
                    .map(|e| e.message().to_owned())
                    .unwrap_or_default();
                emit_notification(app, &format!("Download failed: {fname}"), &err);
                let body = if err.is_empty() {
                    fname.clone()
                } else {
                    format!("{fname}: {err}")
                };
                let show_result = app
                    .notification()
                    .builder()
                    .title("Download failed")
                    .body(body)
                    .show();
                if let Err(error) = show_result {
                    tracing::warn!(%error, "could not show OS failure notification");
                }
                // Auto-retry on failure.
                {
                    let download_id = item.id();
                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<AppState>();
                        let config = state.config_service.load().await.ok();
                        let auto_retry_count =
                            config.as_ref().map(|c| c.auto_retry_count).unwrap_or(3);
                        let delay_seconds = config
                            .as_ref()
                            .map(|c| c.auto_retry_delay_seconds)
                            .unwrap_or(30);
                        if auto_retry_count == 0 {
                            return;
                        }
                        // Check how many times we have already retried by counting
                        // consecutive failures. We approximate by looking at the
                        // current error state: if the item is still Failed after a
                        // brief delay we retry up to auto_retry_count times.
                        // We use a simple per-process counter stored in the app state
                        // via the shared retry_counts map.
                        let count = {
                            let cs = app_handle.state::<AppState>();
                            if let Some(ref core) = cs.core_services {
                                let mut map = core
                                    .retry_counts
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                let entry = map.entry(download_id).or_insert(0);
                                *entry += 1;
                                *entry
                            } else {
                                1
                            }
                        };
                        if count > auto_retry_count {
                            tracing::info!(
                                download_id = %download_id,
                                count,
                                "auto-retry: exhausted retry limit"
                            );
                            return;
                        }
                        tracing::info!(
                            download_id = %download_id,
                            attempt = count,
                            limit = auto_retry_count,
                            delay_seconds,
                            "auto-retry: scheduling retry"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(delay_seconds)).await;
                        let cs = app_handle.state::<AppState>();
                        match cs.queue.retry_download(download_id).await {
                            Ok(()) => {
                                tracing::info!(download_id = %download_id, attempt = count, "auto-retry: queued");
                            }
                            Err(e) => {
                                tracing::warn!(download_id = %download_id, error = %e, "auto-retry: retry failed");
                            }
                        }
                    });
                }
            }
            // Webhook notification
            if matches!(
                status,
                fetchx_models::DownloadStatus::Completed | fetchx_models::DownloadStatus::Failed
            ) {
                let webhook_config = config
                    .as_ref()
                    .map(|c| c.webhook.clone())
                    .unwrap_or_default();
                if webhook_config.enabled
                    && !webhook_config.url.is_empty()
                    && ((status == fetchx_models::DownloadStatus::Completed
                        && webhook_config.on_complete)
                        || (status == fetchx_models::DownloadStatus::Failed
                            && webhook_config.on_fail))
                {
                    let event_str = if status == fetchx_models::DownloadStatus::Completed {
                        "completed"
                    } else {
                        "failed"
                    };
                    let webhook_url = webhook_config.url.clone();
                    let secret = webhook_config.secret.clone();
                    let timestamp_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let body = serde_json::json!({
                        "event": event_str,
                        "download": {
                            "id": item.id().to_string(),
                            "filename": item.filename().as_str(),
                            "url": item.source().url(),
                            "destination": item.destination().to_string_lossy(),
                            "size_bytes": item.progress().total_bytes(),
                        },
                        "timestamp": timestamp_ms,
                    });
                    tauri::async_runtime::spawn(async move {
                        let body_bytes = body.to_string();
                        let client = reqwest::Client::new();
                        let mut req = client
                            .post(&webhook_url)
                            .header("Content-Type", "application/json")
                            .body(body_bytes.clone());
                        if !secret.is_empty() {
                            use std::fmt::Write as _;
                            let key = hmac_sha256_key(secret.as_bytes());
                            let sig_bytes = hmac_sha256_sign(&key, body_bytes.as_bytes());
                            let mut hex_sig = String::with_capacity(sig_bytes.len() * 2);
                            for byte in &sig_bytes {
                                let _ = write!(hex_sig, "{byte:02x}");
                            }
                            req = req.header("X-FetchX-Signature", format!("sha256={hex_sig}"));
                        }
                        match req.send().await {
                            Ok(resp) => {
                                tracing::info!(status = %resp.status(), url = %webhook_url, "webhook: fired");
                            }
                            Err(error) => {
                                tracing::warn!(%error, url = %webhook_url, "webhook: request failed");
                            }
                        }
                    });
                }
            }
            let payload = DownloadUpdatedEvent {
                download: commands::DownloadDto::from(item),
            };
            if let Err(error) = app.emit(crate::events::DOWNLOAD_UPDATED, payload) {
                tracing::warn!(%error, event = crate::events::DOWNLOAD_UPDATED, "could not emit desktop event");
            }
        }
        DownloadEvent::Removed { id, .. } => {
            let payload = crate::events::DownloadRemovedEvent {
                download_id: id.to_string(),
            };
            if let Err(error) = app.emit(crate::events::DOWNLOAD_REMOVED, payload) {
                tracing::warn!(%error, event = crate::events::DOWNLOAD_REMOVED, "could not emit desktop event");
            }
        }
        DownloadEvent::QueueStats {
            speed_bps,
            active_count,
            queued_count,
            estimated_completion_seconds,
            ..
        } => {
            let payload = crate::events::QueueStatsEvent {
                speed_bps,
                active_count,
                queued_count,
                estimated_completion_seconds,
            };
            if let Err(error) = app.emit(crate::events::QUEUE_STATS_UPDATED, payload) {
                tracing::warn!(%error, "could not emit queue stats event");
            }
            update_tray(app, active_count, speed_bps);
        }
        DownloadEvent::QueueIdle => {
            if let Err(error) = app.emit(crate::events::QUEUE_IDLE, ()) {
                tracing::warn!(%error, "could not emit queue idle event");
            }
            update_tray(app, 0, 0);
        }
    }
}

fn emit_notification(app: &tauri::AppHandle, title: &str, body: &str) {
    #[derive(Clone, serde::Serialize)]
    struct NotificationPayload {
        title: String,
        body: String,
    }
    let _ = app.emit(
        "fetchx-notification",
        NotificationPayload {
            title: title.to_owned(),
            body: body.to_owned(),
        },
    );
}

fn update_tray(app: &tauri::AppHandle, active_count: u32, speed_bps: u64) {
    if let Some(tray) = app.tray_by_id("fetchx-tray") {
        let speed_str = if speed_bps > 1024 * 1024 {
            format!("{:.1} MB/s", speed_bps as f64 / 1_048_576.0)
        } else if speed_bps > 1024 {
            format!("{:.0} KB/s", speed_bps as f64 / 1024.0)
        } else {
            "0 KB/s".to_owned()
        };
        let tooltip = if active_count > 0 {
            format!("FetchX â {active_count} active Â· {speed_str}")
        } else {
            "FetchX â idle".to_owned()
        };
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

// ── HMAC-SHA256 helpers (no external crate dependency) ───────────────────────

fn hmac_sha256_key(key: &[u8]) -> [u8; 64] {
    // Pad or hash the key to the SHA-256 block size (64 bytes).
    let mut k = [0u8; 64];
    if key.len() <= 64 {
        k[..key.len()].copy_from_slice(key);
    } else {
        let h = sha256(key);
        k[..32].copy_from_slice(&h);
    }
    k
}

fn hmac_sha256_sign(key_block: &[u8; 64], message: &[u8]) -> [u8; 32] {
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = key_block[i] ^ 0x36;
        opad[i] = key_block[i] ^ 0x5c;
    }
    let mut inner = ipad.to_vec();
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);
    let mut outer = opad.to_vec();
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

/// Minimal pure-Rust SHA-256 implementation (RFC 6234 / FIPS 180-4).
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // Pre-processing: padding.
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] =
            [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ── Virus scan ────────────────────────────────────────────────────────────────

/// Runs a virus scan on a completed download file.
/// Windows: MpCmdRun.exe -Scan -ScanType 3 -File <path>
/// Linux:   clamscan (skipped if not found)
/// macOS:   no-op
async fn spawn_virus_scan(path: std::path::PathBuf, filename: String, app: tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    {
        // Windows Defender is located in a versioned subdirectory under Program Files.
        // Try the most common location; fall back to searching PATH.
        let defender_paths = [
            std::path::PathBuf::from(r"C:\Program Files\Windows Defender\MpCmdRun.exe"),
            std::path::PathBuf::from(r"C:\Program Files (x86)\Windows Defender\MpCmdRun.exe"),
        ];
        let exe = defender_paths
            .iter()
            .find(|p| p.exists())
            .cloned()
            .unwrap_or_else(|| std::path::PathBuf::from("MpCmdRun.exe"));

        tracing::info!(path = %path.display(), scanner = %exe.display(), "virus-scan: starting Windows Defender scan");
        match tokio::process::Command::new(&exe)
            .args([
                "-Scan",
                "-ScanType",
                "3",
                "-File",
                path.to_str().unwrap_or(""),
            ])
            .output()
            .await
        {
            Ok(output) => {
                let clean = output.status.success();
                tracing::info!(
                    path = %path.display(),
                    exit_code = output.status.code(),
                    clean,
                    "virus-scan: Windows Defender finished"
                );
                if !clean {
                    let _ = app.emit(
                        "fetchx-notification",
                        serde_json::json!({
                            "title": "Virus scan warning",
                            "body": format!("Windows Defender flagged: {filename}")
                        }),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "virus-scan: could not launch Windows Defender");
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // clamscan is optional — skip silently if not installed.
        match tokio::process::Command::new("clamscan")
            .arg("--no-summary")
            .arg(path.to_str().unwrap_or(""))
            .output()
            .await
        {
            Ok(output) => {
                let clean = output.status.success();
                tracing::info!(
                    path = %path.display(),
                    exit_code = output.status.code(),
                    clean,
                    "virus-scan: clamscan finished"
                );
                if !clean {
                    let _ = app.emit(
                        "fetchx-notification",
                        serde_json::json!({
                            "title": "Virus scan warning",
                            "body": format!("clamscan flagged: {filename}")
                        }),
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("virus-scan: clamscan not found, skipping");
            }
            Err(e) => {
                tracing::warn!(error = %e, "virus-scan: could not launch clamscan");
            }
        }
    }

    // macOS: no built-in CLI scanner — skip.
    #[cfg(target_os = "macos")]
    {
        let _ = (path, filename, app);
    }

    // Other targets: no-op.
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, filename, app);
    }
}

// ── Mark-of-the-Web ────────────────────────────────────────────────────────────

/// Tags a completed download with a Mark-of-the-Web so the OS and security
/// tools know the file was downloaded from the internet.
///
/// * **Windows** — writes an NTFS Alternate Data Stream `{path}:Zone.Identifier`
///   containing a `[ZoneTransfer]` section with `ZoneId=3` and the source URL.
/// * **macOS** — sets the `com.apple.quarantine` extended attribute via `xattr`.
/// * **Other** — no-op (no standard mechanism exists).
///
/// Failures are logged at WARN level and are never fatal.
fn mark_of_the_web(path: &std::path::Path, url: &str) {
    #[cfg(target_os = "windows")]
    {
        // The `:Zone.Identifier` suffix is an NTFS Alternate Data Stream.
        let ads_path = {
            let mut s = path.as_os_str().to_owned();
            s.push(":Zone.Identifier");
            std::path::PathBuf::from(s)
        };
        let content = format!(
            "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl={url}\r\nHostUrl={url}\r\n",
            url = url
        );
        if let Err(e) = std::fs::write(&ads_path, content.as_bytes()) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "motw: could not write Zone.Identifier ADS"
            );
        } else {
            tracing::info!(path = %path.display(), "motw: Zone.Identifier written");
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Build the quarantine flag string: "0083;<hex-epoch>;<appname>;"
        let epoch_hex = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| format!("{:x}", d.as_secs()))
            .unwrap_or_else(|_| "0".to_owned());
        let quarantine_value = format!("0083;{epoch_hex};FetchX;");
        let result = std::process::Command::new("xattr")
            .args([
                "-w",
                "com.apple.quarantine",
                &quarantine_value,
                path.to_str().unwrap_or(""),
            ])
            .output();
        match result {
            Ok(output) if output.status.success() => {
                tracing::info!(path = %path.display(), "motw: com.apple.quarantine set");
            }
            Ok(output) => {
                tracing::warn!(
                    path = %path.display(),
                    stderr = %String::from_utf8_lossy(&output.stderr),
                    "motw: xattr exited with non-zero status"
                );
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "motw: could not run xattr");
            }
        }
        let _ = url; // URL not embedded in quarantine attribute on macOS
    }

    // Linux and all other platforms: no standard MotW mechanism.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (path, url);
    }
}

// ── Archive extraction ────────────────────────────────────────────────────────

/// Extracts a `.zip`, `.tar.gz`, or `.tgz` archive next to the file.
/// The output directory is `{stem}/` adjacent to the archive.
/// Failures are non-fatal: the caller logs the returned error.
async fn extract_archive(path: &std::path::Path, delete_after: bool) -> Result<(), String> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || extract_archive_blocking(&path, delete_after))
        .await
        .map_err(|e| e.to_string())?
}

fn extract_archive_blocking(path: &std::path::Path, delete_after: bool) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();

    // Determine archive type.
    let is_zip = ext == "zip";
    let is_tar_gz = ext == "gz" || ext == "tgz";

    // Check for .tar.gz by looking at the full filename for double extension.
    let fname = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("");
    let is_tar_gz = is_tar_gz
        || fname.ends_with(".tar.gz")
        || fname.ends_with(".tar.bz2")
        || fname.ends_with(".tar.xz");

    if !is_zip && !is_tar_gz {
        return Err(format!("unsupported archive format: {}", path.display()));
    }

    // Build the output directory: same parent, stem-named subdirectory.
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let stem = if fname.ends_with(".tar.gz") {
        fname.trim_end_matches(".tar.gz")
    } else if fname.ends_with(".tar.bz2") {
        fname.trim_end_matches(".tar.bz2")
    } else if fname.ends_with(".tar.xz") {
        fname.trim_end_matches(".tar.xz")
    } else {
        path.file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("extracted")
    };
    let out_dir = parent.join(stem);

    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("could not create output directory: {e}"))?;

    if is_zip {
        let file = std::fs::File::open(path).map_err(|e| format!("could not open archive: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("could not read zip: {e}"))?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("zip entry error: {e}"))?;
            // Strip the top-level directory component if the zip has one.
            let raw_name = entry.name().to_owned();
            let rel = std::path::Path::new(&raw_name);
            let dest = out_dir.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&dest).map_err(|e| format!("could not create dir: {e}"))?;
            } else {
                if let Some(p) = dest.parent() {
                    std::fs::create_dir_all(p).map_err(|e| format!("could not create dir: {e}"))?;
                }
                let mut out_file = std::fs::File::create(&dest)
                    .map_err(|e| format!("could not create file: {e}"))?;
                std::io::copy(&mut entry, &mut out_file)
                    .map_err(|e| format!("could not write file: {e}"))?;
            }
        }
    } else {
        // tar.gz / tgz
        let file = std::fs::File::open(path).map_err(|e| format!("could not open archive: {e}"))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(&out_dir)
            .map_err(|e| format!("tar extraction failed: {e}"))?;
    }

    if delete_after {
        if let Err(e) = std::fs::remove_file(path) {
            tracing::warn!(path = %path.display(), error = %e, "post-download: could not delete archive after extraction");
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub app_data_dir: std::path::PathBuf,
    pub downloads_dir: std::path::PathBuf,
    pub bundled_engines_dir: std::path::PathBuf,
}

pub fn fallback_app_data_dir() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("com.fetchx.desktop"))
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(std::path::PathBuf::from)
                .map(|p| p.join("com.fetchx.desktop"))
        })
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(std::path::PathBuf::from)
                .map(|p| p.join(".fetchx"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|p| p.join(".fetchx"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("com.fetchx.desktop"))
}

pub fn fallback_downloads_dir(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("Downloads"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|p| p.join("Downloads"))
        })
        .unwrap_or_else(|| app_data_dir.join("downloads"))
}

pub fn fallback_bundled_engines_dir(app_data_dir: &std::path::Path) -> std::path::PathBuf {
    std::env::current_exe()
        .map(|p| {
            p.parent()
                .map(|d| d.join("engines"))
                .unwrap_or_else(|| p.join("engines"))
        })
        .or_else(|_| std::env::current_dir().map(|d| d.join("engines")))
        .unwrap_or_else(|_| app_data_dir.join("engines"))
}

pub fn resolve_app_paths(app: &tauri::App) -> AppPaths {
    // 1. App data directory:
    // On Windows, FOLDERID_RoamingAppData can fail or be unconfigured on some setups.
    // Standard Windows desktop apps store local data in %LOCALAPPDATA% (FOLDERID_LocalAppData).
    // Fall back progressively: LocalAppData -> AppData -> fallback_app_data_dir().
    let app_data_dir = app
        .path()
        .app_local_data_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| fallback_app_data_dir());

    if let Err(error) = std::fs::create_dir_all(&app_data_dir) {
        tracing::warn!(path = %app_data_dir.display(), %error, "could not create app data directory");
    }

    // 2. Downloads directory:
    // Prefer Tauri's download_dir(), falling back to USERPROFILE/Downloads, HOME/Downloads, or app_data_dir/downloads.
    let downloads_dir = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| fallback_downloads_dir(&app_data_dir));

    if let Err(error) = std::fs::create_dir_all(&downloads_dir) {
        tracing::warn!(path = %downloads_dir.display(), %error, "could not create downloads directory");
    }

    // 3. Bundled engines directory:
    // Prefer resource_dir()/engines, falling back to current_exe()/../engines, current_dir()/engines, or app_data_dir/engines.
    let bundled_engines_dir = app
        .path()
        .resource_dir()
        .map(|p| p.join("engines"))
        .unwrap_or_else(|_| fallback_bundled_engines_dir(&app_data_dir));

    AppPaths {
        app_data_dir,
        downloads_dir,
        bundled_engines_dir,
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::list_downloads,
            commands::list_runtime_statuses,
            commands::find_duplicate_download,
            commands::check_duplicate_url,
            commands::extract_download_links,
            commands::extract_links_from_text,
            commands::suggest_download_filename,
            commands::suggest_download_package_name,
            commands::normalize_download_url,
            commands::is_duplicate_download_url,
            commands::match_download_category,
            commands::add_download,
            commands::add_download_batch,
            commands::add_torrent_download,
            commands::add_media_download,
            commands::extract_media_formats,
            commands::discover_ytdlp_formats,
            media_processing::get_media_process_for_download,
            media_processing::start_media_remux,
            media_processing::start_media_merge,
            media_processing::cancel_media_process,
            commands::pause_download,
            commands::resume_download,
            commands::cancel_download,
            commands::delete_download,
            commands::set_download_priority,
            commands::move_download_up_in_queue,
            commands::move_download_down_in_queue,
            commands::schedule_download,
            commands::remove_download_schedule,
            commands::get_download_schedule,
            commands::retry_download,
            commands::update_download_url,
            commands::get_queue_snapshot,
            commands::pause_queue_dispatch,
            commands::resume_queue_dispatch,
            commands::pause_all_downloads,
            commands::resume_all_downloads,
            commands::get_torrent_metadata,
            commands::get_torrent_download_metadata,
            commands::select_torrent_files,
            commands::set_torrent_file_selected,
            commands::set_torrent_selection_all,
            commands::set_torrent_directory_selected,
            commands::inspect_torrent_file,
            commands::get_torrent_telemetry,
            commands::restart_aria2_http_with_native,
            commands::restart_aria2_torrent_with_native,
            commands::restart_aria2_torrent_file_with_native,
            commands::stop_torrent_seeding,
            commands::open_download_folder,
            engine_commands::get_engine_config,
            engine_commands::save_engine_config,
            engine_commands::reset_engine_config,
            engine_commands::test_ytdlp_config,
            engine_commands::get_engine_status,
            engine_commands::set_notification_config,
            commands::register_browser_extensions,
            commands::get_browser_extension_status,
            commands::open_browser_extension_install_page,
            commands::launch_browser_with_extension,
            commands::set_download_bandwidth_limit,
            commands::list_completed_downloads,
            commands::clear_download_history,
            commands::provide_download_credentials,
            commands::get_post_queue_action,
            commands::get_post_download_action,
            commands::set_post_download_action,
            commands::get_folder_categories_config,
            commands::set_folder_categories_config,
            commands::check_for_update,
            commands::export_settings,
            commands::import_settings,
            commands::get_webhook_config,
            commands::set_webhook_config,
            commands::verify_download_checksum,
        ])
        .setup(|app| {
            let paths = resolve_app_paths(app);
            let database_path = paths.app_data_dir.join("fetchx.db");
            let state = tauri::async_runtime::block_on(AppState::initialize(
                database_path,
                paths.downloads_dir,
                paths.bundled_engines_dir.clone(),
            ))?;
            if !app.manage(MediaProcessingManager::default()) {
                return Err("desktop media processing manager was already initialized".into());
            }
            spawn_event_bridge(app.handle().clone(), state.repository.event_bus());

            // Register the OS notification action type for completed downloads.
            {
                use tauri_plugin_notification::{Action, ActionType};
                let show_action = Action::builder("show_file", "Show file").build();
                let action_type = ActionType::builder("fetchx_download_complete")
                    .actions(vec![show_action])
                    .build();
                if let Err(error) = app
                    .notification()
                    .register_action_types(vec![action_type])
                {
                    tracing::warn!(%error, "could not register notification action types");
                }

                // Manage the shared path store before registering the handler.
                if !app.manage(crate::PendingNotificationPaths::default()) {
                    return Err("notification path store already initialized".into());
                }

                let app_handle_for_action = app.handle().clone();
                if let Err(error) = app.notification().on_action(move |action| {
                    if action.action_id() == "show_file"
                        && let Some(notification) = action.notification()
                    {
                        let notif_id = notification.id();
                        let paths = app_handle_for_action
                            .try_state::<crate::PendingNotificationPaths>();
                        let dir = paths.and_then(|p| p.get(notif_id));
                        if let Some(dir) = dir
                            && let Err(error) = open::that(&dir)
                        {
                            tracing::warn!(%error, path = %dir.display(), "could not open download directory");
                        }
                    }
                }) {
                    tracing::warn!(%error, "could not register notification action handler");
                }
            }

            // System tray
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder};
                use tauri::tray::TrayIconBuilder;
                let menu = MenuBuilder::new(app)
                    .item(&MenuItemBuilder::with_id("open", "Open FetchX").build(app)?)
                    .separator()
                    .item(&MenuItemBuilder::with_id("quit", "Quit").build(app)?)
                    .build()?;
                let _tray = TrayIconBuilder::with_id("fetchx-tray")
                    .tooltip("FetchX â idle")
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "open" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let tauri::tray::TrayIconEvent::Click { .. } = event {
                            let app = tray.app_handle();
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            // Spawn clipboard monitor (polls every 1.5s when enabled).
            spawn_clipboard_monitor(app.handle().clone());
            if !app.manage(state) {
                return Err("desktop application state was already initialized".into());
            }

            // Start the browser integration IPC server after state is managed.
            // The server borrows AppState via the Tauri handle so it shares the
            // same queue/engine runtime without duplication.
            // Failures are non-fatal: the desktop remains fully usable.
            {
                let app_handle = app.handle().clone();
                let data_dir = paths.app_data_dir.clone();
                tauri::async_runtime::spawn(async move {
                    let app_state = app_handle.state::<AppState>();
                    match BrowserIpcServer::start(&app_state, data_dir, env!("CARGO_PKG_VERSION"))
                        .await
                    {
                        Ok(srv) => {
                            tracing::info!(port = srv.port, "browser IPC server started");
                            let _ = app_handle.manage(srv);
                        }
                        Err(e) => {
                            tracing::warn!(%e, "browser IPC server failed to start");
                        }
                    }
                });
            }

            // Extract the extension files and register the native messaging host.
            // The browser extension itself must be installed through the browser UI
            // until FetchX is published in the browser stores.
            {
                let app_data_dir = paths.app_data_dir.clone();
                let bundled_engines_dir = paths.bundled_engines_dir.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    match crate::browser_extension_installer::ensure_registered(
                        &app_data_dir,
                        &bundled_engines_dir,
                    ) {
                        Ok(summary) => {
                            let native_host =
                                summary.native_host_binary.as_deref().unwrap_or("missing");
                            if summary.success {
                                tracing::info!(
                                    native_host,
                                    chromium_dir = %summary.chromium_extension_dir,
                                    firefox_dir = %summary.firefox_extension_dir,
                                    "browser extension files prepared and native host registered"
                                );
                            } else {
                                tracing::warn!(
                                    native_host,
                                    chromium_dir = %summary.chromium_extension_dir,
                                    firefox_dir = %summary.firefox_extension_dir,
                                    "browser extension integration is incomplete"
                                );
                            }
                        }
                        Err(err) => {
                            tracing::warn!(%err, "browser extension auto-registration failed");
                        }
                    }
                });
            }

            // Spawn a one-shot background update check.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    match crate::commands::check_for_update().await {
                        Ok(result) if result.available => {
                            let _ = app_handle.emit("fetchx-update-available", result);
                        }
                        _ => {}
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())?;

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let state = app_handle.state::<AppState>();
            match state.shutdown_state.compare_exchange(
                SHUTDOWN_RUNNING,
                SHUTDOWN_IN_PROGRESS,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    api.prevent_exit();
                    let core_services = state.core_services.clone();
                    let scheduler = state.scheduler.clone();
                    let engine_runtime = state.engine_runtime.clone();
                    let media_processing =
                        (*app_handle.state::<MediaProcessingManager>()).clone();
                    let app_handle = app_handle.clone();
                    let shutdown_state = Arc::clone(&state.shutdown_state);
                    tauri::async_runtime::spawn(async move {
                        media_processing.cancel_all_and_wait().await;
                        if let Some(core_services) = core_services {
                            if let Err(error) = core_services.shutdown().await {
                                tracing::error!(%error, "core services failed to shut down cleanly");
                            }
                        } else {
                            if let Err(error) = scheduler.shutdown().await {
                                tracing::error!(%error, "scheduler failed to shut down cleanly");
                            }
                            if let Err(error) = engine_runtime.shutdown().await {
                                tracing::error!(%error, "engine runtime failed to shut down cleanly");
                            }
                        }
                        // Shut down browser IPC server if it is running.
                        if let Some(browser_server) = app_handle.try_state::<BrowserIpcServer>() {
                            browser_server.shutdown().await;
                        }
                        shutdown_state.store(SHUTDOWN_COMPLETE, Ordering::Release);
                        app_handle.exit(0);
                    });
                }
                Err(SHUTDOWN_IN_PROGRESS) => api.prevent_exit(),
                Err(SHUTDOWN_COMPLETE) => {}
                Err(_) => {
                    api.prevent_exit();
                    tracing::error!("desktop shutdown entered an invalid state");
                }
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::AppState;

    #[test]
    fn fallback_paths_resolve_without_panicking() {
        let app_data = super::fallback_app_data_dir();
        assert!(!app_data.as_os_str().is_empty());
        assert!(app_data.to_string_lossy().contains("fetchx"));

        let downloads = super::fallback_downloads_dir(&app_data);
        assert!(!downloads.as_os_str().is_empty());

        let engines = super::fallback_bundled_engines_dir(&app_data);
        assert!(!engines.as_os_str().is_empty());
    }

    #[tokio::test]
    async fn desktop_state_initializes_through_core_services()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let state = AppState::initialize(
            root.path().join("fetchx.db"),
            root.path().join("downloads"),
            root.path().join("engines"),
        )
        .await?;

        let core_services = state
            .core_services
            .as_ref()
            .ok_or("CoreServices was not retained by desktop state")?;
        assert!(
            core_services
                .engine_runtime
                .is_available(fetchx_models::EngineKind::DirectHttp)
        );
        assert!(core_services.queue().snapshot().await?.items.is_empty());
        core_services.shutdown().await?;
        Ok(())
    }
}

use std::sync::{Arc, atomic::Ordering};

use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_queue::{DownloadEvent, DownloadEventBus};
use tauri::{Emitter, Manager, RunEvent};
use tracing_subscriber::EnvFilter;

use crate::browser_ipc::BrowserIpcServer;
use crate::commands::{self, AppState, DownloadDto};
use crate::engine_commands;
use crate::events::{
    DOWNLOAD_REMOVED, DOWNLOAD_UPDATED, DownloadRemovedEvent, DownloadUpdatedEvent,
};
use crate::helper_manager::{self, HelperManager};

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
                Ok(event) => forward_event(&app, event),
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

fn forward_event(app: &tauri::AppHandle, event: DownloadEvent) {
    match event {
        DownloadEvent::Created { item, .. } | DownloadEvent::Updated { item, .. } => {
            let payload = DownloadUpdatedEvent {
                download: DownloadDto::from(item),
            };
            if let Err(error) = app.emit(DOWNLOAD_UPDATED, payload) {
                tracing::warn!(%error, event = DOWNLOAD_UPDATED, "could not emit desktop event");
            }
        }
        DownloadEvent::Removed { id, .. } => {
            let payload = DownloadRemovedEvent {
                download_id: id.to_string(),
            };
            if let Err(error) = app.emit(DOWNLOAD_REMOVED, payload) {
                tracing::warn!(%error, event = DOWNLOAD_REMOVED, "could not emit desktop event");
            }
        }
    }
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
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::list_downloads,
            commands::add_download,
            commands::add_torrent_download,
            commands::add_media_download,
            commands::extract_media_formats,
            commands::pause_download,
            commands::resume_download,
            commands::cancel_download,
            commands::delete_download,
            commands::set_download_priority,
            commands::schedule_download,
            commands::remove_download_schedule,
            commands::get_download_schedule,
            commands::retry_download,
            commands::get_queue_snapshot,
            commands::get_torrent_metadata,
            commands::get_torrent_download_metadata,
            commands::select_torrent_files,
            commands::set_torrent_file_selected,
            commands::set_torrent_selection_all,
            commands::set_torrent_directory_selected,
            commands::inspect_torrent_file,
            commands::get_torrent_telemetry,
            commands::stop_torrent_seeding,
            engine_commands::get_engine_config,
            engine_commands::save_engine_config,
            engine_commands::reset_engine_config,
            engine_commands::test_aria2_config,
            engine_commands::test_ytdlp_config,
            engine_commands::get_engine_status,
            helper_manager::get_helper_status,
            helper_manager::install_helper,
            helper_manager::cancel_helper_install,
            commands::register_browser_extensions,
            commands::get_browser_extension_status,
            commands::open_browser_extension_install_page,
        ])
        .setup(|app| {
            let paths = resolve_app_paths(app);
            let database_path = paths.app_data_dir.join("fetchx.db");
            let helper_manager = HelperManager::new(
                paths.app_data_dir.join("bin"),
                paths.bundled_engines_dir.clone(),
            );
            let state = tauri::async_runtime::block_on(AppState::initialize(
                database_path,
                paths.downloads_dir,
                paths.bundled_engines_dir.clone(),
            ))?;
            if !app.manage(helper_manager) {
                return Err("desktop helper manager was already initialized".into());
            }
            spawn_event_bridge(app.handle().clone(), state.repository.event_bus());
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
                    let app_handle = app_handle.clone();
                    let shutdown_state = Arc::clone(&state.shutdown_state);
                    tauri::async_runtime::spawn(async move {
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

use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use fetchx_queue::{DownloadEvent, DownloadEventBus, EventRepository, QueueConfig};
use fetchx_scheduler::Scheduler;
use fetchx_storage::Database;
use tauri::{Emitter, Manager, RunEvent};
use tokio::fs;
use tracing_subscriber::EnvFilter;

use crate::browser_ipc::BrowserIpcServer;
use crate::commands::{self, AppState, DownloadDto};
use crate::config::ConfigService;
use crate::engine_commands;
use crate::engines::EngineRuntime;
use crate::events::{
    DOWNLOAD_REMOVED, DOWNLOAD_UPDATED, DownloadRemovedEvent, DownloadUpdatedEvent,
};

const SHUTDOWN_RUNNING: u8 = 0;
const SHUTDOWN_IN_PROGRESS: u8 = 1;
const SHUTDOWN_COMPLETE: u8 = 2;

impl AppState {
    async fn initialize(
        database_path: PathBuf,
        downloads_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(parent) = database_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::create_dir_all(&downloads_dir).await?;

        // 1. Database
        let database = Database::connect_path(database_path.clone()).await?;
        let inner_repository = database.downloads();

        // 2. Event bus wrapping the repository
        let event_bus = DownloadEventBus::new();
        let repository = EventRepository::new(inner_repository, event_bus);

        // 3. Load persisted engine configuration
        let config_service = ConfigService::new(database.config());
        let engine_config = config_service.load().await.unwrap_or_default();

        // 4. Initialize engines (optional engines degrade gracefully).
        // The runtime directory lives beside the database in the app-data directory.
        let runtime_dir = database_path
            .parent()
            .map(|p| p.join("runtime"))
            .unwrap_or_else(|| std::env::temp_dir().join("fetchx-runtime"));
        fs::create_dir_all(&runtime_dir).await?;
        let engine_runtime =
            EngineRuntime::initialize(repository.clone(), &engine_config, runtime_dir).await?;

        // 5. Queue + Scheduler using the dispatcher
        let scheduler = Scheduler::start(
            repository.clone(),
            engine_runtime.dispatcher.clone(),
            QueueConfig::default(),
        )
        .await?;
        let queue = scheduler.queue_manager();

        Ok(AppState::new(
            database,
            repository,
            scheduler,
            queue,
            downloads_dir,
            config_service,
            engine_runtime,
        ))
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

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init()
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::list_downloads,
            commands::add_download,
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
            engine_commands::get_engine_config,
            engine_commands::save_engine_config,
            engine_commands::reset_engine_config,
            engine_commands::test_aria2_config,
            engine_commands::test_ytdlp_config,
            engine_commands::get_engine_status,
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let downloads_dir = app.path().download_dir()?;
            let database_path = app_data_dir.join("fetchx.db");
            let state =
                tauri::async_runtime::block_on(AppState::initialize(database_path, downloads_dir))?;
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
                let data_dir = app_data_dir.clone();
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
                    let scheduler = state.scheduler.clone();
                    let engine_runtime = state.engine_runtime.clone();
                    let app_handle = app_handle.clone();
                    let shutdown_state = Arc::clone(&state.shutdown_state);
                    tauri::async_runtime::spawn(async move {
                        // Shut down scheduler + queue first, then optional engines.
                        if let Err(error) = scheduler.shutdown().await {
                            tracing::error!(%error, "scheduler failed to shut down cleanly");
                        }
                        engine_runtime.shutdown().await;
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


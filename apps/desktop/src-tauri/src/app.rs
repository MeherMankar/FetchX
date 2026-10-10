use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use mydownloader_downloader::DirectHttpEngine;
use mydownloader_queue::{DownloadEvent, DownloadEventBus, EventRepository, QueueConfig};
use mydownloader_scheduler::Scheduler;
use mydownloader_storage::Database;
use tauri::{Emitter, Manager, RunEvent};
use tokio::fs;
use tracing_subscriber::EnvFilter;

use crate::commands::{self, AppState, DownloadDto};
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

        let database = Database::connect_path(database_path).await?;
        let inner_repository = database.downloads();

        // Wrap the repository in the event bus so every persistence write
        // automatically publishes a DownloadEvent.
        let event_bus = DownloadEventBus::new();
        let repository = EventRepository::new(inner_repository, event_bus);

        let engine = Arc::new(DirectHttpEngine::new(repository.clone())?);

        let scheduler =
            Scheduler::start(repository.clone(), engine, QueueConfig::default()).await?;
        let queue = scheduler.queue_manager();

        Ok(AppState::new(
            database,
            repository,
            scheduler,
            queue,
            downloads_dir,
        ))
    }
}

/// Spawns a background task that reads from the event bus and forwards each
/// `DownloadEvent` to the Tauri frontend as a typed application event.
///
/// The task exits cleanly when the broadcast sender is dropped (i.e. when the
/// `EventRepository` is dropped during shutdown), so no explicit cancellation
/// signal is required.
pub fn spawn_event_bridge(app: tauri::AppHandle, event_bus: &DownloadEventBus) {
    let mut rx = event_bus.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    forward_event(&app, event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        skipped = n,
                        "event bridge lagged; frontend may have missed events and should refresh"
                    );
                    // Continue — the frontend will reconcile on the next event.
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // The event bus has been dropped (shutdown). Exit the bridge.
                    break;
                }
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
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let downloads_dir = app.path().download_dir()?;
            let database_path = app_data_dir.join("mydownloader.db");
            let state =
                tauri::async_runtime::block_on(AppState::initialize(database_path, downloads_dir))?;
            // Start the event bridge before managing state so events are never
            // lost between setup and the first frontend subscription.
            spawn_event_bridge(app.handle().clone(), state.repository.event_bus());
            if !app.manage(state) {
                return Err("desktop application state was already initialized".into());
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
                    let app_handle = app_handle.clone();
                    let shutdown_state = Arc::clone(&state.shutdown_state);
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = scheduler.shutdown().await {
                            tracing::error!(%error, "desktop services failed to shut down cleanly");
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

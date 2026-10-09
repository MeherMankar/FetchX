use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use mydownloader_downloader::DirectHttpEngine;
use mydownloader_queue::QueueConfig;
use mydownloader_scheduler::Scheduler;
use mydownloader_storage::Database;
use tauri::{Manager, RunEvent};
use tokio::fs;
use tracing_subscriber::EnvFilter;

use crate::commands::{self, AppState};

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
        let repository = database.downloads();
        let engine = Arc::new(DirectHttpEngine::new(repository.clone())?);

        let scheduler =
            Scheduler::start(repository.clone(), engine, QueueConfig::default()).await?;
        let queue = scheduler.queue_manager();

        Ok(Self::new(
            database,
            repository,
            scheduler,
            queue,
            downloads_dir,
        ))
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
            commands::delete_download
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let downloads_dir = app.path().download_dir()?;
            let database_path = app_data_dir.join("mydownloader.db");
            let state =
                tauri::async_runtime::block_on(AppState::initialize(database_path, downloads_dir))?;
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

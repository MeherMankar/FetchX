use std::{
    path::{Component, Path},
    sync::{Arc, atomic::AtomicU8},
};

use fetchx_core::StateMachineError;
use fetchx_downloader::validate_http_source_url;
use fetchx_models::{
    DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadSource, DownloadStatus,
    EngineKind, FileName, Timestamp,
};
use fetchx_queue::{EventRepository, QueueError, QueueManager};
use fetchx_scheduler::{ScheduleError, Scheduler};
use fetchx_storage::{Database, DownloadRepository, SqliteDownloadRepository};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::config::ConfigService;
use crate::engines::EngineRuntime;
use crate::events::{
    DOWNLOAD_REMOVED, DOWNLOAD_UPDATED, DownloadRemovedEvent, DownloadUpdatedEvent,
};

/// The concrete repository type used by the desktop application.
pub(crate) type AppRepository = EventRepository<SqliteDownloadRepository>;

pub struct AppState {
    _database: Database,
    pub(crate) repository: AppRepository,
    queue: QueueManager,
    pub(crate) scheduler: Scheduler<AppRepository>,
    downloads_dir: std::path::PathBuf,
    pub(crate) shutdown_state: Arc<AtomicU8>,
    pub(crate) config_service: ConfigService,
    /// Owns the optional engine supervisors and the dispatcher.
    pub(crate) engine_runtime: Arc<EngineRuntime>,
}

impl AppState {
    pub(crate) fn new(
        database: Database,
        repository: AppRepository,
        scheduler: Scheduler<AppRepository>,
        queue: QueueManager,
        downloads_dir: std::path::PathBuf,
        config_service: ConfigService,
        engine_runtime: EngineRuntime,
    ) -> Self {
        Self {
            _database: database,
            repository,
            queue,
            scheduler,
            downloads_dir,
            shutdown_state: Arc::new(AtomicU8::new(0)),
            config_service,
            engine_runtime: Arc::new(engine_runtime),
        }
    }

    async fn list(&self) -> Result<Vec<DownloadDto>, IpcError> {
        self.repository
            .list()
            .await
            .map(|items| items.into_iter().map(DownloadDto::from).collect())
            .map_err(|_| IpcError::internal())
    }

    async fn add(&self, request: AddDownloadRequest) -> Result<DownloadDto, IpcError> {
        let canonical_url =
            validate_http_source_url(&request.url).map_err(|_| IpcError::invalid_request())?;
        let parsed_url = Url::parse(&canonical_url).map_err(|_| IpcError::invalid_request())?;
        let filename = filename_for_url(&parsed_url);
        let filename = FileName::new(filename).map_err(|_| IpcError::invalid_request())?;
        let destination = self.downloads_dir.join(filename.as_str());
        let id = DownloadId::new();

        // Resolve the requested engine, defaulting to DirectHttp when unspecified.
        let engine_kind = request.engine_kind.unwrap_or(EngineKind::DirectHttp);

        // Backend validates availability — the frontend cannot bypass this check.
        // Direct HTTP is always available; optional engines must be running.
        if !self.engine_runtime.is_available(engine_kind) {
            return Err(IpcError::engine_unavailable(engine_kind));
        }

        let source = DownloadSource::new(canonical_url, engine_kind)
            .map_err(|_| IpcError::invalid_request())?;
        let created_at = Timestamp::now().map_err(|_| IpcError::internal())?;
        let item = DownloadItem::new(id, source, filename, destination, created_at);

        self.queue
            .enqueue(item)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    /// Adds a download from the browser integration IPC path.
    ///
    /// Shares exactly the same validation and queue path as the Tauri command,
    /// but is called from `browser_ipc` rather than through Tauri IPC.
    /// The browser may only supply a URL; engine, filename, and destination
    /// are determined by the application exactly as for the desktop UI.
    pub(crate) async fn add_from_browser(
        &self,
        request: AddDownloadRequest,
    ) -> Result<DownloadDto, IpcError> {
        self.add(request).await
    }

    /// Creates a lightweight clone suitable for the browser IPC service.
    ///
    /// `AppState` is not `Clone` because `_database` is not, but the browser
    /// IPC service only needs the queue, repository, engine_runtime, and
    /// downloads_dir — all of which are already Arc-backed or cheaply cloneable.
    pub(crate) fn clone_for_browser(&self) -> Self {
        Self {
            _database: self._database.clone(),
            repository: self.repository.clone(),
            queue: self.queue.clone(),
            scheduler: self.scheduler.clone(),
            downloads_dir: self.downloads_dir.clone(),
            shutdown_state: Arc::clone(&self.shutdown_state),
            config_service: self.config_service.clone(),
            engine_runtime: Arc::clone(&self.engine_runtime),
        }
    }

    async fn get(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.repository
            .get(id)
            .await
            .map_err(|_| IpcError::internal())?
            .map(DownloadDto::from)
            .ok_or_else(IpcError::not_found)
    }

    async fn pause(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.queue.pause(id).await.map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn resume(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.queue.resume(id).await.map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn cancel(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.queue.cancel(id).await.map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn delete(&self, id: DownloadId) -> Result<(), IpcError> {
        self.queue
            .remove_download(id)
            .await
            .map_err(IpcError::from_queue)
    }

    /// Retry a failed or cancelled download by transitioning it back to Queued.
    ///
    /// The existing `QueueManager::resume` only handles `Paused`.  Failed and
    /// Cancelled items need to be re-enqueued through their own path: the
    /// StateMachine allows `Failed|Cancelled → Preparing`, and QueueManager
    /// startup already reconciles `Preparing → Queued`.  The simplest safe
    /// approach is to transition through the state machine directly, then
    /// update the queue's in-memory entries by calling `enqueue` on the item
    /// after it has been moved to the queue tail (so it sorts after existing
    /// queued items).  We use `QueueManager::enqueue` after a state-machine
    /// reset.  Since enqueue persists the item, we first delete the record
    /// and re-create it via the state machine's own `recover_interrupted`
    /// path — but that requires the engine to be running.
    ///
    /// The cleaner and architecturally correct solution is to expose the
    /// queue's existing `resume` path for the two extra statuses, which would
    /// require a QueueManager change.  For Phase 15B we implement a targeted
    /// Tauri command that does the minimum: transitions the item back to
    /// `Queued` through the StateMachine and then calls `enqueue` on the
    /// existing repository record (the queue's `new_with_deferred` already
    /// loads persisted items at startup; we can't call it again, but we CAN
    /// use `enqueue` because `enqueue` calls `repository.create` which will
    /// fail on a duplicate).  Instead, the cleanest path available is:
    /// delete the record, then re-add it (preserving filename/URL) as a fresh
    /// queued download via the existing `add` path.  This is exactly what a
    /// "retry" means from the user's perspective: a fresh attempt.
    ///
    /// NOTE: This intentionally creates a new download ID. The old failed
    /// record is removed. Partial files and destination are preserved because
    /// `remove_download` preserves filesystem state.  The original engine
    /// kind is preserved and re-validated for current availability.
    async fn retry(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        // Load the existing item before removing it.
        let item = self
            .repository
            .get(id)
            .await
            .map_err(|_| IpcError::internal())?
            .ok_or_else(IpcError::not_found)?;

        // Only failed or cancelled downloads can be retried.
        if !matches!(
            item.status(),
            DownloadStatus::Failed | DownloadStatus::Cancelled
        ) {
            return Err(IpcError::conflict());
        }

        // Remove the old record (preserves partial files).
        self.queue
            .remove_download(id)
            .await
            .map_err(IpcError::from_queue)?;

        // Re-enqueue preserving the original engine kind.
        // The engine is re-validated for current availability so a retry
        // cannot silently switch engines.
        let original_engine = item.source().engine();
        if !self.engine_runtime.is_available(original_engine) {
            // The engine that was used originally is no longer available.
            // Reject rather than silently switching to a different engine.
            return Err(IpcError::engine_unavailable(original_engine));
        }

        let canonical_url = validate_http_source_url(item.source().url())
            .map_err(|_| IpcError::invalid_request())?;
        let parsed_url = Url::parse(&canonical_url).map_err(|_| IpcError::invalid_request())?;
        let filename = filename_for_url(&parsed_url);
        let filename = FileName::new(filename).map_err(|_| IpcError::invalid_request())?;
        let destination = self.downloads_dir.join(filename.as_str());
        let new_id = DownloadId::new();
        let source = DownloadSource::new(canonical_url, original_engine)
            .map_err(|_| IpcError::invalid_request())?;
        let created_at = Timestamp::now().map_err(|_| IpcError::internal())?;
        let new_item = DownloadItem::new(new_id, source, filename, destination, created_at);

        self.queue
            .enqueue(new_item)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(new_id).await
    }

    async fn snapshot(&self) -> Result<QueueSnapshotDto, IpcError> {
        let snap = self.queue.snapshot().await.map_err(IpcError::from_queue)?;

        let active_count = snap.active.len();
        let used_connections: usize = snap
            .active
            .iter()
            .map(|a| a.active_connections.max(a.allocated_connections).max(1))
            .sum();

        Ok(QueueSnapshotDto {
            active_count,
            queued_count: snap.queued.len(),
            paused_count: snap.paused.len(),
            used_connections,
            total_connections: snap.global_connection_limit,
        })
    }

    async fn set_priority(
        &self,
        id: DownloadId,
        priority: DownloadPriority,
    ) -> Result<DownloadDto, IpcError> {
        self.queue
            .set_priority(id, priority)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn schedule(&self, id: DownloadId, scheduled_at_ms: u64) -> Result<(), IpcError> {
        let scheduled_at = Timestamp::from_unix_millis(scheduled_at_ms);
        self.scheduler
            .schedule_download(id, scheduled_at)
            .await
            .map_err(IpcError::from_schedule)
    }

    async fn remove_schedule(&self, id: DownloadId) -> Result<(), IpcError> {
        self.scheduler
            .remove_schedule(id)
            .await
            .map_err(IpcError::from_schedule)
    }

    async fn get_schedule(&self, id: DownloadId) -> Result<Option<ScheduleDto>, IpcError> {
        self.scheduler
            .get_schedule(id)
            .await
            .map_err(IpcError::from_schedule)
            .map(|opt| {
                opt.map(|schedule| ScheduleDto {
                    download_id: schedule.download_id().to_string(),
                    scheduled_at_ms: schedule.scheduled_at().as_unix_millis(),
                    enabled: schedule.enabled(),
                })
            })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddDownloadRequest {
    pub url: String,
    /// Optional engine selection.  `None` (or absent from JSON) means Direct HTTP.
    /// The backend validates availability; the frontend cannot bypass this check.
    #[serde(default)]
    pub engine_kind: Option<EngineKind>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetPriorityRequest {
    pub priority: DownloadPriority,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDownloadRequest {
    /// Unix timestamp in milliseconds at which the download should start.
    pub scheduled_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub ipc_version: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadDto {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub destination: String,
    pub status: DownloadStatus,
    pub priority: DownloadPriority,
    pub engine: EngineKind,
    pub progress: DownloadProgressDto,
    pub error: Option<DownloadErrorDto>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadProgressDto {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: Option<u64>,
    pub estimated_remaining_seconds: Option<u64>,
    pub percentage_basis_points: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadErrorDto {
    pub code: DownloadErrorCode,
    pub message: String,
}

/// Transfer object for a persisted one-shot schedule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduleDto {
    pub download_id: String,
    /// Unix milliseconds at which the download is scheduled to start.
    pub scheduled_at_ms: u64,
    pub enabled: bool,
}

/// Live queue statistics for the status bar.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QueueSnapshotDto {
    pub active_count: usize,
    pub queued_count: usize,
    pub paused_count: usize,
    pub used_connections: usize,
    pub total_connections: usize,
}

impl From<DownloadItem> for DownloadDto {
    fn from(item: DownloadItem) -> Self {
        let progress = item.progress();
        Self {
            id: item.id().to_string(),
            url: item.source().url().to_owned(),
            filename: item.filename().as_str().to_owned(),
            destination: item.destination().to_string_lossy().into_owned(),
            status: item.status(),
            priority: item.priority(),
            engine: item.source().engine(),
            progress: DownloadProgressDto {
                downloaded_bytes: progress.downloaded_bytes(),
                total_bytes: progress.total_bytes(),
                speed_bytes_per_second: progress.speed_bytes_per_second(),
                estimated_remaining_seconds: progress.estimated_remaining_seconds(),
                percentage_basis_points: progress.percentage_basis_points(),
            },
            error: item.error().map(|error| DownloadErrorDto {
                code: error.code(),
                message: error.message().to_owned(),
            }),
            created_at: item.created_at().as_unix_millis(),
            updated_at: item.updated_at().as_unix_millis(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorCode {
    InvalidRequest,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: &'static str,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for IpcError {}

impl IpcError {
    fn invalid_request() -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message: "Enter a valid HTTP or HTTPS URL without embedded credentials.",
        }
    }

    fn not_found() -> Self {
        Self {
            code: IpcErrorCode::NotFound,
            message: "The download was not found.",
        }
    }

    fn conflict() -> Self {
        Self {
            code: IpcErrorCode::Conflict,
            message: "The download cannot perform that action in its current state.",
        }
    }

    fn internal() -> Self {
        Self {
            code: IpcErrorCode::Internal,
            message: "The request could not be completed.",
        }
    }

    fn from_queue(error: QueueError) -> Self {
        match error {
            QueueError::DownloadNotFound(_)
            | QueueError::StateMachine(StateMachineError::NotFound(_)) => Self::not_found(),
            QueueError::InvalidStatus { .. }
            | QueueError::StateMachine(
                StateMachineError::InvalidTransition { .. }
                | StateMachineError::ConcurrentTransition { .. },
            )
            | QueueError::UnsupportedEngineOperation { .. }
            | QueueError::PauseSuperseded
            | QueueError::ResourceLimitInUse => Self::conflict(),
            _ => Self::internal(),
        }
    }

    fn from_schedule(error: ScheduleError) -> Self {
        match error {
            ScheduleError::DownloadNotFound(_) | ScheduleError::ScheduleNotFound(_) => {
                Self::not_found()
            }
            ScheduleError::InvalidState { .. } | ScheduleError::InvalidSchedule(_) => {
                Self::conflict()
            }
            ScheduleError::Timestamp(_) => Self::internal(),
            _ => Self::internal(),
        }
    }

    /// Used when an explicitly requested engine is not available at runtime.
    /// The user's engine selection is respected — we do not silently fall back.
    pub(crate) fn engine_unavailable(kind: EngineKind) -> Self {
        // Static messages per engine; do not expose process details or secrets.
        let message: &'static str = match kind {
            EngineKind::Aria2 => {
                "aria2 is currently unavailable. Please check Settings → Download Engines."
            }
            EngineKind::YtDlp => {
                "yt-dlp is currently unavailable. Please check Settings → Download Engines."
            }
            EngineKind::DirectHttp => {
                // DirectHttp is always available; this branch should not be reached.
                "The requested download engine is not available."
            }
        };
        Self {
            code: IpcErrorCode::Conflict,
            message,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeleteDownloadResponse {
    pub download_id: String,
    pub removed: bool,
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: "FetchX",
        version: env!("CARGO_PKG_VERSION"),
        ipc_version: 1,
    }
}

#[tauri::command]
pub async fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadDto>, IpcError> {
    state.list().await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_download(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AddDownloadRequest,
) -> Result<DownloadDto, IpcError> {
    let download = state.add(request).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pause_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.pause(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn resume_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.resume(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.cancel(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DeleteDownloadResponse, IpcError> {
    let id = DownloadId::parse(&download_id).map_err(|_| IpcError::invalid_request())?;
    state.delete(id).await?;
    emit_event(
        &app,
        DOWNLOAD_REMOVED,
        &DownloadRemovedEvent {
            download_id: download_id.clone(),
        },
    );
    Ok(DeleteDownloadResponse {
        download_id,
        removed: true,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_download_priority(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
    request: SetPriorityRequest,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.set_priority(id, request.priority).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn schedule_download(
    state: State<'_, AppState>,
    download_id: String,
    request: ScheduleDownloadRequest,
) -> Result<ScheduleDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    state.schedule(id, request.scheduled_at_ms).await?;
    // Return the persisted schedule so the frontend has the confirmed value.
    state
        .get_schedule(id)
        .await?
        .ok_or_else(IpcError::not_found)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn remove_download_schedule(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<(), IpcError> {
    let id = parse_download_id(&download_id)?;
    state.remove_schedule(id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_download_schedule(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<Option<ScheduleDto>, IpcError> {
    let id = parse_download_id(&download_id)?;
    state.get_schedule(id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn retry_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.retry(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command]
pub async fn get_queue_snapshot(state: State<'_, AppState>) -> Result<QueueSnapshotDto, IpcError> {
    state.snapshot().await
}

fn emit_event<T: Serialize + Clone>(app: &AppHandle, name: &str, payload: &T) {
    if let Err(error) = app.emit(name, payload.clone()) {
        tracing::warn!(%error, event = name, "could not emit desktop event");
    }
}

fn parse_download_id(value: &str) -> Result<DownloadId, IpcError> {
    DownloadId::parse(value).map_err(|_| IpcError::invalid_request())
}

fn filename_for_url(url: &Url) -> String {
    let candidate = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|segment| !segment.is_empty())
        .filter(|segment| is_single_normal_component(segment));

    candidate.unwrap_or("download").to_owned()
}

fn is_single_normal_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

    use fetchx_core::DownloadStateMachine;
    use fetchx_downloader::{
        DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    };
    use fetchx_models::{DownloadId, DownloadItem, DownloadStatus};
    use fetchx_queue::{DownloadEventBus, EventRepository, QueueConfig};
    use fetchx_scheduler::Scheduler;
    use fetchx_storage::{Database, DownloadRepository, SqliteDownloadRepository};
    use tokio::{sync::watch, time::timeout};

    use super::{
        AddDownloadRequest, AppState, DownloadDto, IpcError, IpcErrorCode, filename_for_url,
        parse_download_id, validate_http_source_url,
    };
    use crate::config::ConfigService;
    use crate::engines::EngineRuntime;

    struct WaitingEngine {
        repository: SqliteDownloadRepository,
    }

    impl DownloadEngine for WaitingEngine {
        async fn download(
            &self,
            _id: DownloadId,
            mut cancellation: watch::Receiver<bool>,
        ) -> Result<DownloadItem, DownloadEngineError> {
            let _ = cancellation.changed().await;
            Err(DownloadEngineError::Cancelled)
        }
    }

    impl ManagedDownloadEngine for WaitingEngine {
        async fn download_managed(
            &self,
            id: DownloadId,
            mut control: ManagedDownloadControl,
        ) -> Result<DownloadItem, DownloadEngineError> {
            let state_machine = DownloadStateMachine::new(self.repository.clone());
            let item = self
                .repository
                .get(id)
                .await?
                .ok_or(DownloadEngineError::NotFound(id))?;
            if item.status() == DownloadStatus::Queued {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
            }
            state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;

            loop {
                tokio::select! {
                    result = control.cancellation.changed() => {
                        if result.is_err() || *control.cancellation.borrow() {
                            state_machine.transition(id, DownloadStatus::Cancelled).await?;
                            return Err(DownloadEngineError::Cancelled);
                        }
                    }
                    result = control.pause.changed() => {
                        if result.is_err() || *control.pause.borrow() {
                            state_machine.transition(id, DownloadStatus::Paused).await?;
                            return Err(DownloadEngineError::Paused);
                        }
                    }
                }
            }
        }
    }

    async fn test_state() -> Result<AppState, Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let inner_repository = database.downloads();
        let event_bus = DownloadEventBus::new();
        let repository = EventRepository::new(inner_repository.clone(), event_bus);
        let config_service = ConfigService::new(database.config());

        // Use the default config (all optional engines disabled) to build an
        // EngineRuntime with only DirectHttp.  The runtime provides the dispatcher
        // reference stored in AppState, but the test scheduler uses WaitingEngine
        // so the existing test behaviour (wait for Downloading, then act) is preserved.
        let engine_config = crate::config::EngineConfig::default();
        let engine_runtime =
            EngineRuntime::initialize(repository.clone(), &engine_config, std::env::temp_dir())
                .await?;

        // Build scheduler with WaitingEngine so tests can observe lifecycle transitions.
        let scheduler = Scheduler::start(
            repository.clone(),
            Arc::new(WaitingEngine {
                repository: inner_repository,
            }),
            QueueConfig::default(),
        )
        .await?;
        let queue = scheduler.queue_manager();
        Ok(AppState::new(
            database,
            repository,
            scheduler,
            queue,
            PathBuf::from("test-downloads"),
            config_service,
            engine_runtime,
        ))
    }

    async fn wait_for_status(
        state: &AppState,
        id: DownloadId,
        status: DownloadStatus,
    ) -> Result<(), Box<dyn Error>> {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(item) = state.repository.get(id).await?
                    && item.status() == status
                {
                    return Ok::<(), fetchx_storage::StorageError>(());
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await??;
        Ok(())
    }

    #[test]
    fn add_request_rejects_unknown_fields_during_deserialization() -> Result<(), serde_json::Error>
    {
        assert!(
            serde_json::from_str::<AddDownloadRequest>(
                r#"{"url":"https://example.test/file","destination":"C:\\unsafe"}"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn download_ids_and_direct_http_urls_are_validated() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            parse_download_id("not-an-id").err().map(|error| error.code),
            Some(IpcErrorCode::InvalidRequest)
        );
        assert!(validate_http_source_url("file:///etc/passwd").is_err());
        let credential_url = format!(
            "https://{}:{}@example.test/file",
            "test-user", "test-password"
        );
        assert!(validate_http_source_url(&credential_url).is_err());
        assert_eq!(
            reqwest::Url::parse(&validate_http_source_url("https://example.test/file")?)?.scheme(),
            "https"
        );
        Ok(())
    }

    #[test]
    fn filename_uses_only_a_safe_url_path_component() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            filename_for_url(&reqwest::Url::parse("https://example.test/a%20file.zip")?),
            "a%20file.zip"
        );
        assert_eq!(
            filename_for_url(&reqwest::Url::parse("https://example.test/")?),
            "download"
        );
        Ok(())
    }

    #[tokio::test]
    async fn add_and_list_download_use_the_queue_and_repository() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/file.zip".to_owned(),
            })
            .await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;
        let listed = state.list().await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, download.id);
        assert_eq!(listed[0].status, DownloadStatus::Downloading);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn pause_resume_cancel_commands_route_through_queue() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/control.bin".to_owned(),
            })
            .await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;

        let paused = state.pause(DownloadId::parse(&download.id)?).await?;
        assert_eq!(paused.status, DownloadStatus::Paused);
        let resumed = state.resume(DownloadId::parse(&download.id)?).await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;
        assert!(matches!(
            resumed.status,
            DownloadStatus::Queued | DownloadStatus::Downloading
        ));
        let cancelled = state.cancel(DownloadId::parse(&download.id)?).await?;
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn delete_routes_through_queue_and_unknown_ids_are_mapped() -> Result<(), Box<dyn Error>>
    {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/delete.bin".to_owned(),
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;
        assert_eq!(state.delete(id).await, Ok(()));
        assert!(state.repository.get(id).await?.is_none());
        assert_eq!(
            state.delete(id).await.err().map(|error| error.code),
            Some(IpcErrorCode::NotFound)
        );
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn ipc_errors_and_event_payloads_have_stable_serialization() -> Result<(), Box<dyn Error>> {
        let error = super::IpcError {
            code: IpcErrorCode::NotFound,
            message: "The download was not found.",
        };
        assert_eq!(
            serde_json::to_string(&error)?,
            r#"{"code":"not_found","message":"The download was not found."}"#
        );
        let event = crate::events::DownloadRemovedEvent {
            download_id: "00000000-0000-4000-8000-000000000000".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&event)?,
            r#"{"download_id":"00000000-0000-4000-8000-000000000000"}"#
        );
        let updated = crate::events::DownloadUpdatedEvent {
            download: DownloadDto {
                id: "00000000-0000-4000-8000-000000000000".to_owned(),
                url: "https://example.test/file".to_owned(),
                filename: "file".to_owned(),
                destination: "downloads/file".to_owned(),
                status: DownloadStatus::Queued,
                priority: fetchx_models::DownloadPriority::Normal,
                engine: fetchx_models::EngineKind::DirectHttp,
                progress: super::DownloadProgressDto {
                    downloaded_bytes: 0,
                    total_bytes: None,
                    speed_bytes_per_second: None,
                    estimated_remaining_seconds: None,
                    percentage_basis_points: None,
                },
                error: None,
                created_at: 1,
                updated_at: 1,
            },
        };
        let serialized = serde_json::to_string(&updated)?;
        assert!(serialized.contains(r#""download":{"id":"00000000-0000-4000-8000-000000000000""#));
        assert!(serialized.contains(r#""status":"queued""#));
        Ok(())
    }

    #[test]
    fn new_request_types_reject_unknown_fields() -> Result<(), serde_json::Error> {
        // SetPriorityRequest must not accept extra fields.
        assert!(
            serde_json::from_str::<super::SetPriorityRequest>(
                r#"{"priority":"high","extra":true}"#
            )
            .is_err()
        );
        // ScheduleDownloadRequest must not accept extra fields.
        assert!(
            serde_json::from_str::<super::ScheduleDownloadRequest>(
                r#"{"scheduled_at_ms":1000000,"extra":true}"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn schedule_dto_serializes_stably() -> Result<(), serde_json::Error> {
        let dto = super::ScheduleDto {
            download_id: "00000000-0000-4000-8000-000000000000".to_owned(),
            scheduled_at_ms: 1_725_000_000_000,
            enabled: true,
        };
        let serialized = serde_json::to_string(&dto)?;
        assert!(serialized.contains(r#""scheduled_at_ms":1725000000000"#));
        assert!(serialized.contains(r#""enabled":true"#));
        Ok(())
    }

    #[tokio::test]
    async fn set_priority_changes_persisted_value() -> Result<(), Box<dyn Error>> {
        use fetchx_models::DownloadPriority;

        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/priority.bin".to_owned(),
            })
            .await?;
        assert_eq!(download.priority, DownloadPriority::Normal);

        let updated = state
            .set_priority(DownloadId::parse(&download.id)?, DownloadPriority::High)
            .await?;
        assert_eq!(updated.priority, DownloadPriority::High);
        assert_eq!(updated.id, download.id);

        // Confirm it is persisted.
        let listed = state.list().await?;
        assert_eq!(listed[0].priority, DownloadPriority::High);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn set_priority_on_missing_download_returns_not_found() -> Result<(), Box<dyn Error>> {
        use fetchx_models::DownloadPriority;

        let state = test_state().await?;
        let result = state
            .set_priority(DownloadId::new(), DownloadPriority::High)
            .await;
        assert_eq!(result.err().map(|e| e.code), Some(IpcErrorCode::NotFound));
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn schedule_and_remove_schedule_round_trip() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/scheduled.bin".to_owned(),
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        // The download must be in Queued or Paused state for scheduling.
        // Pause it immediately so the engine cannot start it first.
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;
        let paused = state.pause(id).await?;
        assert_eq!(paused.status, DownloadStatus::Paused);

        // Schedule one minute into the future so the download stays deferred.
        let future_ms = fetchx_models::Timestamp::now()?.as_unix_millis() + 60_000;
        state.schedule(id, future_ms).await?;

        let schedule = state.get_schedule(id).await?;
        assert!(schedule.is_some());
        let schedule = schedule.unwrap();
        assert_eq!(schedule.scheduled_at_ms, future_ms);
        assert!(schedule.enabled);

        // Remove the schedule — the download is released back to the queue.
        state.remove_schedule(id).await?;
        let after_remove = state.get_schedule(id).await?;
        assert!(after_remove.is_none());

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn schedule_on_missing_download_returns_not_found() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let future_ms = fetchx_models::Timestamp::now()?.as_unix_millis() + 60_000;
        let result = state.schedule(DownloadId::new(), future_ms).await;
        assert_eq!(result.err().map(|e| e.code), Some(IpcErrorCode::NotFound));
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn get_schedule_returns_none_for_unscheduled_download() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/nosched.bin".to_owned(),
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        let schedule = state.get_schedule(id).await?;
        assert!(schedule.is_none());
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn snapshot_reflects_queue_state() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        // Before adding anything, counts should be zero.
        let snap = state.snapshot().await?;
        assert_eq!(snap.queued_count, 0);
        assert_eq!(snap.active_count, 0);

        // Add a download — it starts, so active count becomes non-zero.
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/snap.bin".to_owned(),
            })
            .await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;

        let snap = state.snapshot().await?;
        assert_eq!(snap.active_count, 1);
        assert!(snap.total_connections > 0);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_failed_creates_fresh_download() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/retry.bin".to_owned(),
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;

        // Cancel it so we have a terminal download.
        let cancelled = state.cancel(id).await?;
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);

        // Retry should create a new download.
        let retried = state.retry(id).await?;
        assert_eq!(retried.status, DownloadStatus::Queued);
        assert_ne!(retried.id, download.id, "retry must produce a new ID");

        // Original record must be gone.
        assert!(state.repository.get(id).await?.is_none());

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_on_active_download_returns_conflict() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/activeretry.bin".to_owned(),
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;

        // Cannot retry a download that's still active.
        let result = state.retry(id).await;
        assert_eq!(result.err().map(|e| e.code), Some(IpcErrorCode::Conflict));

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn queue_snapshot_dto_serializes_stably() -> Result<(), serde_json::Error> {
        let dto = super::QueueSnapshotDto {
            active_count: 2,
            queued_count: 5,
            paused_count: 1,
            used_connections: 8,
            total_connections: 16,
        };
        let s = serde_json::to_string(&dto)?;
        assert!(s.contains(r#""active_count":2"#));
        assert!(s.contains(r#""total_connections":16"#));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Phase 16C engine-selection tests
    // -----------------------------------------------------------------------

    #[test]
    fn add_request_accepts_no_engine_kind() -> Result<(), serde_json::Error> {
        // engine_kind is optional with a default of None.
        let req: AddDownloadRequest =
            serde_json::from_str(r#"{"url":"https://example.test/file.zip"}"#)?;
        assert_eq!(req.engine_kind, None);
        Ok(())
    }

    #[test]
    fn add_request_accepts_explicit_direct_http() -> Result<(), serde_json::Error> {
        let req: AddDownloadRequest = serde_json::from_str(
            r#"{"url":"https://example.test/file.zip","engine_kind":"direct_http"}"#,
        )?;
        assert_eq!(req.engine_kind, Some(fetchx_models::EngineKind::DirectHttp));
        Ok(())
    }

    #[test]
    fn add_request_accepts_aria2_engine_kind() -> Result<(), serde_json::Error> {
        let req: AddDownloadRequest = serde_json::from_str(
            r#"{"url":"https://example.test/file.zip","engine_kind":"aria2"}"#,
        )?;
        assert_eq!(req.engine_kind, Some(fetchx_models::EngineKind::Aria2));
        Ok(())
    }

    #[test]
    fn add_request_rejects_unknown_engine_kind() -> Result<(), serde_json::Error> {
        // An unrecognised value must fail deserialization.
        let result: Result<AddDownloadRequest, _> = serde_json::from_str(
            r#"{"url":"https://example.test/file.zip","engine_kind":"unknown_engine"}"#,
        );
        assert!(
            result.is_err(),
            "unknown engine_kind value must be rejected"
        );
        Ok(())
    }

    #[tokio::test]
    async fn add_download_none_engine_uses_direct_http() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                url: "https://example.test/engine-default.zip".to_owned(),
                engine_kind: None,
            })
            .await?;
        assert_eq!(download.engine, fetchx_models::EngineKind::DirectHttp);
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_download_explicit_direct_http_is_accepted() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                url: "https://example.test/explicit-http.zip".to_owned(),
                engine_kind: Some(fetchx_models::EngineKind::DirectHttp),
            })
            .await?;
        assert_eq!(download.engine, fetchx_models::EngineKind::DirectHttp);
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_download_unavailable_aria2_is_rejected() -> Result<(), Box<dyn Error>> {
        // Default EngineConfig has aria2 disabled → unavailable.
        let state = test_state().await?;
        let result = state
            .add(AddDownloadRequest {
                url: "https://example.test/aria2-attempt.zip".to_owned(),
                engine_kind: Some(fetchx_models::EngineKind::Aria2),
            })
            .await;
        // Must be Conflict — not silently accepted as DirectHttp.
        assert_eq!(
            result.err().map(|e| e.code),
            Some(IpcErrorCode::Conflict),
            "disabled aria2 must be rejected with Conflict"
        );
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_download_unavailable_ytdlp_is_rejected() -> Result<(), Box<dyn Error>> {
        // Default EngineConfig has yt-dlp disabled → unavailable.
        let state = test_state().await?;
        let result = state
            .add(AddDownloadRequest {
                url: "https://example.test/ytdlp-attempt.zip".to_owned(),
                engine_kind: Some(fetchx_models::EngineKind::YtDlp),
            })
            .await;
        assert_eq!(
            result.err().map(|e| e.code),
            Some(IpcErrorCode::Conflict),
            "disabled yt-dlp must be rejected with Conflict"
        );
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_preserves_engine_kind_for_direct_http() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        // Add a DirectHttp download and cancel it.
        let download = state
            .add(AddDownloadRequest {
                url: "https://example.test/retry-engine.zip".to_owned(),
                engine_kind: None, // → DirectHttp
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;
        let cancelled = state.cancel(id).await?;
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);

        // Retry must produce a new download with the same engine.
        let retried = state.retry(id).await?;
        assert_eq!(
            retried.engine,
            fetchx_models::EngineKind::DirectHttp,
            "retry must preserve DirectHttp engine assignment"
        );
        assert_ne!(
            retried.id, download.id,
            "retry must create a new download ID"
        );

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn engine_unavailable_error_contains_no_secrets() {
        for kind in [
            fetchx_models::EngineKind::Aria2,
            fetchx_models::EngineKind::YtDlp,
            fetchx_models::EngineKind::DirectHttp,
        ] {
            let err = IpcError::engine_unavailable(kind);
            let json = serde_json::to_string(&err).unwrap();
            assert!(!json.contains("secret"), "error must not leak secrets");
            assert!(!json.contains("token"), "error must not leak tokens");
            assert!(!json.contains("rpc"), "error must not expose RPC details");
            assert_eq!(err.code, IpcErrorCode::Conflict);
        }
    }
}

use std::{
    path::{Component, Path},
    sync::{Arc, atomic::AtomicU8},
};

use fetchx_core::StateMachineError;
use fetchx_core_services::{ConfigService, CoreRepository, CoreServices, EngineRuntime};
use fetchx_downloader::{YtDlpFormat, validate_http_source_url};
use fetchx_models::{
    DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadSource, DownloadStatus,
    EngineKind, FileName, Timestamp, parse_magnet_uri,
};
use fetchx_queue::{QueueError, QueueManager};
use fetchx_scheduler::{ScheduleError, Scheduler};
use fetchx_storage::{Database, DownloadRepository, TorrentMetadataRepository};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::events::{
    DOWNLOAD_REMOVED, DOWNLOAD_UPDATED, DownloadRemovedEvent, DownloadUpdatedEvent,
};

/// The concrete repository type used by the desktop application.
pub(crate) type AppRepository = CoreRepository;

pub struct AppState {
    pub(crate) core_services: Option<Arc<CoreServices>>,
    pub(crate) database: Database,
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
    #[cfg(test)]
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
            core_services: None,
            database,
            repository,
            queue,
            scheduler,
            downloads_dir,
            shutdown_state: Arc::new(AtomicU8::new(0)),
            config_service,
            engine_runtime: Arc::new(engine_runtime),
        }
    }

    pub(crate) fn from_core_services(core: CoreServices) -> Self {
        let core_services = Arc::new(core);
        Self {
            database: core_services.database.clone(),
            repository: core_services.repository.clone(),
            queue: core_services.queue().clone(),
            scheduler: core_services.scheduler.clone(),
            downloads_dir: core_services.downloads_dir.clone(),
            shutdown_state: Arc::new(AtomicU8::new(0)),
            config_service: core_services.config_service.clone(),
            engine_runtime: Arc::clone(&core_services.engine_runtime),
            core_services: Some(core_services),
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
        self.add_with_ytdlp_format(request, None, None).await
    }

    async fn add_with_ytdlp_format(
        &self,
        request: AddDownloadRequest,
        format_id: Option<String>,
        extension: Option<String>,
    ) -> Result<DownloadDto, IpcError> {
        let magnet = if request.url.starts_with("magnet:") {
            Some(parse_magnet_uri(&request.url).map_err(|_| IpcError::invalid_request())?)
        } else {
            None
        };
        let torrent_path = (magnet.is_none()
            && !request.url.contains("://")
            && Path::new(&request.url)
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent")))
        .then(|| Path::new(&request.url).to_path_buf());
        let canonical_url = match (magnet.is_some(), torrent_path.is_some()) {
            (true, _) | (_, true) => request.url.clone(),
            (false, false) => {
                validate_http_source_url(&request.url).map_err(|_| IpcError::invalid_request())?
            }
        };
        let parsed_url = if magnet.is_none() && torrent_path.is_none() {
            Some(Url::parse(&canonical_url).map_err(|_| IpcError::invalid_request())?)
        } else {
            None
        };
        if extension.is_some()
            && (format_id.is_none()
                || magnet.is_some()
                || torrent_path.is_some()
                || !extension_is_safe(extension.as_deref()))
        {
            return Err(IpcError::invalid_request());
        }
        // Resolve the requested engine, defaulting to DirectHttp when unspecified.
        let engine_kind = match (
            magnet.is_some() || torrent_path.is_some(),
            request.engine_kind,
        ) {
            (true, None) => EngineKind::Aria2,
            (true, Some(EngineKind::Aria2)) => EngineKind::Aria2,
            (true, Some(_)) => return Err(IpcError::invalid_request()),
            (false, Some(engine)) => engine,
            (false, None) => EngineKind::DirectHttp,
        };

        // Backend validates availability — the frontend cannot bypass this check.
        // Direct HTTP is always available; optional engines must be running.
        if !self.engine_runtime.is_available(engine_kind) {
            return Err(IpcError::engine_unavailable(engine_kind));
        }

        if magnet.is_some() {
            let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
            let id = core_services
                .enqueue_magnet(&canonical_url, DownloadPriority::Normal, None)
                .await
                .map_err(|_| IpcError::internal())?;
            return self.get(id).await;
        }

        if let Some(torrent_path) = torrent_path {
            let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
            let id = core_services
                .enqueue_torrent_file_with_selection(
                    torrent_path,
                    DownloadPriority::Normal,
                    None,
                    None,
                )
                .await
                .map_err(|_| IpcError::internal())?;
            return self.get(id).await;
        }

        let filename = parsed_url
            .as_ref()
            .map(filename_for_url)
            .ok_or_else(IpcError::invalid_request)?;
        let filename = match extension.as_deref() {
            Some(extension) => Path::new(&filename)
                .with_extension(extension)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(IpcError::invalid_request)?
                .to_owned(),
            None => filename,
        };
        let filename = FileName::new(filename).map_err(|_| IpcError::invalid_request())?;
        let id = DownloadId::new();
        let source = DownloadSource::new(canonical_url, engine_kind)
            .and_then(|source| match format_id {
                Some(format_id) => source.with_yt_dlp_format(format_id),
                None => Ok(source),
            })
            .map_err(|_| IpcError::invalid_request())?;
        let destination = self.downloads_dir.join(filename.as_str());
        let created_at = Timestamp::now().map_err(|_| IpcError::internal())?;
        let item = DownloadItem::new(id, source, filename, destination, created_at);

        self.queue
            .enqueue(item)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn discover_ytdlp_formats(&self, source_url: &str) -> Result<Vec<YtDlpFormat>, IpcError> {
        let canonical_url =
            validate_http_source_url(source_url).map_err(|_| IpcError::invalid_request())?;
        if !self.engine_runtime.is_available(EngineKind::YtDlp) {
            return Err(IpcError::engine_unavailable(EngineKind::YtDlp));
        }
        self.engine_runtime
            .dispatcher
            .discover_ytdlp_formats(&canonical_url)
            .await
            .map_err(|_| IpcError::internal())
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
            database: self.database.clone(),
            core_services: self.core_services.clone(),
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

        let source = item.source();
        let url = source.url();

        // Dispatch based on source type, mirroring the `add` command logic.
        if url.starts_with("magnet:") {
            // Re-enqueue as magnet via CoreServices.
            let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
            let new_id = core_services
                .enqueue_magnet(url, fetchx_models::DownloadPriority::Normal, None)
                .await
                .map_err(|_| IpcError::internal())?;
            return self.get(new_id).await;
        }

        if source.is_torrent_file() {
            // Re-enqueue as torrent file via CoreServices.
            let torrent_path = source
                .torrent_file_path()
                .ok_or_else(IpcError::invalid_request)?;
            let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
            let new_id = core_services
                .enqueue_torrent_file_with_selection(
                    torrent_path,
                    fetchx_models::DownloadPriority::Normal,
                    None,
                    None,
                )
                .await
                .map_err(|_| IpcError::internal())?;
            return self.get(new_id).await;
        }

        // HTTP/HTTPS source — validate and re-enqueue as before.
        let canonical_url =
            validate_http_source_url(url).map_err(|_| IpcError::invalid_request())?;
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
            EngineKind::NativeTorrent => "The native torrent engine is currently unavailable.",
            EngineKind::DirectHttp | EngineKind::Hls => {
                // DirectHttp and Hls are always available; this branch should not be reached.
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
pub async fn add_torrent_download(
    app: AppHandle,
    state: State<'_, AppState>,
    torrent_path: String,
    selected_file_indexes: Option<Vec<u32>>,
) -> Result<DownloadDto, IpcError> {
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let id = core_services
        .enqueue_torrent_file_with_selection(
            Path::new(&torrent_path).to_path_buf(),
            DownloadPriority::Normal,
            None,
            selected_file_indexes,
        )
        .await
        .map_err(|_| IpcError::internal())?;
    let download = state.get(id).await?;
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
pub async fn add_media_download(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    format_id: Option<String>,
    extension: Option<String>,
) -> Result<DownloadDto, IpcError> {
    let download = state
        .add_with_ytdlp_format(
            AddDownloadRequest {
                url,
                engine_kind: Some(EngineKind::YtDlp),
            },
            format_id,
            extension,
        )
        .await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

fn extension_is_safe(extension: Option<&str>) -> bool {
    extension.is_some_and(|value| {
        !value.is_empty()
            && value.len() <= 10
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn discover_ytdlp_formats(
    state: State<'_, AppState>,
    url: String,
) -> Result<Vec<YtDlpFormat>, IpcError> {
    state.discover_ytdlp_formats(&url).await
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

/// Torrent metadata transfer object for the desktop frontend.
#[derive(Clone, Debug, Serialize)]
pub struct TorrentMetadataDto {
    pub download_id: String,
    pub source_type: String,
    pub info_hash: String,
    pub torrent_name: String,
    pub destination: String,
    pub metadata_state: String,
    pub selected_file_indexes: Vec<u32>,
    pub selected_total_bytes: u64,
    pub aria2_gid: Option<String>,
    pub seed_state: String,
    pub uploaded_bytes: u64,
    pub upload_speed_bps: u64,
    pub peer_count: u64,
    pub seed_count: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TorrentFileDto {
    pub index: u32,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TorrentInspectionDto {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub files: Vec<TorrentFileDto>,
}

/// Torrent telemetry transfer object for the desktop frontend.
#[derive(Clone, Debug, Serialize)]
pub struct TorrentTelemetryDto {
    pub download_speed_bps: Option<u64>,
    pub upload_speed_bps: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub peer_count: Option<u64>,
    pub completed_peer_count: Option<u64>,
    pub seed_count: Option<u64>,
    pub share_ratio: Option<f64>,
    pub seeding: Option<bool>,
    pub tracker_uris: Option<Vec<String>>,
}

#[tauri::command]
pub async fn get_torrent_metadata(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<TorrentMetadataDto>, IpcError> {
    let download_id = parse_download_id(&id)?;
    let row = state
        .database
        .torrent_metadata()
        .get_torrent_metadata(download_id)
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(row.map(|r| TorrentMetadataDto {
        download_id: r.download_id.to_string(),
        source_type: format!("{:?}", r.source_type),
        info_hash: r.info_hash,
        torrent_name: r.torrent_name,
        destination: r.destination.to_string_lossy().into_owned(),
        metadata_state: r.metadata_state,
        selected_file_indexes: r.selected_file_indexes,
        selected_total_bytes: r.selected_total_bytes,
        aria2_gid: r.aria2_gid,
        seed_state: format!("{:?}", r.seed_state),
        uploaded_bytes: r.uploaded_bytes,
        upload_speed_bps: r.upload_speed_bps,
        peer_count: r.peer_count,
        seed_count: r.seed_count,
    }))
}

#[tauri::command]
pub async fn get_torrent_download_metadata(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<fetchx_models::TorrentDownloadMetadata>, IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .torrent_download_metadata(download_id)
        .await
        .map_err(|_| IpcError::internal())
}

#[tauri::command]
pub async fn select_torrent_files(
    state: State<'_, AppState>,
    id: String,
    file_indexes: Vec<u32>,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .select_torrent_files(download_id, file_indexes)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command]
pub async fn set_torrent_file_selected(
    state: State<'_, AppState>,
    id: String,
    file_index: u32,
    selected: bool,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .set_torrent_file_selected(download_id, file_index, selected)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command]
pub async fn set_torrent_selection_all(
    state: State<'_, AppState>,
    id: String,
    selected: bool,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .set_torrent_selection_all(download_id, selected)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_torrent_directory_selected(
    state: State<'_, AppState>,
    id: String,
    directory: String,
    selected: bool,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .set_torrent_directory_selected(download_id, directory, selected)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn inspect_torrent_file(
    state: State<'_, AppState>,
    torrent_path: String,
) -> Result<TorrentInspectionDto, IpcError> {
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let metadata = core_services
        .inspect_torrent_file(Path::new(&torrent_path).to_path_buf())
        .await
        .map_err(|_| IpcError::invalid_request())?;
    let files = metadata
        .files
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            Ok(TorrentFileDto {
                index: u32::try_from(index).map_err(|_| IpcError::internal())?,
                path: file.path.join("/"),
                size: file.size,
            })
        })
        .collect::<Result<Vec<_>, IpcError>>()?;
    Ok(TorrentInspectionDto {
        info_hash: metadata.info_hash,
        name: metadata.name,
        total_size: metadata.total_size,
        files,
    })
}

#[tauri::command]
pub async fn get_torrent_telemetry(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<TorrentTelemetryDto>, IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let telemetry = core_services
        .torrent_telemetry(download_id)
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(telemetry.map(|value| TorrentTelemetryDto {
        download_speed_bps: value.download_speed_bps,
        upload_speed_bps: value.upload_speed_bps,
        downloaded_bytes: value.downloaded_bytes,
        uploaded_bytes: value.uploaded_bytes,
        peer_count: value.peer_count,
        completed_peer_count: value.completed_peer_count,
        seed_count: value.seed_count,
        share_ratio: value.share_ratio,
        seeding: value.seeding,
        tracker_uris: value.tracker_uris,
    }))
}

#[tauri::command]
pub async fn stop_torrent_seeding(state: State<'_, AppState>, id: String) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .stop_torrent_seeding(download_id)
        .await
        .map_err(|_| IpcError::internal())
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
        AddDownloadRequest, AppState, DownloadDto, IpcError, IpcErrorCode, extension_is_safe,
        filename_for_url, parse_download_id, validate_http_source_url,
    };
    use fetchx_core_services::{ConfigService, EngineConfig, EngineRuntime};

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
        let engine_config = EngineConfig::default();
        let engine_runtime =
            EngineRuntime::initialize(repository.clone(), &engine_config, &std::env::temp_dir())
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

    #[test]
    fn selected_media_extension_is_safe_for_a_destination_name() {
        assert!(extension_is_safe(Some("webm")));
        assert!(extension_is_safe(Some("m4a")));
        assert!(!extension_is_safe(None));
        assert!(!extension_is_safe(Some("../mp4")));
        assert!(!extension_is_safe(Some(".mp4")));
        assert!(!extension_is_safe(Some("x".repeat(11).as_str())));
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
            DownloadStatus::Queued | DownloadStatus::Preparing | DownloadStatus::Downloading
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

        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;

        let updated = state.set_priority(id, DownloadPriority::High).await?;
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
        assert!(
            matches!(
                retried.status,
                DownloadStatus::Queued | DownloadStatus::Preparing | DownloadStatus::Downloading
            ),
            "unexpected retry status: {:?}",
            retried.status
        );
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
            fetchx_models::EngineKind::NativeTorrent,
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

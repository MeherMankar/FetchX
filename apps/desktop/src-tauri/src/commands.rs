use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, atomic::AtomicU8},
};

use mydownloader_core::StateMachineError;
use mydownloader_downloader::validate_http_source_url;
use mydownloader_models::{
    DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadSource, DownloadStatus,
    EngineKind, FileName, Timestamp,
};
use mydownloader_queue::{EventRepository, QueueError, QueueManager};
use mydownloader_scheduler::{ScheduleError, Scheduler};
use mydownloader_storage::{Database, DownloadRepository, SqliteDownloadRepository};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::events::{
    DOWNLOAD_REMOVED, DOWNLOAD_UPDATED, DownloadRemovedEvent, DownloadUpdatedEvent,
};

/// The concrete repository type used by the desktop application.
///
/// Wrapping `SqliteDownloadRepository` in `EventRepository` causes every
/// persistence write to publish a `DownloadEvent` to the broadcast bus,
/// which the event bridge task then forwards to the Tauri frontend.
pub(crate) type AppRepository = EventRepository<SqliteDownloadRepository>;

pub struct AppState {
    _database: Database,
    pub(crate) repository: AppRepository,
    queue: QueueManager,
    pub(crate) scheduler: Scheduler<AppRepository>,
    downloads_dir: PathBuf,
    pub(crate) shutdown_state: Arc<AtomicU8>,
}

impl AppState {
    pub(crate) fn new(
        database: Database,
        repository: AppRepository,
        scheduler: Scheduler<AppRepository>,
        queue: QueueManager,
        downloads_dir: PathBuf,
    ) -> Self {
        Self {
            _database: database,
            repository,
            queue,
            scheduler,
            downloads_dir,
            shutdown_state: Arc::new(AtomicU8::new(0)),
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
        let source = DownloadSource::new(canonical_url, EngineKind::DirectHttp)
            .map_err(|_| IpcError::invalid_request())?;
        let created_at = Timestamp::now().map_err(|_| IpcError::internal())?;
        let item = DownloadItem::new(id, source, filename, destination, created_at);

        self.queue
            .enqueue(item)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeleteDownloadResponse {
    pub download_id: String,
    pub removed: bool,
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: "MyDownloader",
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

    use mydownloader_core::DownloadStateMachine;
    use mydownloader_downloader::{
        DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    };
    use mydownloader_models::{DownloadId, DownloadItem, DownloadStatus};
    use mydownloader_queue::{DownloadEventBus, EventRepository, QueueConfig};
    use mydownloader_scheduler::Scheduler;
    use mydownloader_storage::{Database, DownloadRepository, SqliteDownloadRepository};
    use tokio::{sync::watch, time::timeout};

    use super::{
        AddDownloadRequest, AppState, DownloadDto, IpcErrorCode, filename_for_url,
        parse_download_id, validate_http_source_url,
    };

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
                    return Ok::<(), mydownloader_storage::StorageError>(());
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
                priority: mydownloader_models::DownloadPriority::Normal,
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
        use mydownloader_models::DownloadPriority;

        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
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
        use mydownloader_models::DownloadPriority;

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
        let future_ms = mydownloader_models::Timestamp::now()?.as_unix_millis() + 60_000;
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
        let future_ms = mydownloader_models::Timestamp::now()?.as_unix_millis() + 60_000;
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
                url: "https://example.test/nosched.bin".to_owned(),
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        let schedule = state.get_schedule(id).await?;
        assert!(schedule.is_none());
        state.scheduler.shutdown().await?;
        Ok(())
    }
}

use std::path::PathBuf;
use std::{
    ffi::OsString,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_downloader::{
    YtDlpError, YtDlpFormat, YtDlpProcessExit, YtDlpProcessRunner, YtDlpRunnerEvent,
    YtDlpRunnerFuture, parse_format_output,
};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
};
use fetchx_storage::DownloadRepository as _;
use tokio::{
    fs,
    sync::{broadcast, mpsc, watch},
    time::sleep,
};
use tracing::warn;

use crate::dto::{
    CoreConfig, DownloadInfoDto, DownloadPriorityDto, YtDlpFormatDto, event_to_dto, item_to_dto,
    priority_from_dto,
};
use crate::error::MobileApiError;
use crate::{DownloadEventListener, YtDlpRuntime};

const MAX_ANDROID_PROGRESS_LINE_BYTES: usize = 1024;

// ── Inner state that lives only while the service is running ──────────────

struct ServiceInner {
    runtime: tokio::runtime::Runtime,
    core: CoreServices,
}

// ── Public DownloadService type exposed via UniFFI ────────────────────────

/// The central Android bridge service. Wraps all core services and
/// provides a synchronous (blocking) API for Kotlin callers via UniFFI.
pub struct DownloadService {
    /// `None` after `shutdown()` has been called.
    inner: Mutex<Option<ServiceInner>>,
}

impl DownloadService {
    /// Creates the service, starts the Tokio runtime, and initialises CoreServices.
    ///
    /// This is the UniFFI constructor; called from Kotlin as `DownloadService(config)`.
    pub fn new(
        config: CoreConfig,
        yt_dlp_runtime: Option<Box<dyn YtDlpRuntime>>,
    ) -> Result<Self, MobileApiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| MobileApiError::Runtime(format!("failed to build Tokio runtime: {e}")))?;

        let runtime_dir = PathBuf::from(&config.runtime_dir);
        let core_config = CoreServicesConfig {
            database_path: PathBuf::from(&config.database_path),
            downloads_dir: PathBuf::from(&config.downloads_dir),
            runtime_dir: PathBuf::from(&config.runtime_dir),
        };
        let ytdlp_runner = yt_dlp_runtime.map(|runtime| {
            Arc::new(AndroidYtDlpProcessRunner {
                runtime: Arc::from(runtime),
                runtime_dir: runtime_dir.clone(),
            }) as Arc<dyn YtDlpProcessRunner>
        });
        let engine_config = {
            let mut engine_config = fetchx_core_services::EngineConfig::default();
            if let Some(path) = config.yt_dlp_executable_path {
                engine_config.yt_dlp.enabled = true;
                engine_config.yt_dlp.executable_path = Some(PathBuf::from(path));
            }
            if ytdlp_runner.is_some() {
                engine_config.yt_dlp.enabled = true;
            }
            if let Some(path) = config.aria2_executable_path {
                engine_config.aria2.enabled = true;
                engine_config.aria2.executable_path = Some(PathBuf::from(path));
            }
            engine_config
        };

        let core = runtime
            .block_on(CoreServices::start_with_engine_config_and_ytdlp_runner(
                core_config,
                Some(engine_config),
                ytdlp_runner,
            ))
            .map_err(MobileApiError::from)?;

        Ok(Self {
            inner: Mutex::new(Some(ServiceInner { runtime, core })),
        })
    }

    // ── Helper: lock inner and execute a blocking async call ──────────────

    fn with_inner<T, F>(&self, f: F) -> Result<T, MobileApiError>
    where
        F: FnOnce(&ServiceInner) -> Result<T, MobileApiError>,
    {
        let guard = self
            .inner
            .lock()
            .map_err(|_| MobileApiError::Runtime("service lock poisoned".to_string()))?;
        match guard.as_ref() {
            Some(inner) => f(inner),
            None => Err(MobileApiError::Runtime(
                "service has been shut down".to_string(),
            )),
        }
    }

    // ── List / Get ────────────────────────────────────────────────────────

    pub fn list_downloads(&self) -> Result<Vec<DownloadInfoDto>, MobileApiError> {
        self.with_inner(|inner| {
            let items = inner
                .runtime
                .block_on(inner.core.repository.list())
                .map_err(|e| MobileApiError::Storage(e.to_string()))?;
            Ok(items.iter().map(item_to_dto).collect())
        })
    }

    pub fn get_download(&self, id: String) -> Result<Option<DownloadInfoDto>, MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            let item = inner
                .runtime
                .block_on(inner.core.repository.get(download_id))
                .map_err(|e| MobileApiError::Storage(e.to_string()))?;
            Ok(item.as_ref().map(item_to_dto))
        })
    }

    // ── Add ────────────────────────────────────────────────────────────────

    pub fn add_download(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
    ) -> Result<String, MobileApiError> {
        self.add_download_for_engine(
            url,
            filename,
            destination,
            priority,
            EngineKind::DirectHttp,
            None,
        )
    }

    pub fn add_media_download(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
        format_id: Option<String>,
    ) -> Result<String, MobileApiError> {
        self.add_download_for_engine(
            url,
            filename,
            destination,
            priority,
            EngineKind::YtDlp,
            format_id,
        )
    }

    pub fn discover_media_formats(
        &self,
        url: String,
    ) -> Result<Vec<YtDlpFormatDto>, MobileApiError> {
        if url.trim().is_empty() {
            return Err(MobileApiError::Validation(
                "URL must not be empty".to_string(),
            ));
        }
        self.with_inner(|inner| {
            if !inner.core.engine_runtime.is_available(EngineKind::YtDlp) {
                return Err(MobileApiError::UnavailableEngine("YtDlp".to_string()));
            }
            let formats = inner
                .runtime
                .block_on(
                    inner
                        .core
                        .engine_runtime
                        .dispatcher
                        .discover_ytdlp_formats(&url),
                )
                .map_err(|error| MobileApiError::Runtime(error.to_string()))?;
            Ok(formats.into_iter().map(format_to_dto).collect())
        })
    }

    fn add_download_for_engine(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
        engine: EngineKind,
        format_id: Option<String>,
    ) -> Result<String, MobileApiError> {
        // Validate inputs before locking the inner service.
        if url.trim().is_empty() {
            return Err(MobileApiError::Validation(
                "URL must not be empty".to_string(),
            ));
        }
        let fname =
            FileName::new(&filename).map_err(|e| MobileApiError::Validation(e.to_string()))?;
        let source = DownloadSource::new(&url, engine)
            .and_then(|source| match format_id {
                Some(format_id) => source.with_yt_dlp_format(format_id),
                None => Ok(source),
            })
            .map_err(|e| MobileApiError::Validation(e.to_string()))?;
        let dest = PathBuf::from(&destination);
        let rust_priority = priority_from_dto(priority);

        self.with_inner(|inner| {
            if engine != EngineKind::DirectHttp && !inner.core.engine_runtime.is_available(engine) {
                return Err(MobileApiError::UnavailableEngine(format!("{engine:?}")));
            }
            let created_at =
                Timestamp::now().map_err(|e| MobileApiError::Runtime(e.to_string()))?;
            let new_id = DownloadId::new();
            let mut item = DownloadItem::new(new_id, source, fname, dest, created_at);
            // Apply the requested priority (default is Normal).
            item.set_priority_at(rust_priority, created_at)
                .map_err(|e| MobileApiError::Runtime(e.to_string()))?;
            inner
                .runtime
                .block_on(inner.core.queue().enqueue(item))
                .map_err(MobileApiError::from)?;
            Ok(new_id.to_string())
        })
    }

    // ── Lifecycle controls ────────────────────────────────────────────────

    pub fn pause_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().pause(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn resume_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().resume(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn cancel_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().cancel(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn remove_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().remove_download(download_id))
                .map_err(MobileApiError::from)
        })
    }

    // ── Priority ──────────────────────────────────────────────────────────

    pub fn set_priority(
        &self,
        id: String,
        priority: DownloadPriorityDto,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        let rust_priority = priority_from_dto(priority);
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().set_priority(download_id, rust_priority))
                .map_err(MobileApiError::from)
        })
    }

    // ── Schedule ──────────────────────────────────────────────────────────

    pub fn schedule_download(
        &self,
        id: String,
        start_at_millis: i64,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        let millis = u64::try_from(start_at_millis).map_err(|_| {
            MobileApiError::Validation("start_at_millis must be non-negative".to_string())
        })?;
        let ts = Timestamp::from_unix_millis(millis);
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.scheduler().schedule_download(download_id, ts))
                .map_err(MobileApiError::from)
        })
    }

    // ── Retry ──────────────────────────────────────────────────────────────

    pub fn retry_download(&self, id: String) -> Result<String, MobileApiError> {
        let old_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;

        self.with_inner(|inner| {
            // 1. Look up the old item.
            let item = inner
                .runtime
                .block_on(inner.core.repository.get(old_id))
                .map_err(|e| MobileApiError::Storage(e.to_string()))?
                .ok_or_else(|| MobileApiError::NotFound(format!("download {old_id} not found")))?;

            // 2. Validate that it is in a retryable state.
            if !matches!(
                item.status(),
                DownloadStatus::Failed | DownloadStatus::Cancelled
            ) {
                return Err(MobileApiError::InvalidStateTransition(format!(
                    "retry requires a failed or cancelled download (current: {:?})",
                    item.status()
                )));
            }

            // 3. Check engine availability.
            let engine = item.source().engine();
            if !inner.core.engine_runtime.is_available(engine) {
                return Err(MobileApiError::UnavailableEngine(format!(
                    "{engine:?} engine is not available"
                )));
            }

            // 4. Remove old record.
            inner
                .runtime
                .block_on(inner.core.queue().remove_download(old_id))
                .map_err(MobileApiError::from)?;

            // 5. Build a new item preserving source/filename/destination.
            let created_at =
                Timestamp::now().map_err(|e| MobileApiError::Runtime(e.to_string()))?;
            let new_id = DownloadId::new();
            let new_item = DownloadItem::new(
                new_id,
                item.source().clone(),
                item.filename().clone(),
                item.destination().to_path_buf(),
                created_at,
            );

            // 6. Enqueue the new item.
            inner
                .runtime
                .block_on(inner.core.queue().enqueue(new_item))
                .map_err(MobileApiError::from)?;

            Ok(new_id.to_string())
        })
    }

    // ── Event listener ────────────────────────────────────────────────────

    /// Registers a Kotlin-implemented event listener. Spawns a background task
    /// on the existing runtime that forwards events until the broadcast channel closes.
    pub fn add_event_listener(&self, listener: Box<dyn DownloadEventListener>) {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(inner) = guard.as_ref() else {
            return;
        };

        let mut rx: broadcast::Receiver<fetchx_queue::DownloadEvent> =
            inner.core.event_bus.subscribe();

        // UniFFI callback interface objects are Send + Sync, wrap in Arc for the async task.
        let listener_arc: Arc<dyn DownloadEventListener> = Arc::from(listener);

        inner.runtime.handle().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let dto = event_to_dto(event);
                        listener_arc.on_event(dto);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            skipped = n,
                            "event listener lagged; some events were skipped"
                        );
                        // Continue — do not break; catch up from next available event.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });
    }

    // ── Shutdown ──────────────────────────────────────────────────────────

    /// Gracefully shuts down the core services and drops the Tokio runtime.
    ///
    /// Calling `shutdown()` more than once is safe; subsequent calls are no-ops.
    pub fn shutdown(&self) {
        let inner_opt = match self.inner.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        if let Some(inner) = inner_opt {
            let ServiceInner { runtime, core } = inner;
            runtime.block_on(async {
                if let Err(e) = core.shutdown().await {
                    warn!(error = %e, "error during CoreServices shutdown");
                }
            });
            // Dropping `runtime` here joins all spawned tasks.
            drop(runtime);
        }
    }
}

struct AndroidYtDlpProcessRunner {
    runtime: Arc<dyn YtDlpRuntime>,
    runtime_dir: PathBuf,
}

static NEXT_YTDLP_RUN_ID: AtomicU64 = AtomicU64::new(1);

impl YtDlpProcessRunner for AndroidYtDlpProcessRunner {
    fn report_diagnostic(&self, code: &'static str) {
        self.runtime.report_failure(code.to_owned());
    }

    fn validate_version<'a>(
        &'a self,
        _executable: &'a Path,
        _timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
        Box::pin(async move {
            let runtime = Arc::clone(&self.runtime);
            tokio::task::spawn_blocking(move || runtime.validate_version())
                .await
                .map_err(|_| YtDlpError::ProcessRunner)
        })
    }

    fn run<'a>(
        &'a self,
        _executable: &'a Path,
        arguments: &'a [OsString],
        stop: watch::Receiver<bool>,
        events: mpsc::Sender<YtDlpRunnerEvent>,
    ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
        let arguments = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let runtime = Arc::clone(&self.runtime);
        let runtime_dir = self.runtime_dir.clone();
        Box::pin(async move {
            let run_id = NEXT_YTDLP_RUN_ID.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| YtDlpError::ProcessRunner)?
                .as_nanos();
            let run_dir = runtime_dir.join(format!("ytdlp-run-{timestamp}-{run_id}"));
            fs::create_dir(&run_dir)
                .await
                .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
            let progress_path = run_dir.join("progress");
            let cancel_path = run_dir.join("cancel");
            let progress_for_worker = progress_path.to_string_lossy().into_owned();
            let cancel_for_worker = cancel_path.to_string_lossy().into_owned();
            let mut worker = tokio::task::spawn_blocking(move || {
                runtime.run(arguments, progress_for_worker, cancel_for_worker)
            });
            let mut last_progress = String::new();
            let mut cancellation_requested = false;

            let result = loop {
                tokio::select! {
                    joined = &mut worker => {
                        break joined.map_err(|_| YtDlpError::ProcessRunner)?;
                    }
                    _ = sleep(Duration::from_millis(150)) => {
                        if *stop.borrow() && !cancellation_requested {
                            fs::write(&cancel_path, b"cancel")
                                .await
                                .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                            cancellation_requested = true;
                        }

                        match fs::read_to_string(&progress_path).await {
                            Ok(contents) if contents.ends_with('\n') => {
                                let line = contents.trim_end_matches(['\r', '\n']);
                                if line.len() > MAX_ANDROID_PROGRESS_LINE_BYTES {
                                    fs::write(&cancel_path, b"cancel")
                                        .await
                                        .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                                    worker.await.map_err(|_| YtDlpError::ProcessRunner)?;
                                    if let Err(error) = fs::remove_dir_all(&run_dir).await {
                                        warn!(%error, "yt-dlp control workspace cleanup failed");
                                    }
                                    return Err(YtDlpError::ProgressOutputTooLarge);
                                }
                                if !line.is_empty() && line != last_progress {
                                    if events.send(YtDlpRunnerEvent::ProgressLine(line.to_owned())).await.is_err() {
                                        fs::write(&cancel_path, b"cancel")
                                            .await
                                            .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                                        cancellation_requested = true;
                                    }
                                    last_progress = line.to_owned();
                                }
                            }
                            Ok(_) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(_) => {
                                fs::write(&cancel_path, b"cancel")
                                    .await
                                    .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                                worker.await.map_err(|_| YtDlpError::ProcessRunner)?;
                                if let Err(cleanup_error) = fs::remove_dir_all(&run_dir).await {
                                    warn!(%cleanup_error, "yt-dlp control workspace cleanup failed");
                                }
                                return Err(YtDlpError::ProgressOutputRead);
                            }
                        }
                    }
                }
            };
            if let Err(error) = fs::remove_dir_all(&run_dir).await {
                warn!(%error, "yt-dlp control workspace cleanup failed");
            }
            if cancellation_requested || *stop.borrow() {
                return Ok(YtDlpProcessExit {
                    success: false,
                    code: Some(130),
                });
            }
            let exit_code = if result.is_empty() { 0 } else { 1 };
            if !result.is_empty() {
                events
                    .send(YtDlpRunnerEvent::ProcessFailed(result))
                    .await
                    .map_err(|_| YtDlpError::ProgressOutputRead)?;
            }
            Ok(YtDlpProcessExit {
                success: exit_code == 0,
                code: Some(exit_code),
            })
        })
    }

    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        url: &'a str,
        timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
        let runtime = Arc::clone(&self.runtime);
        let url = url.to_owned();
        Box::pin(async move {
            let output = tokio::time::timeout(
                timeout,
                tokio::task::spawn_blocking(move || runtime.discover_formats(url)),
            )
            .await
            .map_err(|_| YtDlpError::FormatDiscoveryTimeout)?
            .map_err(|_| YtDlpError::ProcessRunner)?;
            if output.starts_with("ERROR:") {
                return Err(YtDlpError::InvalidFormatOutput);
            }
            parse_format_output(output.as_bytes())
        })
    }
}

fn format_to_dto(format: YtDlpFormat) -> YtDlpFormatDto {
    YtDlpFormatDto {
        format_id: format.format_id,
        label: format.label,
        extension: format.extension,
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use tempfile::tempdir;

    use super::DownloadService;
    use crate::dto::{CoreConfig, DownloadEventDto, DownloadPriorityDto};
    use crate::error::MobileApiError;

    fn make_config() -> (CoreConfig, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let config = CoreConfig {
            database_path: dir.path().join("test.db").to_string_lossy().into_owned(),
            downloads_dir: dir.path().join("downloads").to_string_lossy().into_owned(),
            runtime_dir: dir.path().join("runtime").to_string_lossy().into_owned(),
            yt_dlp_executable_path: None,
            aria2_executable_path: None,
        };
        (config, dir)
    }

    #[test]
    fn init_and_shutdown() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        svc.shutdown();
    }

    #[test]
    fn double_shutdown_is_safe() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        svc.shutdown();
        svc.shutdown(); // second call must not panic
    }

    #[test]
    fn list_downloads_empty_initially() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let items = svc.list_downloads().expect("list failed");
        assert!(items.is_empty());
        svc.shutdown();
    }

    #[test]
    fn add_download_returns_uuid() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                "/tmp/file.zip".to_string(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");
        // Must parse as a UUID.
        uuid::Uuid::parse_str(&id).expect("returned ID is not a valid UUID");
        svc.shutdown();
    }

    #[test]
    fn add_media_download_requires_an_available_ytdlp_engine() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let result = svc.add_media_download(
            "https://www.youtube.com/watch?v=example".to_string(),
            "video".to_string(),
            "/tmp/downloads".to_string(),
            DownloadPriorityDto::Normal,
            None,
        );
        assert!(matches!(result, Err(MobileApiError::UnavailableEngine(_))));
        svc.shutdown();
    }

    #[test]
    fn get_download_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let unknown_id = uuid::Uuid::new_v4().to_string();
        let result = svc.get_download(unknown_id).expect("get_download failed");
        assert!(result.is_none());
        svc.shutdown();
    }

    #[test]
    fn add_and_list_two_downloads() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        svc.add_download(
            "https://example.com/a.zip".to_string(),
            "a.zip".to_string(),
            "/tmp/a.zip".to_string(),
            DownloadPriorityDto::Low,
        )
        .expect("add first failed");
        svc.add_download(
            "https://example.com/b.zip".to_string(),
            "b.zip".to_string(),
            "/tmp/b.zip".to_string(),
            DownloadPriorityDto::High,
        )
        .expect("add second failed");
        let items = svc.list_downloads().expect("list failed");
        assert_eq!(items.len(), 2);
        svc.shutdown();
    }

    #[test]
    fn invalid_url_returns_validation_error() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let result = svc.add_download(
            "   ".to_string(),
            "file.zip".to_string(),
            "/tmp/file.zip".to_string(),
            DownloadPriorityDto::Normal,
        );
        assert!(
            matches!(result, Err(MobileApiError::Validation(_))),
            "expected Validation error, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn pause_nonexistent_returns_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = uuid::Uuid::new_v4().to_string();
        let result = svc.pause_download(id);
        assert!(
            matches!(result, Err(MobileApiError::NotFound(_))),
            "expected NotFound, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn cancel_nonexistent_returns_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = uuid::Uuid::new_v4().to_string();
        let result = svc.cancel_download(id);
        assert!(
            matches!(result, Err(MobileApiError::NotFound(_))),
            "expected NotFound, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn remove_nonexistent_returns_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = uuid::Uuid::new_v4().to_string();
        let result = svc.remove_download(id);
        assert!(
            matches!(result, Err(MobileApiError::NotFound(_))),
            "expected NotFound, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn priority_change_roundtrip() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                "/tmp/file.zip".to_string(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");

        svc.set_priority(id.clone(), DownloadPriorityDto::High)
            .expect("set_priority failed");

        let item = svc
            .get_download(id)
            .expect("get_download failed")
            .expect("item should exist");
        assert!(
            matches!(item.priority, DownloadPriorityDto::High),
            "expected High priority"
        );
        svc.shutdown();
    }

    #[test]
    fn retry_non_failed_returns_invalid_state_error() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                "/tmp/file.zip".to_string(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");

        // The item is Queued, not Failed/Cancelled — retry should fail.
        let result = svc.retry_download(id);
        assert!(
            matches!(result, Err(MobileApiError::InvalidStateTransition(_))),
            "expected InvalidStateTransition, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn concurrent_list_calls() {
        let (config, _dir) = make_config();
        let svc = Arc::new(DownloadService::new(config, None).expect("service creation failed"));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let svc = Arc::clone(&svc);
                thread::spawn(move || {
                    svc.list_downloads().expect("concurrent list failed");
                })
            })
            .collect();

        for h in handles {
            h.join().expect("thread panicked");
        }
        svc.shutdown();
    }

    #[test]
    fn event_listener_receives_created_event() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");

        let received = Arc::new(AtomicBool::new(false));
        let received_clone = Arc::clone(&received);

        struct TestListener {
            received: Arc<AtomicBool>,
        }
        impl crate::DownloadEventListener for TestListener {
            fn on_event(&self, _event: DownloadEventDto) {
                self.received.store(true, Ordering::SeqCst);
            }
        }

        svc.add_event_listener(Box::new(TestListener {
            received: received_clone,
        }));

        svc.add_download(
            "https://example.com/file.zip".to_string(),
            "file.zip".to_string(),
            "/tmp/file.zip".to_string(),
            DownloadPriorityDto::Normal,
        )
        .expect("add_download failed");

        // Give the async event dispatch task time to fire.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !received.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            received.load(Ordering::SeqCst),
            "event listener did not receive Created event within 2 seconds"
        );
        svc.shutdown();
    }
}

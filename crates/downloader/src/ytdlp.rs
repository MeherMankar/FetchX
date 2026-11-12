use std::{
    ffi::OsString,
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};

use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
    YtDlpAudioFormat,
};
use fetchx_storage::DownloadRepository;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, timeout},
};
use tracing::{info, warn};

use crate::{
    BandwidthControl, DownloadEngine, DownloadEngineError, ManagedDownloadControl,
    ManagedDownloadEngine, ResourceDemand, is_dash_url,
};

// Cold Python/packaged-runtime startup can exceed five seconds on Windows and
// on first launch in Android's embedded runtime. Keep validation bounded, but
// leave enough time for a healthy binary to initialize.
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const FORMAT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
const VERSION_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const PROCESS_KILL_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_VERSION_OUTPUT_BYTES: usize = 128;
const MAX_FORMAT_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_DISCOVERED_FORMATS: usize = 100;
const MAX_PROGRESS_LINE_BYTES: usize = 1024;
const PROGRESS_PREFIX: &str = "FETCHX_PROGRESS|";
const PROGRESS_TEMPLATE: &str = "download:FETCHX_PROGRESS|%(progress._percent_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s";
const AUTO_FORMAT_SELECTOR: &str = "bestvideo*+bestaudio/best";
const MAX_PROGRESS_EVENTS: usize = 32;
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_millis(250);

pub type YtDlpRunnerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct YtDlpConfig {
    executable: Option<PathBuf>,
    version_timeout: Duration,
    version_cache_path: Option<PathBuf>,
}

impl Default for YtDlpConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl YtDlpConfig {
    pub fn new() -> Self {
        Self {
            executable: None,
            version_timeout: VERSION_TIMEOUT,
            version_cache_path: None,
        }
    }

    pub fn with_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.executable = Some(executable.into());
        self
    }

    pub fn with_version_timeout(mut self, timeout: Duration) -> Self {
        self.version_timeout = timeout;
        self
    }

    /// Caches successful version checks for the same unchanged executable.
    /// The cache is advisory: unreadable or stale data always triggers validation.
    pub fn with_version_cache_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.version_cache_path = Some(path.into());
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YtDlpProcessExit {
    pub success: bool,
    pub code: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YtDlpProgress {
    pub percentage_basis_points: Option<u16>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: Option<u64>,
    pub estimated_remaining_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct YtDlpFormat {
    pub format_id: String,
    pub label: String,
    pub extension: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub title: Option<String>,
    pub platform: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct YtDlpFinalizationRecord {
    output_name: String,
    output_size: u64,
}

#[derive(Deserialize, Serialize)]
struct YtDlpVersionCacheRecord {
    executable_path: PathBuf,
    executable_size: u64,
    executable_modified_unix_ms: Option<u64>,
    validated_at_unix_secs: u64,
    version: String,
}

struct YtDlpProgressState {
    downloaded_bytes: u64,
    allows_component_reset: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum YtDlpRunnerEvent {
    ProgressLine(String),
    ProcessFailed(String),
    ProgressOutputTooLarge,
    OutputReadFailed,
}

#[derive(Debug, Error)]
pub enum YtDlpError {
    #[error("yt-dlp executable could not be found")]
    ExecutableNotFound,
    #[error("yt-dlp executable is not a regular file")]
    ExecutableInvalid,
    #[error("yt-dlp executable is not executable")]
    ExecutableNotRunnable,
    #[error("yt-dlp executable failed to start")]
    ExecutableStart,
    #[error("yt-dlp version validation timed out")]
    VersionTimeout,
    #[error("yt-dlp version validation returned an invalid response")]
    InvalidVersion,
    #[error("yt-dlp rejected the input URL")]
    InvalidUrl,
    #[error("yt-dlp process exited with status {code:?}")]
    ProcessFailure { code: Option<i32> },
    #[error("yt-dlp progress output was malformed")]
    MalformedProgress,
    #[error("yt-dlp progress output exceeded its size limit")]
    ProgressOutputTooLarge,
    #[error("yt-dlp progress output could not be read")]
    ProgressOutputRead,
    #[error("yt-dlp runtime failed: {0}")]
    RuntimeFailure(String),
    #[error("yt-dlp progress regressed below persisted progress")]
    ProgressRegression,
    #[error("yt-dlp produced no unambiguous regular output file")]
    OutputUnavailable,
    #[error("yt-dlp output destination already exists")]
    DestinationExists,
    #[error("yt-dlp output workspace could not be used safely")]
    WorkspaceUnavailable,
    #[error("yt-dlp process could not be stopped and reaped")]
    ProcessCleanup,
    #[error("active yt-dlp transfers cannot be paused")]
    PauseUnsupported,
    #[error("yt-dlp process runner failed")]
    ProcessRunner,
    #[error("yt-dlp format discovery is not supported by this runtime")]
    FormatDiscoveryUnsupported,
    #[error("yt-dlp format discovery timed out")]
    FormatDiscoveryTimeout,
    #[error("yt-dlp format discovery output exceeded its size limit")]
    FormatOutputTooLarge,
    #[error("yt-dlp format discovery returned invalid data")]
    InvalidFormatOutput,
    #[error("yt-dlp audio extraction destination must use the selected audio extension")]
    AudioDestinationExtensionMismatch,
}

impl YtDlpError {
    fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::ExecutableNotFound => "executable_not_found",
            Self::ExecutableInvalid => "executable_invalid",
            Self::ExecutableNotRunnable => "executable_not_runnable",
            Self::ExecutableStart => "executable_start",
            Self::VersionTimeout => "version_timeout",
            Self::InvalidVersion => "invalid_version",
            Self::InvalidUrl => "invalid_url",
            Self::ProcessFailure { .. } => "process_failure",
            Self::MalformedProgress => "malformed_progress",
            Self::ProgressOutputTooLarge => "progress_output_too_large",
            Self::ProgressOutputRead => "progress_output_read",
            Self::RuntimeFailure(_) => "runtime_failure",
            Self::ProgressRegression => "progress_regression",
            Self::OutputUnavailable => "output_unavailable",
            Self::DestinationExists => "destination_exists",
            Self::WorkspaceUnavailable => "workspace_unavailable",
            Self::ProcessCleanup => "process_cleanup",
            Self::PauseUnsupported => "pause_unsupported",
            Self::ProcessRunner => "process_runner",
            Self::FormatDiscoveryUnsupported => "format_discovery_unsupported",
            Self::FormatDiscoveryTimeout => "format_discovery_timeout",
            Self::FormatOutputTooLarge => "format_output_too_large",
            Self::InvalidFormatOutput => "invalid_format_output",
            Self::AudioDestinationExtensionMismatch => "audio_destination_extension_mismatch",
        }
    }
}

pub trait YtDlpProcessRunner: Send + Sync {
    fn report_diagnostic(&self, _code: &'static str) {}

    fn report_diagnostic_details(&self, _details: String) {}

    fn validate_version<'a>(
        &'a self,
        executable: &'a Path,
        timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>>;

    fn run<'a>(
        &'a self,
        executable: &'a Path,
        arguments: &'a [OsString],
        stop: watch::Receiver<bool>,
        events: mpsc::Sender<YtDlpRunnerEvent>,
    ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>>;

    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        _url: &'a str,
        _timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
        Box::pin(async { Err(YtDlpError::FormatDiscoveryUnsupported) })
    }
}

pub struct YtDlpEngine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    executable: PathBuf,
    version: String,
    runner: Arc<dyn YtDlpProcessRunner>,
}

impl<R> YtDlpEngine<R>
where
    R: DownloadRepository + Clone,
{
    pub async fn initialize(repository: R, config: YtDlpConfig) -> Result<Self, YtDlpError> {
        Self::initialize_with_runner(repository, config, Arc::new(TokioYtDlpRunner)).await
    }

    pub async fn initialize_with_runner(
        repository: R,
        config: YtDlpConfig,
        runner: Arc<dyn YtDlpProcessRunner>,
    ) -> Result<Self, YtDlpError> {
        let executable = resolve_executable(config.executable.as_deref()).await?;
        Self::initialize_with_runner_and_executable(repository, config, executable, runner).await
    }

    pub async fn initialize_with_runner_and_executable(
        repository: R,
        config: YtDlpConfig,
        executable: impl Into<PathBuf>,
        runner: Arc<dyn YtDlpProcessRunner>,
    ) -> Result<Self, YtDlpError> {
        if config.version_timeout.is_zero() {
            return Err(YtDlpError::InvalidVersion);
        }
        let executable = executable.into();
        let version = validate_version_cached(
            &runner,
            &executable,
            config.version_timeout,
            config.version_cache_path.as_deref(),
        )
        .await?;
        if version.is_empty() || version.len() > MAX_VERSION_OUTPUT_BYTES {
            return Err(YtDlpError::InvalidVersion);
        }
        Ok(Self {
            state_machine: DownloadStateMachine::new(repository.clone()),
            repository,
            executable,
            version,
            runner,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub async fn discover_formats(&self, url: &str) -> Result<Vec<YtDlpFormat>, YtDlpError> {
        fetchx_media_extractor::is_safe_media_url(url).map_err(|_| YtDlpError::InvalidUrl)?;
        self.runner
            .discover_formats(&self.executable, url, FORMAT_DISCOVERY_TIMEOUT)
            .await
    }

    async fn download_controlled(
        &self,
        id: DownloadId,
        mut control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if current.source().engine() != EngineKind::YtDlp
            && !(current.source().engine() == EngineKind::DirectHttp
                && is_dash_url(current.source().url()))
        {
            return Err(DownloadEngineError::EngineMismatch);
        }

        let mut item = match current.status() {
            DownloadStatus::Queued | DownloadStatus::Failed | DownloadStatus::Cancelled => {
                self.state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?
            }
            DownloadStatus::Paused => {
                self.state_machine
                    .transition(id, DownloadStatus::Downloading)
                    .await?
            }
            status => {
                return Err(DownloadEngineError::StateMachine(
                    StateMachineError::InvalidTransition {
                        from: status,
                        to: DownloadStatus::Preparing,
                    },
                ));
            }
        };
        let result = self.run_job(id, &mut item, &mut control).await;
        match result {
            Ok(completed) => Ok(completed),
            Err(DownloadEngineError::Cancelled) => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "yt-dlp download cancelled");
                Err(DownloadEngineError::Cancelled)
            }
            Err(error) => {
                self.runner
                    .report_diagnostic(download_failure_diagnostic_code(&error));
                if let Err(persistence_error) = self.persist_failure(id, &error).await {
                    self.runner.report_diagnostic(match &persistence_error {
                        DownloadEngineError::StateMachine(_) => "failure_state_machine",
                        DownloadEngineError::Storage(_) => "failure_storage",
                        _ => "failure_state_persist",
                    });
                    return Err(persistence_error);
                }
                warn!(download_id = %id, "yt-dlp download failed");
                Err(error)
            }
        }
    }

    async fn run_job(
        &self,
        id: DownloadId,
        item: &mut DownloadItem,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if *control.pause.borrow_and_update() {
            return Err(YtDlpError::PauseUnsupported.into());
        }
        let url = validate_source_url(item.source().url())?;
        let workspace = workspace_path(item.destination(), id);
        prepare_workspace(&workspace).await?;
        if Box::pin(self.recover_finalization(item, &workspace)).await? {
            return Ok(item.clone());
        }
        match tokio::fs::symlink_metadata(item.destination()).await {
            Ok(_) => return Err(YtDlpError::DestinationExists.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }
        if control.bandwidth.external_download_rate_bytes_per_second == Some(0) {
            return Err(DownloadEngineError::InvalidConfiguration);
        }
        let audio_format = item.source().yt_dlp_audio_format();
        validate_audio_destination(item.destination(), audio_format)?;
        let arguments = build_arguments(
            &workspace,
            url.as_str(),
            item.source().yt_dlp_format(),
            audio_format,
            control.bandwidth.external_download_rate_bytes_per_second,
        );
        let allows_component_reset = audio_format.is_some()
            || format_selector(item.source().yt_dlp_format())
                .split('/')
                .any(|alternative| alternative.contains('+'));
        report_resource_demand(control, id, 1, 0).await?;

        if item.status() == DownloadStatus::Preparing {
            *item = match self
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await
            {
                Ok(item) => item,
                Err(error) => {
                    self.runner
                        .report_diagnostic(state_machine_diagnostic_code("start", &error));
                    return Err(error.into());
                }
            };
        }
        let mut expected_updated_at = item.updated_at();
        let mut last_persisted_at = Instant::now();
        let mut progress = YtDlpProgressState {
            downloaded_bytes: item.progress().downloaded_bytes(),
            allows_component_reset,
        };
        let (event_sender, mut events) = mpsc::channel(MAX_PROGRESS_EVENTS);
        let mut events_open = true;
        let (stop_sender, stop) = watch::channel(false);
        let run = self
            .runner
            .run(&self.executable, &arguments, stop, event_sender);
        tokio::pin!(run);
        report_resource_demand(control, id, 1, 1).await?;

        let exit = loop {
            tokio::select! {
                biased;
                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        drop(events);
                        let _ = stop_sender.send(true);
                        if let Err(error) = run.await {
                            return Err(error.into());
                        }
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                changed = control.pause.changed() => {
                    if changed.is_err() || *control.pause.borrow_and_update() {
                        drop(events);
                        let _ = stop_sender.send(true);
                        if let Err(error) = run.await {
                            return Err(error.into());
                        }
                        return Err(YtDlpError::PauseUnsupported.into());
                    }
                }
                event = events.recv(), if events_open => {
                    match event {
                        Some(event) => {
                            if let Err(error) = self.handle_runner_event(
                                event,
                                item,
                                &mut expected_updated_at,
                                &mut last_persisted_at,
                                &mut progress,
                                control,
                            ).await {
                                drop(events);
                                let _ = stop_sender.send(true);
                                match run.await {
                                    Ok(exit) => {
                                        self.runner.report_diagnostic_details(format!(
                                            "progress event error; process_success={}; process_exit_code={:?}; error={error}",
                                            exit.success, exit.code
                                        ));
                                        warn!(
                                            download_id = %id,
                                            process_success = exit.success,
                                            process_exit_code = ?exit.code,
                                            error = %error,
                                            "yt-dlp process stopped after a progress event error"
                                        );
                                    }
                                    Err(cleanup_error) => {
                                        return Err(cleanup_error.into());
                                    }
                                }
                                return Err(error);
                            }
                        }
                        None => events_open = false,
                    }
                }
                result = &mut run => {
                    let exit = result?;
                    while let Ok(event) = events.try_recv() {
                        self.handle_runner_event(
                            event,
                            item,
                            &mut expected_updated_at,
                            &mut last_persisted_at,
                            &mut progress,
                            control,
                        ).await?;
                    }
                    break exit;
                },
            }
        };
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if !exit.success {
            return Err(YtDlpError::ProcessFailure { code: exit.code }.into());
        }

        let output = find_output(&workspace).await?;
        let metadata = tokio::fs::symlink_metadata(&output).await?;
        if !metadata.file_type().is_file() {
            return Err(YtDlpError::OutputUnavailable.into());
        }
        let output_size = metadata.len();
        tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&output)
            .await?
            .sync_all()
            .await?;
        let final_downloaded_bytes = if audio_format.is_some() {
            item.progress().downloaded_bytes().max(output_size)
        } else {
            output_size
        };
        let final_progress = YtDlpProgress {
            percentage_basis_points: Some(10_000),
            downloaded_bytes: Some(final_downloaded_bytes),
            total_bytes: Some(final_downloaded_bytes),
            speed_bytes_per_second: None,
            estimated_remaining_seconds: Some(0),
        };
        if audio_format.is_none() && output_size < item.progress().downloaded_bytes() {
            self.runner.report_diagnostic_details(format!(
                "final output smaller than reported progress; process_exit_code={:?}; output_size_bytes={output_size}; reported_downloaded_bytes={}; reported_total_bytes={:?}; automatic_selection={}",
                exit.code,
                item.progress().downloaded_bytes(),
                item.progress().total_bytes(),
                item.source().yt_dlp_format().is_none()
            ));
            warn!(
                download_id = %id,
                process_exit_code = ?exit.code,
                output_size_bytes = output_size,
                reported_downloaded_bytes = item.progress().downloaded_bytes(),
                reported_total_bytes = ?item.progress().total_bytes(),
                automatic_selection = item.source().yt_dlp_format().is_none(),
                "yt-dlp final output is smaller than reported transfer progress"
            );
            return Err(YtDlpError::ProgressRegression.into());
        }
        self.persist_progress(item, &mut expected_updated_at, &final_progress, true)
            .await?;
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        Box::pin(save_finalization_record(&workspace, &output, output_size)).await?;
        promote_output_to_destination(&output, item.destination()).await?;
        *item = match self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await
        {
            Ok(item) => item,
            Err(error) => {
                self.runner
                    .report_diagnostic(state_machine_diagnostic_code("complete", &error));
                return Err(error.into());
            }
        };
        if tokio::fs::remove_dir_all(&workspace).await.is_err() {
            warn!("yt-dlp temporary workspace cleanup failed");
        }
        Ok(item.clone())
    }

    async fn recover_finalization(
        &self,
        item: &mut DownloadItem,
        workspace: &Path,
    ) -> Result<bool, DownloadEngineError> {
        let Some(record) = load_finalization_record(workspace).await? else {
            return Ok(false);
        };
        let output = match find_output(workspace).await {
            Ok(output) => output,
            Err(YtDlpError::OutputUnavailable) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let Some(output_name) = output.file_name().and_then(|name| name.to_str()) else {
            return Ok(false);
        };
        let output_metadata = tokio::fs::symlink_metadata(&output).await?;
        if !output_metadata.file_type().is_file()
            || output_name != record.output_name
            || output_metadata.len() != record.output_size
        {
            return Ok(false);
        }

        match tokio::fs::symlink_metadata(item.destination()).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                if !files_equal(&output, item.destination()).await? {
                    return Err(YtDlpError::DestinationExists.into());
                }
            }
            Ok(_) => return Err(YtDlpError::DestinationExists.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }

        if item.status() == DownloadStatus::Preparing {
            *item = self
                .state_machine
                .transition(item.id(), DownloadStatus::Downloading)
                .await?;
        }
        if item.status() != DownloadStatus::Downloading {
            return Ok(false);
        }
        let mut expected_updated_at = item.updated_at();
        self.persist_progress(
            item,
            &mut expected_updated_at,
            &YtDlpProgress {
                percentage_basis_points: Some(10_000),
                downloaded_bytes: Some(record.output_size),
                total_bytes: Some(record.output_size),
                speed_bytes_per_second: None,
                estimated_remaining_seconds: Some(0),
            },
            true,
        )
        .await?;

        match tokio::fs::symlink_metadata(item.destination()).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                if !files_equal(&output, item.destination()).await? {
                    return Err(YtDlpError::DestinationExists.into());
                }
            }
            Ok(_) => return Err(YtDlpError::DestinationExists.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match promote_output_to_destination(&output, item.destination()).await {
                    Ok(()) => {}
                    Err(DownloadEngineError::YtDlp(YtDlpError::DestinationExists)) => {
                        if !files_equal(&output, item.destination()).await? {
                            return Err(YtDlpError::DestinationExists.into());
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }

        *item = self
            .state_machine
            .transition(item.id(), DownloadStatus::Completed)
            .await?;
        if tokio::fs::remove_dir_all(workspace).await.is_err() {
            warn!("yt-dlp recovered finalization workspace cleanup failed");
        }
        Ok(true)
    }

    async fn persist_progress(
        &self,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        update: &YtDlpProgress,
        allow_total_override: bool,
    ) -> Result<(), DownloadEngineError> {
        let mut downloaded_bytes = update
            .downloaded_bytes
            .ok_or(YtDlpError::MalformedProgress)?;
        if downloaded_bytes < item.progress().downloaded_bytes() {
            return Err(YtDlpError::ProgressRegression.into());
        }
        for _ in 0..3 {
            let total = if allow_total_override {
                update
                    .total_bytes
                    .filter(|total| *total >= downloaded_bytes)
            } else {
                match (item.progress().total_bytes(), update.total_bytes) {
                    (Some(previous_total), _) if previous_total >= downloaded_bytes => {
                        Some(previous_total)
                    }
                    (_, Some(candidate_total)) if candidate_total >= downloaded_bytes => {
                        Some(candidate_total)
                    }
                    _ => None,
                }
            };
            item.set_progress_at(
                DownloadProgress::new(
                    downloaded_bytes,
                    total,
                    update.speed_bytes_per_second,
                    update.estimated_remaining_seconds,
                )?,
                next_timestamp(item.updated_at())?,
            )?;
            if self
                .repository
                .update_if_status_and_timestamp_current(
                    item,
                    DownloadStatus::Downloading,
                    *expected_updated_at,
                )
                .await?
            {
                *expected_updated_at = item.updated_at();
                return Ok(());
            }

            let Some(current) = self.repository.get(item.id()).await? else {
                let error = StateMachineError::NotFound(item.id());
                self.runner
                    .report_diagnostic(state_machine_diagnostic_code("progress", &error));
                return Err(DownloadEngineError::StateMachine(error));
            };
            if current.status() != DownloadStatus::Downloading {
                let error = StateMachineError::ConcurrentTransition {
                    from: DownloadStatus::Downloading,
                    to: current.status(),
                    actual: current.status(),
                };
                self.runner
                    .report_diagnostic(state_machine_diagnostic_code("progress", &error));
                return Err(DownloadEngineError::StateMachine(error));
            }
            downloaded_bytes = downloaded_bytes.max(current.progress().downloaded_bytes());
            *expected_updated_at = current.updated_at();
            *item = current;
        }

        let error = self.current_state_error(item.id()).await?;
        self.runner
            .report_diagnostic(state_machine_diagnostic_code("progress", &error));
        Err(DownloadEngineError::StateMachine(error))
    }

    async fn handle_runner_event(
        &self,
        event: YtDlpRunnerEvent,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        last_persisted_at: &mut Instant,
        progress: &mut YtDlpProgressState,
        control: &mut ManagedDownloadControl,
    ) -> Result<(), DownloadEngineError> {
        let update = match event {
            YtDlpRunnerEvent::ProgressLine(line) => match parse_progress_line(&line) {
                Ok(progress) => progress,
                Err(error) => {
                    warn!(%line, %error, "ignoring unparseable yt-dlp progress line");
                    return Ok(());
                }
            },
            YtDlpRunnerEvent::ProcessFailed(message) => {
                return Err(YtDlpError::RuntimeFailure(message).into());
            }
            YtDlpRunnerEvent::ProgressOutputTooLarge => {
                return Err(YtDlpError::ProgressOutputTooLarge.into());
            }
            YtDlpRunnerEvent::OutputReadFailed => {
                return Err(YtDlpError::ProgressOutputRead.into());
            }
        };
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        let Some(downloaded_bytes) = update.downloaded_bytes else {
            return Ok(());
        };
        if downloaded_bytes < progress.downloaded_bytes {
            let details = format!(
                "progress reset; previous_downloaded_bytes={}; incoming_downloaded_bytes={downloaded_bytes}; previous_total_bytes={:?}; incoming_total_bytes={:?}; incoming_percentage_basis_points={:?}; component_selector={}",
                progress.downloaded_bytes,
                item.progress().total_bytes(),
                update.total_bytes,
                update.percentage_basis_points,
                progress.allows_component_reset
            );
            self.runner.report_diagnostic_details(details);
            warn!(
                download_id = %item.id(),
                previous_downloaded_bytes = progress.downloaded_bytes,
                incoming_downloaded_bytes = downloaded_bytes,
                previous_total_bytes = ?item.progress().total_bytes(),
                incoming_total_bytes = ?update.total_bytes,
                incoming_percentage_basis_points = ?update.percentage_basis_points,
                automatic_selection = item.source().yt_dlp_format().is_none(),
                "yt-dlp progress event regressed"
            );
            if progress.allows_component_reset {
                return Ok(());
            }
            return Err(YtDlpError::ProgressRegression.into());
        }

        let now = Instant::now();
        let changed_progress = downloaded_bytes != progress.downloaded_bytes;
        progress.downloaded_bytes = downloaded_bytes;
        if changed_progress && now.duration_since(*last_persisted_at) >= PROGRESS_PERSIST_INTERVAL {
            self.persist_progress(item, expected_updated_at, &update, false)
                .await?;
            *last_persisted_at = now;
        }
        Ok(())
    }

    async fn current_state_error(
        &self,
        id: DownloadId,
    ) -> Result<StateMachineError, DownloadEngineError> {
        match self.repository.get(id).await? {
            None => Ok(StateMachineError::NotFound(id)),
            Some(item) => Ok(StateMachineError::ConcurrentTransition {
                from: DownloadStatus::Downloading,
                to: item.status(),
                actual: item.status(),
            }),
        }
    }

    async fn transition_to_cancelled(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        match self
            .state_machine
            .transition(id, DownloadStatus::Cancelled)
            .await
        {
            Ok(_) => Ok(()),
            Err(StateMachineError::InvalidTransition {
                from: DownloadStatus::Cancelled,
                to: DownloadStatus::Cancelled,
            }) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn persist_failure(
        &self,
        id: DownloadId,
        operation: &DownloadEngineError,
    ) -> Result<(), DownloadEngineError> {
        let error_repr = match operation.persisted_error() {
            Ok(err) => err,
            Err(_) => fetchx_models::DownloadError::new(
                fetchx_models::DownloadErrorCode::Engine,
                "The download failed.",
            )
            .map_err(DownloadEngineError::DomainError)?,
        };
        for _ in 0..3 {
            let current = match self.repository.get(id).await? {
                Some(item) => item,
                None => return Ok(()),
            };
            if !matches!(
                current.status(),
                DownloadStatus::Preparing | DownloadStatus::Downloading
            ) {
                return Ok(());
            }
            match self
                .state_machine
                .transition_with_error(id, DownloadStatus::Failed, error_repr.clone())
                .await
            {
                Ok(_) => return Ok(()),
                Err(StateMachineError::ConcurrentTransition { .. }) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }
        if let Some(current) = self.repository.get(id).await?
            && matches!(
                current.status(),
                DownloadStatus::Preparing | DownloadStatus::Downloading
            )
        {
            return Err(StateMachineError::ConcurrentTransition {
                from: current.status(),
                to: DownloadStatus::Failed,
                actual: current.status(),
            }
            .into());
        }
        Ok(())
    }
}

async fn validate_version_cached(
    runner: &Arc<dyn YtDlpProcessRunner>,
    executable: &Path,
    timeout: Duration,
    cache_path: Option<&Path>,
) -> Result<String, YtDlpError> {
    let identity = if cache_path.is_some() {
        executable_identity(executable).await
    } else {
        None
    };
    if let (Some(cache_path), Some((size, modified_ms))) = (cache_path, identity)
        && let Some(record) = read_version_cache(cache_path).await
        && record.executable_path == executable
        && record.executable_size == size
        && record.executable_modified_unix_ms == modified_ms
        && record.version.len() <= MAX_VERSION_OUTPUT_BYTES
        && !record.version.trim().is_empty()
        && unix_seconds()
            .and_then(|now| now.checked_sub(record.validated_at_unix_secs))
            .is_some_and(|age| age <= VERSION_CACHE_TTL.as_secs())
    {
        return Ok(record.version);
    }

    let version = runner.validate_version(executable, timeout).await?;
    if let (Some(cache_path), Some((executable_size, executable_modified_unix_ms))) =
        (cache_path, identity)
        && !version.trim().is_empty()
        && version.len() <= MAX_VERSION_OUTPUT_BYTES
        && let Some(validated_at_unix_secs) = unix_seconds()
    {
        let record = YtDlpVersionCacheRecord {
            executable_path: executable.to_path_buf(),
            executable_size,
            executable_modified_unix_ms,
            validated_at_unix_secs,
            version: version.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&record) {
            // A partial write simply invalidates the advisory cache next time.
            let _ = tokio::fs::write(cache_path, bytes).await;
        }
    }
    Ok(version)
}

async fn executable_identity(executable: &Path) -> Option<(u64, Option<u64>)> {
    let metadata = tokio::fs::metadata(executable).await.ok()?;
    let modified_ms = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok());
    Some((metadata.len(), modified_ms))
}

async fn read_version_cache(path: &Path) -> Option<YtDlpVersionCacheRecord> {
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn unix_seconds() -> Option<u64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(duration.as_secs())
}

fn download_failure_diagnostic_code(error: &DownloadEngineError) -> &'static str {
    match error {
        DownloadEngineError::YtDlp(error) => error.diagnostic_code(),
        DownloadEngineError::StateMachine(error) => match error {
            StateMachineError::NotFound(_) => "state_machine_not_found",
            StateMachineError::InvalidTransition { .. } => "state_machine_invalid_transition",
            StateMachineError::ConcurrentTransition { .. } => "state_machine_concurrent_transition",
            StateMachineError::Timestamp(_) => "state_machine_timestamp",
            StateMachineError::Domain(_) => "state_machine_domain",
            StateMachineError::DomainError(_) => "state_machine_domain_error",
            StateMachineError::Storage(_) => "state_machine_storage",
        },
        DownloadEngineError::Storage(_) => "storage",
        DownloadEngineError::FailurePersistence { .. } => "failure_persistence",
        DownloadEngineError::Io(_) => "file_io",
        DownloadEngineError::ResourceManagerUnavailable => "resource_manager",
        DownloadEngineError::ClientConfiguration(_) => "client_configuration",
        DownloadEngineError::InvalidConfiguration => "invalid_configuration",
        DownloadEngineError::Progress(_) => "progress",
        DownloadEngineError::DomainError(_) => "domain_error",
        DownloadEngineError::Domain(_) => "domain",
        DownloadEngineError::Timestamp(_) => "timestamp",
        _ => "download_engine",
    }
}

fn state_machine_diagnostic_code(stage: &'static str, error: &StateMachineError) -> &'static str {
    match (stage, error) {
        ("start", StateMachineError::ConcurrentTransition { .. }) => "start_concurrent_transition",
        ("start", StateMachineError::InvalidTransition { .. }) => "start_invalid_transition",
        ("progress", StateMachineError::ConcurrentTransition { .. }) => {
            "progress_concurrent_transition"
        }
        ("progress", StateMachineError::InvalidTransition { .. }) => "progress_invalid_transition",
        ("complete", StateMachineError::ConcurrentTransition { .. }) => {
            "complete_concurrent_transition"
        }
        ("complete", StateMachineError::InvalidTransition { .. }) => "complete_invalid_transition",
        (_, StateMachineError::Storage(_)) => "stage_state_storage",
        _ => "stage_state_transition",
    }
}

impl<R> DownloadEngine for YtDlpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let (pause_sender, pause) = watch::channel(false);
        let (limit_sender, connection_limit) = watch::channel(1);
        let result = self
            .download_controlled(
                id,
                ManagedDownloadControl {
                    cancellation,
                    pause,
                    connection_limit,
                    resource_demand: None,
                    bandwidth: BandwidthControl::default(),
                },
            )
            .await;
        drop((pause_sender, limit_sender));
        result
    }
}

impl<R> ManagedDownloadEngine for YtDlpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        self.download_controlled(id, control).await
    }

    fn supports_active_pause(&self, engine: EngineKind) -> bool {
        engine != EngineKind::YtDlp
    }
}

pub fn parse_progress_line(line: &str) -> Result<YtDlpProgress, YtDlpError> {
    let body = line
        .strip_prefix(PROGRESS_PREFIX)
        .ok_or(YtDlpError::MalformedProgress)?;
    let mut fields = body.split('|');
    let percentage = fields.next().ok_or(YtDlpError::MalformedProgress)?;
    let downloaded = fields.next().ok_or(YtDlpError::MalformedProgress)?;
    let total = fields.next().ok_or(YtDlpError::MalformedProgress)?;
    let estimate = fields.next().ok_or(YtDlpError::MalformedProgress)?;
    let speed = fields.next().ok_or(YtDlpError::MalformedProgress)?;
    let eta = fields.next().ok_or(YtDlpError::MalformedProgress)?;
    if fields.next().is_some() {
        return Err(YtDlpError::MalformedProgress);
    }
    let percentage_basis_points = parse_percentage(percentage)?;
    let downloaded_bytes = parse_optional_u64(downloaded)?;
    let total_bytes = match parse_optional_u64(total)? {
        Some(total) => Some(total),
        None => parse_optional_estimate(estimate)?,
    };
    let speed_bytes_per_second = parse_optional_float(speed)?;
    let estimated_remaining_seconds = parse_optional_estimate(eta)?;
    Ok(YtDlpProgress {
        percentage_basis_points,
        downloaded_bytes,
        total_bytes,
        speed_bytes_per_second,
        estimated_remaining_seconds,
    })
}

fn parse_percentage(value: &str) -> Result<Option<u16>, YtDlpError> {
    let value = value.trim().trim_end_matches('%').trim();
    if is_unknown(value) {
        return Ok(None);
    }
    let percentage = value
        .parse::<f64>()
        .map_err(|_| YtDlpError::MalformedProgress)?;
    if !percentage.is_finite() || !(0.0..=100.0).contains(&percentage) {
        return Err(YtDlpError::MalformedProgress);
    }
    Ok(Some((percentage * 100.0).round() as u16))
}

fn parse_optional_u64(value: &str) -> Result<Option<u64>, YtDlpError> {
    let value = value.trim();
    if is_unknown(value) {
        return Ok(None);
    }
    if let Ok(val) = value.parse::<u64>() {
        return Ok(Some(val));
    }
    let float_val = value
        .parse::<f64>()
        .map_err(|_| YtDlpError::MalformedProgress)?;
    if !float_val.is_finite() || float_val < 0.0 || float_val > (u64::MAX - 1024) as f64 {
        return Err(YtDlpError::MalformedProgress);
    }
    Ok(Some(float_val.floor() as u64))
}

fn parse_optional_estimate(value: &str) -> Result<Option<u64>, YtDlpError> {
    let value = value.trim();
    if is_unknown(value) {
        return Ok(None);
    }
    if let Ok(estimate) = value.parse::<u64>() {
        return Ok(Some(estimate));
    }
    let estimate = value
        .parse::<f64>()
        .map_err(|_| YtDlpError::MalformedProgress)?;
    if !estimate.is_finite() || estimate < 0.0 || estimate > (u64::MAX - 1024) as f64 {
        return Err(YtDlpError::MalformedProgress);
    }
    Ok(Some(estimate.ceil() as u64))
}

fn parse_optional_float(value: &str) -> Result<Option<u64>, YtDlpError> {
    let value = value.trim();
    if is_unknown(value) {
        return Ok(None);
    }
    let value = value
        .parse::<f64>()
        .map_err(|_| YtDlpError::MalformedProgress)?;
    // Use a conservative upper bound: u64::MAX is not exactly representable as f64,
    // so subtract a margin to avoid undefined cast behaviour near the boundary.
    if !value.is_finite() || value < 0.0 || value > (u64::MAX - 1024) as f64 {
        return Err(YtDlpError::MalformedProgress);
    }
    // floor() rather than round() so sub-1-byte/s speeds truncate to 0 rather than
    // rounding up, which is a more honest representation.
    Ok(Some(value.floor() as u64))
}

fn is_unknown(value: &str) -> bool {
    value.is_empty() || matches!(value, "NA" | "None" | "none" | "N/A")
}

fn build_arguments(
    workspace: &Path,
    url: &str,
    selected_format: Option<&str>,
    audio_format: Option<YtDlpAudioFormat>,
    max_download_rate_bytes_per_second: Option<u64>,
) -> Vec<OsString> {
    let selected_format = if audio_format.is_some() {
        "bestaudio/best"
    } else {
        format_selector(selected_format)
    };
    let mut arguments = vec![
        "--ignore-config".into(),
        "--no-plugin-dirs".into(),
        "--no-colors".into(),
        "--no-playlist".into(),
        "--force-overwrites".into(),
        "--continue".into(),
        "--no-cache-dir".into(),
        "--no-write-thumbnail".into(),
        "--no-write-info-json".into(),
        "--no-write-comments".into(),
        "--format".into(),
        selected_format.into(),
        "--concurrent-fragments".into(),
        "16".into(),
        "--hls-prefer-native".into(),
        "--newline".into(),
        "--progress-template".into(),
        PROGRESS_TEMPLATE.into(),
        "--paths".into(),
    ];
    let mut home_path = OsString::from("home:");
    home_path.push(workspace.as_os_str());
    arguments.push(home_path);
    if let Some(audio_format) = audio_format {
        arguments.extend([
            "--extract-audio".into(),
            "--audio-format".into(),
            audio_format.as_str().into(),
        ]);
    }
    if let Some(rate) = max_download_rate_bytes_per_second {
        arguments.push("--limit-rate".into());
        arguments.push(rate.to_string().into());
    }
    arguments.extend([
        "--output".into(),
        "%(id)s.%(ext)s".into(),
        "--".into(),
        url.into(),
    ]);
    arguments
}

fn validate_audio_destination(
    destination: &Path,
    audio_format: Option<YtDlpAudioFormat>,
) -> Result<(), YtDlpError> {
    if audio_format.is_some_and(|format| {
        !destination
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()))
    }) {
        return Err(YtDlpError::AudioDestinationExtensionMismatch);
    }
    Ok(())
}

fn format_selector(selected_format: Option<&str>) -> &str {
    selected_format.unwrap_or(AUTO_FORMAT_SELECTOR)
}

pub fn parse_format_output(bytes: &[u8]) -> Result<Vec<YtDlpFormat>, YtDlpError> {
    if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
        return Err(YtDlpError::FormatOutputTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| YtDlpError::InvalidFormatOutput)?;
    let formats = value
        .get("formats")
        .and_then(serde_json::Value::as_array)
        .ok_or(YtDlpError::InvalidFormatOutput)?;
    let title = value
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let platform = value
        .get("extractor_key")
        .or_else(|| value.get("extractor"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);

    let mut discovered = Vec::new();
    for format in formats {
        if format.get("has_drm").and_then(serde_json::Value::as_bool) == Some(true)
            || !codec_is_present(format.get("vcodec"))
        {
            continue;
        }
        let format_id_owned;
        let format_id = match format.get("format_id") {
            Some(serde_json::Value::String(s)) => s.as_str(),
            Some(serde_json::Value::Number(n)) => {
                format_id_owned = n.to_string();
                format_id_owned.as_str()
            }
            _ => continue,
        };
        if !is_valid_ytdlp_format_id(format_id) {
            continue;
        }
        let format_id = if codec_is_present(format.get("acodec")) {
            format_id.to_owned()
        } else {
            format!("{}+bestaudio/{}", format_id, format_id)
        };

        let extension = format
            .get("ext")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let height = format.get("height").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
        });
        let resolution = height.map(format_resolution_label).or_else(|| {
            format
                .get("resolution")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty() && *value != "unknown")
                .map(str::to_owned)
        });
        let needs_audio_merge = !codec_is_present(format.get("acodec"));
        let note = format
            .get("format_note")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty());
        let file_size_bytes = format
            .get("filesize")
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                format
                    .get("filesize_approx")
                    .and_then(serde_json::Value::as_u64)
            });
        let mut label = resolution
            .or_else(|| note.map(str::to_owned))
            .unwrap_or_else(|| "Video".to_owned());
        if needs_audio_merge {
            label.push_str(" + audio");
        }
        if let Some(extension) = &extension {
            label.push_str(" - ");
            label.push_str(extension);
        }
        if let Some(size) = file_size_bytes {
            label.push_str(" - ");
            label.push_str(&format_file_size(size));
        }
        discovered.push(YtDlpFormat {
            format_id,
            label,
            extension,
            file_size_bytes,
            title: title.clone(),
            platform: platform.clone(),
        });
        if discovered.len() == MAX_DISCOVERED_FORMATS {
            break;
        }
    }
    Ok(discovered)
}

fn format_resolution_label(height: u64) -> String {
    let (name, threshold) = match height {
        4_320.. => ("8K", 4_320),
        2_160.. => ("4K", 2_160),
        1_440.. => ("2K", 1_440),
        _ => return format!("{height}p"),
    };
    format!("{name} ({threshold}p)")
}

fn codec_is_present(value: Option<&serde_json::Value>) -> bool {
    match value.and_then(serde_json::Value::as_str) {
        // Explicitly absent codec (yt-dlp signals no audio/video track)
        Some("none") | Some("") => false,
        // Codec name present — stream has this track
        Some(_) => true,
        // null / missing field — codec info not reported (common for muxed streams)
        // Treat as present to avoid filtering out valid muxed formats
        None => true,
    }
}

fn is_valid_ytdlp_format_id(format_id: &str) -> bool {
    !format_id.is_empty()
        && format_id.len() <= 128
        && format_id
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && format_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._=-".contains(&byte))
}

fn format_file_size(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn validate_source_url(source: &str) -> Result<reqwest::Url, YtDlpError> {
    let url = reqwest::Url::parse(source).map_err(|_| YtDlpError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(YtDlpError::InvalidUrl);
    }
    Ok(url)
}

async fn resolve_executable(configured: Option<&Path>) -> Result<PathBuf, YtDlpError> {
    let path = if let Some(path) = configured {
        path.to_path_buf()
    } else {
        discover_executable().await?
    };
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| YtDlpError::ExecutableNotFound)?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| YtDlpError::ExecutableInvalid)?;
    if !metadata.is_file() {
        return Err(YtDlpError::ExecutableInvalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(YtDlpError::ExecutableNotRunnable);
        }
    }
    Ok(canonical)
}

async fn discover_executable() -> Result<PathBuf, YtDlpError> {
    let path_value = std::env::var_os("PATH").ok_or(YtDlpError::ExecutableNotFound)?;
    for directory in std::env::split_paths(&path_value) {
        if !directory.is_absolute() {
            continue;
        }
        for name in executable_names() {
            let candidate = directory.join(name);
            if tokio::fs::metadata(&candidate)
                .await
                .is_ok_and(|metadata| metadata.is_file())
            {
                return Ok(candidate);
            }
        }
    }
    Err(YtDlpError::ExecutableNotFound)
}

fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["yt-dlp.exe"]
    } else {
        &["yt-dlp"]
    }
}

fn workspace_path(destination: &Path, id: DownloadId) -> PathBuf {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    parent
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".fetchx-ytdlp-{id}"))
}

fn finalization_record_path(workspace: &Path) -> PathBuf {
    workspace.join(".fetchx-finalizing")
}

fn finalization_record_temporary_path(workspace: &Path) -> PathBuf {
    workspace.join(".fetchx-finalizing.tmp")
}

async fn load_finalization_record(
    workspace: &Path,
) -> Result<Option<YtDlpFinalizationRecord>, YtDlpError> {
    let path = finalization_record_path(workspace);
    let metadata = match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(YtDlpError::WorkspaceUnavailable),
    };
    if !metadata.file_type().is_file() {
        return Err(YtDlpError::WorkspaceUnavailable);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
    match serde_json::from_slice(&bytes) {
        Ok(record) => Ok(Some(record)),
        Err(_) => {
            warn!("ignoring invalid yt-dlp finalization record");
            Ok(None)
        }
    }
}

async fn remove_regular_file_if_present(path: &Path) -> Result<(), YtDlpError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_file() => tokio::fs::remove_file(path)
            .await
            .map_err(|_| YtDlpError::WorkspaceUnavailable),
        Ok(_) => Err(YtDlpError::WorkspaceUnavailable),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(YtDlpError::WorkspaceUnavailable),
    }
}

async fn save_finalization_record(
    workspace: &Path,
    output: &Path,
    output_size: u64,
) -> Result<(), DownloadEngineError> {
    let output_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(YtDlpError::OutputUnavailable)?;
    let bytes = serde_json::to_vec(&YtDlpFinalizationRecord {
        output_name: output_name.to_owned(),
        output_size,
    })
    .map_err(|_| YtDlpError::OutputUnavailable)?;
    let temporary = finalization_record_temporary_path(workspace);
    remove_regular_file_if_present(&temporary).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await?;
    file.write_all(&bytes).await?;
    file.flush().await?;
    file.sync_all().await?;
    remove_regular_file_if_present(&finalization_record_path(workspace)).await?;
    tokio::fs::rename(temporary, finalization_record_path(workspace)).await?;
    Ok(())
}

async fn prepare_workspace(path: &Path) -> Result<(), YtDlpError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_dir() => restrict_workspace_permissions(path).await,
        Ok(_) => Err(YtDlpError::WorkspaceUnavailable),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            tokio::fs::create_dir(path)
                .await
                .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
            restrict_workspace_permissions(path).await
        }
        Err(_) => Err(YtDlpError::WorkspaceUnavailable),
    }
}

#[cfg(unix)]
async fn restrict_workspace_permissions(path: &Path) -> Result<(), YtDlpError> {
    use std::os::unix::fs::PermissionsExt;

    match tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await {
        Ok(()) => Ok(()),
        Err(_) => {
            // Permission changes are unsupported on some filesystems (e.g. Android
            // external storage / FAT32). Treat as non-fatal — the directory is still
            // usable; security is enforced by the Android sandbox instead.
            tracing::warn!(path = %path.display(), "could not restrict workspace permissions (filesystem may not support chmod); continuing");
            Ok(())
        }
    }
}

#[cfg(not(unix))]
async fn restrict_workspace_permissions(_path: &Path) -> Result<(), YtDlpError> {
    Ok(())
}

async fn find_output(workspace: &Path) -> Result<PathBuf, YtDlpError> {
    let mut entries = tokio::fs::read_dir(workspace)
        .await
        .map_err(|_| YtDlpError::OutputUnavailable)?;
    let mut output = None;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| YtDlpError::OutputUnavailable)?
    {
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| YtDlpError::OutputUnavailable)?;
        if !metadata.file_type().is_file() {
            return Err(YtDlpError::OutputUnavailable);
        }
        // Skip yt-dlp temporary/metadata files — these are left in the workspace
        // during and after HLS/DASH downloads:
        //   .part          — the partial download being assembled
        //   .part-FragN.part — individual HLS/DASH fragment in progress
        //   .ytdl          — yt-dlp resume/metadata sidecar
        //   .json          — --write-info-json output (should not appear given our flags)
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".fetchx-finalizing"
            || name == ".fetchx-finalizing.tmp"
            || name.ends_with(".part")
            || name.contains(".part-Frag")
            || name.ends_with(".ytdl")
            || name.ends_with(".json")
        {
            continue;
        }
        if output.is_some() {
            return Err(YtDlpError::OutputUnavailable);
        }
        output = Some(path);
    }
    output.ok_or(YtDlpError::OutputUnavailable)
}

async fn files_equal(left: &Path, right: &Path) -> Result<bool, DownloadEngineError> {
    let left_metadata = tokio::fs::symlink_metadata(left).await?;
    let right_metadata = tokio::fs::symlink_metadata(right).await?;
    if !left_metadata.file_type().is_file()
        || !right_metadata.file_type().is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return Ok(false);
    }

    let mut left_file = tokio::fs::File::open(left).await?;
    let mut right_file = tokio::fs::File::open(right).await?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_read = left_file.read(&mut left_buffer).await?;
        let right_read = right_file.read(&mut right_buffer).await?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

async fn promote_output_to_destination(
    output: &Path,
    destination: &Path,
) -> Result<(), DownloadEngineError> {
    // Try hard link first (zero-copy, atomic on same filesystem).
    match tokio::fs::hard_link(output, destination).await {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(YtDlpError::DestinationExists.into());
        }
        Err(_) => {
            // Hard link failed (cross-device, unsupported FS such as Android external
            // storage). Fall back to copy-then-delete so the download still completes.
        }
    }
    // Copy to destination then remove the source.
    match tokio::fs::copy(output, destination).await {
        Ok(_) => {
            // Best-effort removal of the source; ignore errors.
            let _ = tokio::fs::remove_file(output).await;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(YtDlpError::DestinationExists.into())
        }
        Err(error) => Err(DownloadEngineError::Io(error)),
    }
}

async fn report_resource_demand(
    control: &ManagedDownloadControl,
    download_id: DownloadId,
    desired_concurrency: usize,
    active_concurrency: usize,
) -> Result<(), DownloadEngineError> {
    if let Some(sender) = &control.resource_demand {
        sender
            .send(ResourceDemand {
                download_id,
                desired_concurrency,
                active_concurrency,
            })
            .await
            .map_err(|_| DownloadEngineError::ResourceManagerUnavailable)?;
    }
    Ok(())
}

fn next_timestamp(previous: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    let next = previous
        .as_unix_millis()
        .checked_add(1)
        .ok_or(fetchx_models::TimestampError::OutOfRange)?;
    Ok(Timestamp::now()?.max(Timestamp::from_unix_millis(next)))
}

struct TokioYtDlpRunner;

impl YtDlpProcessRunner for TokioYtDlpRunner {
    fn validate_version<'a>(
        &'a self,
        executable: &'a Path,
        command_timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
        Box::pin(async move {
            let mut command = Command::new(executable);
            command
                .arg("--ignore-config")
                .arg("--no-plugin-dirs")
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            // Suppress the console window that would otherwise flash on Windows.
            #[cfg(windows)]
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let mut child = command.spawn().map_err(|_| YtDlpError::ExecutableStart)?;
            let result = timeout(command_timeout, async {
                let stdout = child.stdout.take().ok_or(YtDlpError::InvalidVersion)?;
                let mut bytes = Vec::new();
                stdout
                    .take((MAX_VERSION_OUTPUT_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|_| YtDlpError::InvalidVersion)?;
                let status = child.wait().await.map_err(|_| YtDlpError::InvalidVersion)?;
                if !status.success() || bytes.is_empty() || bytes.len() > MAX_VERSION_OUTPUT_BYTES {
                    return Err(YtDlpError::InvalidVersion);
                }
                let version = String::from_utf8(bytes).map_err(|_| YtDlpError::InvalidVersion)?;
                Ok(version.trim().to_owned())
            })
            .await;
            match result {
                Ok(result) => result,
                Err(_) => {
                    terminate_child(&mut child).await?;
                    Err(YtDlpError::VersionTimeout)
                }
            }
        })
    }

    fn run<'a>(
        &'a self,
        executable: &'a Path,
        arguments: &'a [OsString],
        mut stop: watch::Receiver<bool>,
        events: mpsc::Sender<YtDlpRunnerEvent>,
    ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
        Box::pin(async move {
            if *stop.borrow_and_update() {
                return Err(YtDlpError::ProcessCleanup);
            }
            let mut command = Command::new(executable);
            command
                .args(arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            // Suppress the console window that would otherwise flash on Windows.
            #[cfg(windows)]
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let mut child = command.spawn().map_err(|_| YtDlpError::ExecutableStart)?;
            let stdout = child.stdout.take().ok_or(YtDlpError::ProcessRunner)?;
            let stderr = child.stderr.take().ok_or(YtDlpError::ProcessRunner)?;
            let stdout_events = events.clone();
            let stderr_events = events;
            let stdout_reader = tokio::spawn(read_progress(stdout, stdout_events));
            let stderr_reader = tokio::spawn(read_progress(stderr, stderr_events));
            let exit = tokio::select! {
                biased;
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow_and_update() {
                        terminate_child(&mut child).await?
                    } else {
                        child.wait().await.map_err(|_| YtDlpError::ProcessRunner)?
                    }
                }
                status = child.wait() => status.map_err(|_| YtDlpError::ProcessRunner)?,
            };
            let stdout_result = finish_reader(stdout_reader).await;
            let stderr_result = finish_reader(stderr_reader).await;
            stdout_result?;
            stderr_result?;
            Ok(YtDlpProcessExit {
                success: exit.success(),
                code: exit.code(),
            })
        })
    }

    fn discover_formats<'a>(
        &'a self,
        executable: &'a Path,
        url: &'a str,
        command_timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
        Box::pin(async move {
            let mut command = Command::new(executable);
            command
                .args([
                    "--ignore-config",
                    "--no-plugin-dirs",
                    "--no-warnings",
                    "--quiet",
                    "--skip-download",
                    "--no-playlist",
                    "--dump-single-json",
                    "--",
                    url,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            // Suppress the console window that would otherwise flash on Windows.
            #[cfg(windows)]
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let mut child = command.spawn().map_err(|_| YtDlpError::ExecutableStart)?;
            let result = timeout(command_timeout, async {
                let stdout = child.stdout.take().ok_or(YtDlpError::ProcessRunner)?;
                let mut bytes = Vec::new();
                stdout
                    .take((MAX_FORMAT_OUTPUT_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|_| YtDlpError::ProcessRunner)?;
                if bytes.len() > MAX_FORMAT_OUTPUT_BYTES {
                    terminate_child(&mut child).await?;
                    return Err(YtDlpError::FormatOutputTooLarge);
                }
                let status = child.wait().await.map_err(|_| YtDlpError::ProcessRunner)?;
                if !status.success() {
                    return Err(YtDlpError::ProcessFailure {
                        code: status.code(),
                    });
                }
                parse_format_output(&bytes)
            })
            .await;
            match result {
                Ok(result) => result,
                Err(_) => {
                    terminate_child(&mut child).await?;
                    Err(YtDlpError::FormatDiscoveryTimeout)
                }
            }
        })
    }
}

async fn terminate_child(child: &mut Child) -> Result<ExitStatus, YtDlpError> {
    if child.id().is_none() {
        return timeout(PROCESS_KILL_TIMEOUT, child.wait())
            .await
            .map_err(|_| YtDlpError::ProcessCleanup)?
            .map_err(|_| YtDlpError::ProcessCleanup);
    }
    match timeout(PROCESS_KILL_TIMEOUT, child.kill()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == io::ErrorKind::InvalidInput => {}
        Ok(Err(_)) | Err(_) => return Err(YtDlpError::ProcessCleanup),
    }
    timeout(PROCESS_KILL_TIMEOUT, child.wait())
        .await
        .map_err(|_| YtDlpError::ProcessCleanup)?
        .map_err(|_| YtDlpError::ProcessCleanup)
}

async fn finish_reader(mut reader: JoinHandle<()>) -> Result<(), YtDlpError> {
    match timeout(PROCESS_KILL_TIMEOUT, &mut reader).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(YtDlpError::ProcessRunner),
        Err(_) => {
            reader.abort();
            let _ = reader.await;
            Err(YtDlpError::ProcessCleanup)
        }
    }
}

async fn read_progress<R>(reader: R, events: mpsc::Sender<YtDlpRunnerEvent>)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    loop {
        match read_bounded_line(&mut reader, MAX_PROGRESS_LINE_BYTES).await {
            Ok(Some((line, too_large))) => {
                let line = String::from_utf8_lossy(&line).into_owned();
                let event = if too_large {
                    line.starts_with(PROGRESS_PREFIX)
                        .then_some(YtDlpRunnerEvent::ProgressOutputTooLarge)
                } else {
                    line.starts_with(PROGRESS_PREFIX)
                        .then_some(YtDlpRunnerEvent::ProgressLine(line))
                };
                if let Some(event) = event
                    && events.send(event).await.is_err()
                {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => {
                let _ = events.send(YtDlpRunnerEvent::OutputReadFailed).await;
                break;
            }
        }
    }
}

async fn read_bounded_line<R>(reader: &mut R, maximum: usize) -> io::Result<Option<(Vec<u8>, bool)>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    let mut too_large = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() && !too_large {
                return Ok(None);
            }
            break;
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        let retained = maximum.saturating_sub(line.len()).min(consumed);
        line.extend_from_slice(&available[..retained]);
        if retained < consumed {
            too_large = true;
        }
        let complete = available[..consumed].last() == Some(&b'\n');
        reader.consume(consumed);
        if complete {
            break;
        }
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(Some((line, too_large)))
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        ffi::OsStr,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use fetchx_models::{DownloadId, DownloadSource, FileName};
    use fetchx_storage::{Database, DownloadRepository};

    use super::*;
    use crate::{DownloadEngine, ManagedDownloadControl};

    #[test]
    fn default_version_validation_timeout_allows_cold_runtime_startup() {
        assert_eq!(YtDlpConfig::new().version_timeout, Duration::from_secs(15));
    }

    #[derive(Default)]
    struct FakeRunner {
        state: Arc<Mutex<FakeRunnerState>>,
    }

    #[derive(Default)]
    struct FakeRunnerState {
        version_validation_count: usize,
        format_discovery_count: usize,
        discovered_urls: Vec<String>,
        discovered_formats: Vec<YtDlpFormat>,
        run_count: usize,
        arguments: Vec<String>,
        diagnostic_codes: Vec<&'static str>,
        diagnostic_details: Vec<String>,
        output: Vec<u8>,
        progress_lines: Vec<String>,
        wait_for_stop: bool,
        started: Option<Arc<tokio::sync::Notify>>,
    }

    impl YtDlpProcessRunner for FakeRunner {
        fn report_diagnostic(&self, code: &'static str) {
            self.state
                .lock()
                .expect("test runner mutex")
                .diagnostic_codes
                .push(code);
        }

        fn report_diagnostic_details(&self, details: String) {
            self.state
                .lock()
                .expect("test runner mutex")
                .diagnostic_details
                .push(details);
        }

        fn validate_version<'a>(
            &'a self,
            _executable: &'a Path,
            _timeout: Duration,
        ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
            Box::pin(async move {
                self.state
                    .lock()
                    .expect("test runner mutex")
                    .version_validation_count += 1;
                Ok("2025.01.01".to_owned())
            })
        }

        fn discover_formats<'a>(
            &'a self,
            _executable: &'a Path,
            url: &'a str,
            _timeout: Duration,
        ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("test runner mutex");
                state.format_discovery_count += 1;
                state.discovered_urls.push(url.to_owned());
                Ok(state.discovered_formats.clone())
            })
        }

        fn run<'a>(
            &'a self,
            _executable: &'a Path,
            arguments: &'a [OsString],
            mut stop: watch::Receiver<bool>,
            events: mpsc::Sender<YtDlpRunnerEvent>,
        ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
            Box::pin(async move {
                let (output, progress_lines, wait_for_stop, started) = {
                    let mut state = self.state.lock().expect("test runner mutex");
                    state.run_count += 1;
                    state.arguments = arguments
                        .iter()
                        .map(|argument| argument.to_string_lossy().into_owned())
                        .collect();
                    (
                        state.output.clone(),
                        state.progress_lines.clone(),
                        state.wait_for_stop,
                        state.started.clone(),
                    )
                };
                if let Some(started) = started {
                    started.notify_one();
                }
                if wait_for_stop {
                    stop.changed()
                        .await
                        .map_err(|_| YtDlpError::ProcessCleanup)?;
                    return Ok(YtDlpProcessExit {
                        success: false,
                        code: None,
                    });
                }
                let paths_argument = arguments
                    .iter()
                    .position(|argument| argument == OsStr::new("--paths"))
                    .and_then(|index| arguments.get(index + 1))
                    .and_then(|argument| argument.to_str())
                    .and_then(|value| value.strip_prefix("home:"))
                    .ok_or(YtDlpError::ProcessRunner)?;
                let output_path = Path::new(paths_argument).join("test.webm");
                tokio::fs::write(output_path, output)
                    .await
                    .map_err(|_| YtDlpError::ProcessRunner)?;
                let progress_lines = if progress_lines.is_empty() {
                    vec!["FETCHX_PROGRESS|50%|5|10|NA|1|1".to_owned()]
                } else {
                    progress_lines
                };
                for line in progress_lines {
                    events
                        .send(YtDlpRunnerEvent::ProgressLine(line))
                        .await
                        .map_err(|_| YtDlpError::ProcessRunner)?;
                }
                Ok(YtDlpProcessExit {
                    success: true,
                    code: Some(0),
                })
            })
        }
    }

    #[tokio::test]
    async fn successful_version_validation_is_reused_for_unchanged_executable()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let runner = Arc::new(FakeRunner::default());
        let state = Arc::clone(&runner.state);
        let executable = directory.path().join("yt-dlp.mock");
        tokio::fs::write(&executable, b"first binary").await?;
        let config =
            YtDlpConfig::new().with_version_cache_path(directory.path().join("version-cache.json"));

        for _ in 0..2 {
            YtDlpEngine::initialize_with_runner_and_executable(
                repository.clone(),
                config.clone(),
                executable.clone(),
                runner.clone(),
            )
            .await?;
        }

        tokio::fs::write(&executable, b"replacement binary with a new size").await?;
        YtDlpEngine::initialize_with_runner_and_executable(
            repository.clone(),
            config,
            executable,
            runner,
        )
        .await?;

        assert_eq!(
            state
                .lock()
                .expect("test runner mutex")
                .version_validation_count,
            2
        );
        Ok(())
    }

    async fn fixture(
        url: &str,
        runner: FakeRunner,
    ) -> Result<
        (
            YtDlpEngine<fetchx_storage::SqliteDownloadRepository>,
            PathBuf,
            DownloadId,
        ),
        Box<dyn Error>,
    > {
        fixture_with_format(url, runner, None).await
    }

    async fn fixture_with_format(
        url: &str,
        runner: FakeRunner,
        format: Option<&str>,
    ) -> Result<
        (
            YtDlpEngine<fetchx_storage::SqliteDownloadRepository>,
            PathBuf,
            DownloadId,
        ),
        Box<dyn Error>,
    > {
        fixture_with_source_options(url, runner, format, None).await
    }

    async fn fixture_with_audio_format(
        url: &str,
        runner: FakeRunner,
        audio_format: YtDlpAudioFormat,
    ) -> Result<
        (
            YtDlpEngine<fetchx_storage::SqliteDownloadRepository>,
            PathBuf,
            DownloadId,
        ),
        Box<dyn Error>,
    > {
        fixture_with_source_options(url, runner, None, Some(audio_format)).await
    }

    async fn fixture_with_source_options(
        url: &str,
        runner: FakeRunner,
        format: Option<&str>,
        audio_format: Option<YtDlpAudioFormat>,
    ) -> Result<
        (
            YtDlpEngine<fetchx_storage::SqliteDownloadRepository>,
            PathBuf,
            DownloadId,
        ),
        Box<dyn Error>,
    > {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let id = DownloadId::new();
        let base = std::env::temp_dir().join(format!("fetchx-ytdlp-test-{id}"));
        tokio::fs::create_dir_all(&base).await?;
        let source = DownloadSource::new(url, EngineKind::YtDlp)?;
        let source = match format {
            Some(format) => source.with_yt_dlp_format(format)?,
            None => source,
        };
        let source = match audio_format {
            Some(audio_format) => source.with_yt_dlp_audio_format(audio_format)?,
            None => source,
        };
        let filename = format!(
            "video.{}",
            audio_format.map_or("webm", YtDlpAudioFormat::extension)
        );
        let item = DownloadItem::new(
            id,
            source,
            FileName::new(&filename)?,
            base.join(&filename),
            Timestamp::now()?,
        );
        repository.create(&item).await?;
        let executable = std::env::current_exe()?;
        let engine = YtDlpEngine::initialize_with_runner(
            repository,
            YtDlpConfig::new().with_executable(executable),
            Arc::new(runner),
        )
        .await?;
        Ok((engine, base, id))
    }

    #[test]
    fn parses_complete_and_indeterminate_progress_records() {
        let complete = parse_progress_line("FETCHX_PROGRESS| 25.4%|254|1000|NA|12.5|8")
            .expect("valid progress");
        assert_eq!(complete.percentage_basis_points, Some(2_540));
        assert_eq!(complete.downloaded_bytes, Some(254));
        assert_eq!(complete.total_bytes, Some(1_000));
        assert_eq!(complete.speed_bytes_per_second, Some(12));
        assert_eq!(complete.estimated_remaining_seconds, Some(8));

        let unknown = parse_progress_line("FETCHX_PROGRESS|NA|73|NA|NA|NA|NA")
            .expect("unknown totals are valid");
        assert_eq!(unknown.percentage_basis_points, None);
        assert_eq!(unknown.downloaded_bytes, Some(73));
        assert_eq!(unknown.total_bytes, None);
        assert_eq!(unknown.speed_bytes_per_second, None);
        assert_eq!(unknown.estimated_remaining_seconds, None);

        let no_downloaded_count = parse_progress_line("FETCHX_PROGRESS|NA|NA|NA|NA|NA|NA")
            .expect("unknown downloaded count is accepted");
        assert_eq!(no_downloaded_count.downloaded_bytes, None);
    }

    #[test]
    fn parses_fractional_hls_total_estimates() {
        let integral_estimate = parse_progress_line("FETCHX_PROGRESS|2.1%|1024|NA|48128.0|0.0|NA")
            .expect("yt-dlp HLS estimate may be serialized as a float");
        assert_eq!(integral_estimate.downloaded_bytes, Some(1_024));
        assert_eq!(integral_estimate.total_bytes, Some(48_128));

        let fractional_estimate =
            parse_progress_line("FETCHX_PROGRESS|2.1%|1024|NA|48128.5|0.0|NA")
                .expect("fractional byte estimates round up");
        assert_eq!(fractional_estimate.total_bytes, Some(48_129));
    }

    #[test]
    fn parses_fractional_hls_eta_seconds() {
        let progress =
            parse_progress_line("FETCHX_PROGRESS|13.9%|15360|NA|110592.0|9941.88|6.9994")
                .expect("yt-dlp HLS ETA may be serialized as a float");
        assert_eq!(progress.estimated_remaining_seconds, Some(7));
    }

    #[tokio::test]
    async fn keeps_total_bytes_stable_while_download_progresses() -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        let (engine, _, id) = fixture("https://example.test/video", runner).await?;
        engine
            .state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        engine
            .state_machine
            .transition(id, DownloadStatus::Downloading)
            .await?;
        let mut item = engine.repository.get(id).await?.expect("item exists");
        let mut expected_updated_at = item.updated_at();

        let initial = YtDlpProgress {
            percentage_basis_points: Some(1_000),
            downloaded_bytes: Some(10_240),
            total_bytes: Some(1_048_576),
            speed_bytes_per_second: Some(1_024),
            estimated_remaining_seconds: Some(60),
        };
        engine
            .persist_progress(&mut item, &mut expected_updated_at, &initial, false)
            .await?;

        let drifted = YtDlpProgress {
            percentage_basis_points: Some(2_000),
            downloaded_bytes: Some(20_480),
            total_bytes: Some(1_572_864),
            speed_bytes_per_second: Some(2_048),
            estimated_remaining_seconds: Some(30),
        };
        engine
            .persist_progress(&mut item, &mut expected_updated_at, &drifted, false)
            .await?;

        let stored = engine.repository.get(id).await?.expect("item still exists");
        assert_eq!(stored.progress().downloaded_bytes(), 20_480);
        assert_eq!(stored.progress().total_bytes(), Some(1_048_576));
        Ok(())
    }

    #[tokio::test]
    async fn replaces_an_estimated_total_when_downloaded_bytes_exceed_it()
    -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        let (engine, _, id) = fixture("https://example.test/video", runner).await?;
        engine
            .state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        engine
            .state_machine
            .transition(id, DownloadStatus::Downloading)
            .await?;
        let mut item = engine.repository.get(id).await?.ok_or("item missing")?;
        let mut expected_updated_at = item.updated_at();

        engine
            .persist_progress(
                &mut item,
                &mut expected_updated_at,
                &YtDlpProgress {
                    percentage_basis_points: Some(9_900),
                    downloaded_bytes: Some(99),
                    total_bytes: Some(100),
                    speed_bytes_per_second: None,
                    estimated_remaining_seconds: None,
                },
                false,
            )
            .await?;
        engine
            .persist_progress(
                &mut item,
                &mut expected_updated_at,
                &YtDlpProgress {
                    percentage_basis_points: None,
                    downloaded_bytes: Some(101),
                    total_bytes: Some(102),
                    speed_bytes_per_second: None,
                    estimated_remaining_seconds: None,
                },
                false,
            )
            .await?;

        let stored = engine.repository.get(id).await?.ok_or("item missing")?;
        assert_eq!(stored.progress().downloaded_bytes(), 101);
        assert_eq!(stored.progress().total_bytes(), Some(102));
        Ok(())
    }

    #[tokio::test]
    async fn progress_checkpoint_retries_after_a_concurrent_snapshot() -> Result<(), Box<dyn Error>>
    {
        let runner = FakeRunner::default();
        let (engine, _, id) = fixture("https://example.test/video", runner).await?;
        engine
            .state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        engine
            .state_machine
            .transition(id, DownloadStatus::Downloading)
            .await?;
        let mut item = engine.repository.get(id).await?.ok_or("item missing")?;
        let mut expected_updated_at = item.updated_at();

        let mut concurrent_snapshot = item.clone();
        concurrent_snapshot.set_progress_at(
            DownloadProgress::new(3, Some(10), None, None)?,
            next_timestamp(concurrent_snapshot.updated_at())?,
        )?;
        assert!(
            engine
                .repository
                .update_if_status_and_timestamp_current(
                    &concurrent_snapshot,
                    DownloadStatus::Downloading,
                    expected_updated_at,
                )
                .await?
        );

        engine
            .persist_progress(
                &mut item,
                &mut expected_updated_at,
                &YtDlpProgress {
                    percentage_basis_points: Some(5000),
                    downloaded_bytes: Some(5),
                    total_bytes: Some(10),
                    speed_bytes_per_second: None,
                    estimated_remaining_seconds: None,
                },
                false,
            )
            .await?;

        let persisted = engine.repository.get(id).await?.ok_or("item missing")?;
        assert_eq!(persisted.progress().downloaded_bytes(), 5);
        assert_eq!(persisted.progress().total_bytes(), Some(10));
        Ok(())
    }

    #[tokio::test]
    async fn format_discovery_uses_runner_and_rejects_unsafe_urls() -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        let state = Arc::clone(&runner.state);
        state.lock().expect("test runner mutex").discovered_formats = vec![YtDlpFormat {
            format_id: "137".to_owned(),
            label: "1080p - mp4".to_owned(),
            extension: Some("mp4".to_owned()),
            file_size_bytes: Some(1_024),
            title: Some("test".to_owned()),
            platform: Some("test-site".to_owned()),
        }];
        let (engine, base, _) = fixture("https://example.test/watch", runner).await?;

        let formats = engine
            .discover_formats("https://example.test/watch")
            .await?;
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format_id, "137");
        assert_eq!(
            state.lock().expect("test runner mutex").discovered_urls,
            ["https://example.test/watch"]
        );

        for unsafe_url in [
            "file:///etc/passwd",
            "http://127.0.0.1/private",
            "https://user:password@example.test/watch",
        ] {
            assert!(matches!(
                engine.discover_formats(unsafe_url).await,
                Err(YtDlpError::InvalidUrl)
            ));
        }
        assert_eq!(
            state
                .lock()
                .expect("test runner mutex")
                .format_discovery_count,
            1
        );
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[test]
    fn rejects_malformed_progress_records() {
        for line in [
            "unrelated output",
            "FETCHX_PROGRESS|101%|1|2|NA|1|0",
            "FETCHX_PROGRESS|10%|not-bytes|20|NA|1|0",
            "FETCHX_PROGRESS|10%|1|2|NA|1",
            "FETCHX_PROGRESS|10%|1|2|NA|1|0|extra",
        ] {
            assert!(matches!(
                parse_progress_line(line),
                Err(YtDlpError::MalformedProgress)
            ));
        }
    }

    #[test]
    fn discovers_safe_combined_and_adaptive_video_formats() {
        let output = br#"{
            "title":"A sample video",
            "extractor_key":"XHamster",
            "formats": [
                {"format_id":"18","ext":"mp4","width":640,"height":360,
                 "vcodec":"avc1","acodec":"mp4a","filesize":1048576},
                {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a"},
                {"format_id":"137","ext":"mp4","width":3840,"height":2160,
                 "vcodec":"avc1","acodec":"none"},
                {"format_id":"unsafe+format","ext":"mp4","vcodec":"avc1","acodec":"mp4a"},
                {"format_id":"drm","ext":"mp4","vcodec":"avc1","acodec":"mp4a","has_drm":true}
            ]
        }"#;
        let formats = parse_format_output(output).expect("valid format output");
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].format_id, "18");
        assert_eq!(formats[0].label, "360p - mp4 - 1.0 MB");
        assert_eq!(formats[0].file_size_bytes, Some(1_048_576));
        assert_eq!(formats[0].title.as_deref(), Some("A sample video"));
        assert_eq!(formats[0].platform.as_deref(), Some("XHamster"));
        assert_eq!(formats[1].format_id, "137+bestaudio/137");
        assert_eq!(formats[1].label, "4K (2160p) + audio - mp4");
        assert_eq!(formats[1].title.as_deref(), Some("A sample video"));
        assert_eq!(formats[1].platform.as_deref(), Some("XHamster"));
    }

    #[test]
    fn rejects_invalid_or_oversized_format_output() {
        assert!(matches!(
            parse_format_output(b"not json"),
            Err(YtDlpError::InvalidFormatOutput)
        ));
        assert!(matches!(
            parse_format_output(&vec![b' '; MAX_FORMAT_OUTPUT_BYTES + 1]),
            Err(YtDlpError::FormatOutputTooLarge)
        ));
    }

    #[test]
    fn runtime_failure_diagnostic_does_not_include_raw_details() {
        let error = YtDlpError::RuntimeFailure(
            "request failed for https://private.example/video?token=secret".to_owned(),
        );
        assert_eq!(error.diagnostic_code(), "runtime_failure");
    }

    #[test]
    fn progress_failure_diagnostic_is_specific() {
        let error = DownloadEngineError::Progress(
            fetchx_models::DownloadProgressError::DownloadedExceedsTotal {
                downloaded_bytes: 2,
                total_bytes: 1,
            },
        );
        assert_eq!(download_failure_diagnostic_code(&error), "progress");
    }

    #[test]
    fn state_machine_diagnostic_reports_only_a_fixed_category() {
        let error = DownloadEngineError::StateMachine(StateMachineError::ConcurrentTransition {
            from: DownloadStatus::Downloading,
            to: DownloadStatus::Failed,
            actual: DownloadStatus::Cancelled,
        });
        assert_eq!(
            download_failure_diagnostic_code(&error),
            "state_machine_concurrent_transition"
        );
    }

    #[test]
    fn state_machine_stage_diagnostic_is_fixed_and_contextual() {
        let error = StateMachineError::ConcurrentTransition {
            from: DownloadStatus::Downloading,
            to: DownloadStatus::Failed,
            actual: DownloadStatus::Cancelled,
        };
        assert_eq!(
            state_machine_diagnostic_code("progress", &error),
            "progress_concurrent_transition"
        );
    }

    #[test]
    fn selected_format_is_passed_as_a_distinct_ytdlp_argument() {
        let args = build_arguments(
            Path::new("workspace"),
            "https://example.test/video",
            Some("137+bestaudio/137"),
            None,
            Some(4096),
        );
        let format_index = args
            .iter()
            .position(|argument| argument == OsStr::new("--format"))
            .expect("format option is present");
        assert_eq!(args[format_index + 1], OsStr::new("137+bestaudio/137"));
        let limit_index = args
            .iter()
            .position(|argument| argument == OsStr::new("--limit-rate"))
            .expect("rate limit option is present");
        assert_eq!(args[limit_index + 1], OsStr::new("4096"));
    }

    #[test]
    fn automatic_selection_uses_best_quality_ytdlp_selector() {
        let args = build_arguments(
            Path::new("workspace"),
            "https://example.test/video",
            None,
            None,
            None,
        );
        let format_index = args
            .iter()
            .position(|argument| argument == OsStr::new("--format"))
            .expect("format option is present");
        assert_eq!(
            args[format_index + 1],
            OsStr::new("bestvideo*+bestaudio/best")
        );
    }

    #[test]
    fn audio_extraction_uses_audio_only_source_and_allowlisted_conversion_flags() {
        let args = build_arguments(
            Path::new("workspace"),
            "https://example.test/video",
            Some("137"),
            Some(YtDlpAudioFormat::Mp3),
            None,
        );
        let format_index = args
            .iter()
            .position(|argument| argument == OsStr::new("--format"))
            .expect("format option is present");
        assert_eq!(args[format_index + 1], OsStr::new("bestaudio/best"));
        let extract_index = args
            .iter()
            .position(|argument| argument == OsStr::new("--extract-audio"))
            .expect("audio extraction is enabled");
        assert_eq!(args[extract_index + 1], OsStr::new("--audio-format"));
        assert_eq!(args[extract_index + 2], OsStr::new("mp3"));
        assert_eq!(args[args.len() - 2], OsStr::new("--"));
        assert_eq!(
            args[args.len() - 1],
            OsStr::new("https://example.test/video")
        );
    }

    #[test]
    fn audio_extraction_requires_matching_destination_extension() {
        assert!(
            validate_audio_destination(Path::new("track.MP3"), Some(YtDlpAudioFormat::Mp3)).is_ok()
        );
        assert!(
            validate_audio_destination(Path::new("track.mp4"), Some(YtDlpAudioFormat::Mp3))
                .is_err()
        );
        assert!(validate_audio_destination(Path::new("track.mp4"), None).is_ok());
    }

    #[tokio::test]
    async fn runs_with_structured_arguments_and_publishes_one_output() -> Result<(), Box<dyn Error>>
    {
        let runner = FakeRunner::default();
        runner.state.lock().expect("test runner mutex").output = b"video".to_vec();
        let state = Arc::clone(&runner.state);
        let (engine, base, id) = fixture("https://example.test/video", runner).await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        let completed = engine.download(id, cancellation).await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(tokio::fs::read(base.join("video.webm")).await?, b"video");
        assert_eq!(completed.progress().downloaded_bytes(), 5);
        {
            let state = state.lock().expect("test runner mutex");
            assert_eq!(state.run_count, 1);
            assert!(state.arguments.contains(&"--ignore-config".to_owned()));
            assert!(state.arguments.contains(&"--no-plugin-dirs".to_owned()));
            assert!(
                state
                    .arguments
                    .contains(&"--concurrent-fragments".to_owned())
            );
            let concurrent_fragments_index = state
                .arguments
                .iter()
                .position(|argument| argument == "--concurrent-fragments")
                .ok_or("missing concurrent fragments option")?;
            assert_eq!(state.arguments[concurrent_fragments_index + 1], "16");
            let delimiter = state
                .arguments
                .iter()
                .position(|arg| arg == "--")
                .ok_or("missing end-of-options delimiter")?;
            assert_eq!(state.arguments[delimiter + 1], "https://example.test/video");
        }
        assert!(!workspace_path(&base.join("video.webm"), id).exists());
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn recovers_a_published_output_after_crash_before_state_completion()
    -> Result<(), Box<dyn Error>> {
        for destination_already_published in [false, true] {
            let runner = FakeRunner::default();
            let state = Arc::clone(&runner.state);
            let (engine, base, id) = fixture("https://example.test/video", runner).await?;
            engine
                .state_machine
                .transition(id, DownloadStatus::Preparing)
                .await?;
            engine
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
            engine.state_machine.recover_interrupted(id).await?;

            let destination = base.join("video.webm");
            let workspace = workspace_path(&destination, id);
            tokio::fs::create_dir_all(&workspace).await?;
            let output = workspace.join("test.webm");
            let contents = b"validated output before crash";
            tokio::fs::write(&output, contents).await?;
            if destination_already_published {
                tokio::fs::hard_link(&output, &destination).await?;
            }
            save_finalization_record(&workspace, &output, contents.len() as u64).await?;

            let (_cancel_sender, cancellation) = watch::channel(false);
            let completed = engine.download(id, cancellation).await?;
            assert_eq!(completed.status(), DownloadStatus::Completed);
            assert_eq!(
                completed.progress().downloaded_bytes(),
                contents.len() as u64
            );
            assert_eq!(tokio::fs::read(&destination).await?, contents);
            assert_eq!(state.lock().expect("test runner mutex").run_count, 0);
            assert!(!workspace.exists());
            tokio::fs::remove_dir_all(base).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn rejects_embedded_credentials_before_starting_process() -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        let state = Arc::clone(&runner.state);
        let (engine, base, id) = fixture("https://user:secret@example.test/video", runner).await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            engine.download(id, cancellation).await,
            Err(DownloadEngineError::YtDlp(YtDlpError::InvalidUrl))
        ));
        let persisted = engine
            .repository
            .get(id)
            .await?
            .ok_or("download not found")?;
        assert_eq!(persisted.status(), DownloadStatus::Failed);
        assert_eq!(state.lock().expect("test runner mutex").run_count, 0);
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn ignores_regressing_progress_for_merged_formats() -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        {
            let mut state = runner.state.lock().expect("test runner mutex");
            state.output = b"video!".to_vec();
            state.progress_lines = vec![
                "FETCHX_PROGRESS|50%|5|10|NA|1|1".to_owned(),
                "FETCHX_PROGRESS|40%|4|10|NA|1|1".to_owned(),
                "FETCHX_PROGRESS|60%|6|10|NA|1|1".to_owned(),
            ];
        }
        let state = Arc::clone(&runner.state);
        let (engine, base, id) = fixture_with_format(
            "https://example.test/video",
            runner,
            Some("602+bestaudio/602"),
        )
        .await?;
        let (_cancel_sender, cancellation) = watch::channel(false);

        let completed = engine.download(id, cancellation).await?;
        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(completed.progress().downloaded_bytes(), 6);
        assert_eq!(tokio::fs::read(base.join("video.webm")).await?, b"video!");
        assert!(
            state
                .lock()
                .expect("test runner mutex")
                .diagnostic_codes
                .is_empty()
        );
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn automatic_merged_progress_reset_completes_and_retains_output()
    -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        {
            let mut state = runner.state.lock().expect("test runner mutex");
            state.output = b"merged-video".to_vec();
            state.progress_lines = vec![
                "FETCHX_PROGRESS|100%|10|10|NA|1|0".to_owned(),
                "FETCHX_PROGRESS|0%|0|2|NA|1|1".to_owned(),
                "FETCHX_PROGRESS|50%|1|2|NA|1|1".to_owned(),
                "FETCHX_PROGRESS|100%|2|2|NA|1|0".to_owned(),
            ];
        }
        let state = Arc::clone(&runner.state);
        let (engine, base, id) = fixture("https://example.test/video", runner).await?;
        let (_cancel_sender, cancellation) = watch::channel(false);

        let completed = engine.download(id, cancellation).await?;
        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(
            tokio::fs::read(base.join("video.webm")).await?,
            b"merged-video"
        );
        {
            let state = state.lock().expect("test runner mutex");
            assert!(state.diagnostic_codes.is_empty());
            assert!(state.diagnostic_details.iter().any(|detail| {
                detail.contains("previous_downloaded_bytes=10")
                    && detail.contains("incoming_downloaded_bytes=0")
                    && detail.contains("component_selector=true")
            }));
        }
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn rejects_regressing_progress_for_single_format() -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        {
            let mut state = runner.state.lock().expect("test runner mutex");
            state.output = b"video".to_vec();
            state.progress_lines = vec![
                "FETCHX_PROGRESS|50%|5|10|NA|1|1".to_owned(),
                "FETCHX_PROGRESS|40%|4|10|NA|1|1".to_owned(),
            ];
        }
        let state = Arc::clone(&runner.state);
        let (engine, base, id) =
            fixture_with_format("https://example.test/video", runner, Some("137")).await?;
        let (_cancel_sender, cancellation) = watch::channel(false);

        let result = engine.download(id, cancellation).await;
        assert!(
            matches!(
                &result,
                Err(DownloadEngineError::YtDlp(YtDlpError::ProgressRegression))
            ),
            "{result:?}"
        );
        assert_eq!(
            state.lock().expect("test runner mutex").diagnostic_codes,
            ["progress_regression"]
        );
        assert!(
            engine
                .repository
                .get(id)
                .await?
                .is_some_and(|item| item.status() == DownloadStatus::Failed)
        );
        assert!(!base.join("video.webm").exists());
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_stops_engine_and_preserves_resume_workspace() -> Result<(), Box<dyn Error>>
    {
        let runner = FakeRunner::default();
        let started = Arc::new(tokio::sync::Notify::new());
        {
            let mut state = runner.state.lock().expect("test runner mutex");
            state.wait_for_stop = true;
            state.started = Some(Arc::clone(&started));
        }
        let (engine, base, id) =
            fixture_with_audio_format("https://example.test/video", runner, YtDlpAudioFormat::Mp3)
                .await?;
        let (cancel_sender, cancellation) = watch::channel(false);
        let (pause_sender, pause) = watch::channel(false);
        let (limit_sender, connection_limit) = watch::channel(1);
        let download = engine.download_managed(
            id,
            ManagedDownloadControl {
                cancellation,
                pause,
                connection_limit,
                resource_demand: None,
                bandwidth: BandwidthControl::default(),
            },
        );
        tokio::pin!(download);
        tokio::select! {
            result = &mut download => return Err(format!("download ended before start: {result:?}").into()),
            _ = started.notified() => {}
        }
        cancel_sender.send(true)?;
        assert!(matches!(
            download.await,
            Err(DownloadEngineError::Cancelled)
        ));
        let persisted = engine
            .repository
            .get(id)
            .await?
            .ok_or("download not found")?;
        assert_eq!(persisted.status(), DownloadStatus::Cancelled);
        assert!(workspace_path(&base.join("video.mp3"), id).is_dir());
        drop((pause_sender, limit_sender));
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }
}

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
};
use fetchx_storage::DownloadRepository;
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, timeout},
};
use tracing::{info, warn};

use crate::{
    DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    ResourceDemand,
};

const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_KILL_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_VERSION_OUTPUT_BYTES: usize = 128;
const MAX_PROGRESS_LINE_BYTES: usize = 1024;
const PROGRESS_PREFIX: &str = "FETCHX_PROGRESS|";
const PROGRESS_TEMPLATE: &str = "download:FETCHX_PROGRESS|%(progress._percent_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s";
const MAX_PROGRESS_EVENTS: usize = 32;
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_millis(250);

pub type YtDlpRunnerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct YtDlpConfig {
    executable: Option<PathBuf>,
    version_timeout: Duration,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum YtDlpRunnerEvent {
    ProgressLine(String),
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
}

pub trait YtDlpProcessRunner: Send + Sync {
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
        if config.version_timeout.is_zero() {
            return Err(YtDlpError::InvalidVersion);
        }
        let executable = resolve_executable(config.executable.as_deref()).await?;
        let version = runner
            .validate_version(&executable, config.version_timeout)
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
        if current.source().engine() != EngineKind::YtDlp {
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
                self.persist_failure(id, &error).await?;
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
        match tokio::fs::symlink_metadata(item.destination()).await {
            Ok(_) => return Err(YtDlpError::DestinationExists.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }
        let workspace = workspace_path(item.destination(), id);
        prepare_workspace(&workspace).await?;
        let arguments = build_arguments(&workspace, url.as_str());
        report_resource_demand(control, id, 1, 0).await?;

        if item.status() == DownloadStatus::Preparing {
            *item = self
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
        }
        let mut expected_updated_at = item.updated_at();
        let mut last_persisted_at = Instant::now();
        let mut progress = item.progress().downloaded_bytes();
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
                                if let Err(cleanup_error) = run.await {
                                    return Err(cleanup_error.into());
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
        let final_progress = YtDlpProgress {
            percentage_basis_points: Some(10_000),
            downloaded_bytes: Some(output_size),
            total_bytes: Some(output_size),
            speed_bytes_per_second: None,
            estimated_remaining_seconds: Some(0),
        };
        if output_size < item.progress().downloaded_bytes() {
            return Err(YtDlpError::ProgressRegression.into());
        }
        self.persist_progress(item, &mut expected_updated_at, &final_progress)
            .await?;
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        tokio::fs::hard_link(&output, item.destination())
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    YtDlpError::DestinationExists.into()
                } else {
                    DownloadEngineError::Io(error)
                }
            })?;
        *item = self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        if tokio::fs::remove_dir_all(&workspace).await.is_err() {
            warn!("yt-dlp temporary workspace cleanup failed");
        }
        Ok(item.clone())
    }

    async fn persist_progress(
        &self,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        update: &YtDlpProgress,
    ) -> Result<(), DownloadEngineError> {
        let downloaded_bytes = update
            .downloaded_bytes
            .ok_or(YtDlpError::MalformedProgress)?;
        if downloaded_bytes < item.progress().downloaded_bytes() {
            return Err(YtDlpError::ProgressRegression.into());
        }
        let total = update
            .total_bytes
            .filter(|total| *total >= downloaded_bytes);
        item.set_progress_at(
            DownloadProgress::new(
                downloaded_bytes,
                total,
                update.speed_bytes_per_second,
                update.estimated_remaining_seconds,
            )?,
            next_timestamp(item.updated_at())?,
        )?;
        if !self
            .repository
            .update_if_status_and_timestamp_current(
                item,
                DownloadStatus::Downloading,
                *expected_updated_at,
            )
            .await?
        {
            return Err(DownloadEngineError::StateMachine(
                self.current_state_error(item.id()).await?,
            ));
        }
        *expected_updated_at = item.updated_at();
        Ok(())
    }

    async fn handle_runner_event(
        &self,
        event: YtDlpRunnerEvent,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        last_persisted_at: &mut Instant,
        progress: &mut u64,
        control: &mut ManagedDownloadControl,
    ) -> Result<(), DownloadEngineError> {
        let update = match event {
            YtDlpRunnerEvent::ProgressLine(line) => parse_progress_line(&line)?,
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
        if downloaded_bytes < *progress {
            return Err(YtDlpError::ProgressRegression.into());
        }

        let now = Instant::now();
        let changed_progress = downloaded_bytes != *progress;
        *progress = downloaded_bytes;
        if changed_progress && now.duration_since(*last_persisted_at) >= PROGRESS_PERSIST_INTERVAL {
            self.persist_progress(item, expected_updated_at, &update)
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
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if !matches!(
            current.status(),
            DownloadStatus::Preparing | DownloadStatus::Downloading
        ) {
            return Ok(());
        }
        self.state_machine
            .transition_with_error(id, DownloadStatus::Failed, operation.persisted_error()?)
            .await?;
        Ok(())
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
    let mut fields = line.split('|');
    if fields.next() != Some(PROGRESS_PREFIX.trim_end_matches('|')) {
        return Err(YtDlpError::MalformedProgress);
    }
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
    let total_bytes = parse_optional_u64(total)?.or(parse_optional_u64(estimate)?);
    let speed_bytes_per_second = parse_optional_float(speed)?;
    let estimated_remaining_seconds = parse_optional_u64(eta)?;
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
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| YtDlpError::MalformedProgress)
}

fn parse_optional_float(value: &str) -> Result<Option<u64>, YtDlpError> {
    let value = value.trim();
    if is_unknown(value) {
        return Ok(None);
    }
    let value = value
        .parse::<f64>()
        .map_err(|_| YtDlpError::MalformedProgress)?;
    if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
        return Err(YtDlpError::MalformedProgress);
    }
    Ok(Some(value.round() as u64))
}

fn is_unknown(value: &str) -> bool {
    value.is_empty() || matches!(value, "NA" | "None" | "none" | "N/A")
}

fn build_arguments(workspace: &Path, url: &str) -> Vec<OsString> {
    let mut arguments = vec![
        "--ignore-config".into(),
        "--no-plugin-dirs".into(),
        "--no-colors".into(),
        "--no-playlist".into(),
        "--no-overwrites".into(),
        "--continue".into(),
        "--no-cache-dir".into(),
        "--format".into(),
        "best".into(),
        "--concurrent-fragments".into(),
        "1".into(),
        "--newline".into(),
        "--progress-template".into(),
        PROGRESS_TEMPLATE.into(),
        "--paths".into(),
    ];
    let mut home_path = OsString::from("home:");
    home_path.push(workspace.as_os_str());
    arguments.push(home_path);
    arguments.extend([
        "--output".into(),
        "%(id)s.%(ext)s".into(),
        "--".into(),
        url.into(),
    ]);
    arguments
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

    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(|_| YtDlpError::WorkspaceUnavailable)
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
        let metadata = tokio::fs::symlink_metadata(entry.path())
            .await
            .map_err(|_| YtDlpError::OutputUnavailable)?;
        if !metadata.file_type().is_file() || output.is_some() {
            return Err(YtDlpError::OutputUnavailable);
        }
        output = Some(entry.path());
    }
    output.ok_or(YtDlpError::OutputUnavailable)
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

    #[derive(Default)]
    struct FakeRunner {
        state: Arc<Mutex<FakeRunnerState>>,
    }

    #[derive(Default)]
    struct FakeRunnerState {
        run_count: usize,
        arguments: Vec<String>,
        output: Vec<u8>,
        progress_lines: Vec<String>,
        wait_for_stop: bool,
        started: Option<Arc<tokio::sync::Notify>>,
    }

    impl YtDlpProcessRunner for FakeRunner {
        fn validate_version<'a>(
            &'a self,
            _executable: &'a Path,
            _timeout: Duration,
        ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
            Box::pin(async { Ok("2025.01.01".to_owned()) })
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
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let id = DownloadId::new();
        let base = std::env::temp_dir().join(format!("fetchx-ytdlp-test-{id}"));
        tokio::fs::create_dir_all(&base).await?;
        let item = DownloadItem::new(
            id,
            DownloadSource::new(url, EngineKind::YtDlp)?,
            FileName::new("video.webm")?,
            base.join("video.webm"),
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
        assert_eq!(complete.speed_bytes_per_second, Some(13));
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
    async fn rejects_regressing_progress_without_publishing_output() -> Result<(), Box<dyn Error>> {
        let runner = FakeRunner::default();
        {
            let mut state = runner.state.lock().expect("test runner mutex");
            state.output = b"video".to_vec();
            state.progress_lines = vec![
                "FETCHX_PROGRESS|50%|5|10|NA|1|1".to_owned(),
                "FETCHX_PROGRESS|40%|4|10|NA|1|1".to_owned(),
            ];
        }
        let (engine, base, id) = fixture("https://example.test/video", runner).await?;
        let (_cancel_sender, cancellation) = watch::channel(false);

        let result = engine.download(id, cancellation).await;
        assert!(
            matches!(
                &result,
                Err(DownloadEngineError::YtDlp(YtDlpError::ProgressRegression))
            ),
            "{result:?}"
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
        let (engine, base, id) = fixture("https://example.test/video", runner).await?;
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
        assert!(workspace_path(&base.join("video.webm"), id).is_dir());
        drop((pause_sender, limit_sender));
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }
}

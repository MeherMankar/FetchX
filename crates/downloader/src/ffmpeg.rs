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

use fetchx_models::ResourcePressure;
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::{Child, Command},
    sync::{Semaphore, mpsc, watch},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout},
};
use tracing::warn;
use uuid::Uuid;

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(60 * 60 * 6);
const MAX_PROCESS_TIMEOUT: Duration = Duration::from_secs(60 * 60 * 24);
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const OUTPUT_SIZE_CHECK_INTERVAL: Duration = Duration::from_millis(25);
const DEFAULT_MAX_CONCURRENT_PROCESSES: usize = 1;
const MAX_CONCURRENT_PROCESSES: usize = 2;
const PROCESS_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_VERSION_OUTPUT_BYTES: usize = 4096;
const MAX_PROBE_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_MEDIA_STREAMS: usize = 128;
const MAX_PROGRESS_LINE_BYTES: usize = 1024;
const PROGRESS_CHANNEL_CAPACITY: usize = 16;
pub type FfmpegFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct FfmpegConfig {
    ffmpeg_executable: Option<PathBuf>,
    ffprobe_executable: Option<PathBuf>,
    process_timeout: Duration,
    max_output_bytes: u64,
    max_concurrent_processes: usize,
}

impl Default for FfmpegConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl FfmpegConfig {
    pub fn new() -> Self {
        Self {
            ffmpeg_executable: None,
            ffprobe_executable: None,
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_concurrent_processes: DEFAULT_MAX_CONCURRENT_PROCESSES,
        }
    }

    pub fn with_ffmpeg_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.ffmpeg_executable = Some(path.into());
        self
    }

    pub fn with_ffprobe_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.ffprobe_executable = Some(path.into());
        self
    }

    pub fn with_process_timeout(mut self, timeout: Duration) -> Self {
        self.process_timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, maximum: u64) -> Self {
        self.max_output_bytes = maximum;
        self
    }

    pub fn with_max_concurrent_processes(mut self, maximum: usize) -> Self {
        self.max_concurrent_processes = maximum;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaContainer {
    Mp4,
    Matroska,
    WebM,
    Mp3,
    Flac,
    Wav,
}

impl MediaContainer {
    fn muxer(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "matroska",
            Self::WebM => "webm",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Wav => "wav",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "mkv",
            Self::WebM => "webm",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Wav => "wav",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscodePreset {
    H264AacMp4,
    HevcAacMp4,
    Vp9OpusWebM,
    Mp3Audio,
    FlacAudio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaOperation {
    Remux { container: MediaContainer },
    Transcode { preset: TranscodePreset },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaProcessRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub operation: MediaOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaProgress {
    pub output_time: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaStreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaStream {
    pub kind: MediaStreamKind,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<FrameRate>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub duration: Option<Duration>,
    pub format_name: Option<String>,
    pub size_bytes: Option<u64>,
    pub bit_rate_bits_per_second: Option<u64>,
    pub streams: Vec<MediaStream>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfmpegProcessExit {
    pub success: bool,
    pub code: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaExecutable {
    Ffmpeg,
    Ffprobe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaProcessOutput {
    pub destination: PathBuf,
    pub size_bytes: u64,
}

pub struct FfmpegProcessSpec<'a> {
    pub arguments: &'a [OsString],
    pub input: tokio::fs::File,
    pub output: &'a Path,
    pub max_output_bytes: u64,
    pub timeout: Duration,
    pub cancellation: watch::Receiver<bool>,
    pub progress: Option<mpsc::Sender<MediaProgress>>,
}

#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error("FFmpeg configuration is invalid")]
    InvalidConfiguration,
    #[error("FFmpeg executable could not be found")]
    FfmpegNotFound,
    #[error("ffprobe executable could not be found")]
    FfprobeNotFound,
    #[error("configured executable is not a regular file")]
    ExecutableInvalid,
    #[error("configured executable is not runnable")]
    ExecutableNotRunnable,
    #[error("external media process could not be started")]
    ProcessStart,
    #[error("external media process timed out")]
    ProcessTimeout,
    #[error("external media process could not be stopped and reaped")]
    ProcessCleanup,
    #[error("FFmpeg exited with status {code:?}")]
    FfmpegFailed { code: Option<i32> },
    #[error("ffprobe exited with status {code:?}")]
    FfprobeFailed { code: Option<i32> },
    #[error("media process runner failed")]
    ProcessRunner,
    #[error("media metadata output exceeded its size limit")]
    MetadataOutputTooLarge,
    #[error("media output exceeded its configured size limit")]
    OutputTooLarge,
    #[error("ffprobe returned invalid or unsupported metadata")]
    InvalidMediaMetadata,
    #[error("input path is not an existing regular file")]
    InvalidInput,
    #[error("output path is invalid or its parent directory does not exist")]
    InvalidOutput,
    #[error("input and output resolve to the same path")]
    InputOutputCollision,
    #[error("output destination already exists")]
    DestinationExists,
    #[error("temporary output workspace could not be used safely")]
    WorkspaceUnavailable,
    #[error("FFmpeg did not produce one valid output file")]
    OutputUnavailable,
    #[error("temporary output cleanup failed")]
    CleanupFailed,
    #[error("media operation was cancelled but temporary output cleanup failed")]
    CleanupFailedAfterCancellation,
    #[error("media operation was cancelled")]
    Cancelled,
}

pub trait FfmpegProcessRunner: Send + Sync {
    fn validate<'a>(
        &'a self,
        executable: &'a Path,
        kind: MediaExecutable,
        timeout: Duration,
        cancellation: watch::Receiver<bool>,
    ) -> FfmpegFuture<'a, Result<(), FfmpegError>>;

    fn probe<'a>(
        &'a self,
        executable: &'a Path,
        arguments: &'a [OsString],
        input: tokio::fs::File,
        timeout: Duration,
        cancellation: watch::Receiver<bool>,
    ) -> FfmpegFuture<'a, Result<Vec<u8>, FfmpegError>>;

    fn process<'a>(
        &'a self,
        executable: &'a Path,
        spec: FfmpegProcessSpec<'a>,
    ) -> FfmpegFuture<'a, Result<FfmpegProcessExit, FfmpegError>>;
}

struct OutputTarget<'a> {
    temporary_output: &'a Path,
    destination: &'a Path,
    max_output_bytes: u64,
}

pub struct FfmpegProcessor {
    ffmpeg_executable: PathBuf,
    ffprobe_executable: PathBuf,
    process_timeout: Duration,
    max_output_bytes: u64,
    process_slots: Arc<Semaphore>,
    pressure_sender: watch::Sender<ResourcePressure>,
    pressure: watch::Receiver<ResourcePressure>,
    runner: Arc<dyn FfmpegProcessRunner>,
}

impl FfmpegProcessor {
    pub async fn initialize(config: FfmpegConfig) -> Result<Self, FfmpegError> {
        Self::initialize_with_runner(config, Arc::new(TokioFfmpegRunner)).await
    }

    pub async fn initialize_with_runner(
        config: FfmpegConfig,
        runner: Arc<dyn FfmpegProcessRunner>,
    ) -> Result<Self, FfmpegError> {
        if config.process_timeout.is_zero()
            || config.process_timeout > MAX_PROCESS_TIMEOUT
            || config.max_output_bytes == 0
            || config.max_output_bytes > MAX_OUTPUT_BYTES
            || config.max_concurrent_processes == 0
            || config.max_concurrent_processes > MAX_CONCURRENT_PROCESSES
        {
            return Err(FfmpegError::InvalidConfiguration);
        }
        let ffmpeg_executable =
            resolve_executable(config.ffmpeg_executable.as_deref(), "ffmpeg").await?;
        let ffprobe_executable =
            resolve_executable(config.ffprobe_executable.as_deref(), "ffprobe").await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        runner
            .validate(
                &ffmpeg_executable,
                MediaExecutable::Ffmpeg,
                config.process_timeout,
                cancellation.clone(),
            )
            .await?;
        runner
            .validate(
                &ffprobe_executable,
                MediaExecutable::Ffprobe,
                config.process_timeout,
                cancellation,
            )
            .await?;
        let (pressure_sender, pressure) = watch::channel(ResourcePressure::Normal);
        Ok(Self {
            ffmpeg_executable,
            ffprobe_executable,
            process_timeout: config.process_timeout,
            max_output_bytes: config.max_output_bytes,
            process_slots: Arc::new(Semaphore::new(config.max_concurrent_processes)),
            pressure_sender,
            pressure,
            runner,
        })
    }

    pub fn ffmpeg_executable(&self) -> &Path {
        &self.ffmpeg_executable
    }

    pub fn ffprobe_executable(&self) -> &Path {
        &self.ffprobe_executable
    }

    pub fn set_resource_pressure(&self, pressure: ResourcePressure) {
        self.pressure_sender.send_replace(pressure);
    }

    pub fn progress_channel() -> (mpsc::Sender<MediaProgress>, mpsc::Receiver<MediaProgress>) {
        mpsc::channel(PROGRESS_CHANNEL_CAPACITY)
    }

    pub async fn probe(
        &self,
        input: impl AsRef<Path>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<MediaInfo, FfmpegError> {
        let _permit = self.acquire_slot(cancellation.clone()).await?;
        let input = canonical_input(input.as_ref()).await?;
        let input_file = tokio::fs::File::open(&input)
            .await
            .map_err(|_| FfmpegError::InvalidInput)?;
        let arguments = build_probe_arguments();
        let output = self
            .runner
            .probe(
                &self.ffprobe_executable,
                &arguments,
                input_file,
                self.process_timeout,
                cancellation,
            )
            .await?;
        parse_probe_output(&output)
    }

    /// Combines the first video stream from one file with the first audio
    /// stream from another, copying both streams without re-encoding.
    pub async fn merge_audio_video(
        &self,
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<MediaProcessOutput, FfmpegError> {
        let _permit = self.acquire_slot(cancellation.clone()).await?;
        if *cancellation.borrow() {
            return Err(FfmpegError::Cancelled);
        }

        let video = canonical_input(video_input.as_ref()).await?;
        let audio = canonical_input(audio_input.as_ref()).await?;
        let destination =
            canonical_input(output.as_ref().parent().unwrap_or(Path::new("."))).await?;
        let destination = destination.join(
            output
                .as_ref()
                .file_name()
                .ok_or(FfmpegError::InvalidOutput)?,
        );
        if tokio::fs::symlink_metadata(&destination).await.is_ok() {
            return Err(FfmpegError::DestinationExists);
        }

        let workspace = create_workspace(&destination).await?;
        let extension = destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("mp4");
        let extension = if extension.eq_ignore_ascii_case("mkv") {
            "mkv"
        } else {
            "mp4"
        };
        let temporary_output = workspace.join(format!("output.{extension}"));
        let arguments = [
            OsString::from("-y"),
            OsString::from("-v"),
            OsString::from("error"),
            OsString::from("-i"),
            video.as_os_str().to_owned(),
            OsString::from("-i"),
            audio.as_os_str().to_owned(),
            OsString::from("-map"),
            OsString::from("0:v:0"),
            OsString::from("-map"),
            OsString::from("1:a:0"),
            OsString::from("-c"),
            OsString::from("copy"),
            OsString::from("-shortest"),
            temporary_output.as_os_str().to_owned(),
        ];

        let mut child = Command::new(&self.ffmpeg_executable)
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| FfmpegError::ProcessStart)?;
        let success = match timeout(self.process_timeout, child.wait()).await {
            Ok(Ok(status)) => status.success(),
            Ok(Err(_)) => return Err(FfmpegError::ProcessRunner),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(FfmpegError::ProcessTimeout);
            }
        };
        if !success {
            let _ = tokio::fs::remove_dir_all(&workspace).await;
            return Err(FfmpegError::FfmpegFailed { code: None });
        }
        if tokio::fs::symlink_metadata(&temporary_output)
            .await
            .is_err()
        {
            return Err(FfmpegError::OutputUnavailable);
        }
        let size = tokio::fs::metadata(&temporary_output)
            .await
            .map_err(|_| FfmpegError::OutputUnavailable)?
            .len();
        tokio::fs::hard_link(&temporary_output, &destination)
            .await
            .map_err(|_| FfmpegError::InvalidOutput)?;
        let _ = tokio::fs::remove_dir_all(&workspace).await;
        Ok(MediaProcessOutput {
            destination,
            size_bytes: size,
        })
    }

    pub async fn process(
        &self,
        request: MediaProcessRequest,
        cancellation: watch::Receiver<bool>,
        progress: Option<mpsc::Sender<MediaProgress>>,
    ) -> Result<MediaProcessOutput, FfmpegError> {
        let _permit = self.acquire_slot(cancellation.clone()).await?;
        let input = canonical_input(&request.input).await?;
        let input_file = tokio::fs::File::open(&input)
            .await
            .map_err(|_| FfmpegError::InvalidInput)?;
        let destination = resolve_destination(&request.output, &input).await?;
        let workspace = create_workspace(&destination).await?;
        let temporary_output = workspace.join(format!("output.{}", request.operation.extension()));
        let arguments = build_process_arguments(&request.operation, &temporary_output);

        let result = self
            .run_and_publish(
                &arguments,
                input_file,
                OutputTarget {
                    temporary_output: &temporary_output,
                    destination: &destination,
                    max_output_bytes: self.max_output_bytes,
                },
                cancellation,
                progress,
            )
            .await;
        match tokio::fs::remove_dir_all(&workspace).await {
            Ok(()) => result,
            Err(_) if matches!(&result, Err(FfmpegError::Cancelled)) => {
                warn!("FFmpeg temporary workspace cleanup failed after cancellation");
                Err(FfmpegError::CleanupFailedAfterCancellation)
            }
            Err(_) => {
                warn!("FFmpeg temporary workspace cleanup failed");
                Err(FfmpegError::CleanupFailed)
            }
        }
    }

    async fn acquire_slot(
        &self,
        mut cancellation: watch::Receiver<bool>,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, FfmpegError> {
        let mut pressure = self.pressure.clone();
        loop {
            if *cancellation.borrow_and_update() {
                return Err(FfmpegError::Cancelled);
            }
            if *pressure.borrow_and_update() == ResourcePressure::Critical {
                tokio::select! {
                    biased;
                    _ = cancellation.changed() => return Err(FfmpegError::Cancelled),
                    changed = pressure.changed() => {
                        if changed.is_err() {
                            return Err(FfmpegError::ProcessRunner);
                        }
                    }
                }
                continue;
            }
            let permit = tokio::select! {
                biased;
                _ = cancellation.changed() => return Err(FfmpegError::Cancelled),
                changed = pressure.changed() => {
                    if changed.is_err() {
                        return Err(FfmpegError::ProcessRunner);
                    }
                    continue;
                }
                permit = Arc::clone(&self.process_slots).acquire_owned() => {
                    permit.map_err(|_| FfmpegError::ProcessRunner)?
                }
            };
            if *pressure.borrow_and_update() == ResourcePressure::Critical {
                drop(permit);
                continue;
            }
            return Ok(permit);
        }
    }

    async fn run_and_publish(
        &self,
        arguments: &[OsString],
        input: tokio::fs::File,
        target: OutputTarget<'_>,
        cancellation: watch::Receiver<bool>,
        progress: Option<mpsc::Sender<MediaProgress>>,
    ) -> Result<MediaProcessOutput, FfmpegError> {
        if *cancellation.borrow() {
            return Err(FfmpegError::Cancelled);
        }
        let exit = self
            .runner
            .process(
                &self.ffmpeg_executable,
                FfmpegProcessSpec {
                    arguments,
                    input,
                    output: target.temporary_output,
                    max_output_bytes: target.max_output_bytes,
                    timeout: self.process_timeout,
                    cancellation: cancellation.clone(),
                    progress,
                },
            )
            .await?;
        if *cancellation.borrow() {
            return Err(FfmpegError::Cancelled);
        }
        if !exit.success {
            return Err(FfmpegError::FfmpegFailed { code: exit.code });
        }
        let metadata = tokio::fs::symlink_metadata(target.temporary_output)
            .await
            .map_err(|_| FfmpegError::OutputUnavailable)?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return Err(FfmpegError::OutputUnavailable);
        }
        if metadata.len() > target.max_output_bytes {
            return Err(FfmpegError::OutputTooLarge);
        }
        if *cancellation.borrow() {
            return Err(FfmpegError::Cancelled);
        }
        match tokio::fs::hard_link(target.temporary_output, target.destination).await {
            Ok(()) => Ok(MediaProcessOutput {
                destination: target.destination.to_path_buf(),
                size_bytes: metadata.len(),
            }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(FfmpegError::DestinationExists)
            }
            Err(_) => Err(FfmpegError::InvalidOutput),
        }
    }
}

impl MediaProcessRequest {
    pub fn remux(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        container: MediaContainer,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            operation: MediaOperation::Remux { container },
        }
    }

    pub fn transcode(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        preset: TranscodePreset,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            operation: MediaOperation::Transcode { preset },
        }
    }
}

impl MediaOperation {
    fn extension(self) -> &'static str {
        match self {
            Self::Remux { container } => container.extension(),
            Self::Transcode { preset } => match preset {
                TranscodePreset::H264AacMp4 | TranscodePreset::HevcAacMp4 => "mp4",
                TranscodePreset::Vp9OpusWebM => "webm",
                TranscodePreset::Mp3Audio => "mp3",
                TranscodePreset::FlacAudio => "flac",
            },
        }
    }
}

fn build_probe_arguments() -> Vec<OsString> {
    vec![
        "-v".into(),
        "error".into(),
        "-protocol_whitelist".into(),
        "pipe".into(),
        "-show_format".into(),
        "-show_streams".into(),
        "-of".into(),
        "json".into(),
        "pipe:0".into(),
    ]
}

fn build_process_arguments(operation: &MediaOperation, output: &Path) -> Vec<OsString> {
    let mut arguments = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-n".into(),
        "-protocol_whitelist".into(),
        "pipe".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        "-i".into(),
        "pipe:0".into(),
    ];
    match operation {
        MediaOperation::Remux { container } => arguments.extend([
            "-map".into(),
            "0".into(),
            "-c".into(),
            "copy".into(),
            "-f".into(),
            container.muxer().into(),
        ]),
        MediaOperation::Transcode { preset } => match preset {
            TranscodePreset::H264AacMp4 => arguments.extend([
                "-map".into(),
                "0:v:0?".into(),
                "-map".into(),
                "0:a:0?".into(),
                "-c:v".into(),
                "libx264".into(),
                "-c:a".into(),
                "aac".into(),
                "-f".into(),
                "mp4".into(),
            ]),
            TranscodePreset::HevcAacMp4 => arguments.extend([
                "-map".into(),
                "0:v:0?".into(),
                "-map".into(),
                "0:a:0?".into(),
                "-c:v".into(),
                "libx265".into(),
                "-c:a".into(),
                "aac".into(),
                "-f".into(),
                "mp4".into(),
            ]),
            TranscodePreset::Vp9OpusWebM => arguments.extend([
                "-map".into(),
                "0:v:0?".into(),
                "-map".into(),
                "0:a:0?".into(),
                "-c:v".into(),
                "libvpx-vp9".into(),
                "-c:a".into(),
                "libopus".into(),
                "-f".into(),
                "webm".into(),
            ]),
            TranscodePreset::Mp3Audio => arguments.extend([
                "-map".into(),
                "0:a:0".into(),
                "-vn".into(),
                "-c:a".into(),
                "libmp3lame".into(),
                "-f".into(),
                "mp3".into(),
            ]),
            TranscodePreset::FlacAudio => arguments.extend([
                "-map".into(),
                "0:a:0".into(),
                "-vn".into(),
                "-c:a".into(),
                "flac".into(),
                "-f".into(),
                "flac".into(),
            ]),
        },
    }
    arguments.push(output.as_os_str().to_owned());
    arguments
}

async fn resolve_executable(
    configured: Option<&Path>,
    name: &'static str,
) -> Result<PathBuf, FfmpegError> {
    let not_found = match name {
        "ffmpeg" => FfmpegError::FfmpegNotFound,
        _ => FfmpegError::FfprobeNotFound,
    };
    let path = if let Some(path) = configured {
        path.to_path_buf()
    } else {
        discover_executable(name).await.ok_or(not_found)?
    };
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| match name {
            "ffmpeg" => FfmpegError::FfmpegNotFound,
            _ => FfmpegError::FfprobeNotFound,
        })?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| FfmpegError::ExecutableInvalid)?;
    if !metadata.is_file() {
        return Err(FfmpegError::ExecutableInvalid);
    }
    let mut executable = tokio::fs::File::open(&canonical)
        .await
        .map_err(|_| FfmpegError::ExecutableInvalid)?;
    let mut signature = [0_u8; 4];
    executable
        .read_exact(&mut signature)
        .await
        .map_err(|_| FfmpegError::ExecutableInvalid)?;
    if !is_native_executable_signature(signature) {
        return Err(FfmpegError::ExecutableInvalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(FfmpegError::ExecutableNotRunnable);
        }
    }
    Ok(canonical)
}

#[cfg(windows)]
fn is_native_executable_signature(signature: [u8; 4]) -> bool {
    signature[..2] == [0x4d, 0x5a]
}

#[cfg(target_os = "macos")]
fn is_native_executable_signature(signature: [u8; 4]) -> bool {
    matches!(
        signature,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_native_executable_signature(signature: [u8; 4]) -> bool {
    signature == [0x7f, b'E', b'L', b'F']
}

#[cfg(not(any(unix, windows)))]
fn is_native_executable_signature(_signature: [u8; 4]) -> bool {
    false
}

async fn discover_executable(name: &str) -> Option<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut directories = Vec::new();

    if cfg!(windows)
        && let Some(app_data) = std::env::var_os("APPDATA")
    {
        directories.push(
            PathBuf::from(app_data)
                .join("com.fetchx.desktop")
                .join("bin"),
        );
    }

    if let Some(path) = std::env::var_os("PATH") {
        directories.extend(std::env::split_paths(&path));
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(dir) = executable.parent() {
            directories.push(dir.to_path_buf());
            directories.push(dir.join("engines"));
            if let Some(parent) = dir.parent() {
                let resources = parent.join("resources").join("engines");
                directories.push(resources);
            }
        }
        if let Some(dir) = std::env::var_os("APPDIR").map(PathBuf::from) {
            directories.push(dir.join("Contents").join("Resources").join("engines"));
            directories.push(dir.join("engines"));
        }
    }

    for directory in directories {
        if !directory.is_absolute() || !seen.insert(directory.clone()) {
            continue;
        }
        if directory.exists() {
            for candidate_name in executable_names(name) {
                let candidate = directory.join(candidate_name);
                if tokio::fs::metadata(&candidate)
                    .await
                    .is_ok_and(|metadata| metadata.is_file())
                {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn executable_names(name: &str) -> Vec<OsString> {
    if cfg!(windows) {
        vec![format!("{name}.exe").into()]
    } else {
        vec![name.into()]
    }
}

async fn canonical_input(path: &Path) -> Result<PathBuf, FfmpegError> {
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| FfmpegError::InvalidInput)?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| FfmpegError::InvalidInput)?;
    if !metadata.is_file() {
        return Err(FfmpegError::InvalidInput);
    }
    Ok(canonical)
}

async fn resolve_destination(path: &Path, input: &Path) -> Result<PathBuf, FfmpegError> {
    let file_name = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or(FfmpegError::InvalidOutput)?;
    if file_name == "." || file_name == ".." {
        return Err(FfmpegError::InvalidOutput);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let parent = match parent {
        Some(parent) => tokio::fs::canonicalize(parent)
            .await
            .map_err(|_| FfmpegError::InvalidOutput)?,
        None => tokio::fs::canonicalize(".")
            .await
            .map_err(|_| FfmpegError::InvalidOutput)?,
    };
    let destination = parent.join(file_name);
    if same_path(input, &destination) {
        return Err(FfmpegError::InputOutputCollision);
    }
    match tokio::fs::symlink_metadata(&destination).await {
        Ok(_) => return Err(FfmpegError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(FfmpegError::InvalidOutput),
    }
    Ok(destination)
}

#[cfg(windows)]
fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

#[cfg(not(windows))]
fn same_path(left: &Path, right: &Path) -> bool {
    left == right
}

async fn create_workspace(destination: &Path) -> Result<PathBuf, FfmpegError> {
    let parent = destination.parent().ok_or(FfmpegError::InvalidOutput)?;
    for _ in 0..3 {
        let workspace = parent.join(format!(".fetchx-ffmpeg-{}", Uuid::new_v4()));
        match tokio::fs::create_dir(&workspace).await {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if tokio::fs::set_permissions(
                        &workspace,
                        std::fs::Permissions::from_mode(0o700),
                    )
                    .await
                    .is_err()
                    {
                        let _ = tokio::fs::remove_dir(&workspace).await;
                        return Err(FfmpegError::WorkspaceUnavailable);
                    }
                }
                return Ok(workspace);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(FfmpegError::WorkspaceUnavailable),
        }
    }
    Err(FfmpegError::WorkspaceUnavailable)
}

#[derive(Deserialize)]
struct ProbeResponse {
    #[serde(default)]
    format: Option<ProbeFormat>,
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    size: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    avg_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<i64>,
}

fn parse_probe_output(bytes: &[u8]) -> Result<MediaInfo, FfmpegError> {
    if bytes.len() > MAX_PROBE_OUTPUT_BYTES {
        return Err(FfmpegError::MetadataOutputTooLarge);
    }
    let response: ProbeResponse =
        serde_json::from_slice(bytes).map_err(|_| FfmpegError::InvalidMediaMetadata)?;
    if response.streams.len() > MAX_MEDIA_STREAMS {
        return Err(FfmpegError::InvalidMediaMetadata);
    }
    let format = response.format;
    let mut streams = Vec::with_capacity(response.streams.len());
    for stream in response.streams {
        let width = optional_dimension(stream.width)?;
        let height = optional_dimension(stream.height)?;
        let channels = optional_channels(stream.channels)?;
        streams.push(MediaStream {
            kind: parse_stream_kind(stream.codec_type.as_deref()),
            codec: stream.codec_name,
            width,
            height,
            frame_rate: stream
                .avg_frame_rate
                .as_deref()
                .map(parse_frame_rate)
                .transpose()?
                .flatten(),
            sample_rate_hz: stream
                .sample_rate
                .as_deref()
                .map(parse_optional_sample_rate)
                .transpose()?
                .flatten(),
            channels,
        });
    }
    Ok(MediaInfo {
        duration: format
            .as_ref()
            .and_then(|format| format.duration.as_deref())
            .map(parse_duration)
            .transpose()?
            .flatten(),
        format_name: format
            .as_ref()
            .and_then(|format| format.format_name.clone()),
        size_bytes: format
            .as_ref()
            .and_then(|format| format.size.as_deref())
            .map(parse_optional_unsigned)
            .transpose()?
            .flatten(),
        bit_rate_bits_per_second: format
            .as_ref()
            .and_then(|format| format.bit_rate.as_deref())
            .map(parse_optional_unsigned)
            .transpose()?
            .flatten(),
        streams,
    })
}

fn parse_stream_kind(kind: Option<&str>) -> MediaStreamKind {
    match kind {
        Some("video") => MediaStreamKind::Video,
        Some("audio") => MediaStreamKind::Audio,
        Some("subtitle") => MediaStreamKind::Subtitle,
        Some("data") => MediaStreamKind::Data,
        Some("attachment") => MediaStreamKind::Attachment,
        _ => MediaStreamKind::Other,
    }
}

fn optional_dimension(value: Option<i64>) -> Result<Option<u32>, FfmpegError> {
    value.map(parse_nonnegative_u32).transpose()
}

fn optional_channels(value: Option<i64>) -> Result<Option<u16>, FfmpegError> {
    value
        .map(|value| u16::try_from(value).map_err(|_| FfmpegError::InvalidMediaMetadata))
        .transpose()
}

fn parse_nonnegative_u32(value: i64) -> Result<u32, FfmpegError> {
    u32::try_from(value).map_err(|_| FfmpegError::InvalidMediaMetadata)
}

fn parse_optional_unsigned(value: &str) -> Result<Option<u64>, FfmpegError> {
    if value.is_empty() || matches!(value, "N/A" | "NA" | "None") {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| FfmpegError::InvalidMediaMetadata)
}

fn parse_optional_sample_rate(value: &str) -> Result<Option<u32>, FfmpegError> {
    parse_optional_unsigned(value)?
        .map(|value| u32::try_from(value).map_err(|_| FfmpegError::InvalidMediaMetadata))
        .transpose()
}

fn parse_duration(value: &str) -> Result<Option<Duration>, FfmpegError> {
    if value.is_empty() || matches!(value, "N/A" | "NA" | "None") {
        return Ok(None);
    }
    let seconds = value
        .parse::<f64>()
        .map_err(|_| FfmpegError::InvalidMediaMetadata)?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(FfmpegError::InvalidMediaMetadata);
    }
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|_| FfmpegError::InvalidMediaMetadata)
}

fn parse_frame_rate(value: &str) -> Result<Option<FrameRate>, FfmpegError> {
    if value.is_empty() || matches!(value, "N/A" | "NA" | "None") {
        return Ok(None);
    }
    let (numerator, denominator) = value
        .split_once('/')
        .ok_or(FfmpegError::InvalidMediaMetadata)?;
    let numerator = numerator
        .parse::<u32>()
        .map_err(|_| FfmpegError::InvalidMediaMetadata)?;
    let denominator = denominator
        .parse::<u32>()
        .map_err(|_| FfmpegError::InvalidMediaMetadata)?;
    if denominator == 0 {
        return Ok(None);
    }
    Ok(Some(FrameRate {
        numerator,
        denominator,
    }))
}

struct TokioFfmpegRunner;

impl FfmpegProcessRunner for TokioFfmpegRunner {
    fn validate<'a>(
        &'a self,
        executable: &'a Path,
        kind: MediaExecutable,
        process_timeout: Duration,
        cancellation: watch::Receiver<bool>,
    ) -> FfmpegFuture<'a, Result<(), FfmpegError>> {
        Box::pin(async move {
            let (exit, output) = run_capture(
                executable,
                &["-version".into()],
                process_timeout.min(Duration::from_secs(10)),
                cancellation,
                None,
                MAX_VERSION_OUTPUT_BYTES,
            )
            .await?;
            let expected = match kind {
                MediaExecutable::Ffmpeg => "ffmpeg version",
                MediaExecutable::Ffprobe => "ffprobe version",
            };
            let output = String::from_utf8(output).map_err(|_| FfmpegError::ExecutableInvalid)?;
            if !exit.success || !output.to_ascii_lowercase().contains(expected) {
                return Err(FfmpegError::ExecutableInvalid);
            }
            Ok(())
        })
    }

    fn probe<'a>(
        &'a self,
        executable: &'a Path,
        arguments: &'a [OsString],
        input: tokio::fs::File,
        process_timeout: Duration,
        cancellation: watch::Receiver<bool>,
    ) -> FfmpegFuture<'a, Result<Vec<u8>, FfmpegError>> {
        Box::pin(async move {
            let (exit, output) = run_capture(
                executable,
                arguments,
                process_timeout,
                cancellation,
                Some(input),
                MAX_PROBE_OUTPUT_BYTES,
            )
            .await?;
            if !exit.success {
                return Err(FfmpegError::FfprobeFailed { code: exit.code });
            }
            Ok(output)
        })
    }

    fn process<'a>(
        &'a self,
        executable: &'a Path,
        spec: FfmpegProcessSpec<'a>,
    ) -> FfmpegFuture<'a, Result<FfmpegProcessExit, FfmpegError>> {
        Box::pin(async move { run_progress_process(executable, spec).await })
    }
}

async fn run_capture(
    executable: &Path,
    arguments: &[OsString],
    process_timeout: Duration,
    cancellation: watch::Receiver<bool>,
    input: Option<tokio::fs::File>,
    output_limit: usize,
) -> Result<(FfmpegProcessExit, Vec<u8>), FfmpegError> {
    let (exit, output) = run_child(
        executable,
        arguments,
        process_timeout,
        cancellation,
        input,
        None,
        PipeMode::Capture(output_limit),
    )
    .await?;
    if output.len() > output_limit {
        return Err(FfmpegError::MetadataOutputTooLarge);
    }
    Ok((exit, output))
}

enum PipeMode {
    Capture(usize),
    Progress(Option<mpsc::Sender<MediaProgress>>),
}

enum StdoutReader {
    Capture(JoinHandle<Result<Vec<u8>, FfmpegError>>),
    Progress(JoinHandle<Result<(), FfmpegError>>),
}

async fn run_child(
    executable: &Path,
    arguments: &[OsString],
    process_timeout: Duration,
    mut cancellation: watch::Receiver<bool>,
    input: Option<tokio::fs::File>,
    output_limit: Option<(PathBuf, u64)>,
    pipe_mode: PipeMode,
) -> Result<(FfmpegProcessExit, Vec<u8>), FfmpegError> {
    if *cancellation.borrow_and_update() {
        return Err(FfmpegError::Cancelled);
    }
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Suppress the console window that would otherwise flash on Windows.
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| FfmpegError::ProcessStart)?;
    let input_writer = match (input, child.stdin.take()) {
        (Some(mut input), Some(mut stdin)) => Some(tokio::spawn(async move {
            tokio::io::copy(&mut input, &mut stdin)
                .await
                .map(|_| ())
                .map_err(|_| FfmpegError::ProcessRunner)
        })),
        (None, None) => None,
        _ => return Err(FfmpegError::ProcessRunner),
    };
    let stdout = child.stdout.take().ok_or(FfmpegError::ProcessRunner)?;
    let stderr = child.stderr.take().ok_or(FfmpegError::ProcessRunner)?;
    let (oversized_sender, mut oversized_receiver) = watch::channel(false);
    let stdout_reader = match pipe_mode {
        PipeMode::Capture(limit) => {
            StdoutReader::Capture(tokio::spawn(read_capture(stdout, limit, oversized_sender)))
        }
        PipeMode::Progress(progress) => {
            StdoutReader::Progress(tokio::spawn(read_progress(stdout, progress)))
        }
    };
    let stderr_reader = tokio::spawn(drain_output(stderr));
    let deadline = Instant::now() + process_timeout;
    let mut output_size_check = tokio::time::interval(OUTPUT_SIZE_CHECK_INTERVAL);
    let mut oversized_channel_open = true;
    let mut cancellation_requested = false;
    let exit = loop {
        tokio::select! {
            biased;
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow_and_update() {
                    cancellation_requested = true;
                    if let Some(status) = child.try_wait().map_err(|_| FfmpegError::ProcessRunner)? {
                        break status;
                    }
                    terminate_child(&mut child).await?;
                    finish_input_writer(input_writer, false).await?;
                    await_stopped_readers(stdout_reader, stderr_reader).await?;
                    return Err(FfmpegError::Cancelled);
                }
            }
            changed = oversized_receiver.changed(), if oversized_channel_open => {
                if changed.is_err() {
                    oversized_channel_open = false;
                } else if *oversized_receiver.borrow_and_update() {
                    terminate_child(&mut child).await?;
                    finish_input_writer(input_writer, false).await?;
                    await_stopped_readers(stdout_reader, stderr_reader).await?;
                    return Err(FfmpegError::MetadataOutputTooLarge);
                }
            }
            _ = output_size_check.tick(), if output_limit.is_some() => {
                if let Some((path, maximum)) = &output_limit {
                    match tokio::fs::metadata(path).await {
                        Ok(metadata) if metadata.len() > *maximum => {
                            terminate_child(&mut child).await?;
                            finish_input_writer(input_writer, false).await?;
                            await_stopped_readers(stdout_reader, stderr_reader).await?;
                            return Err(FfmpegError::OutputTooLarge);
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(_) => {
                            terminate_child(&mut child).await?;
                            finish_input_writer(input_writer, false).await?;
                            await_stopped_readers(stdout_reader, stderr_reader).await?;
                            return Err(FfmpegError::OutputUnavailable);
                        }
                    }
                }
            }
            _ = sleep_until(deadline) => {
                terminate_child(&mut child).await?;
                finish_input_writer(input_writer, false).await?;
                await_stopped_readers(stdout_reader, stderr_reader).await?;
                return Err(FfmpegError::ProcessTimeout);
            }
            status = child.wait() => break status.map_err(|_| FfmpegError::ProcessRunner)?,
        }
    };
    cancellation_requested |= *cancellation.borrow();
    finish_input_writer(input_writer, exit.success() && !cancellation_requested).await?;
    if cancellation_requested {
        return Err(FfmpegError::Cancelled);
    }
    let output = match stdout_reader {
        StdoutReader::Capture(reader) => await_capture_reader(reader).await?,
        StdoutReader::Progress(reader) => {
            await_unit_reader(reader).await?;
            Vec::new()
        }
    };
    await_reader(stderr_reader).await?;

    Ok((
        FfmpegProcessExit {
            success: exit.success(),
            code: exit.code(),
        },
        output,
    ))
}

async fn run_progress_process(
    executable: &Path,
    spec: FfmpegProcessSpec<'_>,
) -> Result<FfmpegProcessExit, FfmpegError> {
    let FfmpegProcessSpec {
        arguments,
        input,
        output,
        max_output_bytes,
        timeout,
        cancellation,
        progress,
    } = spec;
    run_child(
        executable,
        arguments,
        timeout,
        cancellation,
        Some(input),
        Some((output.to_path_buf(), max_output_bytes)),
        PipeMode::Progress(progress),
    )
    .await
    .map(|(exit, _)| exit)
}

async fn finish_input_writer(
    writer: Option<JoinHandle<Result<(), FfmpegError>>>,
    propagate_error: bool,
) -> Result<(), FfmpegError> {
    let Some(mut writer) = writer else {
        return Ok(());
    };
    match timeout(PROCESS_CLEANUP_TIMEOUT, &mut writer).await {
        Ok(Ok(result)) if propagate_error => result,
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Err(FfmpegError::ProcessRunner),
        Err(_) => {
            writer.abort();
            Err(FfmpegError::ProcessCleanup)
        }
    }
}

async fn terminate_child(child: &mut Child) -> Result<ExitStatus, FfmpegError> {
    if child.id().is_none() {
        return timeout(PROCESS_CLEANUP_TIMEOUT, child.wait())
            .await
            .map_err(|_| FfmpegError::ProcessCleanup)?
            .map_err(|_| FfmpegError::ProcessCleanup);
    }
    match timeout(PROCESS_CLEANUP_TIMEOUT, child.kill()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == io::ErrorKind::InvalidInput => {}
        Ok(Err(_)) | Err(_) => return Err(FfmpegError::ProcessCleanup),
    }
    timeout(PROCESS_CLEANUP_TIMEOUT, child.wait())
        .await
        .map_err(|_| FfmpegError::ProcessCleanup)?
        .map_err(|_| FfmpegError::ProcessCleanup)
}

async fn await_reader(reader: JoinHandle<Result<(), FfmpegError>>) -> Result<(), FfmpegError> {
    match timeout(PROCESS_CLEANUP_TIMEOUT, reader).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(FfmpegError::ProcessRunner),
        Err(_) => Err(FfmpegError::ProcessCleanup),
    }
}

async fn await_stopped_readers(
    stdout: StdoutReader,
    stderr: JoinHandle<Result<(), FfmpegError>>,
) -> Result<(), FfmpegError> {
    match stdout {
        StdoutReader::Capture(reader) => match await_capture_reader(reader).await {
            Ok(_) | Err(FfmpegError::MetadataOutputTooLarge) => {}
            Err(error) => return Err(error),
        },
        StdoutReader::Progress(reader) => await_unit_reader(reader).await?,
    }
    await_reader(stderr).await
}

async fn await_capture_reader(
    reader: JoinHandle<Result<Vec<u8>, FfmpegError>>,
) -> Result<Vec<u8>, FfmpegError> {
    match timeout(PROCESS_CLEANUP_TIMEOUT, reader).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(FfmpegError::ProcessRunner),
        Err(_) => Err(FfmpegError::ProcessCleanup),
    }
}

async fn await_unit_reader(reader: JoinHandle<Result<(), FfmpegError>>) -> Result<(), FfmpegError> {
    match timeout(PROCESS_CLEANUP_TIMEOUT, reader).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(FfmpegError::ProcessRunner),
        Err(_) => Err(FfmpegError::ProcessCleanup),
    }
}

async fn read_capture<R>(
    reader: R,
    output_limit: usize,
    oversized: watch::Sender<bool>,
) -> Result<Vec<u8>, FfmpegError>
where
    R: AsyncRead + Unpin,
{
    let mut reader = reader;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| FfmpegError::ProcessRunner)?;
        if count == 0 {
            break;
        }
        if output.len().saturating_add(count) > output_limit {
            let _ = oversized.send(true);
            return Err(FfmpegError::MetadataOutputTooLarge);
        }
        output.extend_from_slice(&buffer[..count]);
    }
    Ok(output)
}

async fn read_progress<R>(
    reader: R,
    progress: Option<mpsc::Sender<MediaProgress>>,
) -> Result<(), FfmpegError>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    loop {
        let Some((line, too_large)) = read_bounded_line(&mut reader, MAX_PROGRESS_LINE_BYTES)
            .await
            .map_err(|_| FfmpegError::ProcessRunner)?
        else {
            return Ok(());
        };
        if too_large {
            continue;
        }
        if let (Some(sender), Some(update)) = (&progress, parse_progress_line(&line))
            && sender.try_send(update).is_err()
        {
            continue;
        }
    }
}

async fn drain_output<R>(mut reader: R) -> Result<(), FfmpegError>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| FfmpegError::ProcessRunner)?;
        if count == 0 {
            return Ok(());
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

fn parse_progress_line(line: &[u8]) -> Option<MediaProgress> {
    let line = std::str::from_utf8(line).ok()?;
    let (key, value) = line.split_once('=')?;
    if key != "out_time_us" {
        return None;
    }
    let micros = value.parse::<u64>().ok()?;
    Some(MediaProgress {
        output_time: Duration::from_micros(micros),
    })
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        ffi::OsString,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tokio::io::AsyncWriteExt;
    use tokio::sync::{mpsc, watch};

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        state: Arc<Mutex<FakeRunnerState>>,
    }

    struct FakeRunnerState {
        calls: Vec<(String, Vec<String>)>,
        probe_output: Vec<u8>,
        exit: FfmpegProcessExit,
        create_output: bool,
        cancel_on_process: bool,
        process_started: Option<Arc<tokio::sync::Notify>>,
    }

    impl Default for FakeRunnerState {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                probe_output: Vec::new(),
                exit: FfmpegProcessExit {
                    success: true,
                    code: Some(0),
                },
                create_output: false,
                cancel_on_process: false,
                process_started: None,
            }
        }
    }

    impl FfmpegProcessRunner for FakeRunner {
        fn validate<'a>(
            &'a self,
            executable: &'a Path,
            _kind: MediaExecutable,
            _timeout: Duration,
            _cancellation: watch::Receiver<bool>,
        ) -> FfmpegFuture<'a, Result<(), FfmpegError>> {
            let mut state = self.state.lock().expect("fake runner mutex");
            state.calls.push((
                executable.to_string_lossy().into_owned(),
                vec!["-version".into()],
            ));
            Box::pin(async { Ok(()) })
        }

        fn probe<'a>(
            &'a self,
            executable: &'a Path,
            arguments: &'a [OsString],
            _input: tokio::fs::File,
            _timeout: Duration,
            _cancellation: watch::Receiver<bool>,
        ) -> FfmpegFuture<'a, Result<Vec<u8>, FfmpegError>> {
            let mut state = self.state.lock().expect("fake runner mutex");
            state.calls.push((
                executable.to_string_lossy().into_owned(),
                to_strings(arguments),
            ));
            let output = state.probe_output.clone();
            Box::pin(async move { Ok(output) })
        }

        fn process<'a>(
            &'a self,
            executable: &'a Path,
            spec: FfmpegProcessSpec<'a>,
        ) -> FfmpegFuture<'a, Result<FfmpegProcessExit, FfmpegError>> {
            let FfmpegProcessSpec {
                arguments,
                input: _input,
                cancellation,
                ..
            } = spec;
            let mut cancellation = cancellation;
            let (exit, create_output, cancel_on_process, process_started, arguments) = {
                let mut state = self.state.lock().expect("fake runner mutex");
                state.calls.push((
                    executable.to_string_lossy().into_owned(),
                    to_strings(arguments),
                ));
                (
                    state.exit,
                    state.create_output,
                    state.cancel_on_process,
                    state.process_started.clone(),
                    to_strings(arguments),
                )
            };
            Box::pin(async move {
                if let Some(started) = process_started {
                    started.notify_one();
                }
                if cancel_on_process {
                    let _ = cancellation.changed().await;
                    return Err(FfmpegError::Cancelled);
                }
                if create_output && let Some(path) = arguments.last() {
                    tokio::fs::write(path, b"transcoded")
                        .await
                        .map_err(|_| FfmpegError::ProcessRunner)?;
                }
                Ok(exit)
            })
        }
    }

    fn to_strings(arguments: &[OsString]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    async fn setup(runner: FakeRunner, base: &Path) -> Result<FfmpegProcessor, Box<dyn Error>> {
        tokio::fs::create_dir_all(base).await?;
        let executable = std::env::current_exe()?;
        Ok(FfmpegProcessor::initialize_with_runner(
            FfmpegConfig::new()
                .with_ffmpeg_executable(&executable)
                .with_ffprobe_executable(&executable),
            Arc::new(runner),
        )
        .await?)
    }

    async fn setup_with_paths(
        runner: FakeRunner,
        ffmpeg_executable: PathBuf,
        ffprobe_executable: PathBuf,
    ) -> Result<FfmpegProcessor, FfmpegError> {
        FfmpegProcessor::initialize_with_runner(
            FfmpegConfig::new()
                .with_ffmpeg_executable(ffmpeg_executable)
                .with_ffprobe_executable(ffprobe_executable),
            Arc::new(runner),
        )
        .await
    }

    fn parse_fixture() -> &'static [u8] {
        br#"{"format":{"format_name":"mov,mp4","duration":"12.5","size":"1024","bit_rate":"650"},"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30000/1001"},{"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}]}"#
    }

    #[test]
    fn parses_probe_metadata_and_missing_values() -> Result<(), FfmpegError> {
        let info = parse_probe_output(parse_fixture())?;
        assert_eq!(info.format_name.as_deref(), Some("mov,mp4"));
        assert_eq!(info.duration, Some(Duration::from_millis(12_500)));
        assert_eq!(info.size_bytes, Some(1024));
        assert_eq!(info.bit_rate_bits_per_second, Some(650));
        assert_eq!(info.streams.len(), 2);
        assert_eq!(info.streams[0].kind, MediaStreamKind::Video);
        assert_eq!(info.streams[0].width, Some(1920));
        assert_eq!(
            info.streams[0].frame_rate,
            Some(FrameRate {
                numerator: 30000,
                denominator: 1001
            })
        );
        assert_eq!(info.streams[1].sample_rate_hz, Some(48000));
        assert_eq!(info.streams[1].channels, Some(2));

        let partial = parse_probe_output(br#"{"streams":[{"codec_type":"other"}]}"#)?;
        assert_eq!(partial.duration, None);
        assert_eq!(partial.streams[0].kind, MediaStreamKind::Other);
        Ok(())
    }

    #[test]
    fn rejects_malformed_or_out_of_range_probe_metadata() {
        assert!(matches!(
            parse_probe_output(b"not json"),
            Err(FfmpegError::InvalidMediaMetadata)
        ));
        assert!(matches!(
            parse_probe_output(br#"{"streams":[{"width":-1}]}"#),
            Err(FfmpegError::InvalidMediaMetadata)
        ));
        assert!(matches!(
            parse_probe_output(br#"{"format":{"duration":"NaN"}}"#),
            Err(FfmpegError::InvalidMediaMetadata)
        ));
        assert!(matches!(
            parse_probe_output(br#"{"streams":[{"avg_frame_rate":"25/0"}]}"#),
            Ok(MediaInfo {
                streams,
                ..
            }) if streams[0].frame_rate.is_none()
        ));
        assert!(matches!(
            parse_probe_output(&vec![b' '; MAX_PROBE_OUTPUT_BYTES + 1]),
            Err(FfmpegError::MetadataOutputTooLarge)
        ));
    }

    #[test]
    fn progress_parser_is_bounded_and_ignores_unknown_or_malformed_fields() {
        assert_eq!(
            parse_progress_line(b"out_time_us=1250000"),
            Some(MediaProgress {
                output_time: Duration::from_micros(1_250_000),
            })
        );
        assert_eq!(parse_progress_line(b"progress=continue"), None);
        assert_eq!(parse_progress_line(b"out_time_us=NA"), None);
        assert_eq!(parse_progress_line(b"out_time_us=-1"), None);
        assert_eq!(
            parse_progress_line(&vec![b'x'; MAX_PROGRESS_LINE_BYTES + 1]),
            None
        );
    }

    #[tokio::test]
    async fn progress_reader_handles_partial_and_oversized_lines() -> Result<(), Box<dyn Error>> {
        let (mut writer, reader) = tokio::io::duplex(64);
        let (sender, mut receiver) = mpsc::channel(4);
        let reader_task = tokio::spawn(read_progress(reader, Some(sender)));
        writer.write_all(b"out_time_us=125").await?;
        writer
            .write_all(b"0000\nprogress=continue\nout_time_us=NA\n")
            .await?;
        writer
            .write_all(format!("ignored={}\n", "x".repeat(MAX_PROGRESS_LINE_BYTES)).as_bytes())
            .await?;
        drop(writer);

        assert_eq!(
            receiver.recv().await,
            Some(MediaProgress {
                output_time: Duration::from_micros(1_250_000),
            })
        );
        assert_eq!(receiver.recv().await, None);
        reader_task.await??;
        Ok(())
    }

    #[test]
    fn arguments_are_structured_and_only_use_closed_options() {
        let input = Path::new(r#"C:\media\source with "quotes".mkv"#);
        let output =
            Path::new(r#"C:\media\odd "quote"; 'apostrophe' & | `tick` $(false) åª’ä½“.mp4"#);
        let arguments = build_process_arguments(
            &MediaOperation::Remux {
                container: MediaContainer::Mp4,
            },
            output,
        );
        assert!(
            arguments
                .iter()
                .any(|arg| arg == std::ffi::OsStr::new("-i"))
        );
        assert!(arguments.iter().any(|arg| arg == output.as_os_str()));
        assert!(
            arguments
                .iter()
                .any(|arg| arg == std::ffi::OsStr::new("-n"))
        );
        assert!(
            arguments
                .iter()
                .any(|arg| arg == std::ffi::OsStr::new("pipe:1"))
        );
        assert!(
            arguments
                .windows(2)
                .any(|window| window == ["-protocol_whitelist", "pipe"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|window| window == ["-i", "pipe:0"])
        );
        assert!(!arguments.iter().any(|arg| arg == input.as_os_str()));
        assert!(
            !arguments
                .iter()
                .any(|arg| arg.to_string_lossy().contains("&&"))
        );

        let transcoding = build_process_arguments(
            &MediaOperation::Transcode {
                preset: TranscodePreset::H264AacMp4,
            },
            output,
        );
        assert!(
            transcoding
                .windows(2)
                .any(|window| window == ["-c:v", "libx264"])
        );
    }

    #[tokio::test]
    async fn configured_executables_are_validated_and_ffprobe_is_selected()
    -> Result<(), Box<dyn Error>> {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&base).await?;
        let executable = std::env::current_exe()?;
        let runner = FakeRunner::default();
        let state = Arc::clone(&runner.state);
        let processor = setup_with_paths(runner, executable.clone(), executable.clone()).await?;
        assert_eq!(
            processor.ffmpeg_executable(),
            tokio::fs::canonicalize(&executable).await?
        );
        assert_eq!(
            processor.ffprobe_executable(),
            processor.ffmpeg_executable()
        );

        let input_names: &[&str] = if cfg!(windows) {
            &[
                "media with spaces.mkv",
                "apostrophe's.mkv",
                "unicode-äöü.mkv",
                "semi;colon.mkv",
                "amp&ersand.mkv",
                "backtick`file.mkv",
                "$(false).mkv",
                "(parentheses).mkv",
                "-leading-hyphen.mkv",
                "..-traversal-like.mkv",
            ]
        } else {
            &[
                "media with spaces.mkv",
                r#"quoted"name.mkv"#,
                "apostrophe's.mkv",
                "unicode-äöü.mkv",
                "semi;colon.mkv",
                "amp&ersand.mkv",
                "pipe|and`backtick.mkv",
                "$(false).mkv",
                "(parentheses).mkv",
                "-leading-hyphen.mkv",
                "..-traversal-like.mkv",
            ]
        };
        state.lock().expect("fake runner mutex").probe_output = parse_fixture().to_vec();
        let (_cancel_sender, cancellation) = watch::channel(false);
        for name in input_names {
            let input = base.join(name);
            tokio::fs::write(&input, b"media").await?;
            let info = processor.probe(&input, cancellation.clone()).await?;
            assert_eq!(info.streams.len(), 2);
        }
        {
            let calls = state.lock().expect("fake runner mutex");
            let probe_calls: Vec<_> = calls
                .calls
                .iter()
                .filter(|(_, args)| args.contains(&"-show_streams".to_owned()))
                .collect();
            assert_eq!(probe_calls.len(), input_names.len());
            for (_, arguments) in probe_calls {
                assert!(arguments.contains(&"pipe:0".to_owned()));
                assert!(
                    arguments
                        .windows(2)
                        .any(|window| window == ["-protocol_whitelist", "pipe"])
                );
                assert!(arguments.contains(&"json".to_owned()));
                assert!(!input_names.iter().any(|name| {
                    arguments.contains(&base.join(name).to_string_lossy().into_owned())
                }));
            }
        }

        assert!(matches!(
            setup_with_paths(
                FakeRunner::default(),
                base.join("missing-ffmpeg.exe"),
                executable.clone(),
            )
            .await,
            Err(FfmpegError::FfmpegNotFound)
        ));
        assert!(matches!(
            setup_with_paths(FakeRunner::default(), base.clone(), executable).await,
            Err(FfmpegError::ExecutableInvalid)
        ));
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn processor_rejects_unbounded_process_configuration() -> Result<(), Box<dyn Error>> {
        let executable = std::env::current_exe()?;
        for config in [
            FfmpegConfig::new().with_max_concurrent_processes(0),
            FfmpegConfig::new().with_max_concurrent_processes(MAX_CONCURRENT_PROCESSES + 1),
            FfmpegConfig::new().with_process_timeout(Duration::ZERO),
            FfmpegConfig::new().with_process_timeout(MAX_PROCESS_TIMEOUT + Duration::from_secs(1)),
            FfmpegConfig::new().with_max_output_bytes(0),
            FfmpegConfig::new().with_max_output_bytes(MAX_OUTPUT_BYTES + 1),
        ] {
            let config = config
                .with_ffmpeg_executable(&executable)
                .with_ffprobe_executable(&executable);
            assert!(matches!(
                FfmpegProcessor::initialize_with_runner(config, Arc::new(FakeRunner::default()))
                    .await,
                Err(FfmpegError::InvalidConfiguration)
            ));
        }
        Ok(())
    }

    #[tokio::test]
    async fn critical_pressure_gates_new_ffmpeg_processes_until_relieved()
    -> Result<(), Box<dyn Error>> {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-pressure-{}", Uuid::new_v4()));
        let processor = setup(FakeRunner::default(), &base).await?;
        processor.set_resource_pressure(ResourcePressure::Critical);
        let (_cancel_sender, cancellation) = watch::channel(false);

        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                processor.acquire_slot(cancellation.clone()),
            )
            .await
            .is_err()
        );
        processor.set_resource_pressure(ResourcePressure::Normal);
        let permit =
            tokio::time::timeout(Duration::from_secs(1), processor.acquire_slot(cancellation))
                .await??;
        drop(permit);
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn remux_publishes_without_overwriting_and_cleans_workspace() -> Result<(), Box<dyn Error>>
    {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-test-{}", Uuid::new_v4()));
        let runner = FakeRunner::default();
        runner
            .state
            .lock()
            .expect("fake runner mutex")
            .create_output = true;
        let processor = setup(runner, &base).await?;
        let input = base.join("input with spaces; &.mkv");
        let output = base.join("output with spaces; &.mp4");
        tokio::fs::write(&input, b"source").await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        let result = processor
            .process(
                MediaProcessRequest::remux(input.clone(), output.clone(), MediaContainer::Mp4),
                cancellation.clone(),
                None,
            )
            .await?;
        assert_eq!(
            result.destination,
            tokio::fs::canonicalize(&base)
                .await?
                .join(output.file_name().ok_or("missing output file name")?)
        );
        assert_eq!(result.size_bytes, 10);
        assert_eq!(tokio::fs::read(&input).await?, b"source");
        assert_eq!(tokio::fs::read(&result.destination).await?, b"transcoded");
        assert!(matches!(
            processor
                .process(
                    MediaProcessRequest::remux(
                        input,
                        result.destination.clone(),
                        MediaContainer::Mp4
                    ),
                    cancellation,
                    None,
                )
                .await,
            Err(FfmpegError::DestinationExists)
        ));
        let mut entries = tokio::fs::read_dir(&base).await?;
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            names.push(entry.file_name());
        }
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|name| name == "input with spaces; &.mkv"));
        assert!(names.iter().any(|name| name == "output with spaces; &.mp4"));
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn process_failure_and_cancellation_preserve_input_and_remove_workspace()
    -> Result<(), Box<dyn Error>> {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-test-{}", Uuid::new_v4()));
        let runner = FakeRunner::default();
        {
            let mut state = runner.state.lock().expect("fake runner mutex");
            state.exit = FfmpegProcessExit {
                success: false,
                code: Some(2),
            };
            state.create_output = true;
        }
        let processor = setup(runner, &base).await?;
        let input = base.join("source.mkv");
        let output = base.join("destination.mp4");
        tokio::fs::write(&input, b"source").await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            processor
                .process(
                    MediaProcessRequest::remux(input.clone(), output.clone(), MediaContainer::Mp4),
                    cancellation,
                    None,
                )
                .await,
            Err(FfmpegError::FfmpegFailed { code: Some(2) })
        ));
        assert_eq!(tokio::fs::read(&input).await?, b"source");
        assert!(!output.exists());
        let mut entries = tokio::fs::read_dir(&base).await?;
        let mut count = 0;
        while entries.next_entry().await?.is_some() {
            count += 1;
        }
        assert_eq!(count, 1);
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn output_larger_than_configured_limit_is_not_published() -> Result<(), Box<dyn Error>> {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&base).await?;
        let runner = FakeRunner::default();
        runner
            .state
            .lock()
            .expect("fake runner mutex")
            .create_output = true;
        let executable = std::env::current_exe()?;
        let processor = FfmpegProcessor::initialize_with_runner(
            FfmpegConfig::new()
                .with_ffmpeg_executable(&executable)
                .with_ffprobe_executable(&executable)
                .with_max_output_bytes(9),
            Arc::new(runner),
        )
        .await?;
        let input = base.join("source.mkv");
        let output = base.join("destination.mp4");
        tokio::fs::write(&input, b"source").await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            processor
                .process(
                    MediaProcessRequest::remux(input.clone(), output.clone(), MediaContainer::Mp4),
                    cancellation,
                    None,
                )
                .await,
            Err(FfmpegError::OutputTooLarge)
        ));
        assert_eq!(tokio::fs::read(&input).await?, b"source");
        assert!(!output.exists());
        let mut entries = tokio::fs::read_dir(&base).await?;
        let mut count = 0;
        while entries.next_entry().await?.is_some() {
            count += 1;
        }
        assert_eq!(count, 1);
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_waits_for_runner_and_cleans_temporary_output()
    -> Result<(), Box<dyn Error>> {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-test-{}", Uuid::new_v4()));
        let runner = FakeRunner::default();
        let started = Arc::new(tokio::sync::Notify::new());
        {
            let mut state = runner.state.lock().expect("fake runner mutex");
            state.cancel_on_process = true;
            state.process_started = Some(Arc::clone(&started));
        }
        let processor = setup(runner, &base).await?;
        let input = base.join("source.mkv");
        let output = base.join("destination.mp4");
        tokio::fs::write(&input, b"source").await?;
        let (cancel_sender, cancellation) = watch::channel(false);
        let operation = processor.process(
            MediaProcessRequest::remux(input.clone(), output.clone(), MediaContainer::Mp4),
            cancellation,
            None,
        );
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => return Err(format!("operation ended before runner start: {result:?}").into()),
            _ = started.notified() => {}
        }
        cancel_sender.send(true)?;
        assert!(matches!(operation.await, Err(FfmpegError::Cancelled)));
        assert_eq!(tokio::fs::read(&input).await?, b"source");
        assert!(!output.exists());
        let mut entries = tokio::fs::read_dir(&base).await?;
        let mut count = 0;
        while entries.next_entry().await?.is_some() {
            count += 1;
        }
        assert_eq!(count, 1);
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn rejects_input_output_collision_and_pre_cancelled_work() -> Result<(), Box<dyn Error>> {
        let base = std::env::temp_dir().join(format!("fetchx-ffmpeg-test-{}", Uuid::new_v4()));
        let processor = setup(FakeRunner::default(), &base).await?;
        let input = base.join("source with spaces.mkv");
        tokio::fs::write(&input, b"source").await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            processor
                .process(
                    MediaProcessRequest::remux(input.clone(), input.clone(), MediaContainer::Mp4),
                    cancellation,
                    None,
                )
                .await,
            Err(FfmpegError::InputOutputCollision)
        ));

        let (cancel_sender, cancellation) = watch::channel(true);
        assert!(matches!(
            processor.probe(&input, cancellation).await,
            Err(FfmpegError::Cancelled)
        ));
        drop(cancel_sender);
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }
}

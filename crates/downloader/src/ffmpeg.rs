use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use fetchx_models::ResourcePressure;
use thiserror::Error;
use tokio::{
    sync::{Semaphore, watch},
    time::sleep,
};
use tracing::debug;

use crate::native_media::{NativeMediaError, NativeMediaProcessor};
use crate::native_mp4::{NativeMp4Error, NativeMp4Info, NativeMp4Processor};
use crate::native_oxideav::{NativeOxideavError, NativeOxideavProcessor};

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(60 * 60 * 6);
const MAX_PROCESS_TIMEOUT: Duration = Duration::from_secs(60 * 60 * 24);
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_PROCESSES: usize = 1;
const MAX_CONCURRENT_PROCESSES: usize = 2;

#[derive(Clone, Debug)]
pub struct FfmpegConfig {
    process_timeout: Duration,
    max_output_bytes: u64,
    max_concurrent_processes: usize,
}

fn is_srt_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("srt"))
}

fn is_wav_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
}

impl Default for FfmpegConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl FfmpegConfig {
    pub fn new() -> Self {
        Self {
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_concurrent_processes: DEFAULT_MAX_CONCURRENT_PROCESSES,
        }
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
    WebVtt,
    Mp3,
    Flac,
    Wav,
}

impl MediaContainer {
    fn oxideav_muxer(self) -> &'static str {
        match self {
            Self::Mp4 => "mov",
            Self::Matroska => "matroska",
            Self::WebM => "webm",
            Self::WebVtt => "webvtt",
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
    WavAudio,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaProcessOutput {
    pub destination: PathBuf,
    pub size_bytes: u64,
}

#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error("media processing configuration is invalid")]
    InvalidConfiguration,
    #[error("the requested media operation is not supported by native processing")]
    NativeUnsupported,
    #[error("native media processing task failed")]
    NativeTaskFailed,
    #[error("native media processing task timed out")]
    NativeTaskTimeout,
    #[error("media output exceeded its configured size limit")]
    OutputTooLarge,
    #[error("input path is not an existing regular file")]
    InvalidInput,
    #[error("output path is invalid or its parent directory does not exist")]
    InvalidOutput,
    #[error("input and output resolve to the same path")]
    InputOutputCollision,
    #[error("output destination already exists")]
    DestinationExists,
    #[error("media operation was cancelled")]
    Cancelled,
    #[error("native MP4 processing failed: {0}")]
    NativeMp4(#[from] NativeMp4Error),
    #[error("native media processing failed: {0}")]
    NativeMedia(#[from] NativeMediaError),
    #[error("native OxideAV media processing failed: {0}")]
    NativeOxideav(#[from] NativeOxideavError),
}

pub struct FfmpegProcessor {
    process_timeout: Duration,
    max_output_bytes: u64,
    process_slots: Arc<Semaphore>,
    pressure_sender: watch::Sender<ResourcePressure>,
    pressure: watch::Receiver<ResourcePressure>,
}

impl FfmpegProcessor {
    pub async fn initialize(config: FfmpegConfig) -> Result<Self, FfmpegError> {
        if config.process_timeout.is_zero()
            || config.process_timeout > MAX_PROCESS_TIMEOUT
            || config.max_output_bytes == 0
            || config.max_output_bytes > MAX_OUTPUT_BYTES
            || config.max_concurrent_processes == 0
            || config.max_concurrent_processes > MAX_CONCURRENT_PROCESSES
        {
            return Err(FfmpegError::InvalidConfiguration);
        }
        let (pressure_sender, pressure) = watch::channel(ResourcePressure::Normal);
        Ok(Self {
            process_timeout: config.process_timeout,
            max_output_bytes: config.max_output_bytes,
            process_slots: Arc::new(Semaphore::new(config.max_concurrent_processes)),
            pressure_sender,
            pressure,
        })
    }

    pub fn set_resource_pressure(&self, pressure: ResourcePressure) {
        self.pressure_sender.send_replace(pressure);
    }

    pub async fn probe(
        &self,
        input: impl AsRef<Path>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<MediaInfo, FfmpegError> {
        let _permit = self.acquire_slot(cancellation.clone()).await?;
        let input = canonical_input(input.as_ref()).await?;
        if is_srt_path(&input) {
            if *cancellation.borrow() {
                return Err(FfmpegError::Cancelled);
            }
            return Ok(MediaInfo {
                duration: None,
                format_name: Some("srt".to_owned()),
                size_bytes: Some(
                    tokio::fs::metadata(&input)
                        .await
                        .map_err(|_| FfmpegError::InvalidInput)?
                        .len(),
                ),
                bit_rate_bits_per_second: None,
                streams: vec![MediaStream {
                    kind: MediaStreamKind::Subtitle,
                    codec: Some("subrip".to_owned()),
                    width: None,
                    height: None,
                    frame_rate: None,
                    sample_rate_hz: None,
                    channels: None,
                }],
            });
        }
        if is_wav_path(&input) {
            let native_input = input.clone();
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::probe_wav(&native_input, || {
                        cancelled.load(Ordering::Relaxed) || *receiver.borrow()
                    })
                },
            )
            .await
            {
                Ok(info) => return Ok(info),
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native WAV probe unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if is_mp4_path(&input) {
            let native_input = input.clone();
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMp4Processor::probe_with_cancel(&native_input, || {
                        cancelled.load(Ordering::Relaxed) || *receiver.borrow()
                    })
                },
            )
            .await
            {
                Ok(info) => return Ok(media_info_from_native_mp4(info)),
                Err(FfmpegError::NativeMp4(NativeMp4Error::InvalidInput))
                | Err(FfmpegError::NativeMp4(NativeMp4Error::UnsupportedCodec)) => {
                    debug!("native MP4 probe unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if NativeMediaProcessor::supports_audio_probe_path(&input) {
            let native_input = input.clone();
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::probe_audio(&native_input, || {
                        cancelled.load(Ordering::Relaxed) || *receiver.borrow()
                    })
                },
            )
            .await
            {
                Ok(info) => return Ok(info),
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native audio probe unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        let native_input = input.clone();
        match run_native_task(
            self.process_timeout,
            cancellation.clone(),
            move |cancelled, receiver| {
                NativeOxideavProcessor::probe(&native_input, || {
                    cancelled.load(Ordering::Relaxed) || *receiver.borrow()
                })
            },
        )
        .await
        {
            Ok(info) => return Ok(info),
            Err(FfmpegError::NativeOxideav(
                NativeOxideavError::UnsupportedInput | NativeOxideavError::Container(_),
            )) => {
                debug!("native OxideAV probe unsupported");
            }
            Err(error) => return Err(error),
        }
        Err(FfmpegError::NativeUnsupported)
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

        if is_mp4_path(&video) && is_mp4_path(&audio) && uses_mp4_output(&destination) {
            let native_video = video.clone();
            let native_audio = audio.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMp4Processor::merge_h264_aac(
                        &native_video,
                        &native_audio,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(info) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes: info.size_bytes,
                    });
                }
                Err(FfmpegError::NativeMp4(
                    NativeMp4Error::InvalidInput
                    | NativeMp4Error::UnsupportedCodec
                    | NativeMp4Error::RequiredTrackMissing
                    | NativeMp4Error::UnsupportedTiming,
                )) => {
                    debug!("native MP4 merge unsupported");
                }
                Err(error) => return Err(error),
            }
        }

        let oxideav_muxer = destination
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| {
                if extension.eq_ignore_ascii_case("mkv") {
                    "matroska"
                } else if extension.eq_ignore_ascii_case("mp4") {
                    "mov"
                } else {
                    ""
                }
            })
            .filter(|muxer| !muxer.is_empty());
        if let Some(output_container) = oxideav_muxer {
            let native_video = video.clone();
            let native_audio = audio.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeOxideavProcessor::merge_video_audio(
                        &native_video,
                        &native_audio,
                        &native_output,
                        output_container,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeOxideav(
                    NativeOxideavError::UnsupportedInput | NativeOxideavError::Container(_),
                )) => {
                    debug!("native OxideAV merge unsupported");
                }
                Err(error) => return Err(error),
            }
        }

        Err(FfmpegError::NativeUnsupported)
    }

    pub async fn process(
        &self,
        request: MediaProcessRequest,
        cancellation: watch::Receiver<bool>,
    ) -> Result<MediaProcessOutput, FfmpegError> {
        let _permit = self.acquire_slot(cancellation.clone()).await?;
        let input = canonical_input(&request.input).await?;
        let destination = resolve_destination(&request.output, &input).await?;
        if is_srt_path(&input)
            && matches!(
                request.operation,
                MediaOperation::Remux {
                    container: MediaContainer::WebVtt
                }
            )
        {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::srt_to_webvtt(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native SRT-to-WebVTT conversion unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if is_wav_path(&input)
            && matches!(
                request.operation,
                MediaOperation::Transcode {
                    preset: TranscodePreset::FlacAudio
                }
            )
        {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::wav_to_flac(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native WAV-to-FLAC conversion unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if matches!(
            request.operation,
            MediaOperation::Transcode {
                preset: TranscodePreset::FlacAudio
            }
        ) {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::audio_to_flac(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native audio-to-FLAC conversion unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if matches!(
            request.operation,
            MediaOperation::Transcode {
                preset: TranscodePreset::Mp3Audio
            }
        ) {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::audio_to_mp3(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native audio-to-MP3 conversion unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if matches!(
            request.operation,
            MediaOperation::Transcode {
                preset: TranscodePreset::WavAudio
            }
        ) {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMediaProcessor::audio_to_wav(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeMedia(NativeMediaError::InvalidInput))
                | Err(FfmpegError::NativeMedia(NativeMediaError::UnsupportedInput)) => {
                    debug!("native audio-to-WAV conversion unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if is_mp4_path(&input)
            && matches!(
                request.operation,
                MediaOperation::Remux {
                    container: MediaContainer::WebM
                }
            )
        {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMp4Processor::remux_to_webm(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeMp4(NativeMp4Error::InvalidInput))
                | Err(FfmpegError::NativeMp4(NativeMp4Error::UnsupportedCodec))
                | Err(FfmpegError::NativeMp4(NativeMp4Error::UnsupportedTiming)) => {
                    debug!("native MP4-to-WebM remux unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if is_mp4_path(&input)
            && matches!(
                request.operation,
                MediaOperation::Remux {
                    container: MediaContainer::Mp4
                }
            )
        {
            let native_input = input.clone();
            let native_output = destination.clone();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeMp4Processor::remux(
                        &native_input,
                        &native_output,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(info) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes: info.size_bytes,
                    });
                }
                Err(FfmpegError::NativeMp4(NativeMp4Error::InvalidInput))
                | Err(FfmpegError::NativeMp4(NativeMp4Error::UnsupportedCodec)) => {
                    debug!("native MP4 remux unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        if let MediaOperation::Remux { container } = request.operation {
            let native_input = input.clone();
            let native_output = destination.clone();
            let output_container = container.oxideav_muxer();
            let max_output_bytes = self.max_output_bytes;
            match run_native_task(
                self.process_timeout,
                cancellation.clone(),
                move |cancelled, receiver| {
                    NativeOxideavProcessor::remux(
                        &native_input,
                        &native_output,
                        output_container,
                        max_output_bytes,
                        || cancelled.load(Ordering::Relaxed) || *receiver.borrow(),
                    )
                },
            )
            .await
            {
                Ok(size_bytes) => {
                    return Ok(MediaProcessOutput {
                        destination,
                        size_bytes,
                    });
                }
                Err(FfmpegError::NativeOxideav(
                    NativeOxideavError::UnsupportedInput | NativeOxideavError::Container(_),
                )) => {
                    debug!("native OxideAV remux unsupported");
                }
                Err(error) => return Err(error),
            }
        }
        Err(FfmpegError::NativeUnsupported)
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
                            return Err(FfmpegError::NativeTaskFailed);
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
                        return Err(FfmpegError::NativeTaskFailed);
                    }
                    continue;
                }
                permit = Arc::clone(&self.process_slots).acquire_owned() => {
                    permit.map_err(|_| FfmpegError::NativeTaskFailed)?
                }
            };
            if *pressure.borrow_and_update() == ResourcePressure::Critical {
                drop(permit);
                continue;
            }
            return Ok(permit);
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

fn is_mp4_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

fn uses_mp4_output(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("mkv"))
}

fn media_info_from_native_mp4(info: NativeMp4Info) -> MediaInfo {
    let streams = info
        .tracks
        .into_iter()
        .map(|track| MediaStream {
            kind: match track.media_type.as_str() {
                "video" => MediaStreamKind::Video,
                "audio" => MediaStreamKind::Audio,
                "subtitle" => MediaStreamKind::Subtitle,
                _ => MediaStreamKind::Other,
            },
            codec: track.codec,
            width: track.width.map(u32::from),
            height: track.height.map(u32::from),
            frame_rate: None,
            sample_rate_hz: track.sample_rate_hz,
            channels: track.channels,
        })
        .collect();
    let bit_rate_bits_per_second = (info.duration.as_nanos() > 0).then(|| {
        u64::try_from((u128::from(info.size_bytes) * 8 * 1_000_000_000) / info.duration.as_nanos())
            .ok()
    });
    MediaInfo {
        duration: Some(info.duration),
        format_name: Some("mp4".to_owned()),
        size_bytes: Some(info.size_bytes),
        bit_rate_bits_per_second: bit_rate_bits_per_second.flatten(),
        streams,
    }
}

async fn run_native_task<T, E, F>(
    process_timeout: Duration,
    mut cancellation: watch::Receiver<bool>,
    operation: F,
) -> Result<T, FfmpegError>
where
    T: Send + 'static,
    E: Into<FfmpegError> + Send + 'static,
    F: FnOnce(Arc<AtomicBool>, watch::Receiver<bool>) -> Result<T, E> + Send + 'static,
{
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let worker_cancellation = cancellation.clone();
    let mut worker =
        tokio::task::spawn_blocking(move || operation(worker_cancelled, worker_cancellation));
    tokio::select! {
        biased;
        result = &mut worker => match result {
            Ok(result) => result.map_err(Into::into),
            Err(_) => Err(FfmpegError::NativeTaskFailed),
        },
        result = wait_for_native_cancellation(&mut cancellation) => {
            cancelled.store(true, Ordering::Relaxed);
            let _ = worker.await;
            match result {
                Ok(()) => Err(FfmpegError::Cancelled),
                Err(error) => Err(error),
            }
        }
        _ = sleep(process_timeout) => {
            cancelled.store(true, Ordering::Relaxed);
            let _ = worker.await;
            Err(FfmpegError::NativeTaskTimeout)
        }
    }
}

async fn wait_for_native_cancellation(
    cancellation: &mut watch::Receiver<bool>,
) -> Result<(), FfmpegError> {
    loop {
        if *cancellation.borrow_and_update() {
            return Ok(());
        }
        cancellation
            .changed()
            .await
            .map_err(|_| FfmpegError::NativeTaskFailed)?;
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

#[cfg(test)]
mod native_only_tests {
    use tempfile::tempdir;
    use tokio::sync::watch;

    use super::*;

    #[tokio::test]
    async fn converts_subtitles_natively_and_reports_unsupported_inputs() {
        let directory = tempdir().expect("temporary directory");
        let processor = FfmpegProcessor::initialize(FfmpegConfig::new())
            .await
            .expect("native processor initialization");
        let subtitle = directory.path().join("captions.srt");
        let webvtt = directory.path().join("captions.vtt");
        tokio::fs::write(&subtitle, "1\n00:00:01,000 --> 00:00:02,000\ncaption\n")
            .await
            .expect("write subtitle");
        let (_cancel_sender, cancellation) = watch::channel(false);

        processor
            .process(
                MediaProcessRequest::remux(&subtitle, &webvtt, MediaContainer::WebVtt),
                cancellation.clone(),
            )
            .await
            .expect("native subtitle conversion");
        assert_eq!(
            tokio::fs::read_to_string(&webvtt)
                .await
                .expect("read WebVTT"),
            "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\ncaption\n\n"
        );

        let unsupported = directory.path().join("unknown.mp4");
        tokio::fs::write(&unsupported, b"not a supported media file")
            .await
            .expect("write unsupported media");
        assert!(matches!(
            processor.probe(&unsupported, cancellation).await,
            Err(FfmpegError::NativeUnsupported)
        ));
    }

    #[tokio::test]
    async fn transcodes_audio_to_wav_through_the_shared_processor() {
        let directory = tempdir().expect("temporary directory");
        let input = directory.path().join("input.wav");
        let output = directory.path().join("output.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec).expect("create input WAV");
        writer.write_sample(12_345_i16).expect("write input sample");
        writer.finalize().expect("finalize input WAV");
        let processor = FfmpegProcessor::initialize(FfmpegConfig::new())
            .await
            .expect("native processor initialization");
        let (_cancel_sender, cancellation) = watch::channel(false);

        processor
            .process(
                MediaProcessRequest::transcode(&input, &output, TranscodePreset::WavAudio),
                cancellation,
            )
            .await
            .expect("native audio-to-WAV conversion");
        let mut reader = hound::WavReader::open(output).expect("read output WAV");
        assert_eq!(
            reader
                .samples::<i16>()
                .next()
                .expect("read output sample")
                .expect("valid output sample"),
            12_345
        );
    }
}

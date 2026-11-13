use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use fetchx_downloader::{
    FfmpegConfig, FfmpegError, FfmpegProcessor, MediaContainer, MediaOperation, MediaProcessRequest,
};
use fetchx_models::{DownloadId, DownloadStatus};
use fetchx_storage::DownloadRepository;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::{
    sync::{Mutex, Notify, watch},
    time::Instant,
};
use uuid::Uuid;

use crate::commands::AppState;

const MEDIA_PROCESS_UPDATED: &str = "media_process_updated";
const MAX_OUTPUT_NAME_ATTEMPTS: u32 = 100_000;
const MAX_RETAINED_JOBS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaJobStatus {
    Preparing,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

impl MediaJobStatus {
    fn is_active(self) -> bool {
        matches!(self, Self::Preparing | Self::Processing)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MediaProcessJob {
    pub job_id: String,
    pub download_id: String,
    pub status: MediaJobStatus,
    pub output_time_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub output_path: Option<String>,
    pub output_size_bytes: Option<u64>,
    pub error: Option<String>,
}

struct JobEntry {
    snapshot: MediaProcessJob,
    cancellation: watch::Sender<bool>,
    started_at: Instant,
}

#[derive(Clone, Default)]
pub struct MediaProcessingManager {
    inner: Arc<MediaProcessingInner>,
}

#[derive(Default)]
struct MediaProcessingInner {
    processor: Mutex<Option<Arc<FfmpegProcessor>>>,
    jobs: Mutex<HashMap<String, JobEntry>>,
    start_lock: Mutex<()>,
    closed: AtomicBool,
    idle: Notify,
}

impl MediaProcessingManager {
    pub async fn cancel_all_and_wait(&self) {
        self.inner.closed.store(true, Ordering::Release);
        let _start_guard = self.inner.start_lock.lock().await;
        {
            let jobs = self.inner.jobs.lock().await;
            for entry in jobs.values() {
                if entry.snapshot.status.is_active() {
                    entry.cancellation.send_replace(true);
                }
            }
        }
        drop(_start_guard);

        loop {
            let notified = self.inner.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let jobs = self.inner.jobs.lock().await;
            if jobs
                .values()
                .all(|entry| !entry.snapshot.status.is_active())
            {
                return;
            }
            drop(jobs);
            notified.await;
        }
    }

    async fn processor(&self) -> Result<Arc<FfmpegProcessor>, String> {
        let mut processor = self.inner.processor.lock().await;
        if let Some(processor) = processor.as_ref() {
            return Ok(Arc::clone(processor));
        }

        let processor_config = FfmpegConfig::new().with_max_concurrent_processes(1);
        let initialized = FfmpegProcessor::initialize(processor_config)
            .await
            .map_err(|error| format!("Could not initialize native media processing: {error}"))?;
        let initialized = Arc::new(initialized);
        *processor = Some(Arc::clone(&initialized));
        Ok(initialized)
    }

    async fn get_job(&self, download_id: &str) -> Option<MediaProcessJob> {
        self.inner
            .jobs
            .lock()
            .await
            .get(download_id)
            .map(|entry| entry.snapshot.clone())
    }

    async fn cancel_job(&self, job_id: &str) -> bool {
        let jobs = self.inner.jobs.lock().await;
        let Some(entry) = jobs
            .values()
            .find(|entry| entry.snapshot.job_id == job_id && entry.snapshot.status.is_active())
        else {
            return false;
        };
        entry.cancellation.send_replace(true);
        true
    }

    async fn start_remux(
        &self,
        app: AppHandle,
        processor: Arc<FfmpegProcessor>,
        download_id: String,
        input: PathBuf,
        container: MediaContainer,
    ) -> Result<MediaProcessJob, String> {
        let _start_guard = self.inner.start_lock.lock().await;
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("FetchX is shutting down.".to_owned());
        }
        {
            let jobs = self.inner.jobs.lock().await;
            if jobs.values().any(|entry| entry.snapshot.status.is_active()) {
                return Err("Another media process is already running.".to_owned());
            }
        }

        let output = next_output_path(&input, container).await?;
        let job_id = Uuid::new_v4().to_string();
        let (cancellation, cancellation_receiver) = watch::channel(false);
        let snapshot = MediaProcessJob {
            job_id: job_id.clone(),
            download_id: download_id.clone(),
            status: MediaJobStatus::Preparing,
            output_time_seconds: 0.0,
            duration_seconds: None,
            output_path: None,
            output_size_bytes: None,
            error: None,
        };
        let mut jobs = self.inner.jobs.lock().await;
        if jobs.len() >= MAX_RETAINED_JOBS {
            let oldest_job = jobs
                .iter()
                .filter(|(_, entry)| !entry.snapshot.status.is_active())
                .min_by_key(|(_, entry)| entry.started_at)
                .map(|(download_id, _)| download_id.clone())
                .ok_or_else(|| "The media processing job limit has been reached.".to_owned())?;
            jobs.remove(&oldest_job);
        }
        jobs.insert(
            download_id.clone(),
            JobEntry {
                snapshot: snapshot.clone(),
                cancellation,
                started_at: Instant::now(),
            },
        );
        drop(jobs);
        emit_job(&app, &snapshot);

        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let media_info = processor.probe(&input, cancellation_receiver.clone()).await;
            let duration_seconds = match media_info {
                Ok(info) => info.duration.map(|duration| duration.as_secs_f64()),
                Err(error) => {
                    manager
                        .finish_job(
                            &app,
                            &download_id,
                            match error {
                                FfmpegError::Cancelled => {
                                    (MediaJobStatus::Cancelled, None, None, None)
                                }
                                error => {
                                    (MediaJobStatus::Failed, None, None, Some(error.to_string()))
                                }
                            },
                        )
                        .await;
                    return;
                }
            };

            manager
                .update_job(&app, &download_id, |job| {
                    job.status = MediaJobStatus::Processing;
                    job.duration_seconds = duration_seconds;
                })
                .await;

            let request = MediaProcessRequest {
                input,
                output,
                operation: MediaOperation::Remux { container },
            };
            let result = processor.process(request, cancellation_receiver).await;
            match result {
                Ok(output) => {
                    manager
                        .finish_job(
                            &app,
                            &download_id,
                            (
                                MediaJobStatus::Completed,
                                Some(output.destination.to_string_lossy().into_owned()),
                                Some(output.size_bytes),
                                None,
                            ),
                        )
                        .await;
                }
                Err(FfmpegError::Cancelled) => {
                    manager
                        .finish_job(
                            &app,
                            &download_id,
                            (MediaJobStatus::Cancelled, None, None, None),
                        )
                        .await;
                }
                Err(error) => {
                    manager
                        .finish_job(
                            &app,
                            &download_id,
                            (MediaJobStatus::Failed, None, None, Some(error.to_string())),
                        )
                        .await;
                }
            }
        });

        Ok(snapshot)
    }

    async fn update_job(
        &self,
        app: &AppHandle,
        download_id: &str,
        update: impl FnOnce(&mut MediaProcessJob),
    ) {
        let snapshot = {
            let mut jobs = self.inner.jobs.lock().await;
            let Some(entry) = jobs.get_mut(download_id) else {
                return;
            };
            update(&mut entry.snapshot);
            entry.snapshot.clone()
        };
        emit_job(app, &snapshot);
    }

    async fn finish_job(
        &self,
        app: &AppHandle,
        download_id: &str,
        result: (MediaJobStatus, Option<String>, Option<u64>, Option<String>),
    ) {
        self.update_job(app, download_id, |job| {
            job.status = result.0;
            job.output_path = result.1;
            job.output_size_bytes = result.2;
            job.error = result.3;
        })
        .await;
        self.inner.idle.notify_one();
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_media_process_for_download(
    manager: State<'_, MediaProcessingManager>,
    download_id: String,
) -> Result<Option<MediaProcessJob>, String> {
    Ok(manager.get_job(&download_id).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn start_media_remux(
    app: AppHandle,
    state: State<'_, AppState>,
    manager: State<'_, MediaProcessingManager>,
    download_id: String,
    container: String,
) -> Result<MediaProcessJob, String> {
    let download_id_parsed =
        DownloadId::parse(&download_id).map_err(|_| "The download identifier is invalid.")?;
    let item = state
        .repository
        .get(download_id_parsed)
        .await
        .map_err(|error| format!("Could not load the download: {error}"))?
        .ok_or_else(|| "The download was not found.".to_owned())?;
    let input = item.destination().to_path_buf();
    validate_remux_source(item.status(), &input)?;
    let container = parse_container(&container)?;
    let processor = manager.processor().await?;
    manager
        .start_remux(app, processor, download_id, input, container)
        .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cancel_media_process(
    manager: State<'_, MediaProcessingManager>,
    job_id: String,
) -> Result<bool, String> {
    Ok(manager.cancel_job(&job_id).await)
}

fn emit_job(app: &AppHandle, job: &MediaProcessJob) {
    if let Err(error) = app.emit(MEDIA_PROCESS_UPDATED, job) {
        tracing::warn!(%error, event = MEDIA_PROCESS_UPDATED, "could not emit media process event");
    }
}

fn parse_container(value: &str) -> Result<MediaContainer, String> {
    match value {
        "mp4" => Ok(MediaContainer::Mp4),
        "mkv" => Ok(MediaContainer::Matroska),
        "webm" => Ok(MediaContainer::WebM),
        "vtt" => Ok(MediaContainer::WebVtt),
        _ => Err("The requested output container is not supported.".to_owned()),
    }
}

fn container_extension(container: MediaContainer) -> &'static str {
    match container {
        MediaContainer::Mp4 => "mp4",
        MediaContainer::Matroska => "mkv",
        MediaContainer::WebM => "webm",
        MediaContainer::WebVtt => "vtt",
        MediaContainer::Mp3 => "mp3",
        MediaContainer::Flac => "flac",
        MediaContainer::Wav => "wav",
    }
}

fn is_supported_media_input(input: &Path) -> bool {
    let Some(extension) = input.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "aac"
            | "avi"
            | "flac"
            | "m4a"
            | "m4v"
            | "mkv"
            | "mov"
            | "mp3"
            | "mp4"
            | "mpeg"
            | "mpg"
            | "ogg"
            | "srt"
            | "ts"
            | "wav"
            | "webm"
            | "wma"
            | "wmv"
    )
}

fn validate_remux_source(status: DownloadStatus, input: &Path) -> Result<(), String> {
    if status != DownloadStatus::Completed {
        return Err("Only completed downloads can be processed.".to_owned());
    }
    if !is_supported_media_input(input) {
        return Err("This file type is not supported for remuxing.".to_owned());
    }
    Ok(())
}

async fn next_output_path(input: &Path, container: MediaContainer) -> Result<PathBuf, String> {
    let parent = input
        .parent()
        .ok_or_else(|| "The source file has no parent directory.".to_owned())?;
    let stem = input
        .file_stem()
        .ok_or_else(|| "The source file has no valid name.".to_owned())?;

    for suffix in 1..=MAX_OUTPUT_NAME_ATTEMPTS {
        let mut filename = OsString::from(stem);
        filename.push("-remuxed");
        if suffix > 1 {
            filename.push(format!("-{suffix}"));
        }
        filename.push(".");
        filename.push(container_extension(container));
        let candidate = parent.join(filename);
        match tokio::fs::symlink_metadata(&candidate).await {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(candidate);
            }
            Err(error) => {
                return Err(format!("Could not inspect the output path: {error}"));
            }
        }
    }

    Err("Could not find an unused output filename.".to_owned())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn accepts_supported_media_and_subtitle_outputs() -> Result<(), Box<dyn Error>> {
        assert_eq!(parse_container("mp4")?, MediaContainer::Mp4);
        assert_eq!(parse_container("mkv")?, MediaContainer::Matroska);
        assert_eq!(parse_container("webm")?, MediaContainer::WebM);
        assert_eq!(parse_container("vtt")?, MediaContainer::WebVtt);
        assert!(parse_container("mp3").is_err());
        assert!(parse_container("../mp4").is_err());
        Ok(())
    }

    #[tokio::test]
    async fn remux_output_names_are_adjacent_unique_and_non_overwriting()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("archive.video.mkv");
        tokio::fs::write(&input, b"source").await?;

        let first = next_output_path(&input, MediaContainer::Mp4).await?;
        assert_eq!(first, directory.path().join("archive.video-remuxed.mp4"));
        tokio::fs::write(&first, b"existing output").await?;
        let second = next_output_path(&input, MediaContainer::Mp4).await?;
        assert_eq!(second, directory.path().join("archive.video-remuxed-2.mp4"));
        assert_eq!(tokio::fs::read(&input).await?, b"source");
        Ok(())
    }

    #[test]
    fn remux_requires_a_completed_supported_media_file() {
        assert!(validate_remux_source(DownloadStatus::Completed, Path::new("source.mkv")).is_ok());
        assert!(
            validate_remux_source(DownloadStatus::Completed, Path::new("captions.srt")).is_ok()
        );
        assert!(
            validate_remux_source(DownloadStatus::Downloading, Path::new("source.mkv")).is_err()
        );
        assert!(validate_remux_source(DownloadStatus::Completed, Path::new("source.bin")).is_err());
    }

    #[tokio::test]
    async fn cancellation_signals_only_active_media_jobs() {
        let manager = MediaProcessingManager::default();
        let (cancellation, mut receiver) = watch::channel(false);
        let job = MediaProcessJob {
            job_id: "job-id".to_owned(),
            download_id: "download-id".to_owned(),
            status: MediaJobStatus::Preparing,
            output_time_seconds: 0.0,
            duration_seconds: None,
            output_path: None,
            output_size_bytes: None,
            error: None,
        };
        manager.inner.jobs.lock().await.insert(
            job.download_id.clone(),
            JobEntry {
                snapshot: job.clone(),
                cancellation,
                started_at: Instant::now(),
            },
        );

        assert!(manager.cancel_job(&job.job_id).await);
        assert!(*receiver.borrow_and_update());
        assert!(!manager.cancel_job("unknown-job").await);
    }

    #[tokio::test]
    async fn shutdown_cancels_and_waits_for_active_media_jobs() {
        let manager = MediaProcessingManager::default();
        let (cancellation, mut receiver) = watch::channel(false);
        let job = MediaProcessJob {
            job_id: "job-id".to_owned(),
            download_id: "download-id".to_owned(),
            status: MediaJobStatus::Preparing,
            output_time_seconds: 0.0,
            duration_seconds: None,
            output_path: None,
            output_size_bytes: None,
            error: None,
        };
        manager.inner.jobs.lock().await.insert(
            job.download_id.clone(),
            JobEntry {
                snapshot: job.clone(),
                cancellation,
                started_at: Instant::now(),
            },
        );

        let worker_manager = manager.clone();
        let worker_download_id = job.download_id.clone();
        tokio::spawn(async move {
            let _ = receiver.changed().await;
            if let Some(entry) = worker_manager
                .inner
                .jobs
                .lock()
                .await
                .get_mut(&worker_download_id)
            {
                entry.snapshot.status = MediaJobStatus::Cancelled;
            }
            worker_manager.inner.idle.notify_one();
        });

        manager.cancel_all_and_wait().await;
        assert!(manager.inner.closed.load(Ordering::Acquire));
        assert_eq!(
            manager
                .get_job(&job.download_id)
                .await
                .map(|job| job.status),
            Some(MediaJobStatus::Cancelled)
        );
    }
}

use std::{
    collections::VecDeque,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
    TimestampError,
};
use fetchx_storage::DownloadRepository;
use reqwest::{
    Client, Response, StatusCode, Url,
    header::{
        ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE,
        RETRY_AFTER,
    },
};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom},
    sync::watch,
    task::JoinSet,
    time::interval,
};
use tracing::{info, warn};

use crate::adaptive::{AdaptiveConfig, AdaptiveController, AdaptiveDecision, AdaptiveSample};
use crate::engine::{ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand};
use crate::retry::{RetryPolicy, retry_after_delay};
use crate::segments::{self, Segment, SegmentResult, SegmentState};
use crate::{DownloadEngine, DownloadEngineError};

const DEFAULT_USER_AGENT: &str = concat!("fetchx/", env!("CARGO_PKG_VERSION"));
const MAX_SEGMENT_ATTEMPTS: u8 = 4;

fn measure_speed(bytes: u64, elapsed: Duration) -> Option<u64> {
    let elapsed_nanos = elapsed.as_nanos();
    if elapsed_nanos == 0 {
        return None;
    }
    let bytes_per_second = u128::from(bytes)
        .saturating_mul(1_000_000_000)
        .checked_div(elapsed_nanos)?;
    Some(bytes_per_second.min(u128::from(u64::MAX)) as u64)
}

fn estimate_remaining(total_bytes: u64, downloaded_bytes: u64, speed: Option<u64>) -> Option<u64> {
    let speed = speed.filter(|speed| *speed > 0)?;
    Some(total_bytes.saturating_sub(downloaded_bytes) / speed)
}

/// Timeouts, request identity, and progress checkpoint policy for HTTP downloads.
///
/// By default, progress is persisted after each MiB or one second, whichever comes
/// first. Connection and body-read timeouts default to 15 and 30 seconds.
#[derive(Clone, Debug)]
pub struct HttpEngineConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub progress_bytes_interval: u64,
    pub progress_time_interval: Duration,
    pub user_agent: String,
    /// Maximum number of planned ranges; values outside 2..=32 disable segmentation.
    pub segment_count: usize,
    /// Files smaller than this threshold use the single-stream path.
    pub minimum_segmented_file_size: u64,
    /// Bounded throughput-driven concurrency control for segmented transfers.
    pub adaptive: AdaptiveConfig,
}

struct BodyContext {
    id: DownloadId,
    item: DownloadItem,
    expected_updated_at: Timestamp,
    total_bytes: Option<u64>,
    expected_response_bytes: Option<u64>,
    temporary_path: PathBuf,
}

struct SegmentedContext {
    id: DownloadId,
    item: DownloadItem,
    expected_updated_at: Timestamp,
    status: DownloadStatus,
    url: Url,
    total_size: u64,
    segment_count: usize,
    validator: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ResourceIdentity {
    url: String,
    validator: Option<String>,
    #[serde(default)]
    finalizing: bool,
    #[serde(default)]
    total_size: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct ContentRange {
    start: u64,
    end: u64,
    total: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct UnsatisfiedRange {
    total: u64,
}

enum SegmentedOutcome {
    Completed(Box<DownloadItem>),
    Fallback,
}

enum SegmentedStop {
    Fallback,
    Cancelled,
    Paused,
    Failure(DownloadEngineError),
}

impl Default for HttpEngineConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            progress_bytes_interval: 1024 * 1024,
            progress_time_interval: Duration::from_secs(1),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            segment_count: 8,
            minimum_segmented_file_size: 4 * 1024 * 1024,
            adaptive: AdaptiveConfig::default(),
        }
    }
}

/// Single-stream HTTP/HTTPS downloader with validated Range-based resume.
pub struct DirectHttpEngine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    client: Client,
    config: HttpEngineConfig,
}

impl<R> DirectHttpEngine<R>
where
    R: DownloadRepository + Clone,
{
    /// Creates an engine with the default client and progress configuration.
    pub fn new(repository: R) -> Result<Self, DownloadEngineError> {
        Self::with_config(repository, HttpEngineConfig::default())
    }

    /// Creates an engine with caller-selected timeouts, checkpoint, and adaptive policies.
    pub fn with_config(
        repository: R,
        config: HttpEngineConfig,
    ) -> Result<Self, DownloadEngineError> {
        if config.connect_timeout.is_zero()
            || config.read_timeout.is_zero()
            || config.progress_bytes_interval == 0
            || config.progress_time_interval.is_zero()
            || config.user_agent.trim().is_empty()
            || !config.adaptive.is_valid()
        {
            return Err(DownloadEngineError::InvalidConfiguration);
        }

        let client = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let next = attempt.url();
                let previous = attempt.previous();
                let is_http = matches!(next.scheme(), "http" | "https");
                let has_credentials = !next.username().is_empty() || next.password().is_some();
                let downgrades_tls = previous
                    .last()
                    .is_some_and(|url| url.scheme() == "https" && next.scheme() == "http");

                if previous.len() >= 10 || !is_http || has_credentials || downgrades_tls {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(DownloadEngineError::ClientConfiguration)?;
        let state_machine = DownloadStateMachine::new(repository.clone());

        Ok(Self {
            repository,
            state_machine,
            client,
            config,
        })
    }

    /// Runs a queued, paused, failed, or cancelled item, resuming its partial file if possible.
    /// Set the watch value to `true` to cancel cooperatively.
    ///
    /// The final destination is never overwritten. Response bytes are streamed into
    /// a sibling `.part` file and published without replacement after validation.
    pub async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let (pause_sender, pause) = watch::channel(false);
        let (limit_sender, connection_limit) = watch::channel(self.config.adaptive.max_concurrency);
        let result = self
            .download_controlled(
                id,
                ManagedDownloadControl {
                    cancellation,
                    pause,
                    connection_limit,
                    resource_demand: None,
                },
                false,
            )
            .await;
        drop((pause_sender, limit_sender));
        result
    }

    async fn download_controlled(
        &self,
        id: DownloadId,
        mut control: ManagedDownloadControl,
        already_downloading: bool,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        let already_downloading = if already_downloading {
            true
        } else {
            match current.status() {
                DownloadStatus::Paused => {
                    self.state_machine
                        .transition(id, DownloadStatus::Downloading)
                        .await?;
                    true
                }
                DownloadStatus::Queued | DownloadStatus::Failed | DownloadStatus::Cancelled => {
                    self.state_machine
                        .transition(id, DownloadStatus::Preparing)
                        .await?;
                    false
                }
                status => {
                    return Err(DownloadEngineError::StateMachine(
                        StateMachineError::InvalidTransition {
                            from: status,
                            to: DownloadStatus::Preparing,
                        },
                    ));
                }
            }
        };

        let result = self
            .download_prepared(id, already_downloading, &mut control)
            .await;
        match result {
            Ok(item) => Ok(item),
            Err(error) if error.is_paused() => {
                self.transition_to_paused(id).await?;
                info!(download_id = %id, "download paused");
                Err(error)
            }
            Err(error) if error.is_cancelled() || *control.cancellation.borrow() => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "download cancelled");
                Err(DownloadEngineError::Cancelled)
            }
            Err(error) => {
                if let Err(state) = self.persist_failure(id, &error).await {
                    return Err(DownloadEngineError::FailurePersistence {
                        operation: Box::new(error),
                        state,
                    });
                }
                warn!(download_id = %id, "download failed");
                Err(error)
            }
        }
    }

    async fn download_prepared(
        &self,
        id: DownloadId,
        already_downloading: bool,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let mut item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if item.source().engine() != EngineKind::DirectHttp {
            return Err(DownloadEngineError::EngineMismatch);
        }
        let url = parse_http_url(item.source().url())?;
        let destination = item.destination().to_path_buf();
        let temporary_path = temporary_path(&destination);
        if let Some(completed) = self
            .recover_finalization(&item, &url, &temporary_path, &destination)
            .await?
        {
            return Ok(completed);
        }
        if fs::try_exists(&destination)
            .await
            .map_err(DownloadEngineError::Io)?
        {
            return Err(DownloadEngineError::DestinationExists);
        }

        let partial_size = inspect_partial_file(&temporary_path).await?;
        let partial_exists = partial_size.is_some();
        let existing_size = partial_size.unwrap_or(0);
        let stored_identity = load_resource_identity(&temporary_path).await?;
        let resume_validator = stored_identity
            .as_ref()
            .filter(|identity| identity.url == url.as_str())
            .and_then(|identity| identity.validator.clone());
        let range_start =
            (existing_size > 0 && resume_validator.is_some()).then_some(existing_size);
        let mut expected_updated_at = item.updated_at();
        let initial_progress = DownloadProgress::new(existing_size, None, None, None)?;
        item.set_progress_at(initial_progress, next_timestamp(item.updated_at())?)?;
        let mut current_status = if already_downloading {
            DownloadStatus::Downloading
        } else {
            DownloadStatus::Preparing
        };
        self.persist_if_status(&item, current_status, expected_updated_at)
            .await?;
        expected_updated_at = item.updated_at();

        info!(download_id = %id, "starting HTTP download");
        let mut response = self
            .send_request(url.clone(), range_start, resume_validator.clone(), control)
            .await?;
        let initial_validator = response_validator(&response);
        let segmentation_enabled = (2..=32).contains(&self.config.segment_count)
            && self.config.minimum_segmented_file_size > 0;
        if !partial_exists
            && segmentation_enabled
            && initial_validator.is_some()
            && response.status() == StatusCode::OK
            && response
                .headers()
                .get(ACCEPT_RANGES)
                .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"bytes"))
            && response.url() == &url
            && parse_content_length(&response)?
                .is_some_and(|size| size >= self.config.minimum_segmented_file_size)
        {
            let total_size = parse_content_length(&response)?
                .ok_or(DownloadEngineError::InvalidContentLength)?;
            let segment_count =
                segmented_layout(&temporary_path, self.config.segment_count, total_size).await?;
            let validator = initial_validator
                .clone()
                .ok_or(DownloadEngineError::InvalidConfiguration)?;
            let can_resume_segments = stored_identity.as_ref().is_some_and(|identity| {
                identity.url == url.as_str()
                    && identity.validator.as_deref() == Some(validator.as_str())
            });
            if !can_resume_segments {
                cleanup_segment_files(&temporary_path, segment_count, total_size).await?;
            }
            save_resource_identity(
                &temporary_path,
                &ResourceIdentity {
                    url: url.to_string(),
                    validator: Some(validator.clone()),
                    finalizing: false,
                    total_size: None,
                },
            )
            .await?;
            drop(response);
            match self
                .download_segmented(
                    SegmentedContext {
                        id,
                        item,
                        expected_updated_at,
                        status: current_status,
                        url: url.clone(),
                        total_size,
                        segment_count,
                        validator,
                    },
                    control,
                )
                .await?
            {
                SegmentedOutcome::Completed(completed) => return Ok(*completed),
                SegmentedOutcome::Fallback => {
                    cleanup_segment_files(&temporary_path, segment_count, total_size).await?;
                    item = self
                        .repository
                        .get(id)
                        .await?
                        .ok_or(DownloadEngineError::NotFound(id))?;
                    current_status = item.status();
                    expected_updated_at = item.updated_at();
                    response = self.send_request(url.clone(), None, None, control).await?;
                }
            }
        }

        let mut append = false;
        let mut reset_before_download = false;
        let mut expected_response_bytes = None;
        let mut partial_content_range = None;
        if partial_exists && range_start.is_none() {
            reset_before_download = true;
        }

        if range_start.is_some() && response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            let unsatisfied = parse_unsatisfied_range(&response)?;
            if existing_size < unsatisfied.total {
                return Err(DownloadEngineError::RangeNotSatisfiable);
            }

            // A matching length does not prove that a stale partial belongs to the
            // current representation, so restart instead of publishing it.
            drop(response);
            response = self.send_request(url.clone(), None, None, control).await?;
            reset_before_download = true;
        } else if range_start.is_some() && response.status() == StatusCode::OK {
            reset_before_download = true;
        } else if range_start.is_some() && response.status() == StatusCode::PARTIAL_CONTENT {
            let range = parse_content_range(&response)?;
            if Some(range.start) != range_start
                || range.end < range.start
                || range.total.is_some_and(|total| range.end >= total)
            {
                return Err(DownloadEngineError::RangeMismatch);
            }
            let range_bytes = range
                .end
                .checked_sub(range.start)
                .and_then(|bytes| bytes.checked_add(1))
                .ok_or(DownloadEngineError::ByteCountOverflow)?;
            expected_response_bytes = Some(range_bytes);
            if parse_content_length(&response)?
                .is_some_and(|length| Some(length) != expected_response_bytes)
            {
                return Err(DownloadEngineError::InvalidContentLength);
            }
            partial_content_range = Some(range);
            append = true;
        } else if response.status() == StatusCode::PARTIAL_CONTENT {
            return Err(DownloadEngineError::UnexpectedPartialResponse);
        }

        if append && !same_http_target(response.url(), &url) {
            return Err(DownloadEngineError::RangeMismatch);
        }

        if response.status() == StatusCode::PARTIAL_CONTENT && !append {
            return Err(DownloadEngineError::UnexpectedPartialResponse);
        }
        if !response.status().is_success() {
            return Err(DownloadEngineError::HttpStatus(response.status().as_u16()));
        }
        let current_validator = response_validator(&response);
        if append && current_validator.as_deref() != resume_validator.as_deref() {
            return Err(DownloadEngineError::RangeMismatch);
        }
        if !append {
            save_resource_identity(
                &temporary_path,
                &ResourceIdentity {
                    url: url.to_string(),
                    validator: current_validator,
                    finalizing: false,
                    total_size: None,
                },
            )
            .await?;
        }

        if reset_before_download {
            reset_partial_file(&temporary_path, partial_exists).await?;
        }

        let total_bytes = if append {
            partial_content_range.and_then(|range| range.total)
        } else {
            parse_content_length(&response)?
        };
        let file =
            open_partial_file(&temporary_path, existing_size, append, partial_exists).await?;
        let progress_offset = if append { existing_size } else { 0 };
        item.set_progress_at(
            DownloadProgress::new(progress_offset, total_bytes, None, None)?,
            next_timestamp(item.updated_at())?,
        )?;
        self.persist_if_status(&item, current_status, expected_updated_at)
            .await?;
        expected_updated_at = item.updated_at();

        self.receive_body(
            response,
            file,
            BodyContext {
                id,
                item,
                expected_updated_at,
                total_bytes,
                expected_response_bytes,
                temporary_path,
            },
            control,
        )
        .await
    }

    async fn recover_finalization(
        &self,
        item: &DownloadItem,
        url: &Url,
        temporary: &Path,
        destination: &Path,
    ) -> Result<Option<DownloadItem>, DownloadEngineError> {
        let Some(identity) = load_resource_identity(temporary).await? else {
            return Ok(None);
        };
        let Some(expected_size) = identity.total_size else {
            return Ok(None);
        };
        if !identity.finalizing || identity.url != url.as_str() {
            return Ok(None);
        }
        let temporary_metadata = match fs::symlink_metadata(temporary).await {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => return Err(DownloadEngineError::InvalidPartialFile),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(DownloadEngineError::Io(error)),
        };
        if temporary_metadata.len() != expected_size {
            return Ok(None);
        }

        if fs::try_exists(destination)
            .await
            .map_err(DownloadEngineError::Io)?
        {
            if !files_equal(temporary, destination).await? {
                return Ok(None);
            }
        } else {
            publish_without_replacement(temporary, destination).await?;
        }

        let mut recovered = item.clone();
        recovered.set_progress_at(
            DownloadProgress::new(expected_size, Some(expected_size), None, Some(0))?,
            next_timestamp(item.updated_at())?,
        )?;
        self.persist_if_status(&recovered, item.status(), item.updated_at())
            .await?;
        if item.status() == DownloadStatus::Preparing {
            self.state_machine
                .transition(item.id(), DownloadStatus::Downloading)
                .await?;
        } else if item.status() != DownloadStatus::Downloading {
            return Ok(None);
        }
        let completed = self
            .state_machine
            .transition(item.id(), DownloadStatus::Completed)
            .await?;
        remove_published_temporary(temporary).await;
        Ok(Some(completed))
    }

    async fn send_request(
        &self,
        url: Url,
        range_start: Option<u64>,
        if_range: Option<String>,
        control: &mut ManagedDownloadControl,
    ) -> Result<Response, DownloadEngineError> {
        let started_at = Instant::now();
        let mut failed_attempts = 0_u8;
        loop {
            if *control.cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow() {
                return Err(DownloadEngineError::Paused);
            }

            tokio::select! {
                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                changed = control.pause.changed() => {
                    if changed.is_err() || *control.pause.borrow() {
                        return Err(DownloadEngineError::Paused);
                    }
                }
                response = async {
                    let request = self.client.get(url.clone());
                    let request = if let Some(start) = range_start {
                        let request = request.header(RANGE, format!("bytes={start}-"));
                        if let Some(validator) = &if_range {
                            request.header(IF_RANGE, validator)
                        } else {
                            request
                        }
                    } else {
                        request
                    };
                    request.send().await
                } => {
                    match response {
                        Ok(response)
                            if RetryPolicy::is_transient_status(response.status().as_u16()) =>
                        {
                            let retry_after = parse_retry_after(&response);
                            failed_attempts = failed_attempts.saturating_add(1);
                            if let Some(delay) = RetryPolicy::HTTP.delay(
                                failed_attempts,
                                retry_after,
                                started_at.elapsed(),
                            ) {
                                drop(response);
                                wait_for_retry(delay, control).await?;
                            } else {
                                return Ok(response);
                            }
                        }
                        Ok(response) => return Ok(response),
                        Err(error) => {
                            if !RetryPolicy::is_transient_request(&error) {
                                return Err(DownloadEngineError::Request(error));
                            }
                            failed_attempts = failed_attempts.saturating_add(1);
                            if let Some(delay) = RetryPolicy::HTTP.delay(
                                failed_attempts,
                                None,
                                started_at.elapsed(),
                            ) {
                                wait_for_retry(delay, control).await?;
                            } else {
                                return Err(DownloadEngineError::Request(error));
                            }
                        }
                    }
                }
            }
        }
    }

    async fn download_segmented(
        &self,
        context: SegmentedContext,
        control: &mut ManagedDownloadControl,
    ) -> Result<SegmentedOutcome, DownloadEngineError> {
        let SegmentedContext {
            id,
            mut item,
            mut expected_updated_at,
            mut status,
            url,
            total_size,
            segment_count,
            validator,
        } = context;
        let temporary_path = temporary_path(item.destination());
        let mut segments = segments::plan_segments(total_size, segment_count, |index| {
            segment_file_path(&temporary_path, segment_count, total_size, index)
        })
        .map_err(|_| DownloadEngineError::InvalidConfiguration)?
        .ok_or(DownloadEngineError::InvalidConfiguration)?;

        let mut already_downloaded = 0_u64;
        for segment in &mut segments {
            if !segments::prepare_segment(segment).await? {
                return Ok(SegmentedOutcome::Fallback);
            }
            already_downloaded = already_downloaded
                .checked_add(segment.current)
                .ok_or(DownloadEngineError::ByteCountOverflow)?;
        }
        if already_downloaded > total_size {
            return Ok(SegmentedOutcome::Fallback);
        }

        if status == DownloadStatus::Preparing {
            item = self
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
            status = DownloadStatus::Downloading;
            expected_updated_at = item.updated_at();
        }
        item.set_progress_at(
            DownloadProgress::new(already_downloaded, Some(total_size), None, None)?,
            next_timestamp(item.updated_at())?,
        )?;
        self.persist_if_status(&item, status, expected_updated_at)
            .await?;
        expected_updated_at = item.updated_at();

        let downloaded = Arc::new(AtomicU64::new(already_downloaded));
        let (stop_sender, stop_receiver) = watch::channel(false);
        let mut tasks: JoinSet<(usize, Segment, Result<SegmentResult, DownloadEngineError>)> =
            JoinSet::new();
        let mut pending = segments
            .iter()
            .filter(|segment| segment.state != SegmentState::Complete)
            .map(|segment| segment.index)
            .collect::<VecDeque<_>>();
        let mut controller = AdaptiveController::new(self.config.adaptive.clone());
        let mut connection_limit = *control.connection_limit.borrow_and_update();
        let initial_target = controller
            .desired_concurrency(pending.len())
            .min(connection_limit);
        spawn_segment_workers(
            &mut tasks,
            &mut pending,
            &segments,
            initial_target,
            total_size,
            &url,
            &validator,
            &self.client,
            &control.cancellation,
            &stop_receiver,
            &downloaded,
        );
        let remaining_segments = segments
            .iter()
            .filter(|segment| segment.state != SegmentState::Complete)
            .count();
        report_resource_demand(
            control,
            id,
            controller.desired_concurrency(remaining_segments),
            tasks.len(),
        )
        .await?;
        let mut stop_reason = None;
        let mut failures_since_sample = 0_u8;
        let mut wait_for_error_sample = false;
        let mut completed_since_sample = 0_usize;

        let poll_interval = self
            .config
            .progress_time_interval
            .min(Duration::from_millis(50));
        let mut progress_timer = interval(poll_interval);
        let _ = progress_timer.tick().await;
        let mut adaptive_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + self.config.adaptive.measurement_window,
            self.config.adaptive.measurement_window,
        );
        let mut last_sampled_bytes = already_downloaded;
        let mut last_sampled_at = Instant::now();
        let mut last_persisted_bytes = already_downloaded;
        let mut last_persisted_at = Instant::now();

        while !tasks.is_empty() || (stop_reason.is_none() && !pending.is_empty()) {
            tokio::select! {
                changed = control.cancellation.changed(), if stop_reason.is_none() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        stop_reason = Some(SegmentedStop::Cancelled);
                        let _ = stop_sender.send(true);
                    }
                }
                changed = control.pause.changed(), if stop_reason.is_none() => {
                    if changed.is_err() || *control.pause.borrow() {
                        stop_reason = Some(SegmentedStop::Paused);
                        let _ = stop_sender.send(true);
                    }
                }
                changed = control.connection_limit.changed(), if stop_reason.is_none() => {
                    if changed.is_err() {
                        stop_reason = Some(SegmentedStop::Failure(
                            DownloadEngineError::SegmentWorkerFailed
                        ));
                        let _ = stop_sender.send(true);
                    } else {
                        connection_limit = *control.connection_limit.borrow_and_update();
                        let remaining_segments = segments
                            .iter()
                            .filter(|segment| segment.state != SegmentState::Complete)
                            .count();
                        spawn_segment_workers(
                            &mut tasks,
                            &mut pending,
                            &segments,
                            controller
                                .desired_concurrency(remaining_segments)
                                .min(connection_limit),
                            total_size,
                            &url,
                            &validator,
                            &self.client,
                            &control.cancellation,
                            &stop_receiver,
                            &downloaded,
                        );
                    }
                }
                _ = progress_timer.tick(), if stop_reason.is_none() => {
                    let current_bytes = downloaded.load(Ordering::Acquire);
                    if current_bytes.saturating_sub(last_persisted_bytes)
                        >= self.config.progress_bytes_interval
                        || last_persisted_at.elapsed() >= self.config.progress_time_interval
                    {
                        if let Err(error) = self.persist_segment_progress(
                            &mut item,
                            &mut expected_updated_at,
                            status,
                            total_size,
                            &downloaded,
                            &mut last_persisted_bytes,
                            &mut last_persisted_at,
                        ).await {
                            stop_reason = Some(SegmentedStop::Failure(error));
                            let _ = stop_sender.send(true);
                        }
                    }
                }
                _ = adaptive_timer.tick(), if stop_reason.is_none() => {
                    let now = Instant::now();
                    let current_bytes = downloaded.load(Ordering::Acquire);
                    let remaining_segments = segments
                        .iter()
                        .filter(|segment| segment.state != SegmentState::Complete)
                        .count();
                    let (decision, decrease_reason) = controller.observe(AdaptiveSample {
                        bytes: current_bytes.saturating_sub(last_sampled_bytes),
                        elapsed: now.duration_since(last_sampled_at),
                        active_concurrency: tasks.len(),
                        remaining_segments,
                        completed_segments: completed_since_sample,
                        request_failures: failures_since_sample,
                        cancelled: *control.cancellation.borrow() || *control.pause.borrow(),
                    });
                    last_sampled_bytes = current_bytes;
                    last_sampled_at = now;
                    failures_since_sample = 0;
                    completed_since_sample = 0;
                    wait_for_error_sample = false;
                    match decision {
                        AdaptiveDecision::Increase { from, to } => {
                            info!(download_id = %id, from, to, "increasing HTTP segment concurrency");
                        }
                        AdaptiveDecision::Decrease { from, to } => {
                            warn!(
                                download_id = %id,
                                from,
                                to,
                                reason = ?decrease_reason,
                                "decreasing HTTP segment concurrency"
                            );
                        }
                        AdaptiveDecision::Maintain => {}
                    }
                    let target = controller
                        .desired_concurrency(remaining_segments)
                        .min(connection_limit);
                    spawn_segment_workers(
                        &mut tasks,
                        &mut pending,
                        &segments,
                        target,
                        total_size,
                        &url,
                        &validator,
                        &self.client,
                        &control.cancellation,
                        &stop_receiver,
                        &downloaded,
                    );
                    report_resource_demand(
                        control,
                        id,
                        controller.desired_concurrency(remaining_segments),
                        tasks.len(),
                    )
                    .await?;
                }
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    match joined {
                        Some(Ok((index, segment, Ok(SegmentResult::Complete)))) => {
                            segments[index] = segment;
                            completed_since_sample = completed_since_sample.saturating_add(1);
                        }
                        Some(Ok((_, _, Ok(SegmentResult::Fallback)))) if stop_reason.is_none() => {
                            stop_reason = Some(SegmentedStop::Fallback);
                            let _ = stop_sender.send(true);
                        }
                        Some(Ok((index, segment, Ok(SegmentResult::Cancelled)))) => {
                            segments[index] = segment;
                        }
                        Some(Ok((index, segment, Err(error)))) if stop_reason.is_none() => {
                            segments[index] = segment;
                            failures_since_sample = failures_since_sample.saturating_add(1);
                            wait_for_error_sample = true;
                            if segments[index].attempts < MAX_SEGMENT_ATTEMPTS
                                && matches!(error, DownloadEngineError::SegmentFailed { .. })
                            {
                                match segments::prepare_segment(&mut segments[index]).await {
                                    Ok(true) if segments[index].state != SegmentState::Complete => {
                                        pending.push_front(index);
                                    }
                                    Ok(true) => {
                                        stop_reason = Some(SegmentedStop::Failure(error));
                                        let _ = stop_sender.send(true);
                                    }
                                    Ok(false) => {
                                        stop_reason = Some(SegmentedStop::Failure(
                                            DownloadEngineError::SegmentFailed {
                                                index,
                                                start: segments[index].start,
                                                end: segments[index].end,
                                            },
                                        ));
                                        let _ = stop_sender.send(true);
                                    }
                                    Err(reconcile_error) => {
                                        stop_reason =
                                            Some(SegmentedStop::Failure(reconcile_error));
                                        let _ = stop_sender.send(true);
                                    }
                                }
                            } else {
                                stop_reason = Some(SegmentedStop::Failure(error));
                                let _ = stop_sender.send(true);
                            }
                        }
                        Some(Err(_)) if stop_reason.is_none() => {
                            stop_reason = Some(SegmentedStop::Failure(
                                DownloadEngineError::SegmentWorkerFailed
                            ));
                            let _ = stop_sender.send(true);
                        }
                        Some(_) => {}
                        None => {}
                    }

                    if stop_reason.is_none() && !wait_for_error_sample {
                        let remaining_segments = segments
                            .iter()
                            .filter(|segment| segment.state != SegmentState::Complete)
                            .count();
                        let target = controller
                            .desired_concurrency(remaining_segments)
                            .min(connection_limit);
                        spawn_segment_workers(
                            &mut tasks,
                            &mut pending,
                            &segments,
                            target,
                            total_size,
                            &url,
                            &validator,
                            &self.client,
                            &control.cancellation,
                            &stop_receiver,
                            &downloaded,
                        );
                        report_resource_demand(
                            control,
                            id,
                            controller.desired_concurrency(remaining_segments),
                            tasks.len(),
                        )
                        .await?;
                    }
                }
            }
        }

        let actual_segment_bytes =
            segment_files_size(&temporary_path, segment_count, total_size).await?;
        downloaded.store(actual_segment_bytes, Ordering::Release);
        if let Some(reason) = stop_reason {
            self.persist_segment_progress(
                &mut item,
                &mut expected_updated_at,
                status,
                total_size,
                &downloaded,
                &mut last_persisted_bytes,
                &mut last_persisted_at,
            )
            .await?;
            return match reason {
                SegmentedStop::Fallback => Ok(SegmentedOutcome::Fallback),
                SegmentedStop::Cancelled => Err(DownloadEngineError::Cancelled),
                SegmentedStop::Paused => Err(DownloadEngineError::Paused),
                SegmentedStop::Failure(error) => Err(error),
            };
        }

        self.persist_segment_progress(
            &mut item,
            &mut expected_updated_at,
            status,
            total_size,
            &downloaded,
            &mut last_persisted_bytes,
            &mut last_persisted_at,
        )
        .await?;
        if downloaded.load(Ordering::Acquire) != total_size {
            return Err(DownloadEngineError::IncompleteResponse {
                expected: total_size,
                actual: downloaded.load(Ordering::Acquire),
            });
        }

        assemble_segments(
            &segments::plan_segments(total_size, segment_count, |index| {
                segment_file_path(&temporary_path, segment_count, total_size, index)
            })
            .map_err(|_| DownloadEngineError::InvalidConfiguration)?
            .ok_or(DownloadEngineError::InvalidConfiguration)?,
            &temporary_path,
            total_size,
            &mut control.cancellation,
            &mut control.pause,
        )
        .await?;
        cleanup_segment_files(&temporary_path, segment_count, total_size).await?;
        mark_resource_finalizing(&temporary_path, total_size).await?;
        publish_without_replacement(&temporary_path, item.destination()).await?;

        let completed = self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        remove_published_temporary(&temporary_path).await;
        info!(download_id = %id, bytes = total_size, "segmented download completed");
        Ok(SegmentedOutcome::Completed(Box::new(completed)))
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_segment_progress(
        &self,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        status: DownloadStatus,
        total_size: u64,
        downloaded: &AtomicU64,
        last_sampled_bytes: &mut u64,
        last_sampled_at: &mut Instant,
    ) -> Result<(), DownloadEngineError> {
        let downloaded_bytes = downloaded.load(Ordering::Acquire);
        let now = Instant::now();
        let speed = measure_speed(
            downloaded_bytes.saturating_sub(*last_sampled_bytes),
            now.duration_since(*last_sampled_at),
        );
        item.set_progress_at(
            DownloadProgress::new(
                downloaded_bytes,
                Some(total_size),
                speed,
                estimate_remaining(total_size, downloaded_bytes, speed),
            )?,
            next_timestamp(item.updated_at())?,
        )?;
        self.persist_if_status(item, status, *expected_updated_at)
            .await?;
        *expected_updated_at = item.updated_at();
        *last_sampled_bytes = downloaded_bytes;
        *last_sampled_at = now;
        Ok(())
    }

    async fn receive_body(
        &self,
        mut response: Response,
        mut file: File,
        context: BodyContext,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let BodyContext {
            id,
            mut item,
            expected_updated_at: mut persisted_updated_at,
            total_bytes,
            expected_response_bytes,
            temporary_path,
        } = context;
        let mut downloading_started = item.status() == DownloadStatus::Downloading;
        let result = async {
            if item.status() == DownloadStatus::Preparing {
                item = self
                    .state_machine
                    .transition(id, DownloadStatus::Downloading)
                    .await?;
                downloading_started = true;
                persisted_updated_at = item.updated_at();
            }
            let progress_offset = item.progress().downloaded_bytes();
            item.set_progress_at(
                DownloadProgress::new(progress_offset, total_bytes, None, None)?,
                next_timestamp(item.updated_at())?,
            )?;
            self.persist_if_status(&item, DownloadStatus::Downloading, persisted_updated_at)
                .await?;
            persisted_updated_at = item.updated_at();

            let mut downloaded_bytes = progress_offset;
            let mut response_bytes = 0_u64;
            let mut last_persisted_bytes = downloaded_bytes;
            let mut last_persisted_at = Instant::now();

            loop {
                let next_chunk = loop {
                    if *control.cancellation.borrow_and_update() {
                        self.persist_if_status(
                            &item,
                            DownloadStatus::Downloading,
                            persisted_updated_at,
                        )
                        .await?;
                        persisted_updated_at = item.updated_at();
                        return Err(DownloadEngineError::Cancelled);
                    }

                    tokio::select! {
                        changed = control.cancellation.changed() => {
                            if changed.is_err() || *control.cancellation.borrow_and_update() {
                                self.persist_if_status(
                                    &item,
                                    DownloadStatus::Downloading,
                                    persisted_updated_at,
                                )
                                    .await?;
                                persisted_updated_at = item.updated_at();
                                return Err(DownloadEngineError::Cancelled);
                            }
                        }
                        changed = control.pause.changed() => {
                            if changed.is_err() || *control.pause.borrow() {
                                self.persist_if_status(
                                    &item,
                                    DownloadStatus::Downloading,
                                    persisted_updated_at,
                                ).await?;
                                persisted_updated_at = item.updated_at();
                                return Err(DownloadEngineError::Paused);
                            }
                        }
                        chunk = response.chunk() => {
                            break chunk.map_err(DownloadEngineError::Request)?;
                        }
                    }
                };

                let Some(chunk) = next_chunk else {
                    break;
                };
                let chunk_size = u64::try_from(chunk.len())
                    .map_err(|_| DownloadEngineError::ByteCountOverflow)?;
                let next_downloaded = downloaded_bytes
                    .checked_add(chunk_size)
                    .ok_or(DownloadEngineError::ByteCountOverflow)?;
                let next_response_bytes = response_bytes
                    .checked_add(chunk_size)
                    .ok_or(DownloadEngineError::ByteCountOverflow)?;
                if let Some(expected) = total_bytes.filter(|total| next_downloaded > *total) {
                    return Err(DownloadEngineError::ContentLengthExceeded { expected });
                }

                file.write_all(&chunk)
                    .await
                    .map_err(DownloadEngineError::Io)?;
                downloaded_bytes = next_downloaded;
                response_bytes = next_response_bytes;
                let now = Instant::now();
                let speed = measure_speed(
                    downloaded_bytes.saturating_sub(last_persisted_bytes),
                    now.duration_since(last_persisted_at),
                );
                item.set_progress_at(
                    DownloadProgress::new(
                        downloaded_bytes,
                        total_bytes,
                        speed,
                        total_bytes
                            .and_then(|total| estimate_remaining(total, downloaded_bytes, speed)),
                    )?,
                    next_timestamp(item.updated_at())?,
                )?;

                if downloaded_bytes.saturating_sub(last_persisted_bytes)
                    >= self.config.progress_bytes_interval
                    || now.duration_since(last_persisted_at) >= self.config.progress_time_interval
                {
                    self.persist_if_status(
                        &item,
                        DownloadStatus::Downloading,
                        persisted_updated_at,
                    )
                    .await?;
                    persisted_updated_at = item.updated_at();
                    last_persisted_bytes = downloaded_bytes;
                    last_persisted_at = now;
                }
            }

            if let Some(expected) = total_bytes {
                if downloaded_bytes != expected {
                    return Err(DownloadEngineError::IncompleteResponse {
                        expected,
                        actual: downloaded_bytes,
                    });
                }
            }
            if let Some(expected) = expected_response_bytes
                && response_bytes != expected
            {
                return Err(DownloadEngineError::IncompleteResponse {
                    expected,
                    actual: response_bytes,
                });
            }

            if *control.cancellation.borrow_and_update() {
                self.persist_if_status(&item, DownloadStatus::Downloading, persisted_updated_at)
                    .await?;
                persisted_updated_at = item.updated_at();
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow() {
                self.persist_if_status(&item, DownloadStatus::Downloading, persisted_updated_at)
                    .await?;
                persisted_updated_at = item.updated_at();
                return Err(DownloadEngineError::Paused);
            }
            file.flush().await.map_err(DownloadEngineError::Io)?;
            file.sync_all().await.map_err(DownloadEngineError::Io)?;
            let actual_size = fs::metadata(&temporary_path)
                .await
                .map_err(DownloadEngineError::Io)?
                .len();
            if actual_size != downloaded_bytes {
                return Err(DownloadEngineError::IncompleteResponse {
                    expected: downloaded_bytes,
                    actual: actual_size,
                });
            }
            self.persist_if_status(&item, DownloadStatus::Downloading, persisted_updated_at)
                .await?;
            persisted_updated_at = item.updated_at();
            Ok(())
        }
        .await;

        drop(file);
        match result {
            Ok(()) => {
                if *control.cancellation.borrow_and_update() {
                    return Err(DownloadEngineError::Cancelled);
                }
                mark_resource_finalizing(&temporary_path, item.progress().downloaded_bytes())
                    .await?;
                publish_without_replacement(&temporary_path, item.destination()).await?;

                let completed = self
                    .state_machine
                    .transition(id, DownloadStatus::Completed)
                    .await?;
                remove_published_temporary(&temporary_path).await;
                info!(download_id = %id, bytes = item.progress().downloaded_bytes(), "download completed");
                Ok(completed)
            }
            Err(error) => {
                let operation = if downloading_started {
                    self.persist_if_status(&item, DownloadStatus::Downloading, persisted_updated_at)
                        .await
                        .err()
                        .unwrap_or(error)
                } else {
                    error
                };
                Err(operation)
            }
        }
    }

    async fn persist_if_status(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
        expected_updated_at: Timestamp,
    ) -> Result<(), DownloadEngineError> {
        if self
            .repository
            .update_if_status_and_timestamp_current(item, expected_status, expected_updated_at)
            .await?
        {
            return Ok(());
        }

        Err(DownloadEngineError::StateMachine(
            self.current_state_error(item.id(), expected_status).await?,
        ))
    }

    async fn current_state_error(
        &self,
        id: DownloadId,
        expected: DownloadStatus,
    ) -> Result<StateMachineError, DownloadEngineError> {
        match self.repository.get(id).await? {
            None => Ok(StateMachineError::NotFound(id)),
            Some(item) => Ok(StateMachineError::ConcurrentTransition {
                from: expected,
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

    async fn transition_to_paused(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if current.status() == DownloadStatus::Paused {
            return Ok(());
        }
        self.state_machine
            .transition(id, DownloadStatus::Paused)
            .await?;
        Ok(())
    }

    async fn persist_failure(
        &self,
        id: DownloadId,
        operation: &DownloadEngineError,
    ) -> Result<(), StateMachineError> {
        let current = self
            .repository
            .get(id)
            .await
            .map_err(StateMachineError::Storage)?
            .ok_or(StateMachineError::NotFound(id))?;
        if !matches!(
            current.status(),
            DownloadStatus::Preparing | DownloadStatus::Downloading
        ) {
            return Ok(());
        }

        let domain_error = operation
            .persisted_error()
            .map_err(StateMachineError::DomainError)?;
        self.state_machine
            .transition_with_error(id, DownloadStatus::Failed, domain_error)
            .await
            .map(|_| ())
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_segment_workers(
    tasks: &mut JoinSet<(usize, Segment, Result<SegmentResult, DownloadEngineError>)>,
    pending: &mut VecDeque<usize>,
    segments: &[Segment],
    target_concurrency: usize,
    total_size: u64,
    url: &Url,
    validator: &str,
    client: &Client,
    cancellation: &watch::Receiver<bool>,
    stop: &watch::Receiver<bool>,
    downloaded: &Arc<AtomicU64>,
) {
    while tasks.len() < target_concurrency {
        let Some(index) = pending.pop_front() else {
            break;
        };
        let mut segment = segments[index].clone();
        let client = client.clone();
        let url = url.clone();
        let validator = validator.to_owned();
        let mut cancellation = cancellation.clone();
        let mut stop = stop.clone();
        let downloaded = Arc::clone(downloaded);
        tasks.spawn(async move {
            let result = segments::download_segment(
                &mut segment,
                total_size,
                &url,
                &validator,
                &client,
                &mut cancellation,
                &mut stop,
                &downloaded,
            )
            .await;
            (index, segment, result)
        });
    }
}

async fn report_resource_demand(
    control: &ManagedDownloadControl,
    download_id: DownloadId,
    desired_concurrency: usize,
    active_concurrency: usize,
) -> Result<(), DownloadEngineError> {
    let Some(resource_demand) = &control.resource_demand else {
        return Ok(());
    };
    resource_demand
        .send(ResourceDemand {
            download_id,
            desired_concurrency,
            active_concurrency,
        })
        .await
        .map_err(|_| DownloadEngineError::ResourceManagerUnavailable)
}

impl<R> DownloadEngine for DirectHttpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        DirectHttpEngine::download(self, id, cancellation).await
    }
}

impl<R> ManagedDownloadEngine for DirectHttpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        self.download_controlled(id, control, false).await
    }
}

fn parse_http_url(source: &str) -> Result<Url, DownloadEngineError> {
    if source.trim().is_empty() {
        return Err(DownloadEngineError::InvalidUrl);
    }

    let url = Url::parse(source).map_err(|_| DownloadEngineError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DownloadEngineError::UnsupportedScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DownloadEngineError::EmbeddedCredentials);
    }

    Ok(url)
}

/// Validates and canonicalizes a direct HTTP/HTTPS source URL for application adapters.
pub fn validate_http_source_url(source: &str) -> Result<String, DownloadEngineError> {
    parse_http_url(source).map(|url| url.to_string())
}

fn same_http_target(response: &Url, requested: &Url) -> bool {
    let mut response = response.clone();
    let mut requested = requested.clone();
    response.set_fragment(None);
    requested.set_fragment(None);
    response == requested
}

pub(crate) fn response_validator(response: &Response) -> Option<String> {
    response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim_start().starts_with("W/"))
        .or_else(|| {
            response
                .headers()
                .get(LAST_MODIFIED)
                .and_then(|value| value.to_str().ok())
        })
        .map(str::to_owned)
}

fn resource_identity_path(temporary: &Path) -> PathBuf {
    let mut path = temporary.as_os_str().to_os_string();
    path.push(".meta");
    PathBuf::from(path)
}

async fn load_resource_identity(
    temporary: &Path,
) -> Result<Option<ResourceIdentity>, DownloadEngineError> {
    let path = resource_identity_path(temporary);
    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(DownloadEngineError::Io(error)),
    };
    match serde_json::from_slice(&bytes) {
        Ok(identity) => Ok(Some(identity)),
        Err(error) => {
            warn!(path = %path.display(), %error, "ignoring invalid HTTP resume metadata");
            Ok(None)
        }
    }
}

async fn save_resource_identity(
    temporary: &Path,
    identity: &ResourceIdentity,
) -> Result<(), DownloadEngineError> {
    let bytes = serde_json::to_vec(identity)
        .map_err(|error| DownloadEngineError::InvalidPartialMetadata(error.to_string()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(resource_identity_path(temporary))
        .await
        .map_err(DownloadEngineError::Io)?;
    file.write_all(&bytes)
        .await
        .map_err(DownloadEngineError::Io)?;
    file.flush().await.map_err(DownloadEngineError::Io)?;
    file.sync_all().await.map_err(DownloadEngineError::Io)?;
    Ok(())
}

async fn mark_resource_finalizing(
    temporary: &Path,
    total_size: u64,
) -> Result<(), DownloadEngineError> {
    let mut identity = load_resource_identity(temporary).await?.ok_or_else(|| {
        DownloadEngineError::InvalidPartialMetadata(
            "resume identity is missing before finalization".to_owned(),
        )
    })?;
    identity.finalizing = true;
    identity.total_size = Some(total_size);
    save_resource_identity(temporary, &identity).await
}

async fn files_equal(left: &Path, right: &Path) -> Result<bool, DownloadEngineError> {
    let left_metadata = fs::symlink_metadata(left)
        .await
        .map_err(DownloadEngineError::Io)?;
    let right_metadata = fs::symlink_metadata(right)
        .await
        .map_err(DownloadEngineError::Io)?;
    if !left_metadata.file_type().is_file()
        || !right_metadata.file_type().is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return Ok(false);
    }

    let mut left_file = File::open(left).await.map_err(DownloadEngineError::Io)?;
    let mut right_file = File::open(right).await.map_err(DownloadEngineError::Io)?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_read = left_file
            .read(&mut left_buffer)
            .await
            .map_err(DownloadEngineError::Io)?;
        let right_read = right_file
            .read(&mut right_buffer)
            .await
            .map_err(DownloadEngineError::Io)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

async fn remove_published_temporary(path: &Path) {
    if let Err(error) = fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(path = %path.display(), %error, "could not remove finalized temporary file");
    }
}

fn parse_retry_after(response: &Response) -> Option<Duration> {
    let value = response.headers().get(RETRY_AFTER)?.to_str().ok()?;
    retry_after_delay(value)
}

async fn wait_for_retry(
    delay: Duration,
    control: &mut ManagedDownloadControl,
) -> Result<(), DownloadEngineError> {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if *control.pause.borrow() {
            return Err(DownloadEngineError::Paused);
        }
        tokio::select! {
            _ = &mut sleep => return Ok(()),
            changed = control.cancellation.changed() => {
                if changed.is_err() || *control.cancellation.borrow_and_update() {
                    return Err(DownloadEngineError::Cancelled);
                }
            }
            changed = control.pause.changed() => {
                if changed.is_err() || *control.pause.borrow() {
                    return Err(DownloadEngineError::Paused);
                }
            }
        }
    }
}

fn parse_content_length(response: &Response) -> Result<Option<u64>, DownloadEngineError> {
    response
        .headers()
        .get(CONTENT_LENGTH)
        .map(|header| {
            header
                .to_str()
                .ok()
                .and_then(|value| value.parse().ok())
                .ok_or(DownloadEngineError::InvalidContentLength)
        })
        .transpose()
}

fn parse_content_range(response: &Response) -> Result<ContentRange, DownloadEngineError> {
    let value = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or(DownloadEngineError::InvalidContentRange)?;
    let value = value
        .strip_prefix("bytes ")
        .ok_or(DownloadEngineError::InvalidContentRange)?;
    let (range, total) = value
        .split_once('/')
        .ok_or(DownloadEngineError::InvalidContentRange)?;
    let (start, end) = range
        .split_once('-')
        .ok_or(DownloadEngineError::InvalidContentRange)?;
    let start = start
        .parse::<u64>()
        .map_err(|_| DownloadEngineError::InvalidContentRange)?;
    let end = end
        .parse::<u64>()
        .map_err(|_| DownloadEngineError::InvalidContentRange)?;
    let total = if total == "*" {
        None
    } else {
        Some(
            total
                .parse::<u64>()
                .map_err(|_| DownloadEngineError::InvalidContentRange)?,
        )
    };

    if end < start || total.is_some_and(|total| total <= end) {
        return Err(DownloadEngineError::InvalidContentRange);
    }

    Ok(ContentRange { start, end, total })
}

fn parse_unsatisfied_range(response: &Response) -> Result<UnsatisfiedRange, DownloadEngineError> {
    let value = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or(DownloadEngineError::InvalidContentRange)?;
    let total = value
        .strip_prefix("bytes */")
        .ok_or(DownloadEngineError::InvalidContentRange)?
        .parse::<u64>()
        .map_err(|_| DownloadEngineError::InvalidContentRange)?;

    Ok(UnsatisfiedRange { total })
}

async fn inspect_partial_file(path: &Path) -> Result<Option<u64>, DownloadEngineError> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata.len())),
        Ok(_) => Err(DownloadEngineError::InvalidPartialFile),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DownloadEngineError::Io(error)),
    }
}

async fn open_partial_file(
    path: &Path,
    expected_size: u64,
    append: bool,
    exists: bool,
) -> Result<File, DownloadEngineError> {
    if exists {
        let path_metadata = fs::symlink_metadata(path)
            .await
            .map_err(DownloadEngineError::Io)?;
        if !path_metadata.file_type().is_file() {
            return Err(DownloadEngineError::InvalidPartialFile);
        }
        if append && path_metadata.len() != expected_size {
            return Err(DownloadEngineError::PartialFileChanged);
        }

        let mut file = OpenOptions::new()
            .write(true)
            .truncate(!append)
            .open(path)
            .await
            .map_err(DownloadEngineError::Io)?;
        if append {
            if file
                .metadata()
                .await
                .map_err(DownloadEngineError::Io)?
                .len()
                != expected_size
            {
                return Err(DownloadEngineError::PartialFileChanged);
            }
            file.seek(SeekFrom::End(0))
                .await
                .map_err(DownloadEngineError::Io)?;
        }
        return Ok(file);
    }

    if append {
        return Err(DownloadEngineError::PartialFileChanged);
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                DownloadEngineError::TemporaryFileExists
            } else {
                DownloadEngineError::Io(error)
            }
        })
}

async fn reset_partial_file(path: &Path, exists: bool) -> Result<(), DownloadEngineError> {
    drop(open_partial_file(path, 0, false, exists).await?);
    Ok(())
}

fn next_timestamp(previous: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    let next_millisecond = previous
        .as_unix_millis()
        .checked_add(1)
        .ok_or(TimestampError::OutOfRange)?;
    Ok(Timestamp::now()?.max(Timestamp::from_unix_millis(next_millisecond)))
}

fn temporary_path(destination: &Path) -> PathBuf {
    let mut name: OsString = destination.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

fn segment_file_path(
    temporary: &Path,
    segment_count: usize,
    total_size: u64,
    index: usize,
) -> PathBuf {
    let mut name: OsString = temporary.as_os_str().to_os_string();
    name.push(format!(".segments-{segment_count}-{total_size}-{index}"));
    PathBuf::from(name)
}

async fn segmented_layout(
    temporary: &Path,
    configured_segment_count: usize,
    total_size: u64,
) -> Result<usize, DownloadEngineError> {
    if configured_segment_count != 4 {
        for candidate in std::iter::once(4).chain(2..=32) {
            if candidate == configured_segment_count {
                continue;
            }
            for index in 0..candidate {
                let path = segment_file_path(temporary, candidate, total_size, index);
                match fs::symlink_metadata(path).await {
                    Ok(_) => return Ok(candidate),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(DownloadEngineError::Io(error)),
                }
            }
        }
    }
    Ok(configured_segment_count)
}

async fn segment_files_size(
    temporary: &Path,
    segment_count: usize,
    total_size: u64,
) -> Result<u64, DownloadEngineError> {
    let planned = segments::plan_segments(total_size, segment_count, |index| {
        segment_file_path(temporary, segment_count, total_size, index)
    })
    .map_err(|_| DownloadEngineError::InvalidConfiguration)?
    .ok_or(DownloadEngineError::InvalidConfiguration)?;
    let mut total = 0_u64;
    for segment in planned {
        let size = match fs::symlink_metadata(&segment.path).await {
            Ok(metadata) if metadata.file_type().is_file() => metadata.len(),
            Ok(_) => return Err(DownloadEngineError::InvalidPartialFile),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(DownloadEngineError::Io(error)),
        };
        if size > segment.expected_length {
            return Err(DownloadEngineError::SegmentFailed {
                index: segment.index,
                start: segment.start,
                end: segment.end,
            });
        }
        total = total
            .checked_add(size)
            .ok_or(DownloadEngineError::ByteCountOverflow)?;
    }
    Ok(total)
}

async fn assemble_segments(
    segments: &[Segment],
    temporary: &Path,
    total_size: u64,
    cancellation: &mut watch::Receiver<bool>,
    pause: &mut watch::Receiver<bool>,
) -> Result<(), DownloadEngineError> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .await
        .map_err(DownloadEngineError::Io)?;
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];

    for segment in segments {
        if segment.expected_length != segment.end - segment.start + 1 {
            return Err(DownloadEngineError::SegmentFailed {
                index: segment.index,
                start: segment.start,
                end: segment.end,
            });
        }
        let metadata = fs::symlink_metadata(&segment.path)
            .await
            .map_err(DownloadEngineError::Io)?;
        if !metadata.file_type().is_file() || metadata.len() != segment.expected_length {
            return Err(DownloadEngineError::SegmentFailed {
                index: segment.index,
                start: segment.start,
                end: segment.end,
            });
        }
        let mut input = File::open(&segment.path)
            .await
            .map_err(DownloadEngineError::Io)?;
        let mut segment_copied = 0_u64;
        loop {
            if *cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *pause.borrow() {
                return Err(DownloadEngineError::Paused);
            }
            let read = input
                .read(&mut buffer)
                .await
                .map_err(DownloadEngineError::Io)?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .await
                .map_err(DownloadEngineError::Io)?;
            let read = u64::try_from(read).map_err(|_| DownloadEngineError::ByteCountOverflow)?;
            segment_copied = segment_copied
                .checked_add(read)
                .ok_or(DownloadEngineError::ByteCountOverflow)?;
            copied = copied
                .checked_add(read)
                .ok_or(DownloadEngineError::ByteCountOverflow)?;
            if segment_copied > segment.expected_length || copied > total_size {
                return Err(DownloadEngineError::SegmentFailed {
                    index: segment.index,
                    start: segment.start,
                    end: segment.end,
                });
            }
        }
        if segment_copied != segment.expected_length {
            return Err(DownloadEngineError::SegmentFailed {
                index: segment.index,
                start: segment.start,
                end: segment.end,
            });
        }
    }

    if copied != total_size {
        return Err(DownloadEngineError::IncompleteResponse {
            expected: total_size,
            actual: copied,
        });
    }
    output.flush().await.map_err(DownloadEngineError::Io)?;
    output.sync_all().await.map_err(DownloadEngineError::Io)?;
    let final_size = output
        .metadata()
        .await
        .map_err(DownloadEngineError::Io)?
        .len();
    if final_size != total_size {
        return Err(DownloadEngineError::IncompleteResponse {
            expected: total_size,
            actual: final_size,
        });
    }
    Ok(())
}

async fn cleanup_segment_files(
    temporary: &Path,
    segment_count: usize,
    total_size: u64,
) -> Result<(), DownloadEngineError> {
    for index in 0..segment_count {
        let path = segment_file_path(temporary, segment_count, total_size, index);
        match fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(path)
                    .await
                    .map_err(DownloadEngineError::Io)?;
            }
            Ok(_) => return Err(DownloadEngineError::InvalidPartialFile),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }
    }
    Ok(())
}

async fn publish_without_replacement(
    temporary: &Path,
    destination: &Path,
) -> Result<(), DownloadEngineError> {
    match fs::hard_link(temporary, destination).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(DownloadEngineError::DestinationExists)
        }
        Err(error) => Err(DownloadEngineError::Io(error)),
    }
}

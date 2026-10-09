use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use mydownloader_core::{DownloadStateMachine, StateMachineError};
use mydownloader_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
    TimestampError,
};
use mydownloader_storage::DownloadRepository;
use reqwest::{
    Client, Response, StatusCode, Url,
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, RANGE},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom},
    sync::watch,
    task::JoinSet,
    time::interval,
};
use tracing::{info, warn};

use crate::segments::{self, Segment, SegmentResult, SegmentState};
use crate::{DownloadEngine, DownloadEngineError};

const DEFAULT_USER_AGENT: &str = concat!("MyDownloader/", env!("CARGO_PKG_VERSION"));

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
    /// Fixed number of simultaneous byte ranges; values outside 2..=16 disable segmentation.
    pub segment_count: usize,
    /// Files smaller than this threshold use the single-stream path.
    pub minimum_segmented_file_size: u64,
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
            segment_count: 4,
            minimum_segmented_file_size: 4 * 1024 * 1024,
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

    /// Creates an engine with caller-selected timeouts and progress checkpoint policy.
    pub fn with_config(
        repository: R,
        config: HttpEngineConfig,
    ) -> Result<Self, DownloadEngineError> {
        if config.connect_timeout.is_zero()
            || config.read_timeout.is_zero()
            || config.progress_bytes_interval == 0
            || config.progress_time_interval.is_zero()
            || config.user_agent.trim().is_empty()
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
        mut cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        let already_downloading = match current.status() {
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
                self.state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
                return Err(DownloadEngineError::StateMachine(
                    StateMachineError::InvalidTransition {
                        from: status,
                        to: DownloadStatus::Preparing,
                    },
                ));
            }
        };

        let result = self
            .download_prepared(id, already_downloading, &mut cancellation)
            .await;
        match result {
            Ok(item) => Ok(item),
            Err(error) if error.is_cancelled() => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "download cancelled");
                Err(error)
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
        cancellation: &mut watch::Receiver<bool>,
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
        if fs::try_exists(&destination)
            .await
            .map_err(DownloadEngineError::Io)?
        {
            return Err(DownloadEngineError::DestinationExists);
        }
        let temporary_path = temporary_path(&destination);

        let partial_size = inspect_partial_file(&temporary_path).await?;
        let partial_exists = partial_size.is_some();
        let existing_size = partial_size.unwrap_or(0);
        let range_start = (existing_size > 0).then_some(existing_size);
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
            .send_request(url.clone(), range_start, cancellation)
            .await?;
        let segmentation_enabled = (2..=16).contains(&self.config.segment_count)
            && self.config.minimum_segmented_file_size > 0;
        if !partial_exists
            && segmentation_enabled
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
                    },
                    cancellation,
                )
                .await?
            {
                SegmentedOutcome::Completed(completed) => return Ok(*completed),
                SegmentedOutcome::Fallback => {
                    cleanup_segment_files(&temporary_path, self.config.segment_count, total_size)
                        .await?;
                    item = self
                        .repository
                        .get(id)
                        .await?
                        .ok_or(DownloadEngineError::NotFound(id))?;
                    current_status = item.status();
                    expected_updated_at = item.updated_at();
                    response = self.send_request(url.clone(), None, cancellation).await?;
                }
            }
        }

        let mut append = false;
        let mut reset_before_download = false;
        let mut expected_response_bytes = None;
        let mut partial_content_range = None;

        if range_start.is_some() && response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            let unsatisfied = parse_unsatisfied_range(&response)?;
            if existing_size < unsatisfied.total {
                return Err(DownloadEngineError::RangeNotSatisfiable);
            }

            // A matching length does not prove that a stale partial belongs to the
            // current representation, so restart instead of publishing it.
            drop(response);
            response = self.send_request(url.clone(), None, cancellation).await?;
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
            cancellation,
        )
        .await
    }

    async fn send_request(
        &self,
        url: Url,
        range_start: Option<u64>,
        cancellation: &mut watch::Receiver<bool>,
    ) -> Result<Response, DownloadEngineError> {
        loop {
            if *cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }

            tokio::select! {
                changed = cancellation.changed() => {
                    if changed.is_err() || *cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                response = async {
                    let request = self.client.get(url.clone());
                    let request = if let Some(start) = range_start {
                        request.header(RANGE, format!("bytes={start}-"))
                    } else {
                        request
                    };
                    request.send().await
                } => {
                    return response.map_err(DownloadEngineError::Request);
                }
            }
        }
    }

    async fn download_segmented(
        &self,
        context: SegmentedContext,
        cancellation: &mut watch::Receiver<bool>,
    ) -> Result<SegmentedOutcome, DownloadEngineError> {
        let SegmentedContext {
            id,
            mut item,
            mut expected_updated_at,
            mut status,
            url,
            total_size,
        } = context;
        let temporary_path = temporary_path(item.destination());
        let mut segments =
            segments::plan_segments(total_size, self.config.segment_count, |index| {
                segment_file_path(
                    &temporary_path,
                    self.config.segment_count,
                    total_size,
                    index,
                )
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
        let mut tasks = JoinSet::new();
        for mut segment in segments {
            if segment.state == SegmentState::Complete {
                continue;
            }
            let client = self.client.clone();
            let url = url.clone();
            let mut cancellation = cancellation.clone();
            let mut stop = stop_receiver.clone();
            let downloaded = Arc::clone(&downloaded);
            tasks.spawn(async move {
                let index = segment.index;
                let result = segments::download_segment(
                    &mut segment,
                    total_size,
                    &url,
                    &client,
                    &mut cancellation,
                    &mut stop,
                    &downloaded,
                )
                .await;
                result.map(|result| (index, result))
            });
        }

        let mut remaining = tasks.len();
        let mut stop_reason = None;
        let poll_interval = self
            .config
            .progress_time_interval
            .min(Duration::from_millis(50));
        let mut progress_timer = interval(poll_interval);
        let _ = progress_timer.tick().await;
        let mut last_persisted_bytes = already_downloaded;
        let mut last_persisted_at = Instant::now();
        while remaining > 0 {
            tokio::select! {
                changed = cancellation.changed(), if stop_reason.is_none() => {
                    if changed.is_err() || *cancellation.borrow_and_update() {
                        stop_reason = Some(SegmentedStop::Cancelled);
                        let _ = stop_sender.send(true);
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
                        ).await {
                            stop_reason = Some(SegmentedStop::Failure(error));
                            let _ = stop_sender.send(true);
                        } else {
                            last_persisted_bytes = current_bytes;
                            last_persisted_at = Instant::now();
                        }
                    }
                }
                joined = tasks.join_next() => {
                    remaining = remaining.saturating_sub(1);
                    match joined {
                        Some(Ok(Ok((_, SegmentResult::Complete)))) => {}
                        Some(Ok(Ok((_, SegmentResult::Fallback)))) if stop_reason.is_none() => {
                            stop_reason = Some(SegmentedStop::Fallback);
                            let _ = stop_sender.send(true);
                        }
                        Some(Ok(Ok((_, SegmentResult::Cancelled)))) => {}
                        Some(Ok(Err(error))) if stop_reason.is_none() => {
                            stop_reason = Some(SegmentedStop::Failure(error));
                            let _ = stop_sender.send(true);
                        }
                        Some(Err(_)) if stop_reason.is_none() => {
                            stop_reason = Some(SegmentedStop::Failure(
                                DownloadEngineError::SegmentWorkerFailed
                            ));
                            let _ = stop_sender.send(true);
                        }
                        Some(_) => {}
                        None => remaining = 0,
                    }
                }
            }
        }

        let actual_segment_bytes =
            segment_files_size(&temporary_path, self.config.segment_count, total_size).await?;
        downloaded.store(actual_segment_bytes, Ordering::Release);
        if let Some(reason) = stop_reason {
            self.persist_segment_progress(
                &mut item,
                &mut expected_updated_at,
                status,
                total_size,
                &downloaded,
            )
            .await?;
            return match reason {
                SegmentedStop::Fallback => Ok(SegmentedOutcome::Fallback),
                SegmentedStop::Cancelled => Err(DownloadEngineError::Cancelled),
                SegmentedStop::Failure(error) => Err(error),
            };
        }

        self.persist_segment_progress(
            &mut item,
            &mut expected_updated_at,
            status,
            total_size,
            &downloaded,
        )
        .await?;
        if downloaded.load(Ordering::Acquire) != total_size {
            return Err(DownloadEngineError::IncompleteResponse {
                expected: total_size,
                actual: downloaded.load(Ordering::Acquire),
            });
        }

        assemble_segments(
            &segments::plan_segments(total_size, self.config.segment_count, |index| {
                segment_file_path(
                    &temporary_path,
                    self.config.segment_count,
                    total_size,
                    index,
                )
            })
            .map_err(|_| DownloadEngineError::InvalidConfiguration)?
            .ok_or(DownloadEngineError::InvalidConfiguration)?,
            &temporary_path,
            total_size,
            cancellation,
        )
        .await?;
        cleanup_segment_files(&temporary_path, self.config.segment_count, total_size).await?;
        publish_without_replacement(&temporary_path, item.destination()).await?;

        let completed = self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        info!(download_id = %id, bytes = total_size, "segmented download completed");
        Ok(SegmentedOutcome::Completed(Box::new(completed)))
    }

    async fn persist_segment_progress(
        &self,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        status: DownloadStatus,
        total_size: u64,
        downloaded: &AtomicU64,
    ) -> Result<(), DownloadEngineError> {
        let downloaded_bytes = downloaded.load(Ordering::Acquire);
        item.set_progress_at(
            DownloadProgress::new(downloaded_bytes, Some(total_size), None, None)?,
            next_timestamp(item.updated_at())?,
        )?;
        self.persist_if_status(item, status, *expected_updated_at)
            .await?;
        *expected_updated_at = item.updated_at();
        Ok(())
    }

    async fn receive_body(
        &self,
        mut response: Response,
        mut file: File,
        context: BodyContext,
        cancellation: &mut watch::Receiver<bool>,
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
                    if *cancellation.borrow_and_update() {
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
                        changed = cancellation.changed() => {
                            if changed.is_err() || *cancellation.borrow_and_update() {
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
                item.set_progress_at(
                    DownloadProgress::new(downloaded_bytes, total_bytes, None, None)?,
                    next_timestamp(item.updated_at())?,
                )?;

                let now = Instant::now();
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

            if *cancellation.borrow_and_update() {
                self.persist_if_status(&item, DownloadStatus::Downloading, persisted_updated_at)
                    .await?;
                persisted_updated_at = item.updated_at();
                return Err(DownloadEngineError::Cancelled);
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
                if *cancellation.borrow_and_update() {
                    return Err(DownloadEngineError::Cancelled);
                }
                publish_without_replacement(&temporary_path, item.destination()).await?;

                let completed = self
                    .state_machine
                    .transition(id, DownloadStatus::Completed)
                    .await?;
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

fn same_http_target(response: &Url, requested: &Url) -> bool {
    let mut response = response.clone();
    let mut requested = requested.clone();
    response.set_fragment(None);
    requested.set_fragment(None);
    response == requested
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
            .read(append)
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
        Ok(()) => {
            fs::remove_file(temporary)
                .await
                .map_err(DownloadEngineError::Io)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(DownloadEngineError::DestinationExists)
        }
        Err(error) => Err(DownloadEngineError::Io(error)),
    }
}

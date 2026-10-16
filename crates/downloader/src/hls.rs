//! Native parallel HLS (HTTP Live Streaming) download engine.
//!
//! # How it works
//!
//! 1. Fetch the `.m3u8` manifest (master or media playlist).
//! 2. If master playlist: select the highest-bitrate variant.
//! 3. Parse the media playlist to get an ordered list of segment URIs.
//! 4. Download all segments concurrently, limited by a [`tokio::sync::Semaphore`].
//! 5. If `#EXT-X-KEY:METHOD=AES-128` is present, decrypt each segment after download.
//! 6. Concatenate / remux the assembled segments into the final destination file.
//!
//! Segment writes are positional: segment `i` is written at byte-offset
//! `sum(sizes[0..i])`.  For segments with unknown lengths the engine falls
//! back to sequential in-memory buffering before stitching.

use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
};
use fetchx_storage::DownloadRepository;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Semaphore, watch},
    time::sleep,
};
use tracing::{info, warn};

use crate::{
    DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    retry::{RetryPolicy, retry_after_delay},
};

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_SEGMENT_CONCURRENCY: usize = 4;
const MAX_SEGMENT_CONCURRENCY: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_millis(500);
const MAX_MANIFEST_BYTES: usize = 10 * 1024 * 1024; // 10 MiB guard
const MAX_SEGMENT_SIZE: u64 = 512 * 1024 * 1024; // 512 MiB per segment guard
const DEFAULT_USER_AGENT: &str = concat!("fetchx/", env!("CARGO_PKG_VERSION"));
const AES128_BLOCK_SIZE: usize = 16;
const AES128_KEY_LEN: usize = 16;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Configuration for the HLS engine.
#[derive(Clone, Debug)]
pub struct HlsEngineConfig {
    /// Maximum number of concurrent segment downloads. Clamped to 1–[`MAX_SEGMENT_CONCURRENCY`].
    pub segment_concurrency: usize,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub user_agent: String,
}

impl Default for HlsEngineConfig {
    fn default() -> Self {
        Self {
            segment_concurrency: DEFAULT_SEGMENT_CONCURRENCY,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

// ── Manifest types ────────────────────────────────────────────────────────────

/// A single media segment parsed from a media playlist.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct HlsSegment {
    /// Absolute URI of the segment.
    uri: String,
    /// Approximate duration from `#EXTINF`, used for total-size estimation.
    duration_secs: f32,
    sequence: u64,
    /// Encryption key info if `#EXT-X-KEY` was seen before this segment.
    key: Option<HlsKey>,
}

/// Encryption key descriptor for an HLS segment.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct HlsKey {
    uri: String,
    /// 16-byte IV; if absent, the segment sequence number is used.
    iv: Option<[u8; AES128_BLOCK_SIZE]>,
}

#[derive(Deserialize, Serialize)]
struct HlsSegmentIdentity {
    segment: HlsSegment,
    size: u64,
}

struct SegmentAttemptError {
    error: DownloadEngineError,
    retry_after: Option<Duration>,
}

/// Outcome of parsing an `.m3u8` file.
enum PlaylistKind {
    /// A master playlist pointing to variant streams.
    Master(Vec<VariantStream>),
    /// A media playlist with a concrete segment list.
    Media(Vec<HlsSegment>),
}

struct VariantStream {
    bandwidth: u64,
    uri: String,
}

// ── HLS Engine ────────────────────────────────────────────────────────────────

/// Parallel HLS downloader.  Implements both [`DownloadEngine`] and
/// [`ManagedDownloadEngine`].
pub struct HlsEngine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    client: Client,
    config: HlsEngineConfig,
}

impl<R> HlsEngine<R>
where
    R: DownloadRepository + Clone,
{
    /// Creates an engine with default configuration.
    pub fn new(repository: R) -> Result<Self, DownloadEngineError> {
        Self::with_config(repository, HlsEngineConfig::default())
    }

    /// Creates an engine with caller-supplied configuration.
    pub fn with_config(
        repository: R,
        config: HlsEngineConfig,
    ) -> Result<Self, DownloadEngineError> {
        let client = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .build()
            .map_err(DownloadEngineError::ClientConfiguration)?;
        Ok(Self {
            state_machine: DownloadStateMachine::new(repository.clone()),
            repository,
            client,
            config,
        })
    }

    // ── Entry point ───────────────────────────────────────────────────────────

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
        let is_hls = current.source().engine() == EngineKind::Hls
            || (current.source().engine() == EngineKind::DirectHttp
                && is_hls_url(current.source().url()));
        if !is_hls {
            return Err(DownloadEngineError::EngineMismatch);
        }

        // Validate the URL before touching state.
        let url = validate_hls_url(current.source().url())?;

        // Lifecycle: Queued/Failed/Cancelled → Preparing
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

        let result = self.run_job(id, &mut item, &url, &mut control).await;

        match result {
            Ok(completed) => Ok(completed),
            Err(DownloadEngineError::Cancelled) => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "HLS download cancelled");
                Err(DownloadEngineError::Cancelled)
            }
            Err(DownloadEngineError::Paused) => {
                // Caller (queue) handles the state transition.
                Err(DownloadEngineError::Paused)
            }
            Err(error) => {
                if let Err(e) = self.persist_failure(id, &error).await {
                    warn!(download_id = %id, %e, "HLS: could not persist failure state");
                }
                warn!(download_id = %id, %error, "HLS download failed");
                Err(error)
            }
        }
    }

    // ── Main job logic ────────────────────────────────────────────────────────

    async fn run_job(
        &self,
        id: DownloadId,
        item: &mut DownloadItem,
        url: &Url,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        // Quick cancellation check before any network I/O.
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if *control.pause.borrow_and_update() {
            return Err(DownloadEngineError::Paused);
        }

        // ── Step 1: Fetch and parse the manifest ───────────────────────────────
        if item.status() == DownloadStatus::Preparing {
            *item = self
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
        }

        let workspace = hls_workspace_path(item.destination(), id);
        let part_path = workspace.join("assembled.ts");
        match tokio::fs::symlink_metadata(item.destination()).await {
            Ok(destination_metadata)
                if destination_metadata.file_type().is_file()
                    && files_equal(item.destination(), &part_path).await? =>
            {
                *item = self
                    .state_machine
                    .transition(id, DownloadStatus::Completed)
                    .await?;
                if tokio::fs::remove_dir_all(&workspace).await.is_err() {
                    warn!(download_id = %id, "HLS recovered workspace cleanup failed");
                }
                return Ok(item.clone());
            }
            Ok(_) => return Err(DownloadEngineError::DestinationExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }

        let segments = self.resolve_segments(url, control).await?;

        if segments.is_empty() {
            return Err(DownloadEngineError::InvalidUrl);
        }

        // ── Step 2: Determine workspace paths ──────────────────────────────────
        // Create or reuse existing workspace (for resume).
        ensure_workspace(&workspace).await?;

        // ── Step 3: Parallel segment download ─────────────────────────────────
        let total_segments = segments.len();
        let downloaded_bytes_counter = Arc::new(AtomicU64::new(item.progress().downloaded_bytes()));

        // Estimate total bytes: sum of already-downloaded bytes per existing
        // segment files gives a floor; we also use duration hints if known.
        let total_bytes_estimate = estimate_total_bytes(&segments, &workspace).await;

        self.download_all_segments(
            id,
            item,
            &segments,
            &workspace,
            control,
            Arc::clone(&downloaded_bytes_counter),
            total_bytes_estimate,
            total_segments,
        )
        .await?;

        // ── Step 4: Assemble segments into a single .ts file ──────────────────
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        assemble_segments(&segments, &workspace, &part_path).await?;

        // ── Step 5: Finalize ───────────────────────────────────────────────────
        let assembled_size = tokio::fs::metadata(&part_path)
            .await
            .map_err(DownloadEngineError::Io)?
            .len();

        // Final progress at 100 %.
        let mut expected_ts = item.updated_at();
        update_progress(
            &self.repository,
            item,
            &mut expected_ts,
            assembled_size,
            Some(assembled_size),
            None,
            Some(0),
        )
        .await?;

        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }

        // Promote the assembled .ts file to the final destination.
        promote_to_destination(&part_path, item.destination()).await?;

        *item = self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;

        // Best-effort workspace cleanup.
        if tokio::fs::remove_dir_all(&workspace).await.is_err() {
            warn!(download_id = %id, "HLS workspace cleanup failed");
        }

        Ok(item.clone())
    }

    // ── Manifest resolution ───────────────────────────────────────────────────

    /// Fetches the m3u8 URL, parses it, and follows master playlists to the
    /// highest-bandwidth variant.  Returns the ordered list of media segments.
    async fn resolve_segments(
        &self,
        url: &Url,
        control: &mut ManagedDownloadControl,
    ) -> Result<Vec<HlsSegment>, DownloadEngineError> {
        let mut current_url = url.clone();
        for _ in 0..5 {
            // Up to 5 redirects through master → variant chains.
            if *control.cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            let body = self.fetch_manifest(&current_url).await?;
            let text = String::from_utf8(body).map_err(|_| DownloadEngineError::InvalidUrl)?;
            match parse_playlist(&text, &current_url)? {
                PlaylistKind::Media(segments) => return Ok(segments),
                PlaylistKind::Master(variants) => {
                    let best = variants
                        .into_iter()
                        .max_by_key(|v| v.bandwidth)
                        .ok_or(DownloadEngineError::InvalidUrl)?;
                    current_url =
                        Url::parse(&best.uri).map_err(|_| DownloadEngineError::InvalidUrl)?;
                }
            }
        }
        Err(DownloadEngineError::InvalidUrl)
    }

    async fn fetch_manifest(&self, url: &Url) -> Result<Vec<u8>, DownloadEngineError> {
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(DownloadEngineError::Request)?;
        if !response.status().is_success() {
            return Err(DownloadEngineError::HttpStatus(response.status().as_u16()));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(DownloadEngineError::Request)?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(DownloadEngineError::InvalidUrl);
        }
        Ok(bytes.to_vec())
    }

    // ── Parallel segment downloader ───────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    async fn download_all_segments(
        &self,
        id: DownloadId,
        item: &mut DownloadItem,
        segments: &[HlsSegment],
        workspace: &Path,
        control: &mut ManagedDownloadControl,
        downloaded_bytes: Arc<AtomicU64>,
        total_bytes_estimate: Option<u64>,
        _total_segments: usize,
    ) -> Result<(), DownloadEngineError> {
        let concurrency = self
            .config
            .segment_concurrency
            .clamp(1, MAX_SEGMENT_CONCURRENCY);
        let semaphore = Arc::new(Semaphore::new(concurrency));

        // Collect segment tasks into a JoinSet.
        let mut join_set: tokio::task::JoinSet<Result<(usize, u64), DownloadEngineError>> =
            tokio::task::JoinSet::new();

        let (cancel_tx, _) = watch::channel(false);

        let total_segs = segments.len();
        let mut completed_segs: usize = 0;

        // Track which segments were already complete from a previous partial run.
        let mut already_done = vec![false; total_segs];
        for (i, segment) in segments.iter().enumerate() {
            let seg_path = segment_path(workspace, i);
            if is_segment_complete(&seg_path, segment).await? {
                already_done[i] = true;
                completed_segs += 1;
            }
        }

        let mut expected_ts = item.updated_at();
        let mut last_persist = tokio::time::Instant::now();

        // Launch tasks for incomplete segments.
        for (i, seg) in segments.iter().enumerate() {
            // Check cancellation before launching each wave of tasks.
            if *control.cancellation.borrow_and_update() {
                // Signal all in-flight tasks to stop.
                let _ = cancel_tx.send(true);
                // Drain the join set.
                while join_set.join_next().await.is_some() {}
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() {
                let _ = cancel_tx.send(true);
                while join_set.join_next().await.is_some() {}
                return Err(DownloadEngineError::Paused);
            }

            if already_done[i] {
                continue;
            }

            let seg = seg.clone();
            let seg_path = segment_path(workspace, i);
            let client = self.client.clone();
            let sem = Arc::clone(&semaphore);
            let dl_counter = Arc::clone(&downloaded_bytes);
            let cancel_rx = cancel_tx.subscribe();

            join_set.spawn(async move {
                let _permit = sem
                    .acquire()
                    .await
                    .map_err(|_| DownloadEngineError::SegmentWorkerFailed)?;
                let size = download_one_segment(&client, &seg, &seg_path, i, cancel_rx, dl_counter)
                    .await?;
                Ok((i, size))
            });
            // Reap finished tasks eagerly to emit progress without waiting for
            // all launches.
            while let Some(result) = join_set.try_join_next() {
                match result {
                    Ok(Ok((idx, size))) => {
                        let _ = (idx, size);
                        completed_segs += 1;
                    }
                    Ok(Err(e)) => {
                        let _ = cancel_tx.send(true);
                        while join_set.join_next().await.is_some() {}
                        return Err(e);
                    }
                    Err(_join_err) => {
                        let _ = cancel_tx.send(true);
                        while join_set.join_next().await.is_some() {}
                        return Err(DownloadEngineError::SegmentWorkerFailed);
                    }
                }
            }

            // Throttled progress persistence.
            let now = tokio::time::Instant::now();
            if now.duration_since(last_persist) >= PROGRESS_PERSIST_INTERVAL {
                let dl_now = downloaded_bytes.load(Ordering::Relaxed);
                let _ = update_progress(
                    &self.repository,
                    item,
                    &mut expected_ts,
                    dl_now,
                    total_bytes_estimate,
                    None,
                    None,
                )
                .await;
                last_persist = now;
            }
        }

        // Await remaining tasks.
        while !join_set.is_empty() {
            let result = tokio::select! {
                biased;
                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        let _ = cancel_tx.send(true);
                        while join_set.join_next().await.is_some() {}
                        return Err(DownloadEngineError::Cancelled);
                    }
                    continue;
                }
                changed = control.pause.changed() => {
                    if changed.is_err() || *control.pause.borrow_and_update() {
                        let _ = cancel_tx.send(true);
                        while join_set.join_next().await.is_some() {}
                        return Err(DownloadEngineError::Paused);
                    }
                    continue;
                }
                result = join_set.join_next() => result,
            };
            let Some(result) = result else {
                break;
            };
            match result {
                Ok(Ok((_idx, _size))) => {
                    completed_segs += 1;
                    // Throttled progress.
                    let now = tokio::time::Instant::now();
                    if now.duration_since(last_persist) >= PROGRESS_PERSIST_INTERVAL {
                        let dl_now = downloaded_bytes.load(Ordering::Relaxed);
                        let _ = update_progress(
                            &self.repository,
                            item,
                            &mut expected_ts,
                            dl_now,
                            total_bytes_estimate,
                            None,
                            None,
                        )
                        .await;
                        last_persist = now;
                    }
                }
                Ok(Err(e)) => {
                    let _ = cancel_tx.send(true);
                    while join_set.join_next().await.is_some() {}
                    return Err(e);
                }
                Err(_join_err) => {
                    let _ = cancel_tx.send(true);
                    while join_set.join_next().await.is_some() {}
                    return Err(DownloadEngineError::SegmentWorkerFailed);
                }
            }
        }

        info!(
            download_id = %id,
            completed = completed_segs,
            total = total_segs,
            "HLS: all segments downloaded"
        );
        Ok(())
    }

    // ── State helpers ─────────────────────────────────────────────────────────

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
            Err(e) => Err(e.into()),
        }
    }

    async fn persist_failure(
        &self,
        id: DownloadId,
        operation: &DownloadEngineError,
    ) -> Result<(), DownloadEngineError> {
        let error_repr = match operation.persisted_error() {
            Ok(e) => e,
            Err(_) => fetchx_models::DownloadError::new(
                fetchx_models::DownloadErrorCode::Engine,
                "The HLS download failed.",
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
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

// ── DownloadEngine / ManagedDownloadEngine impls ──────────────────────────────

impl<R> DownloadEngine for HlsEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let (pause_tx, pause) = watch::channel(false);
        let (limit_tx, connection_limit) = watch::channel(1usize);
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
        drop((pause_tx, limit_tx));
        result
    }
}

impl<R> ManagedDownloadEngine for HlsEngine<R>
where
    R: DownloadRepository + Clone,
{
    fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> impl std::future::Future<Output = Result<DownloadItem, DownloadEngineError>> + Send {
        self.download_controlled(id, control)
    }

    fn supports_active_pause(&self, _engine: EngineKind) -> bool {
        true // HLS can pause between segments.
    }
}

// ── Segment download (single segment with retries) ────────────────────────────

/// Downloads a single HLS segment with bounded retries and exponential back-off.
async fn download_one_segment(
    client: &Client,
    seg: &HlsSegment,
    path: &Path,
    segment_index: usize,
    mut cancel: watch::Receiver<bool>,
    downloaded_bytes: Arc<AtomicU64>,
) -> Result<u64, DownloadEngineError> {
    let url = Url::parse(&seg.uri).map_err(|_| DownloadEngineError::InvalidUrl)?;

    remove_segment_identity(path).await?;
    let started_at = tokio::time::Instant::now();
    let mut attempts: u8 = 0;
    loop {
        if *cancel.borrow() {
            return Err(DownloadEngineError::Cancelled);
        }

        attempts = attempts.saturating_add(1);
        match try_download_segment(
            client,
            &url,
            path,
            segment_index,
            &mut cancel,
            &downloaded_bytes,
        )
        .await
        {
            Ok(_size) => {
                // AES-128 decryption if a key is present.
                if let Some(key_info) = &seg.key {
                    decrypt_segment_aes128(client, key_info, path, seg.sequence).await?;
                }
                let size = tokio::fs::metadata(path)
                    .await
                    .map_err(DownloadEngineError::Io)?
                    .len();
                if size == 0 {
                    return Err(DownloadEngineError::InvalidUrl);
                }
                save_segment_identity(path, seg, size).await?;
                return Ok(size);
            }
            Err(attempt_error) => {
                if !is_retryable_segment_error(&attempt_error.error) {
                    return Err(attempt_error.error);
                }
                let Some(delay) = RetryPolicy::HTTP.delay(
                    attempts,
                    attempt_error.retry_after,
                    started_at.elapsed(),
                ) else {
                    return Err(attempt_error.error);
                };
                let sleep_fut = sleep(delay);
                tokio::select! {
                    _ = sleep_fut => {}
                    changed = cancel.changed() => {
                        if changed.is_err() || *cancel.borrow() {
                            return Err(DownloadEngineError::Cancelled);
                        }
                    }
                }
            }
        }
    }
}

fn is_retryable_segment_error(error: &DownloadEngineError) -> bool {
    match error {
        DownloadEngineError::HttpStatus(status) => RetryPolicy::is_transient_status(*status),
        DownloadEngineError::Request(error) => RetryPolicy::is_transient_request(error),
        DownloadEngineError::IncompleteResponse { .. } => true,
        _ => false,
    }
}

/// Makes a single attempt to download a segment.
async fn try_download_segment(
    client: &Client,
    url: &Url,
    path: &Path,
    _segment_index: usize,
    cancel: &mut watch::Receiver<bool>,
    downloaded_bytes: &AtomicU64,
) -> Result<u64, SegmentAttemptError> {
    // Remove stale partial file so we always start fresh per attempt.
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SegmentAttemptError {
                error: DownloadEngineError::Io(error),
                retry_after: None,
            });
        }
    }

    let response = {
        let send_fut = client.get(url.clone()).send();
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                let _ = changed;
                return Err(SegmentAttemptError {
                    error: DownloadEngineError::Cancelled,
                    retry_after: None,
                });
            }
            result = send_fut => result.map_err(|error| SegmentAttemptError {
                error: DownloadEngineError::Request(error),
                retry_after: None,
            })?
        }
    };

    if !response.status().is_success() {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(retry_after_delay);
        return Err(SegmentAttemptError {
            error: DownloadEngineError::HttpStatus(response.status().as_u16()),
            retry_after,
        });
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|error| SegmentAttemptError {
            error: DownloadEngineError::Io(error),
            retry_after: None,
        })?;

    let mut written: u64 = 0;
    let expected_length = response.content_length();
    if expected_length.is_some_and(|size| size > MAX_SEGMENT_SIZE) {
        return Err(SegmentAttemptError {
            error: DownloadEngineError::ContentLengthExceeded {
                expected: MAX_SEGMENT_SIZE,
            },
            retry_after: None,
        });
    }
    let mut response = response;

    loop {
        let chunk_fut = response.chunk();
        let chunk = tokio::select! {
            biased;
            _ = cancel.changed() => {
                if *cancel.borrow() {
                    let _ = file.flush().await;
                    let _ = tokio::fs::remove_file(path).await;
                    return Err(SegmentAttemptError {
                        error: DownloadEngineError::Cancelled,
                        retry_after: None,
                    });
                }
                continue;
            }
            result = chunk_fut => result.map_err(|error| SegmentAttemptError {
                error: DownloadEngineError::Request(error),
                retry_after: None,
            })?
        };

        let Some(chunk) = chunk else { break };

        let chunk_len = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        let next_written = written
            .checked_add(chunk_len)
            .ok_or_else(|| SegmentAttemptError {
                error: DownloadEngineError::ByteCountOverflow,
                retry_after: None,
            })?;
        if next_written > MAX_SEGMENT_SIZE {
            let _ = tokio::fs::remove_file(path).await;
            return Err(SegmentAttemptError {
                error: DownloadEngineError::ContentLengthExceeded {
                    expected: MAX_SEGMENT_SIZE,
                },
                retry_after: None,
            });
        }

        file.write_all(&chunk)
            .await
            .map_err(|error| SegmentAttemptError {
                error: DownloadEngineError::Io(error),
                retry_after: None,
            })?;
        written = next_written;
        downloaded_bytes.fetch_add(chunk_len, Ordering::AcqRel);
    }

    if expected_length.is_some_and(|expected| expected != written) {
        return Err(SegmentAttemptError {
            error: DownloadEngineError::IncompleteResponse {
                expected: expected_length.unwrap_or_default(),
                actual: written,
            },
            retry_after: None,
        });
    }
    file.flush().await.map_err(|error| SegmentAttemptError {
        error: DownloadEngineError::Io(error),
        retry_after: None,
    })?;
    file.sync_all().await.map_err(|error| SegmentAttemptError {
        error: DownloadEngineError::Io(error),
        retry_after: None,
    })?;
    Ok(written)
}

// ── AES-128 decryption ────────────────────────────────────────────────────────

/// Decrypts a segment file in-place using AES-128-CBC.
///
/// If `METHOD=SAMPLE-AES` or other DRM is encountered the engine already
/// returns `UnsupportedFormat` in the manifest parser — this function is only
/// called for plain `AES-128`.
async fn decrypt_segment_aes128(
    client: &Client,
    key_info: &HlsKey,
    path: &Path,
    segment_sequence: u64,
) -> Result<(), DownloadEngineError> {
    // Fetch the AES key.
    let key_url = Url::parse(&key_info.uri).map_err(|_| DownloadEngineError::InvalidUrl)?;
    let mut key_response = client
        .get(key_url)
        .send()
        .await
        .map_err(DownloadEngineError::Request)?;
    if !key_response.status().is_success() {
        return Err(DownloadEngineError::HttpStatus(
            key_response.status().as_u16(),
        ));
    }
    let mut key_bytes = Vec::with_capacity(AES128_KEY_LEN);
    while let Some(chunk) = key_response
        .chunk()
        .await
        .map_err(DownloadEngineError::Request)?
    {
        if key_bytes.len().saturating_add(chunk.len()) > AES128_KEY_LEN {
            return Err(DownloadEngineError::InvalidUrl);
        }
        key_bytes.extend_from_slice(&chunk);
    }
    if key_bytes.len() != AES128_KEY_LEN {
        return Err(DownloadEngineError::InvalidUrl);
    }
    let key: [u8; AES128_KEY_LEN] = key_bytes[..AES128_KEY_LEN]
        .try_into()
        .map_err(|_| DownloadEngineError::InvalidUrl)?;

    // Determine IV: explicit from manifest or derived from sequence number.
    let iv: [u8; AES128_BLOCK_SIZE] = match &key_info.iv {
        Some(iv) => *iv,
        None => {
            let mut buf = [0u8; AES128_BLOCK_SIZE];
            let seq_bytes = segment_sequence.to_be_bytes();
            buf[8..].copy_from_slice(&seq_bytes);
            buf
        }
    };

    // Read the ciphertext.
    let ciphertext = tokio::fs::read(path)
        .await
        .map_err(DownloadEngineError::Io)?;
    if ciphertext.is_empty() || ciphertext.len() % AES128_BLOCK_SIZE != 0 {
        return Err(DownloadEngineError::InvalidUrl);
    }

    // Decrypt using pure-Rust AES-128-CBC (no unsafe, PKCS7 padding).
    let plaintext =
        aes128_cbc_decrypt(&key, &iv, &ciphertext).ok_or(DownloadEngineError::InvalidUrl)?;

    tokio::fs::write(path, &plaintext)
        .await
        .map_err(DownloadEngineError::Io)?;

    Ok(())
}

/// Pure-Rust AES-128-CBC decrypt with PKCS#7 un-padding.
///
/// Returns `None` if the padding is invalid.
fn aes128_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], ciphertext: &[u8]) -> Option<Vec<u8>> {
    use std::num::Wrapping;

    // Sub-key expansion (AES-128: 10 rounds, 11 round keys of 16 bytes each).
    let round_keys = aes128_key_schedule(key);

    let mut plaintext = Vec::with_capacity(ciphertext.len());
    let mut prev_block = *iv;

    for block in ciphertext.chunks(AES128_BLOCK_SIZE) {
        let block: [u8; 16] = block.try_into().ok()?;
        let decrypted = aes128_decrypt_block(&round_keys, &block);
        // XOR with previous ciphertext block (CBC).
        let mut xored = [0u8; 16];
        for j in 0..16 {
            xored[j] = decrypted[j] ^ prev_block[j];
        }
        plaintext.extend_from_slice(&xored);
        prev_block = block;
    }

    // PKCS#7 un-padding.
    let pad_byte = *plaintext.last()? as usize;
    if pad_byte == 0 || pad_byte > AES128_BLOCK_SIZE {
        return None;
    }
    let pad_start = plaintext.len().checked_sub(pad_byte)?;
    if plaintext[pad_start..]
        .iter()
        .any(|&b| b as usize != pad_byte)
    {
        return None;
    }
    plaintext.truncate(pad_start);
    let _ = Wrapping(0u8); // suppress unused import warning
    Some(plaintext)
}

/// AES-128 key schedule — produces 11 × 16-byte round keys.
fn aes128_key_schedule(key: &[u8; 16]) -> [[u8; 16]; 11] {
    // AES S-box.
    #[rustfmt::skip]
    const S_BOX: [u8; 256] = [
        0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x76,
        0xca,0x82,0xc9,0x7d,0xfa,0x59,0x47,0xf0,0xad,0xd4,0xa2,0xaf,0x9c,0xa4,0x72,0xc0,
        0xb7,0xfd,0x93,0x26,0x36,0x3f,0xf7,0xcc,0x34,0xa5,0xe5,0xf1,0x71,0xd8,0x31,0x15,
        0x04,0xc7,0x23,0xc3,0x18,0x96,0x05,0x9a,0x07,0x12,0x80,0xe2,0xeb,0x27,0xb2,0x75,
        0x09,0x83,0x2c,0x1a,0x1b,0x6e,0x5a,0xa0,0x52,0x3b,0xd6,0xb3,0x29,0xe3,0x2f,0x84,
        0x53,0xd1,0x00,0xed,0x20,0xfc,0xb1,0x5b,0x6a,0xcb,0xbe,0x39,0x4a,0x4c,0x58,0xcf,
        0xd0,0xef,0xaa,0xfb,0x43,0x4d,0x33,0x85,0x45,0xf9,0x02,0x7f,0x50,0x3c,0x9f,0xa8,
        0x51,0xa3,0x40,0x8f,0x92,0x9d,0x38,0xf5,0xbc,0xb6,0xda,0x21,0x10,0xff,0xf3,0xd2,
        0xcd,0x0c,0x13,0xec,0x5f,0x97,0x44,0x17,0xc4,0xa7,0x7e,0x3d,0x64,0x5d,0x19,0x73,
        0x60,0x81,0x4f,0xdc,0x22,0x2a,0x90,0x88,0x46,0xee,0xb8,0x14,0xde,0x5e,0x0b,0xdb,
        0xe0,0x32,0x3a,0x0a,0x49,0x06,0x24,0x5c,0xc2,0xd3,0xac,0x62,0x91,0x95,0xe4,0x79,
        0xe7,0xc8,0x37,0x6d,0x8d,0xd5,0x4e,0xa9,0x6c,0x56,0xf4,0xea,0x65,0x7a,0xae,0x08,
        0xba,0x78,0x25,0x2e,0x1c,0xa6,0xb4,0xc6,0xe8,0xdd,0x74,0x1f,0x4b,0xbd,0x8b,0x8a,
        0x70,0x3e,0xb5,0x66,0x48,0x03,0xf6,0x0e,0x61,0x35,0x57,0xb9,0x86,0xc1,0x1d,0x9e,
        0xe1,0xf8,0x98,0x11,0x69,0xd9,0x8e,0x94,0x9b,0x1e,0x87,0xe9,0xce,0x55,0x28,0xdf,
        0x8c,0xa1,0x89,0x0d,0xbf,0xe6,0x42,0x68,0x41,0x99,0x2d,0x0f,0xb0,0x54,0xbb,0x16,
    ];

    const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

    let mut w = [[0u8; 4]; 44];
    for i in 0..4 {
        w[i] = [key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]];
    }
    for i in 4..44 {
        let mut temp = w[i - 1];
        if i % 4 == 0 {
            // RotWord + SubWord + Rcon
            temp = [temp[1], temp[2], temp[3], temp[0]];
            temp = [
                S_BOX[temp[0] as usize],
                S_BOX[temp[1] as usize],
                S_BOX[temp[2] as usize],
                S_BOX[temp[3] as usize],
            ];
            temp[0] ^= RCON[i / 4 - 1];
        }
        w[i] = [
            w[i - 4][0] ^ temp[0],
            w[i - 4][1] ^ temp[1],
            w[i - 4][2] ^ temp[2],
            w[i - 4][3] ^ temp[3],
        ];
    }

    let mut rk = [[0u8; 16]; 11];
    for r in 0..11 {
        for c in 0..4 {
            rk[r][4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
        }
    }
    rk
}

/// AES-128 block decryption (single 16-byte block).
fn aes128_decrypt_block(round_keys: &[[u8; 16]; 11], block: &[u8; 16]) -> [u8; 16] {
    #[rustfmt::skip]
    const INV_S_BOX: [u8; 256] = [
        0x52,0x09,0x6a,0xd5,0x30,0x36,0xa5,0x38,0xbf,0x40,0xa3,0x9e,0x81,0xf3,0xd7,0xfb,
        0x7c,0xe3,0x39,0x82,0x9b,0x2f,0xff,0x87,0x34,0x8e,0x43,0x44,0xc4,0xde,0xe9,0xcb,
        0x54,0x7b,0x94,0x32,0xa6,0xc2,0x23,0x3d,0xee,0x4c,0x95,0x0b,0x42,0xfa,0xc3,0x4e,
        0x08,0x2e,0xa1,0x66,0x28,0xd9,0x24,0xb2,0x76,0x5b,0xa2,0x49,0x6d,0x8b,0xd1,0x25,
        0x72,0xf8,0xf6,0x64,0x86,0x68,0x98,0x16,0xd4,0xa4,0x5c,0xcc,0x5d,0x65,0xb6,0x92,
        0x6c,0x70,0x48,0x50,0xfd,0xed,0xb9,0xda,0x5e,0x15,0x46,0x57,0xa7,0x8d,0x9d,0x84,
        0x90,0xd8,0xab,0x00,0x8c,0xbc,0xd3,0x0a,0xf7,0xe4,0x58,0x05,0xb8,0xb3,0x45,0x06,
        0xd0,0x2c,0x1e,0x8f,0xca,0x3f,0x0f,0x02,0xc1,0xaf,0xbd,0x03,0x01,0x13,0x8a,0x6b,
        0x3a,0x91,0x11,0x41,0x4f,0x67,0xdc,0xea,0x97,0xf2,0xcf,0xce,0xf0,0xb4,0xe6,0x73,
        0x96,0xac,0x74,0x22,0xe7,0xad,0x35,0x85,0xe2,0xf9,0x37,0xe8,0x1c,0x75,0xdf,0x6e,
        0x47,0xf1,0x1a,0x71,0x1d,0x29,0xc5,0x89,0x6f,0xb7,0x62,0x0e,0xaa,0x18,0xbe,0x1b,
        0xfc,0x56,0x3e,0x4b,0xc6,0xd2,0x79,0x20,0x9a,0xdb,0xc0,0xfe,0x78,0xcd,0x5a,0xf4,
        0x1f,0xdd,0xa8,0x33,0x88,0x07,0xc7,0x31,0xb1,0x12,0x10,0x59,0x27,0x80,0xec,0x5f,
        0x60,0x51,0x7f,0xa9,0x19,0xb5,0x4a,0x0d,0x2d,0xe5,0x7a,0x9f,0x93,0xc9,0x9c,0xef,
        0xa0,0xe0,0x3b,0x4d,0xae,0x2a,0xf5,0xb0,0xc8,0xeb,0xbb,0x3c,0x83,0x53,0x99,0x61,
        0x17,0x2b,0x04,0x7e,0xba,0x77,0xd6,0x26,0xe1,0x69,0x14,0x63,0x55,0x21,0x0c,0x7d,
    ];

    let mut state = *block;

    // Add initial round key.
    xor_round_key(&mut state, &round_keys[10]);

    for round in (1..10).rev() {
        inv_shift_rows(&mut state);
        inv_sub_bytes(&mut state, &INV_S_BOX);
        xor_round_key(&mut state, &round_keys[round]);
        inv_mix_columns(&mut state);
    }

    inv_shift_rows(&mut state);
    inv_sub_bytes(&mut state, &INV_S_BOX);
    xor_round_key(&mut state, &round_keys[0]);

    state
}

fn xor_round_key(state: &mut [u8; 16], rk: &[u8; 16]) {
    for i in 0..16 {
        state[i] ^= rk[i];
    }
}

fn inv_sub_bytes(state: &mut [u8; 16], inv_s_box: &[u8; 256]) {
    for b in state.iter_mut() {
        *b = inv_s_box[*b as usize];
    }
}

fn inv_shift_rows(state: &mut [u8; 16]) {
    // Row 0: no shift [0, 4, 8, 12]
    // Row 1: right-shift by 1: (1, 5, 9, 13) -> (13, 1, 5, 9)
    let (s1, s5, s9, s13) = (state[1], state[5], state[9], state[13]);
    state[1] = s13;
    state[5] = s1;
    state[9] = s5;
    state[13] = s9;

    // Row 2: right-shift by 2: (2, 6, 10, 14) -> (10, 14, 2, 6)
    let (s2, s6, s10, s14) = (state[2], state[6], state[10], state[14]);
    state[2] = s10;
    state[6] = s14;
    state[10] = s2;
    state[14] = s6;

    // Row 3: right-shift by 3 (left-shift by 1): (3, 7, 11, 15) -> (7, 11, 15, 3)
    let (s3, s7, s11, s15) = (state[3], state[7], state[11], state[15]);
    state[3] = s7;
    state[7] = s11;
    state[11] = s15;
    state[15] = s3;
}

fn inv_mix_columns(state: &mut [u8; 16]) {
    for col in 0..4 {
        let s0 = state[col * 4];
        let s1 = state[col * 4 + 1];
        let s2 = state[col * 4 + 2];
        let s3 = state[col * 4 + 3];
        state[col * 4] = gf_mul(s0, 0x0e) ^ gf_mul(s1, 0x0b) ^ gf_mul(s2, 0x0d) ^ gf_mul(s3, 0x09);
        state[col * 4 + 1] =
            gf_mul(s0, 0x09) ^ gf_mul(s1, 0x0e) ^ gf_mul(s2, 0x0b) ^ gf_mul(s3, 0x0d);
        state[col * 4 + 2] =
            gf_mul(s0, 0x0d) ^ gf_mul(s1, 0x09) ^ gf_mul(s2, 0x0e) ^ gf_mul(s3, 0x0b);
        state[col * 4 + 3] =
            gf_mul(s0, 0x0b) ^ gf_mul(s1, 0x0d) ^ gf_mul(s2, 0x09) ^ gf_mul(s3, 0x0e);
    }
}

/// Galois field GF(2^8) multiplication with reduction polynomial 0x11b.
fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut result = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            result ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    result
}

// ── Segment assembly ──────────────────────────────────────────────────────────

/// Concatenates all downloaded segment files into a single `.ts` file.
async fn assemble_segments(
    segments: &[HlsSegment],
    workspace: &Path,
    output: &Path,
) -> Result<(), DownloadEngineError> {
    let mut out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)
        .await
        .map_err(DownloadEngineError::Io)?;

    for (i, _seg) in segments.iter().enumerate() {
        let seg_path = segment_path(workspace, i);
        let data = tokio::fs::read(&seg_path)
            .await
            .map_err(DownloadEngineError::Io)?;
        out.write_all(&data)
            .await
            .map_err(DownloadEngineError::Io)?;
    }
    out.flush().await.map_err(DownloadEngineError::Io)?;
    out.sync_all().await.map_err(DownloadEngineError::Io)?;
    Ok(())
}

// ── Manifest parser ───────────────────────────────────────────────────────────

/// Parses an M3U8 text into either a master or media playlist.
fn parse_playlist(text: &str, base_url: &Url) -> Result<PlaylistKind, DownloadEngineError> {
    let lines: Vec<&str> = text.lines().collect();
    if !lines
        .first()
        .map(|l| l.trim() == "#EXTM3U")
        .unwrap_or(false)
    {
        return Err(DownloadEngineError::InvalidUrl);
    }

    let has_stream_inf = lines.iter().any(|l| l.starts_with("#EXT-X-STREAM-INF:"));
    if has_stream_inf {
        return Ok(PlaylistKind::Master(parse_master_playlist(
            &lines, base_url,
        )));
    }

    Ok(PlaylistKind::Media(parse_media_playlist(&lines, base_url)?))
}

fn parse_master_playlist(lines: &[&str], base_url: &Url) -> Vec<VariantStream> {
    let mut variants = Vec::new();
    let mut bandwidth: Option<u64> = None;

    for line in lines {
        let line = line.trim();
        if line.starts_with("#EXT-X-STREAM-INF:") {
            bandwidth = extract_attribute(line, "BANDWIDTH").and_then(|v| v.parse::<u64>().ok());
        } else if !line.starts_with('#') && !line.is_empty() {
            let uri = resolve_uri(line, base_url);
            variants.push(VariantStream {
                bandwidth: bandwidth.unwrap_or(0),
                uri,
            });
            bandwidth = None;
        }
    }
    variants
}

fn parse_media_playlist(
    lines: &[&str],
    base_url: &Url,
) -> Result<Vec<HlsSegment>, DownloadEngineError> {
    let mut segments = Vec::new();
    let mut duration: f32 = 0.0;
    let mut current_key: Option<HlsKey> = None;
    let mut sequence = 0_u64;

    for line in lines {
        let line = line.trim();

        if let Some(body) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if let Ok(val) = body.trim().parse::<u64>() {
                sequence = val;
            }
            continue;
        }

        if line.starts_with("#EXT-X-KEY:") {
            let method = extract_attribute(line, "METHOD").unwrap_or_default();
            match method {
                "AES-128" => {
                    if extract_attribute(line, "KEYFORMAT")
                        .is_some_and(|format| format != "identity")
                    {
                        return Err(DownloadEngineError::UnsupportedFormat(
                            "non-identity HLS encryption key format",
                        ));
                    }
                    let uri = extract_attribute(line, "URI")
                        .map(|u| u.trim_matches('"').to_owned())
                        .map(|u| resolve_uri(&u, base_url))
                        .ok_or(DownloadEngineError::InvalidUrl)?;
                    let iv = extract_attribute(line, "IV").and_then(parse_iv_hex);
                    current_key = Some(HlsKey { uri, iv });
                }
                "NONE" => {
                    current_key = None;
                }
                // SAMPLE-AES, other DRM → unsupported.
                _ => {
                    return Err(DownloadEngineError::UnsupportedFormat(
                        "SAMPLE-AES or other HLS encryption",
                    ));
                }
            }
            continue;
        }

        if line.starts_with("#EXT-X-MAP:") || line.starts_with("#EXT-X-BYTERANGE:") {
            return Err(DownloadEngineError::UnsupportedFormat(
                "HLS initialization maps and byte-range segments",
            ));
        }

        if let Some(body) = line.strip_prefix("#EXTINF:") {
            let dur_str = body.split(',').next().unwrap_or("0");
            duration = dur_str.trim().parse::<f32>().unwrap_or(0.0);
            continue;
        }

        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // This is a segment URI.
        segments.push(HlsSegment {
            uri: resolve_uri(line, base_url),
            duration_secs: duration,
            sequence,
            key: current_key.clone(),
        });
        sequence = sequence
            .checked_add(1)
            .ok_or(DownloadEngineError::ByteCountOverflow)?;
        duration = 0.0;
    }

    Ok(segments)
}

// ── Manifest parsing helpers ──────────────────────────────────────────────────

fn extract_attribute<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("{key}=");
    let start = line.find(search.as_str())? + search.len();
    let rest = &line[start..];
    if let Some(inner) = rest.strip_prefix('"') {
        let end = inner.find('"')?;
        return Some(&inner[..end]);
    }
    // Unquoted value (ends at comma or end of string).
    let end = rest.find(',').unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn parse_iv_hex(hex_str: &str) -> Option<[u8; 16]> {
    let hex_str = hex_str.trim_start_matches("0x").trim_start_matches("0X");
    if hex_str.len() != 32 {
        return None;
    }
    let mut buf = [0u8; 16];
    for (i, chunk) in hex_str.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        buf[i] = (hi << 4) | lo;
    }
    Some(buf)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn resolve_uri(uri: &str, base: &Url) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_owned();
    }
    base.join(uri)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| uri.to_owned())
}

// ── Progress persistence ──────────────────────────────────────────────────────

async fn update_progress<R: DownloadRepository>(
    repository: &R,
    item: &mut DownloadItem,
    expected_ts: &mut Timestamp,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    speed: Option<u64>,
    eta: Option<u64>,
) -> Result<(), DownloadEngineError> {
    let now = next_timestamp(item.updated_at())?;
    item.set_progress_at(
        DownloadProgress::new(downloaded_bytes, total_bytes, speed, eta)?,
        now,
    )?;

    // Best-effort: if the row has moved (concurrent update) we just skip.
    let _ = repository
        .update_if_status_and_timestamp_current(item, DownloadStatus::Downloading, *expected_ts)
        .await;

    *expected_ts = item.updated_at();
    Ok(())
}

// ── Filesystem helpers ────────────────────────────────────────────────────────

fn hls_workspace_path(destination: &Path, id: DownloadId) -> PathBuf {
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty());
    parent
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".fetchx-hls-{id}"))
}

fn segment_path(workspace: &Path, index: usize) -> PathBuf {
    workspace.join(format!("seg-{index:08}.ts"))
}

async fn ensure_workspace(path: &Path) -> Result<(), DownloadEngineError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(DownloadEngineError::TemporaryFileExists),
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)
            .await
            .map_err(DownloadEngineError::Io),
        Err(e) => Err(DownloadEngineError::Io(e)),
    }
}

async fn is_segment_complete(
    path: &Path,
    segment: &HlsSegment,
) -> Result<bool, DownloadEngineError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(DownloadEngineError::Io(error)),
    };
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Ok(false);
    }

    let identity = match tokio::fs::read(segment_identity_path(path)).await {
        Ok(bytes) => serde_json::from_slice::<HlsSegmentIdentity>(&bytes).ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(DownloadEngineError::Io(error)),
    };
    Ok(identity
        .is_some_and(|identity| identity.segment == *segment && identity.size == metadata.len()))
}

async fn files_equal(left: &Path, right: &Path) -> Result<bool, DownloadEngineError> {
    let left_metadata = match tokio::fs::symlink_metadata(left).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(DownloadEngineError::Io(error)),
    };
    let right_metadata = match tokio::fs::symlink_metadata(right).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(DownloadEngineError::Io(error)),
    };
    if !left_metadata.file_type().is_file()
        || !right_metadata.file_type().is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return Ok(false);
    }

    let mut left_file = tokio::fs::File::open(left)
        .await
        .map_err(DownloadEngineError::Io)?;
    let mut right_file = tokio::fs::File::open(right)
        .await
        .map_err(DownloadEngineError::Io)?;
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

fn segment_identity_path(path: &Path) -> PathBuf {
    let mut identity = path.as_os_str().to_os_string();
    identity.push(".complete");
    PathBuf::from(identity)
}

fn segment_identity_temporary_path(path: &Path) -> PathBuf {
    let mut identity = segment_identity_path(path).into_os_string();
    identity.push(".tmp");
    PathBuf::from(identity)
}

async fn remove_segment_identity(path: &Path) -> Result<(), DownloadEngineError> {
    for identity_path in [
        segment_identity_path(path),
        segment_identity_temporary_path(path),
    ] {
        match tokio::fs::remove_file(identity_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }
    }
    Ok(())
}

async fn save_segment_identity(
    path: &Path,
    segment: &HlsSegment,
    size: u64,
) -> Result<(), DownloadEngineError> {
    let bytes = serde_json::to_vec(&HlsSegmentIdentity {
        segment: segment.clone(),
        size,
    })
    .map_err(|error| DownloadEngineError::InvalidPartialMetadata(error.to_string()))?;
    let temporary_path = segment_identity_temporary_path(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(DownloadEngineError::Io)?;
    file.write_all(&bytes)
        .await
        .map_err(DownloadEngineError::Io)?;
    file.flush().await.map_err(DownloadEngineError::Io)?;
    file.sync_all().await.map_err(DownloadEngineError::Io)?;
    tokio::fs::rename(temporary_path, segment_identity_path(path))
        .await
        .map_err(DownloadEngineError::Io)
}

/// Estimates total download bytes from segment durations and existing file sizes.
async fn estimate_total_bytes(segments: &[HlsSegment], workspace: &Path) -> Option<u64> {
    let total_duration: f32 = segments.iter().map(|s| s.duration_secs).sum();
    if total_duration <= 0.0 {
        return None;
    }

    // Collect sizes of already-downloaded segments.
    let mut known_bytes: u64 = 0;
    let mut known_duration: f32 = 0.0;
    for (i, seg) in segments.iter().enumerate() {
        let path = segment_path(workspace, i);
        if let Ok(m) = tokio::fs::metadata(&path).await {
            known_bytes = known_bytes.saturating_add(m.len());
            known_duration += seg.duration_secs;
        }
    }

    if known_duration <= 0.0 || known_bytes == 0 {
        return None;
    }

    let rate = known_bytes as f64 / known_duration as f64;
    let total = (rate * total_duration as f64) as u64;
    Some(total)
}

/// Atomically promotes a temp file to its final destination, mirroring the
/// `promote_output_to_destination` pattern used in `ytdlp.rs`.
async fn promote_to_destination(
    source: &Path,
    destination: &Path,
) -> Result<(), DownloadEngineError> {
    match tokio::fs::hard_link(source, destination).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            Err(DownloadEngineError::DestinationExists)
        }
        Err(error) => Err(DownloadEngineError::Io(error)),
    }
}

// ── URL validation ────────────────────────────────────────────────────────────

fn validate_hls_url(source: &str) -> Result<Url, DownloadEngineError> {
    let url = Url::parse(source).map_err(|_| DownloadEngineError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(DownloadEngineError::InvalidUrl);
    }
    Ok(url)
}

/// Returns `true` when the URL looks like an HLS manifest.
pub fn is_hls_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.ends_with(".m3u8") || lower.contains(".m3u8?") || lower.contains(".m3u8#")
}

// ── Timestamp helper (shared pattern with ytdlp.rs) ───────────────────────────

fn next_timestamp(previous: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    let next = previous
        .as_unix_millis()
        .checked_add(1)
        .ok_or(fetchx_models::TimestampError::OutOfRange)?;
    Ok(Timestamp::now()?.max(Timestamp::from_unix_millis(next)))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use fetchx_models::{DownloadSource, FileName, Timestamp};
    use fetchx_storage::{Database, DownloadRepository};
    use std::{
        collections::{HashMap, VecDeque},
        error::Error,
        path::PathBuf,
        sync::Arc,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{Mutex, oneshot, watch},
        task::JoinHandle,
        time::timeout,
    };

    // ── Manifest parsing ──────────────────────────────────────────────────────

    struct LocalResponse {
        status: u16,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
        chunk_size: usize,
        chunk_delay: Duration,
        first_chunk: Option<oneshot::Sender<()>>,
    }

    impl LocalResponse {
        fn ok(body: impl AsRef<[u8]>) -> Self {
            Self {
                status: 200,
                body: body.as_ref().to_vec(),
                headers: Vec::new(),
                chunk_size: 16 * 1024,
                chunk_delay: Duration::ZERO,
                first_chunk: None,
            }
        }

        fn status(status: u16) -> Self {
            Self {
                status,
                body: Vec::new(),
                headers: Vec::new(),
                chunk_size: 16 * 1024,
                chunk_delay: Duration::ZERO,
                first_chunk: None,
            }
        }
    }

    struct LocalHlsServer {
        base_url: String,
        routes: Arc<Mutex<HashMap<String, VecDeque<LocalResponse>>>>,
        task: JoinHandle<Vec<String>>,
    }

    impl LocalHlsServer {
        async fn start(expected_requests: usize) -> Result<Self, Box<dyn Error>> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let routes = Arc::new(Mutex::new(HashMap::<String, VecDeque<LocalResponse>>::new()));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let task_routes = Arc::clone(&routes);
            let task_requests = Arc::clone(&requests);
            let task = tokio::spawn(async move {
                let mut connections = Vec::with_capacity(expected_requests);
                for _ in 0..expected_requests {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let routes = Arc::clone(&task_routes);
                    let requests = Arc::clone(&task_requests);
                    connections.push(tokio::spawn(async move {
                        serve_local_response(stream, routes, requests).await;
                    }));
                }
                for connection in connections {
                    let _ = connection.await;
                }
                requests.lock().await.clone()
            });

            Ok(Self {
                base_url: format!("http://{address}"),
                routes,
                task,
            })
        }

        async fn add_responses(
            &self,
            path: &str,
            responses: impl IntoIterator<Item = LocalResponse>,
        ) {
            self.routes
                .lock()
                .await
                .insert(path.to_owned(), responses.into_iter().collect());
        }

        async fn finish(self) -> Result<Vec<String>, Box<dyn Error>> {
            Ok(timeout(Duration::from_secs(10), self.task).await??)
        }
    }

    async fn serve_local_response(
        mut stream: TcpStream,
        routes: Arc<Mutex<HashMap<String, VecDeque<LocalResponse>>>>,
        requests: Arc<Mutex<Vec<String>>>,
    ) {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let Ok(read) = stream.read(&mut buffer).await else {
                return;
            };
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let request = String::from_utf8_lossy(&request).into_owned();
        let Some(path) = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
        else {
            return;
        };
        requests.lock().await.push(path.to_owned());
        let mut response = routes
            .lock()
            .await
            .get_mut(path)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| LocalResponse::status(404));

        let reason = match response.status {
            200 => "OK",
            404 => "Not Found",
            503 => "Service Unavailable",
            _ => "Test Response",
        };
        let mut headers = format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            response.status,
            reason,
            response.body.len()
        );
        for (name, value) in response.headers {
            headers.push_str(&format!("{name}: {value}\r\n"));
        }
        headers.push_str("\r\n");
        if stream.write_all(headers.as_bytes()).await.is_err() {
            return;
        }

        let chunk_size = response.chunk_size.max(1);
        for (index, chunk) in response.body.chunks(chunk_size).enumerate() {
            if stream.write_all(chunk).await.is_err() {
                return;
            }
            if index == 0
                && let Some(signal) = response.first_chunk.take()
            {
                let _ = signal.send(());
            }
            if !response.chunk_delay.is_zero() {
                sleep(response.chunk_delay).await;
            }
        }
        let _ = stream.flush().await;
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Result<Self, std::io::Error> {
            let path = std::env::temp_dir().join(format!("FetchX-HLS-{}", DownloadId::new()));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn destination(&self) -> PathBuf {
            self.0.join("capture.ts")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn create_hls_item(
        database: &Database,
        url: &str,
        destination: &Path,
    ) -> Result<DownloadItem, Box<dyn Error>> {
        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(url, EngineKind::Hls)?,
            FileName::new("capture.ts")?,
            destination.to_path_buf(),
            Timestamp::now()?,
        );
        database.downloads().create(&item).await?;
        Ok(item)
    }

    async fn create_hls_engine(
        database: &Database,
        concurrency: usize,
    ) -> Result<HlsEngine<fetchx_storage::SqliteDownloadRepository>, DownloadEngineError> {
        HlsEngine::with_config(
            database.downloads(),
            HlsEngineConfig {
                segment_concurrency: concurrency,
                ..HlsEngineConfig::default()
            },
        )
    }

    fn media_playlist(segments: &[&str]) -> String {
        format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n{}#EXT-X-ENDLIST\n",
            segments
                .iter()
                .map(|segment| format!("#EXTINF:1.0,\n{segment}\n"))
                .collect::<String>()
        )
    }

    const MEDIA_PLAYLIST: &str = r#"#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXTINF:10.0,
seg0.ts
#EXTINF:10.0,
seg1.ts
#EXTINF:5.0,
seg2.ts
#EXT-X-ENDLIST
"#;

    const MASTER_PLAYLIST: &str = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720
high.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=854x480
mid.m3u8
"#;

    fn base() -> Url {
        Url::parse("https://example.test/stream/").unwrap()
    }

    #[test]
    fn parses_three_segment_vod_media_playlist() {
        let segments = match parse_playlist(MEDIA_PLAYLIST, &base()).unwrap() {
            PlaylistKind::Media(s) => s,
            PlaylistKind::Master(_) => panic!("expected media playlist"),
        };
        assert_eq!(segments.len(), 3);
        assert!(segments[0].uri.ends_with("seg0.ts"));
        assert!(segments[1].uri.ends_with("seg1.ts"));
        assert!(segments[2].uri.ends_with("seg2.ts"));
        assert!((segments[0].duration_secs - 10.0).abs() < 0.01);
        assert!((segments[2].duration_secs - 5.0).abs() < 0.01);
    }

    #[test]
    fn parses_master_playlist_and_selects_highest_bandwidth() {
        let variants = match parse_playlist(MASTER_PLAYLIST, &base()).unwrap() {
            PlaylistKind::Master(v) => v,
            PlaylistKind::Media(_) => panic!("expected master playlist"),
        };
        let best = variants.into_iter().max_by_key(|v| v.bandwidth).unwrap();
        assert_eq!(best.bandwidth, 3_000_000);
        assert!(best.uri.contains("high.m3u8"));
    }

    #[test]
    fn rejects_aes_sample_drm_in_manifest() {
        let drm_playlist = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key\"\nseg0.ts\n";
        assert!(matches!(
            parse_playlist(drm_playlist, &base()),
            Err(DownloadEngineError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn parses_aes128_key_line_with_explicit_iv() {
        let playlist = r#"#EXTM3U
#EXT-X-KEY:METHOD=AES-128,URI="https://keys.example.test/key",IV=0x00000000000000000000000000000001
#EXTINF:10.0,
seg0.ts
#EXT-X-ENDLIST
"#;
        let segments = match parse_playlist(playlist, &base()).unwrap() {
            PlaylistKind::Media(s) => s,
            _ => panic!("expected media"),
        };
        let key = segments[0].key.as_ref().expect("key present");
        assert!(key.uri.contains("key"));
        assert_eq!(
            key.iv,
            Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])
        );
    }

    #[test]
    fn is_hls_url_detects_m3u8_patterns() {
        assert!(is_hls_url("https://example.test/stream/playlist.m3u8"));
        assert!(is_hls_url("https://cdn.test/live.m3u8?token=abc"));
        assert!(!is_hls_url("https://example.test/video.mp4"));
        assert!(!is_hls_url("https://example.test/video.ts"));
    }

    #[test]
    fn resolves_relative_segment_uris() {
        let base = Url::parse("https://cdn.test/streams/live/").unwrap();
        let resolved = resolve_uri("seg0.ts", &base);
        assert_eq!(resolved, "https://cdn.test/streams/live/seg0.ts");

        let abs = resolve_uri("https://other.cdn.test/seg0.ts", &base);
        assert_eq!(abs, "https://other.cdn.test/seg0.ts");
    }

    // ── AES-128 decryption ────────────────────────────────────────────────────

    #[test]
    fn aes128_decrypts_known_test_vector() {
        // NIST AES-128-CBC test vector (from NIST SP 800-38A, F.2.2).
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let iv = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        // Ciphertext from NIST test vector, padded with PKCS#7 for one extra block.
        // Plaintext block 1: 6bc1bee22e409f96e93d7e117393172a
        let ciphertext = [
            // block 1 ciphertext from NIST
            0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9,
            0x19, 0x7d, // PKCS#7 padding block (16 bytes of 0x10)
            0xb2, 0x62, 0xc3, 0x44, 0xb0, 0x11, 0xfb, 0x55, 0x98, 0x59, 0xd0, 0xa4, 0xe5, 0x12,
            0x31, 0xa2,
        ];
        let round_keys = aes128_key_schedule(&key);
        // Verify first block decrypts correctly.
        let block1: [u8; 16] = ciphertext[0..16].try_into().unwrap();
        let decrypted1 = aes128_decrypt_block(&round_keys, &block1);
        // XOR with IV to get plaintext.
        let mut plaintext1 = [0u8; 16];
        for i in 0..16 {
            plaintext1[i] = decrypted1[i] ^ iv[i];
        }
        let expected_pt1 = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];
        assert_eq!(plaintext1, expected_pt1);
    }

    #[test]
    fn gf_mul_satisfies_basic_properties() {
        // 1 is the multiplicative identity.
        assert_eq!(gf_mul(0xab, 1), 0xab);
        // 0 annihilates.
        assert_eq!(gf_mul(0xcd, 0), 0);
        // Commutativity.
        assert_eq!(gf_mul(0x53, 0xca), gf_mul(0xca, 0x53));
    }

    #[test]
    fn extract_attribute_handles_quoted_and_unquoted() {
        let line = r#"#EXT-X-KEY:METHOD=AES-128,URI="https://example.test/key",IV=0xdeadbeef"#;
        assert_eq!(extract_attribute(line, "METHOD"), Some("AES-128"));
        assert_eq!(
            extract_attribute(line, "URI"),
            Some("https://example.test/key")
        );
        assert_eq!(extract_attribute(line, "IV"), Some("0xdeadbeef"));
    }

    #[test]
    fn parse_iv_hex_roundtrips_known_value() {
        let iv_str = "0x000102030405060708090a0b0c0d0e0f";
        let iv = parse_iv_hex(iv_str).unwrap();
        let expected: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        assert_eq!(iv, expected);
    }

    // ── Integration: engine creation ──────────────────────────────────────────

    #[tokio::test]
    async fn hls_engine_creates_with_default_config() {
        use fetchx_storage::Database;
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let repo = db.downloads();
        let engine = HlsEngine::new(repo);
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn engine_rejects_engine_mismatch() {
        use fetchx_models::{DownloadSource, EngineKind, FileName, Timestamp};
        use fetchx_storage::Database;
        use std::path::PathBuf;
        use tokio::sync::watch;

        let db = Database::connect("sqlite::memory:").await.unwrap();
        let repo = db.downloads();
        // Create a download with DirectHttp engine, then try to run with HlsEngine.
        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file.mp4", EngineKind::DirectHttp).unwrap(),
            FileName::new("file.mp4").unwrap(),
            PathBuf::from("file.mp4"),
            Timestamp::now().unwrap(),
        );
        let id = item.id();
        repo.create(&item).await.unwrap();
        let engine = HlsEngine::new(repo).unwrap();
        let (_tx, rx) = watch::channel(false);
        assert!(matches!(
            engine.download(id, rx).await,
            Err(DownloadEngineError::EngineMismatch)
        ));
    }

    #[tokio::test]
    async fn engine_reports_not_found_for_missing_id() {
        use fetchx_storage::Database;
        use tokio::sync::watch;

        let db = Database::connect("sqlite::memory:").await.unwrap();
        let repo = db.downloads();
        let engine = HlsEngine::new(repo).unwrap();
        let (_tx, rx) = watch::channel(false);
        let missing_id = DownloadId::new();
        assert!(matches!(
            engine.download(missing_id, rx).await,
            Err(DownloadEngineError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn media_playlist_downloads_fragments_and_cleans_workspace() -> Result<(), Box<dyn Error>>
    {
        let server = LocalHlsServer::start(3).await?;
        server
            .add_responses(
                "/media.m3u8",
                [LocalResponse::ok(media_playlist(&["a.ts", "b.ts"]))],
            )
            .await;
        server
            .add_responses("/a.ts", [LocalResponse::ok(b"first")])
            .await;
        server
            .add_responses("/b.ts", [LocalResponse::ok(b"second")])
            .await;

        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let item = create_hls_item(
            &database,
            &format!("{}/media.m3u8", server.base_url),
            &directory.destination(),
        )
        .await?;
        let engine = create_hls_engine(&database, 2).await?;
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let completed = engine.download(item.id(), cancel_rx).await?;
        let requests = server.finish().await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(
            tokio::fs::read(directory.destination()).await?,
            b"firstsecond"
        );
        assert_eq!(requests.len(), 3);
        assert!(!hls_workspace_path(&directory.destination(), item.id()).exists());
        Ok(())
    }

    #[tokio::test]
    async fn master_playlist_selects_variant_and_downloads_absolute_and_relative_uris()
    -> Result<(), Box<dyn Error>> {
        let server = LocalHlsServer::start(4).await?;
        server
            .add_responses(
                "/master.m3u8",
                [LocalResponse::ok(
                    "#EXTM3U\n\
                     #EXT-X-STREAM-INF:BANDWIDTH=100\nlow/playlist.m3u8\n\
                     #EXT-X-STREAM-INF:BANDWIDTH=900\nhigh/playlist.m3u8\n",
                )],
            )
            .await;
        server
            .add_responses(
                "/high/playlist.m3u8",
                [LocalResponse::ok(media_playlist(&[
                    &format!("{}/absolute.ts", server.base_url),
                    "relative.ts",
                ]))],
            )
            .await;
        server
            .add_responses("/absolute.ts", [LocalResponse::ok(b"absolute")])
            .await;
        server
            .add_responses("/high/relative.ts", [LocalResponse::ok(b"relative")])
            .await;

        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let item = create_hls_item(
            &database,
            &format!("{}/master.m3u8", server.base_url),
            &directory.destination(),
        )
        .await?;
        let engine = create_hls_engine(&database, 2).await?;
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let completed = engine.download(item.id(), cancel_rx).await?;
        let requests = server.finish().await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(
            tokio::fs::read(directory.destination()).await?,
            b"absoluterelative"
        );
        assert!(requests.contains(&"/high/playlist.m3u8".to_owned()));
        assert!(requests.contains(&"/absolute.ts".to_owned()));
        assert!(requests.contains(&"/high/relative.ts".to_owned()));
        assert!(!requests.contains(&"/low/playlist.m3u8".to_owned()));
        assert!(!hls_workspace_path(&directory.destination(), item.id()).exists());
        Ok(())
    }

    #[tokio::test]
    async fn transient_hls_fragment_failure_retries_and_honors_retry_after()
    -> Result<(), Box<dyn Error>> {
        let server = LocalHlsServer::start(3).await?;
        server
            .add_responses(
                "/retry.m3u8",
                [LocalResponse::ok(media_playlist(&["flaky.ts"]))],
            )
            .await;
        let mut rate_limited = LocalResponse::status(503);
        rate_limited
            .headers
            .push(("Retry-After".to_owned(), "0".to_owned()));
        server
            .add_responses("/flaky.ts", [rate_limited, LocalResponse::ok(b"recovered")])
            .await;

        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let item = create_hls_item(
            &database,
            &format!("{}/retry.m3u8", server.base_url),
            &directory.destination(),
        )
        .await?;
        let engine = create_hls_engine(&database, 1).await?;
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let completed = engine.download(item.id(), cancel_rx).await?;
        let requests = server.finish().await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(
            tokio::fs::read(directory.destination()).await?,
            b"recovered"
        );
        assert_eq!(
            requests.iter().filter(|path| *path == "/flaky.ts").count(),
            2
        );
        Ok(())
    }

    #[tokio::test]
    async fn permanent_hls_fragment_failure_is_not_retried_or_published()
    -> Result<(), Box<dyn Error>> {
        let server = LocalHlsServer::start(2).await?;
        server
            .add_responses(
                "/failure.m3u8",
                [LocalResponse::ok(media_playlist(&["missing.ts"]))],
            )
            .await;
        server
            .add_responses("/missing.ts", [LocalResponse::status(404)])
            .await;

        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let item = create_hls_item(
            &database,
            &format!("{}/failure.m3u8", server.base_url),
            &directory.destination(),
        )
        .await?;
        let engine = create_hls_engine(&database, 1).await?;
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        assert!(matches!(
            engine.download(item.id(), cancel_rx).await,
            Err(DownloadEngineError::HttpStatus(404))
        ));
        let requests = server.finish().await?;
        let persisted = database
            .downloads()
            .get(item.id())
            .await?
            .ok_or("failed HLS download was not persisted")?;

        assert_eq!(persisted.status(), DownloadStatus::Failed);
        assert_eq!(
            requests
                .iter()
                .filter(|path| *path == "/missing.ts")
                .count(),
            1
        );
        assert!(!directory.destination().exists());
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_resumes_only_identity_validated_hls_fragments()
    -> Result<(), Box<dyn Error>> {
        let server = LocalHlsServer::start(6).await?;
        let playlist = media_playlist(&["complete.ts", "slow.ts", "tail.ts"]);
        server
            .add_responses(
                "/resume.m3u8",
                [
                    LocalResponse::ok(playlist.clone()),
                    LocalResponse::ok(playlist),
                ],
            )
            .await;
        server
            .add_responses("/complete.ts", [LocalResponse::ok(b"saved")])
            .await;
        let (first_chunk_tx, first_chunk_rx) = oneshot::channel();
        let mut slow_response = LocalResponse::ok(vec![7; 512 * 1024]);
        slow_response.chunk_size = 1024;
        slow_response.chunk_delay = Duration::from_millis(20);
        slow_response.first_chunk = Some(first_chunk_tx);
        server
            .add_responses("/slow.ts", [slow_response, LocalResponse::ok(b"resumed")])
            .await;
        server
            .add_responses("/tail.ts", [LocalResponse::ok(b"tail")])
            .await;

        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let item = create_hls_item(
            &database,
            &format!("{}/resume.m3u8", server.base_url),
            &directory.destination(),
        )
        .await?;
        let engine = create_hls_engine(&database, 1).await?;
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let id = item.id();
        let download = tokio::spawn(async move { engine.download(id, cancel_rx).await });

        timeout(Duration::from_secs(5), first_chunk_rx).await??;
        cancel_tx.send(true)?;
        assert!(matches!(
            timeout(Duration::from_secs(5), download).await??,
            Err(DownloadEngineError::Cancelled)
        ));

        let workspace = hls_workspace_path(&directory.destination(), id);
        assert!(workspace.exists());
        assert!(segment_identity_path(&segment_path(&workspace, 0)).exists());
        assert!(!directory.destination().exists());

        let engine = create_hls_engine(&database, 1).await?;
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let completed = engine.download(id, cancel_rx).await?;
        let requests = server.finish().await?;
        let persisted = database
            .downloads()
            .get(id)
            .await?
            .ok_or("resumed HLS download was not persisted")?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(persisted.status(), DownloadStatus::Completed);
        assert_eq!(
            tokio::fs::read(directory.destination()).await?,
            b"savedresumedtail"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|path| *path == "/complete.ts")
                .count(),
            1
        );
        assert_eq!(
            requests.iter().filter(|path| *path == "/slow.ts").count(),
            2
        );
        assert!(!workspace.exists());
        Ok(())
    }

    #[tokio::test]
    async fn recovers_hls_crash_after_destination_publication() -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let item = create_hls_item(
            &database,
            "http://127.0.0.1:1/not-requested.m3u8",
            &directory.destination(),
        )
        .await?;
        let workspace = hls_workspace_path(&directory.destination(), item.id());
        tokio::fs::create_dir_all(&workspace).await?;
        let assembled = workspace.join("assembled.ts");
        tokio::fs::write(&assembled, b"finalized-before-crash").await?;
        tokio::fs::hard_link(&assembled, directory.destination()).await?;
        let engine = create_hls_engine(&database, 1).await?;
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let completed = engine.download(item.id(), cancel_rx).await?;
        let persisted = database
            .downloads()
            .get(item.id())
            .await?
            .ok_or("recovered HLS download was not persisted")?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(persisted.status(), DownloadStatus::Completed);
        assert_eq!(
            tokio::fs::read(directory.destination()).await?,
            b"finalized-before-crash"
        );
        assert!(!workspace.exists());
        Ok(())
    }
}

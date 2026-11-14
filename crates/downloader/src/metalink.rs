#![forbid(unsafe_code)]
//! Metalink orchestration engine.
//!
//! Parses `.metalink` (v3) and `.meta4` (v4) XML manifests, resolves mirrors,
//! and dispatches actual downloads through the existing [`DirectHttpEngine`].
//! Mirror failover and checksum verification are handled here.
//!
//! # Security
//! - Filenames containing `..` components, absolute paths, or Windows drive
//!   prefixes are rejected before any network access.
//! - Checksums are verified before the staged file is moved to its final
//!   destination; a mismatch triggers failover to the next mirror.
//!
//! # Architecture
//! The engine creates a transient [`DownloadItem`] for each mirror attempt and
//! routes it through [`DirectHttpEngine::download_managed`].  This reuses
//! DirectHttp's resume/redirect/Range/retry logic at no cost.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use digest::Digest;
use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadSource, DownloadStatus, EngineKind,
    FileName, Timestamp,
};
use fetchx_storage::DownloadRepository;
use md5::Md5;
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::{fs, sync::watch};
use tracing::{info, warn};

use crate::{
    BandwidthControl, DirectHttpEngine, DownloadEngine, DownloadEngineError,
    ManagedDownloadControl, ManagedDownloadEngine,
};

// ── Constants ─────────────────────────────────────────────────────────────────

// ── Public data types ─────────────────────────────────────────────────────────

/// A mirror URL with its priority (lower = higher priority).
#[derive(Clone, Debug)]
pub struct MetalinkMirror {
    pub url: String,
    pub priority: i32,
}

/// A single file entry parsed from a Metalink manifest.
#[derive(Clone, Debug)]
pub struct MetalinkFile {
    pub name: String,
    pub size: Option<u64>,
    /// `(algorithm_lowercase, hex_digest)` pairs (e.g. `"sha-256"`, `"aabb…"`).
    pub hashes: Vec<(String, String)>,
    /// Ordered chunk hashes from a Metalink `<pieces>` element.
    pub chunk_checksums: Vec<MetalinkChunkChecksum>,
    /// Mirrors sorted by ascending priority (priority 1 is highest).
    pub mirrors: Vec<MetalinkMirror>,
}

/// Hashes for consecutive fixed-size pieces of one Metalink file.
#[derive(Clone, Debug)]
pub struct MetalinkChunkChecksum {
    pub algorithm: String,
    pub chunk_length: u64,
    pub hashes: Vec<String>,
}

// ── Engine ─────────────────────────────────────────────────────────────────────

/// Metalink orchestration engine.
///
/// Parses `.metalink` / `.meta4` manifests and dispatches per-mirror downloads
/// through an inner [`DirectHttpEngine`].
pub struct MetalinkEngine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    direct_http: Arc<DirectHttpEngine<R>>,
}

impl<R> MetalinkEngine<R>
where
    R: DownloadRepository + Clone + Send + Sync + 'static,
{
    /// Creates a new engine backed by `direct_http`.
    pub fn new(repository: R, direct_http: Arc<DirectHttpEngine<R>>) -> Self {
        Self {
            state_machine: DownloadStateMachine::new(repository.clone()),
            repository,
            direct_http,
        }
    }

    // ── Core logic ─────────────────────────────────────────────────────────────

    async fn download_inner(
        &self,
        id: DownloadId,
        mut control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;

        if item.source().engine() != EngineKind::Metalink {
            return Err(DownloadEngineError::EngineMismatch);
        }

        // State transition.
        let item = match item.status() {
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

        let result = self.run(id, &item, &mut control).await;

        match result {
            Ok(completed) => Ok(completed),
            Err(DownloadEngineError::Cancelled) => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "Metalink download cancelled");
                Err(DownloadEngineError::Cancelled)
            }
            Err(DownloadEngineError::Paused) => {
                self.transition_to_paused(id).await?;
                info!(download_id = %id, "Metalink download paused");
                Err(DownloadEngineError::Paused)
            }
            Err(error) => {
                if let Err(e) = self.persist_failure(id, &error).await {
                    warn!(download_id = %id, %e, "Metalink: could not persist failure state");
                }
                warn!(download_id = %id, %error, "Metalink download failed");
                Err(error)
            }
        }
    }

    async fn run(
        &self,
        id: DownloadId,
        item: &DownloadItem,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        // Early cooperative check.
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if *control.pause.borrow_and_update() {
            return Err(DownloadEngineError::Paused);
        }

        // Acquire and parse the manifest.
        let manifest_url = item.source().url().to_owned();
        let xml = self.fetch_manifest(&manifest_url, control).await?;

        // Cooperative check after manifest fetch.
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }

        let files = parse_metalink(&xml)?;

        if files.is_empty() {
            return Err(DownloadEngineError::MetalinkError(
                "manifest contains no files".to_owned(),
            ));
        }

        // Validate all filenames before starting any download.
        for mf in &files {
            check_filename_safety(&mf.name)?;
        }

        // Check for missing mirrors early if single-file.
        for mf in &files {
            if mf.mirrors.is_empty() {
                return Err(DownloadEngineError::MetalinkError(format!(
                    "file '{}' has no mirrors",
                    mf.name
                )));
            }
        }

        let destination = item.destination().to_path_buf();
        let multi_file = files.len() > 1;

        // Refuse if destination is an existing plain file and manifest has >1 file.
        if multi_file
            && fs::symlink_metadata(&destination)
                .await
                .ok()
                .is_some_and(|m| m.file_type().is_file())
        {
            return Err(DownloadEngineError::MetalinkError(
                "destination exists as a file but manifest has multiple files".to_owned(),
            ));
        }

        // Keep this ID-scoped staging directory on interruption; DirectHTTP
        // validates its saved URL and range validator before resuming a partial.
        let temp_dir = std::env::temp_dir().join(format!("fetchx-metalink-{id}"));
        if let Err(e) = fs::create_dir_all(&temp_dir).await {
            return Err(DownloadEngineError::Io(e));
        }

        // Transition to Downloading.
        let mut tracked = if item.status() == DownloadStatus::Preparing {
            self.state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?
        } else {
            item.clone()
        };

        let total_files = files.len() as u64;
        let mut files_done: u64 = 0;

        for mf in &files {
            // Cooperative check between files.
            if *control.cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() {
                return Err(DownloadEngineError::Paused);
            }

            let final_path = if multi_file {
                destination.join(&mf.name)
            } else {
                destination.clone()
            };

            // Ensure parent directory exists for multi-file case.
            if multi_file
                && let Some(parent) = final_path.parent()
                && let Err(e) = fs::create_dir_all(parent).await
            {
                return Err(DownloadEngineError::Io(e));
            }

            let result = self
                .download_file(id, mf, &temp_dir, &final_path, &mut tracked, control)
                .await;

            match result {
                Ok(()) => {
                    files_done += 1;
                    // Update aggregate progress: fraction of files done.
                    let progress =
                        DownloadProgress::new(files_done, Some(total_files), None, None)?;
                    tracked.set_progress_at(progress, next_ts(tracked.updated_at())?)?;
                    if let Err(e) = self.repository.update(&tracked).await {
                        warn!(download_id = %id, %e, "Metalink: progress persist error");
                    }
                }
                Err(DownloadEngineError::Cancelled) => {
                    return Err(DownloadEngineError::Cancelled);
                }
                Err(DownloadEngineError::Paused) => {
                    return Err(DownloadEngineError::Paused);
                }
                Err(e) => return Err(e),
            }
        }

        // Cleanup temp dir (should be empty now).
        let _ = fs::remove_dir_all(&temp_dir).await;

        let completed = self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        info!(download_id = %id, "Metalink download completed ({total_files} file(s))");
        Ok(completed)
    }

    /// Downloads a single file from its mirrors in priority order.
    async fn download_file(
        &self,
        parent_id: DownloadId,
        mf: &MetalinkFile,
        temp_dir: &Path,
        final_path: &Path,
        tracked: &mut DownloadItem,
        control: &mut ManagedDownloadControl,
    ) -> Result<(), DownloadEngineError> {
        // Use a unique staging path per file.
        let staging_path = temp_dir.join(format!("{}-{}", parent_id, sanitize_filename(&mf.name)));
        let chunk_dir = temp_dir.join(format!(
            "{}-{}-verified-chunks",
            parent_id,
            sanitize_filename(&mf.name)
        ));
        if !mf.chunk_checksums.is_empty() {
            fs::create_dir_all(&chunk_dir)
                .await
                .map_err(DownloadEngineError::Io)?;
            match self.download_chunk_ranges(mf, &chunk_dir, control).await {
                Ok(()) => {
                    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
                    fs::create_dir_all(parent)
                        .await
                        .map_err(DownloadEngineError::Io)?;
                    let assembled_path = parent.join(format!(
                        ".fetchx-metalink-{}-{}.{}",
                        parent_id,
                        sanitize_filename(&mf.name),
                        DownloadId::new()
                    ));
                    let assembled_path = assemble_verified_chunks(
                        &chunk_dir,
                        &assembled_path,
                        &mf.chunk_checksums[0],
                    )
                    .await?;
                    if let Err(error) =
                        verify_chunk_checksums(&assembled_path, mf.size, &mf.chunk_checksums).await
                    {
                        let _ = fs::remove_file(&assembled_path).await;
                        return Err(error);
                    }
                    if let Err(error) = verify_checksums(&assembled_path, &mf.hashes).await {
                        let _ = fs::remove_file(&assembled_path).await;
                        return Err(error);
                    }
                    match fs::hard_link(&assembled_path, final_path).await {
                        Ok(()) => {}
                        Err(error) => {
                            let _ = fs::remove_file(&assembled_path).await;
                            if error.kind() == std::io::ErrorKind::AlreadyExists {
                                return Err(DownloadEngineError::DestinationExists);
                            }
                            return Err(DownloadEngineError::Io(error));
                        }
                    }
                    if let Err(error) = fs::remove_file(&assembled_path).await {
                        warn!(
                            path = %assembled_path.display(),
                            %error,
                            "Metalink: could not remove assembled staging file after publication"
                        );
                    }
                    let progress = DownloadProgress::new(1, Some(1), None, None)?;
                    tracked.set_progress_at(progress, next_ts(tracked.updated_at())?)?;
                    if let Err(error) = self.repository.update(tracked).await {
                        warn!("Metalink: parent progress persist error: {error}");
                    }
                    return Ok(());
                }
                Err(DownloadEngineError::Cancelled) => {
                    return Err(DownloadEngineError::Cancelled);
                }
                Err(DownloadEngineError::Paused) => return Err(DownloadEngineError::Paused),
                Err(error @ DownloadEngineError::MetalinkChunkChecksumMismatch { .. }) => {
                    return Err(error);
                }
                Err(error) => {
                    warn!(
                        "Metalink: range-based piece transfer unavailable for '{}': {}; trying safe whole-file mirrors",
                        mf.name, error
                    );
                }
            }
        }

        let mut last_error: Option<DownloadEngineError> = None;

        for mirror in &mf.mirrors {
            // Cooperative check between mirror attempts.
            if *control.cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() {
                return Err(DownloadEngineError::Paused);
            }

            // Remove any stale staging file from a previous attempt.
            let _ = fs::remove_file(&staging_path).await;

            // Create a transient DownloadItem for this mirror attempt.
            let transient_id = DownloadId::new();
            let source =
                DownloadSource::new(mirror.url.clone(), EngineKind::DirectHttp).map_err(|_| {
                    DownloadEngineError::MetalinkError(format!(
                        "invalid mirror URL: {}",
                        mirror.url
                    ))
                })?;
            let filename = FileName::new(sanitize_filename(&mf.name)).map_err(|_| {
                DownloadEngineError::MetalinkError(format!("invalid filename: {}", mf.name))
            })?;
            let created_at = Timestamp::now().map_err(DownloadEngineError::Timestamp)?;
            let transient_item = DownloadItem::new(
                transient_id,
                source,
                filename,
                staging_path.clone(),
                created_at,
            );
            self.repository.create(&transient_item).await?;

            // Derive control channels that share the parent's cancellation/pause.
            let (limit_tx, connection_limit) = watch::channel(8_usize);
            let mirror_control = ManagedDownloadControl {
                cancellation: control.cancellation.clone(),
                pause: control.pause.clone(),
                connection_limit,
                resource_demand: None,
                bandwidth: BandwidthControl::default(),
            };
            // Note: limit_tx is moved into the spawned task to keep the channel open
            // for the duration of the download (dropping it early causes the engine
            // to treat the closed channel as a failure).

            let http_engine = Arc::clone(&self.direct_http);
            let download_result = tokio::task::spawn(async move {
                let _limit_tx = limit_tx; // keep sender alive for the task's lifetime
                http_engine
                    .download_managed(transient_id, mirror_control)
                    .await
            })
            .await
            .map_err(|e| DownloadEngineError::MetalinkError(format!("task join: {e}")))?;

            match download_result {
                Err(DownloadEngineError::Cancelled) => {
                    // Cleanup transient item.
                    let _ = self.repository.delete(transient_id).await;
                    let _ = fs::remove_file(&staging_path).await;
                    return Err(DownloadEngineError::Cancelled);
                }
                Err(DownloadEngineError::Paused) => {
                    let _ = self.repository.delete(transient_id).await;
                    let _ = fs::remove_file(&staging_path).await;
                    return Err(DownloadEngineError::Paused);
                }
                Err(e) => {
                    warn!(
                        "Metalink: mirror '{}' for '{}' failed: {}",
                        mirror.url, mf.name, e
                    );
                    let _ = self.repository.delete(transient_id).await;
                    let _ = fs::remove_file(&staging_path).await;
                    last_error = Some(e);
                    // Try next mirror.
                    continue;
                }
                Ok(_completed_transient) => {
                    let publication_path = if mf.chunk_checksums.is_empty() {
                        staging_path.clone()
                    } else {
                        match stage_verified_chunks(
                            &staging_path,
                            mf.size,
                            &mf.chunk_checksums,
                            &chunk_dir,
                        )
                        .await
                        {
                            Ok(()) => match assemble_verified_chunks(
                                &chunk_dir,
                                &staging_path.with_extension("assembled"),
                                &mf.chunk_checksums[0],
                            )
                            .await
                            {
                                Ok(path) => path,
                                Err(error) => {
                                    let _ = self.repository.delete(transient_id).await;
                                    let _ = fs::remove_file(&staging_path).await;
                                    last_error = Some(error);
                                    continue;
                                }
                            },
                            Err(error) => {
                                warn!(
                                    "Metalink: chunk verification failed for '{}' via '{}': {}",
                                    mf.name, mirror.url, error
                                );
                                let _ = self.repository.delete(transient_id).await;
                                let _ = fs::remove_file(&staging_path).await;
                                last_error = Some(error);
                                continue;
                            }
                        }
                    };
                    let chunk_validation =
                        verify_chunk_checksums(&publication_path, mf.size, &mf.chunk_checksums)
                            .await;
                    let checksum_result = match chunk_validation {
                        Ok(()) => verify_checksums(&publication_path, &mf.hashes).await,
                        Err(error) => Err(error),
                    };
                    match checksum_result {
                        Err(
                            error @ (DownloadEngineError::MetalinkChecksumMismatch { .. }
                            | DownloadEngineError::MetalinkChunkChecksumMismatch { .. }),
                        ) => {
                            warn!(
                                "Metalink: checksum mismatch for '{}' via '{}': {}",
                                mf.name, mirror.url, error
                            );
                            let _ = self.repository.delete(transient_id).await;
                            let _ = fs::remove_file(&staging_path).await;
                            if publication_path != staging_path {
                                let _ = fs::remove_file(&publication_path).await;
                            }
                            last_error = Some(error);
                            // Try next mirror.
                            continue;
                        }
                        Err(e) => {
                            let _ = self.repository.delete(transient_id).await;
                            let _ = fs::remove_file(&staging_path).await;
                            if publication_path != staging_path {
                                let _ = fs::remove_file(&publication_path).await;
                            }
                            last_error = Some(e);
                            continue;
                        }
                        Ok(()) => {
                            // Checksum OK. Move staged file to final destination.
                            let _ = self.repository.delete(transient_id).await;
                            if let Err(e) = fs::rename(&publication_path, final_path).await {
                                return Err(DownloadEngineError::Io(e));
                            }
                            // Update tracked progress on parent item.
                            let progress = DownloadProgress::new(1, Some(1), None, None)?;
                            tracked.set_progress_at(progress, next_ts(tracked.updated_at())?)?;
                            if let Err(e) = self.repository.update(tracked).await {
                                warn!("Metalink: parent progress persist error: {e}");
                            }
                            return Ok(());
                        }
                    }
                }
            }
        }

        // All mirrors exhausted.
        Err(last_error.unwrap_or_else(|| {
            DownloadEngineError::MetalinkError(format!("all mirrors failed for file '{}'", mf.name))
        }))
    }

    async fn download_chunk_ranges(
        &self,
        mf: &MetalinkFile,
        chunk_dir: &Path,
        control: &mut ManagedDownloadControl,
    ) -> Result<(), DownloadEngineError> {
        cleanup_chunk_attempts(chunk_dir).await?;
        let mut total_size = mf.size;
        if let Some(size) = total_size {
            validate_chunk_metadata(Some(size), &mf.chunk_checksums)?;
        }
        let mut last_error = None;
        let mut remaining = (0..mf.chunk_checksums[0].hashes.len()).collect::<Vec<_>>();

        for mirror in &mf.mirrors {
            if remaining.is_empty() {
                return Ok(());
            }
            if *control.cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() {
                return Err(DownloadEngineError::Paused);
            }

            let (mut probe, probed_total) = match self
                .direct_http
                .request_metalink_range(&mirror.url, 0, 0, total_size, control)
                .await
            {
                Ok(probe) => probe,
                Err(error @ (DownloadEngineError::Cancelled | DownloadEngineError::Paused)) => {
                    return Err(error);
                }
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            if let Err(error) = consume_probe(&mut probe, control).await {
                if matches!(
                    error,
                    DownloadEngineError::Cancelled | DownloadEngineError::Paused
                ) {
                    return Err(error);
                }
                last_error = Some(error);
                continue;
            }
            if let Err(error) = validate_chunk_metadata(Some(probed_total), &mf.chunk_checksums) {
                last_error = Some(error);
                continue;
            }
            total_size = Some(probed_total);
            remaining = validate_staged_chunks(chunk_dir, total_size, &mf.chunk_checksums).await?;
            if remaining.is_empty() {
                return Ok(());
            }

            let mut mirror_failed = false;
            for index in remaining.iter().copied() {
                if *control.cancellation.borrow_and_update() {
                    return Err(DownloadEngineError::Cancelled);
                }
                if *control.pause.borrow_and_update() {
                    return Err(DownloadEngineError::Paused);
                }
                let first = &mf.chunk_checksums[0];
                let start = (index as u64)
                    .checked_mul(first.chunk_length)
                    .ok_or_else(|| {
                        DownloadEngineError::MetalinkError("chunk range offset overflow".to_owned())
                    })?;
                let end = start
                    .checked_add(first.chunk_length.min(probed_total - start) - 1)
                    .ok_or_else(|| {
                        DownloadEngineError::MetalinkError("chunk range overflow".to_owned())
                    })?;
                let (mut response, _) = match self
                    .direct_http
                    .request_metalink_range(&mirror.url, start, end, total_size, control)
                    .await
                {
                    Ok(response) => response,
                    Err(error @ (DownloadEngineError::Cancelled | DownloadEngineError::Paused)) => {
                        return Err(error);
                    }
                    Err(error) => {
                        last_error = Some(error);
                        mirror_failed = true;
                        break;
                    }
                };
                let piece_path = chunk_dir.join(format!("{index:08}.piece"));
                match receive_and_verify_piece(
                    &mut response,
                    &piece_path,
                    index,
                    end - start + 1,
                    &mf.chunk_checksums,
                    control,
                )
                .await
                {
                    Ok(()) => {}
                    Err(error @ (DownloadEngineError::Cancelled | DownloadEngineError::Paused)) => {
                        return Err(error);
                    }
                    Err(error) => {
                        last_error = Some(error);
                        mirror_failed = true;
                        break;
                    }
                }
            }

            remaining = validate_staged_chunks(chunk_dir, total_size, &mf.chunk_checksums).await?;
            if remaining.is_empty() {
                return Ok(());
            }
            if !mirror_failed {
                last_error = Some(DownloadEngineError::MetalinkError(format!(
                    "mirror '{}' did not provide all required Metalink pieces",
                    mirror.url
                )));
            }
        }

        Err(last_error.unwrap_or_else(|| {
            DownloadEngineError::MetalinkError(format!(
                "no mirror provided the missing pieces for '{}'",
                mf.name
            ))
        }))
    }

    /// Fetches the manifest XML content from a URL or local path.
    async fn fetch_manifest(
        &self,
        source: &str,
        _control: &ManagedDownloadControl,
    ) -> Result<String, DownloadEngineError> {
        if source.starts_with("http://") || source.starts_with("https://") {
            // Use a simple one-off reqwest GET for the manifest.
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(DownloadEngineError::ClientConfiguration)?;
            let response = client
                .get(source)
                .send()
                .await
                .map_err(DownloadEngineError::Request)?;
            if !response.status().is_success() {
                return Err(DownloadEngineError::HttpStatus(response.status().as_u16()));
            }
            response.text().await.map_err(DownloadEngineError::Request)
        } else {
            // Local file path or file:// URI.
            let path = if let Some(stripped) = source.strip_prefix("file://") {
                PathBuf::from(stripped)
            } else {
                PathBuf::from(source)
            };
            let bytes = fs::read(&path).await.map_err(DownloadEngineError::Io)?;
            String::from_utf8(bytes).map_err(|_| {
                DownloadEngineError::MetalinkError("manifest file is not valid UTF-8".to_owned())
            })
        }
    }

    // ── State helpers ──────────────────────────────────────────────────────────

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

    async fn transition_to_paused(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        match self
            .state_machine
            .transition(id, DownloadStatus::Paused)
            .await
        {
            Ok(_) => Ok(()),
            Err(StateMachineError::InvalidTransition {
                from: DownloadStatus::Paused,
                to: DownloadStatus::Paused,
            }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn persist_failure(
        &self,
        id: DownloadId,
        op: &DownloadEngineError,
    ) -> Result<(), DownloadEngineError> {
        let error_repr = match op.persisted_error() {
            Ok(e) => e,
            Err(_) => fetchx_models::DownloadError::new(
                fetchx_models::DownloadErrorCode::Engine,
                "The Metalink download failed.",
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
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

// ── Trait impls ────────────────────────────────────────────────────────────────

impl<R> DownloadEngine for MetalinkEngine<R>
where
    R: DownloadRepository + Clone + Send + Sync + 'static,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let (_pause_tx, pause) = watch::channel(false);
        let (_limit_tx, connection_limit) = watch::channel(8_usize);
        self.download_inner(
            id,
            ManagedDownloadControl {
                cancellation,
                pause,
                connection_limit,
                resource_demand: None,
                bandwidth: BandwidthControl::default(),
            },
        )
        .await
    }
}

impl<R> ManagedDownloadEngine for MetalinkEngine<R>
where
    R: DownloadRepository + Clone + Send + Sync + 'static,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        self.download_inner(id, control).await
    }

    fn supports_active_pause(&self, _engine: EngineKind) -> bool {
        true
    }
}

// ── Manifest parser ────────────────────────────────────────────────────────────

/// Parses a Metalink v3 or v4 XML manifest and returns the list of files.
///
/// - v4: namespace `urn:ietf:params:xml:ns:metalink`, `<file name="…">` with
///   `<url location="…" priority="…">`.
/// - v3: namespace `http://www.metalinker.org/` or the old v3 namespace, root
///   `<metalink version="3.0">`, files under `<files>/<file name="…">` with
///   mirrors under `<resources>/<url priority="…">` and hashes under
///   `<verification>/<hash type="…">`.
///
/// Unknown or missing elements are tolerated; malformed XML returns an error.
pub fn parse_metalink(xml: &str) -> Result<Vec<MetalinkFile>, DownloadEngineError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| DownloadEngineError::MetalinkError(format!("XML parse error: {e}")))?;

    let root = doc.root_element();
    let ns = root.tag_name().namespace().unwrap_or("");
    let root_local = root.tag_name().name();

    if root_local != "metalink" {
        return Err(DownloadEngineError::MetalinkError(format!(
            "unexpected root element '{root_local}'"
        )));
    }

    let is_v4 = ns == "urn:ietf:params:xml:ns:metalink";
    // v3 can have namespace "http://www.metalinker.org/" or no namespace.
    let is_v3 = !is_v4;

    let mut files = Vec::new();

    if is_v4 {
        // v4: files are direct children of <metalink> with local name "file".
        for node in root.children().filter(|n| n.is_element()) {
            if node.tag_name().name() == "file" {
                let mf = parse_v4_file(node)?;
                files.push(mf);
            }
        }
    } else if is_v3 {
        // v3: files are under <files>/<file> hierarchy.
        files = parse_v3_files(root)?;
    }

    // Sort mirrors by priority within each file.
    for mf in &mut files {
        mf.mirrors.sort_by_key(|m| m.priority);
    }

    Ok(files)
}

fn parse_v4_file(node: roxmltree::Node<'_, '_>) -> Result<MetalinkFile, DownloadEngineError> {
    let name = node
        .attribute("name")
        .ok_or_else(|| {
            DownloadEngineError::MetalinkError("<file> missing 'name' attribute".to_owned())
        })?
        .to_owned();

    let mut size: Option<u64> = None;
    let mut hashes: Vec<(String, String)> = Vec::new();
    let mut chunk_checksums = Vec::new();
    let mut mirrors: Vec<MetalinkMirror> = Vec::new();

    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "size" => {
                if let Some(text) = child.text() {
                    size = text.trim().parse::<u64>().ok();
                }
            }
            "hash" => {
                if let (Some(alg), Some(text)) = (child.attribute("type"), child.text()) {
                    hashes.push((alg.to_ascii_lowercase(), text.trim().to_ascii_lowercase()));
                }
            }
            "pieces" => chunk_checksums.push(parse_chunk_checksum(child)?),
            "url" => {
                if let Some(text) = child.text() {
                    let url = text.trim().to_owned();
                    if !url.is_empty() {
                        let priority = child
                            .attribute("priority")
                            .and_then(|p| p.parse::<i32>().ok())
                            .unwrap_or(999);
                        mirrors.push(MetalinkMirror { url, priority });
                    }
                }
            }
            _ => {}
        }
    }

    validate_chunk_metadata(size, &chunk_checksums)?;
    Ok(MetalinkFile {
        name,
        size,
        hashes,
        chunk_checksums,
        mirrors,
    })
}

fn parse_v3_files(root: roxmltree::Node<'_, '_>) -> Result<Vec<MetalinkFile>, DownloadEngineError> {
    let mut files = Vec::new();

    // Find <files> element (may or may not be namespaced).
    let files_node = root
        .children()
        .filter(|n| n.is_element())
        .find(|n| n.tag_name().name() == "files");

    let file_container = files_node.unwrap_or(root);

    for node in file_container.children().filter(|n| n.is_element()) {
        if node.tag_name().name() != "file" {
            continue;
        }

        let name = node
            .attribute("name")
            .ok_or_else(|| {
                DownloadEngineError::MetalinkError("<file> missing 'name' attribute".to_owned())
            })?
            .to_owned();

        let mut size: Option<u64> = None;
        let mut hashes: Vec<(String, String)> = Vec::new();
        let mut chunk_checksums = Vec::new();
        let mut mirrors: Vec<MetalinkMirror> = Vec::new();

        for child in node.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "size" => {
                    if let Some(text) = child.text() {
                        size = text.trim().parse::<u64>().ok();
                    }
                }
                "verification" => {
                    // v3 hashes: <verification><hash type="sha-256">…</hash></verification>
                    for hash_node in child.children().filter(|n| n.is_element()) {
                        if hash_node.tag_name().name() == "hash"
                            && let (Some(alg), Some(text)) =
                                (hash_node.attribute("type"), hash_node.text())
                        {
                            hashes
                                .push((alg.to_ascii_lowercase(), text.trim().to_ascii_lowercase()));
                        } else if hash_node.tag_name().name() == "pieces" {
                            chunk_checksums.push(parse_chunk_checksum(hash_node)?);
                        }
                    }
                }
                "resources" => {
                    // v3 mirrors: <resources><url priority="1">…</url></resources>
                    for url_node in child.children().filter(|n| n.is_element()) {
                        if url_node.tag_name().name() == "url"
                            && let Some(text) = url_node.text()
                        {
                            let url = text.trim().to_owned();
                            if !url.is_empty() {
                                let priority = url_node
                                    .attribute("priority")
                                    .and_then(|p| p.parse::<i32>().ok())
                                    .unwrap_or(999);
                                mirrors.push(MetalinkMirror { url, priority });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        validate_chunk_metadata(size, &chunk_checksums)?;
        files.push(MetalinkFile {
            name,
            size,
            hashes,
            chunk_checksums,
            mirrors,
        });
    }

    Ok(files)
}

const MAX_METALINK_CHUNK_HASHES: usize = 1_000_000;

fn parse_chunk_checksum(
    node: roxmltree::Node<'_, '_>,
) -> Result<MetalinkChunkChecksum, DownloadEngineError> {
    let algorithm = node
        .attribute("type")
        .ok_or_else(|| DownloadEngineError::MetalinkError("<pieces> missing 'type'".to_owned()))?
        .trim()
        .to_ascii_lowercase();
    let chunk_length = node
        .attribute("length")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|length| *length > 0)
        .ok_or_else(|| {
            DownloadEngineError::MetalinkError("<pieces> has an invalid 'length'".to_owned())
        })?;

    let mut hashes = Vec::new();
    for child in node.children().filter(|child| child.is_element()) {
        if child.tag_name().name() != "hash" {
            continue;
        }
        let hash = child
            .text()
            .map(str::trim)
            .filter(|hash| !hash.is_empty())
            .ok_or_else(|| {
                DownloadEngineError::MetalinkError("<pieces> contains an empty hash".to_owned())
            })?
            .to_ascii_lowercase();
        validate_hash_value(&algorithm, &hash)?;
        hashes.push(hash);
        if hashes.len() > MAX_METALINK_CHUNK_HASHES {
            return Err(DownloadEngineError::MetalinkError(
                "<pieces> contains too many hashes".to_owned(),
            ));
        }
    }

    if hashes.is_empty() {
        return Err(DownloadEngineError::MetalinkError(
            "<pieces> contains no hashes".to_owned(),
        ));
    }

    Ok(MetalinkChunkChecksum {
        algorithm,
        chunk_length,
        hashes,
    })
}

fn validate_hash_value(algorithm: &str, hash: &str) -> Result<(), DownloadEngineError> {
    let digest_length = compute_hash(algorithm, &[])?.len();
    if hash.len() != digest_length || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DownloadEngineError::MetalinkError(format!(
            "invalid {algorithm} checksum value"
        )));
    }
    Ok(())
}

fn validate_chunk_metadata(
    size: Option<u64>,
    checksums: &[MetalinkChunkChecksum],
) -> Result<(), DownloadEngineError> {
    let Some(first) = checksums.first() else {
        return Ok(());
    };
    if checksums
        .iter()
        .any(|checksum| checksum.chunk_length != first.chunk_length)
    {
        return Err(DownloadEngineError::MetalinkError(
            "Metalink chunk checksum algorithms use conflicting chunk lengths".to_owned(),
        ));
    }
    if let Some(size) = size {
        let expected_count = if size == 0 {
            0
        } else {
            (size - 1) / first.chunk_length + 1
        };
        if expected_count > MAX_METALINK_CHUNK_HASHES as u64
            || checksums
                .iter()
                .any(|checksum| checksum.hashes.len() as u64 != expected_count)
        {
            return Err(DownloadEngineError::MetalinkError(
                "chunk hashes do not match the manifest file boundaries".to_owned(),
            ));
        }
    }
    Ok(())
}

// ── Path traversal guard ───────────────────────────────────────────────────────

/// Rejects filenames that could escape the destination directory.
///
/// Rejected patterns:
/// - Any component equal to `..`
/// - Absolute paths (start with `/` or `\`)
/// - Windows drive prefixes (e.g. `C:\`, `\\?\`)
fn check_filename_safety(name: &str) -> Result<(), DownloadEngineError> {
    // Reject absolute paths.
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(DownloadEngineError::MetalinkError(format!(
            "filename '{name}' is an absolute path"
        )));
    }
    // Reject Windows drive paths (e.g. C:\ or \\?\).
    if name.len() >= 2 {
        let bytes = name.as_bytes();
        if bytes[1] == b':' {
            return Err(DownloadEngineError::MetalinkError(format!(
                "filename '{name}' contains a Windows drive prefix"
            )));
        }
    }
    if name.starts_with("\\\\") {
        return Err(DownloadEngineError::MetalinkError(format!(
            "filename '{name}' is a UNC path"
        )));
    }
    // Reject any `..` path component (split on both forward and backslashes).
    for component in name.split(['/', '\\']) {
        if component == ".." {
            return Err(DownloadEngineError::MetalinkError(format!(
                "filename '{name}' contains a path traversal sequence (..)"
            )));
        }
    }
    Ok(())
}

/// Returns a safe filename for use as a staging file name (replaces path separators).
fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', ':'], "_")
}

// ── Checksum verification ──────────────────────────────────────────────────────

/// Verifies all hashes listed in `hashes` against the content of `path`.
///
/// On mismatch, returns [`DownloadEngineError::MetalinkChecksumMismatch`] for
/// the first failing algorithm. If no hashes are listed, returns `Ok(())`.
async fn verify_checksums(
    path: &Path,
    hashes: &[(String, String)],
) -> Result<(), DownloadEngineError> {
    if hashes.is_empty() {
        return Ok(());
    }

    // Read the file once and compute all requested hashes.
    let bytes = fs::read(path).await.map_err(DownloadEngineError::Io)?;

    for (algorithm, expected_hex) in hashes {
        let actual_hex = compute_hash(algorithm, &bytes)?;
        if actual_hex != *expected_hex {
            return Err(DownloadEngineError::MetalinkChecksumMismatch {
                algorithm: algorithm.clone(),
                expected: expected_hex.clone(),
                actual: actual_hex,
            });
        }
    }

    Ok(())
}

async fn consume_probe(
    response: &mut reqwest::Response,
    control: &mut ManagedDownloadControl,
) -> Result<(), DownloadEngineError> {
    let mut received = 0_usize;
    loop {
        let chunk = tokio::select! {
            changed = control.cancellation.changed() => {
                if changed.is_err() || *control.cancellation.borrow_and_update() {
                    return Err(DownloadEngineError::Cancelled);
                }
                continue;
            }
            changed = control.pause.changed() => {
                if changed.is_err() || *control.pause.borrow() {
                    return Err(DownloadEngineError::Paused);
                }
                continue;
            }
            chunk = response.chunk() => chunk.map_err(DownloadEngineError::Request)?,
        };
        match chunk {
            Some(bytes) => {
                received = received
                    .checked_add(bytes.len())
                    .ok_or(DownloadEngineError::ByteCountOverflow)?;
                if received > 1 {
                    return Err(DownloadEngineError::RangeMismatch);
                }
            }
            None => break,
        }
    }
    if received != 1 {
        return Err(DownloadEngineError::RangeMismatch);
    }
    Ok(())
}

async fn receive_and_verify_piece(
    response: &mut reqwest::Response,
    piece_path: &Path,
    index: usize,
    expected_length: u64,
    checksums: &[MetalinkChunkChecksum],
    control: &mut ManagedDownloadControl,
) -> Result<(), DownloadEngineError> {
    let attempt_path = piece_path.with_extension(format!("attempt-{}", DownloadId::new()));
    let result = async {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&attempt_path)
            .await
            .map_err(DownloadEngineError::Io)?;
        let mut hashers = checksums
            .iter()
            .map(|checksum| new_chunk_hasher(&checksum.algorithm))
            .collect::<Result<Vec<_>, _>>()?;
        let mut received = 0_u64;
        loop {
            let chunk = tokio::select! {
                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                    continue;
                }
                changed = control.pause.changed() => {
                    if changed.is_err() || *control.pause.borrow() {
                        return Err(DownloadEngineError::Paused);
                    }
                    continue;
                }
                chunk = response.chunk() => chunk.map_err(DownloadEngineError::Request)?,
            };
            let Some(bytes) = chunk else {
                break;
            };
            let chunk_size =
                u64::try_from(bytes.len()).map_err(|_| DownloadEngineError::ByteCountOverflow)?;
            received = received
                .checked_add(chunk_size)
                .ok_or(DownloadEngineError::ByteCountOverflow)?;
            if received > expected_length {
                return Err(DownloadEngineError::RangeMismatch);
            }
            for hasher in &mut hashers {
                hasher.update(&bytes);
            }
            output
                .write_all(&bytes)
                .await
                .map_err(DownloadEngineError::Io)?;
        }
        if received != expected_length {
            return Err(DownloadEngineError::RangeMismatch);
        }
        output.flush().await.map_err(DownloadEngineError::Io)?;
        drop(output);

        for (checksum, hasher) in checksums.iter().zip(hashers) {
            let actual = hex::encode(hasher.finalize());
            let expected = &checksum.hashes[index];
            if actual != *expected {
                return Err(DownloadEngineError::MetalinkChunkChecksumMismatch {
                    algorithm: checksum.algorithm.clone(),
                    chunk_index: index,
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        fs::rename(&attempt_path, piece_path)
            .await
            .map_err(DownloadEngineError::Io)?;
        Ok(())
    }
    .await;

    if result.is_err() {
        match fs::remove_file(&attempt_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }
    }
    result
}

async fn validate_staged_chunks(
    chunk_dir: &Path,
    total_size: Option<u64>,
    checksums: &[MetalinkChunkChecksum],
) -> Result<Vec<usize>, DownloadEngineError> {
    let Some(total_size) = total_size else {
        return Ok((0..checksums[0].hashes.len()).collect());
    };
    validate_chunk_metadata(Some(total_size), checksums)?;
    let first = &checksums[0];
    let mut missing = Vec::new();
    for index in 0..first.hashes.len() {
        let piece_path = chunk_dir.join(format!("{index:08}.piece"));
        let metadata = match fs::symlink_metadata(&piece_path).await {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => {
                fs::remove_file(&piece_path)
                    .await
                    .map_err(DownloadEngineError::Io)?;
                missing.push(index);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(index);
                continue;
            }
            Err(error) => return Err(DownloadEngineError::Io(error)),
        };
        let offset = (index as u64)
            .checked_mul(first.chunk_length)
            .ok_or_else(|| {
                DownloadEngineError::MetalinkError("chunk offset overflow".to_owned())
            })?;
        let expected_length = (total_size - offset).min(first.chunk_length);
        let valid = if metadata.len() == expected_length {
            verify_chunk_file(&piece_path, index, expected_length, checksums).await?
        } else {
            false
        };
        if !valid {
            fs::remove_file(&piece_path)
                .await
                .map_err(DownloadEngineError::Io)?;
            missing.push(index);
        }
    }
    Ok(missing)
}

async fn cleanup_chunk_attempts(chunk_dir: &Path) -> Result<(), DownloadEngineError> {
    let mut entries = fs::read_dir(chunk_dir)
        .await
        .map_err(DownloadEngineError::Io)?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(DownloadEngineError::Io)?
    {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((index, suffix)) = name.split_once(".attempt-") else {
            continue;
        };
        if index.len() == 8 && index.bytes().all(|byte| byte.is_ascii_digit()) && !suffix.is_empty()
        {
            fs::remove_file(entry.path())
                .await
                .map_err(DownloadEngineError::Io)?;
        }
    }
    Ok(())
}

async fn verify_chunk_file(
    path: &Path,
    index: usize,
    expected_length: u64,
    checksums: &[MetalinkChunkChecksum],
) -> Result<bool, DownloadEngineError> {
    let mut file = fs::File::open(path)
        .await
        .map_err(DownloadEngineError::Io)?;
    let mut hashers = checksums
        .iter()
        .map(|checksum| new_chunk_hasher(&checksum.algorithm))
        .collect::<Result<Vec<_>, _>>()?;
    let mut remaining = expected_length;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = file
            .read(&mut buffer[..limit])
            .await
            .map_err(DownloadEngineError::Io)?;
        if read == 0 {
            return Ok(false);
        }
        for hasher in &mut hashers {
            hasher.update(&buffer[..read]);
        }
        remaining -= read as u64;
    }
    for (checksum, hasher) in checksums.iter().zip(hashers) {
        if hex::encode(hasher.finalize()) != checksum.hashes[index] {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn stage_verified_chunks(
    path: &Path,
    expected_size: Option<u64>,
    checksums: &[MetalinkChunkChecksum],
    chunk_dir: &Path,
) -> Result<(), DownloadEngineError> {
    let file_size = fs::metadata(path)
        .await
        .map_err(DownloadEngineError::Io)?
        .len();
    if let Some(expected_size) = expected_size.filter(|expected| *expected != file_size) {
        return Err(DownloadEngineError::MetalinkError(format!(
            "downloaded file size {file_size} does not match manifest size {expected_size}"
        )));
    }

    let first = checksums.first().ok_or_else(|| {
        DownloadEngineError::MetalinkError("no chunk checksum metadata".to_owned())
    })?;
    if checksums
        .iter()
        .any(|checksum| checksum.chunk_length != first.chunk_length)
    {
        return Err(DownloadEngineError::MetalinkError(
            "Metalink chunk checksum algorithms use conflicting chunk lengths".to_owned(),
        ));
    }

    let chunk_count = if file_size == 0 {
        0
    } else {
        (file_size - 1) / first.chunk_length + 1
    };
    if chunk_count > MAX_METALINK_CHUNK_HASHES as u64
        || checksums
            .iter()
            .any(|checksum| usize::try_from(chunk_count).ok() != Some(checksum.hashes.len()))
    {
        return Err(DownloadEngineError::MetalinkError(
            "chunk hashes do not match the downloaded file boundaries".to_owned(),
        ));
    }

    let mut file = fs::File::open(path)
        .await
        .map_err(DownloadEngineError::Io)?;
    let missing = validate_staged_chunks(chunk_dir, Some(file_size), checksums).await?;
    for index in missing {
        let piece_path = chunk_dir.join(format!("{index:08}.piece"));

        let offset = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(first.chunk_length))
            .ok_or_else(|| {
                DownloadEngineError::MetalinkError("chunk checksum offset overflow".to_owned())
            })?;
        let chunk_size = (file_size - offset).min(first.chunk_length);
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(DownloadEngineError::Io)?;
        let mut hashers = checksums
            .iter()
            .map(|checksum| new_chunk_hasher(&checksum.algorithm))
            .collect::<Result<Vec<_>, _>>()?;
        let mut remaining = chunk_size;
        let mut buffer = [0_u8; 16 * 1024];
        while remaining > 0 {
            let read_limit =
                usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
            let read = file
                .read(&mut buffer[..read_limit])
                .await
                .map_err(DownloadEngineError::Io)?;
            if read == 0 {
                return Err(DownloadEngineError::MetalinkError(
                    "downloaded file ended inside a checksummed chunk".to_owned(),
                ));
            }
            for hasher in &mut hashers {
                hasher.update(&buffer[..read]);
            }
            remaining -= read as u64;
        }

        for (checksum, hasher) in checksums.iter().zip(hashers) {
            let actual = hex::encode(hasher.finalize());
            let expected = &checksum.hashes[index];
            if actual != *expected {
                return Err(DownloadEngineError::MetalinkChunkChecksumMismatch {
                    algorithm: checksum.algorithm.clone(),
                    chunk_index: index,
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(DownloadEngineError::Io)?;
        let attempt_path = piece_path.with_extension(format!("attempt-{}", DownloadId::new()));
        let result = async {
            let mut attempt = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&attempt_path)
                .await
                .map_err(DownloadEngineError::Io)?;
            let mut remaining = chunk_size;
            while remaining > 0 {
                let read_limit =
                    usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
                let read = file
                    .read(&mut buffer[..read_limit])
                    .await
                    .map_err(DownloadEngineError::Io)?;
                if read == 0 {
                    return Err(DownloadEngineError::MetalinkError(
                        "downloaded file ended inside a checksummed chunk".to_owned(),
                    ));
                }
                attempt
                    .write_all(&buffer[..read])
                    .await
                    .map_err(DownloadEngineError::Io)?;
                remaining -= read as u64;
            }
            attempt.flush().await.map_err(DownloadEngineError::Io)?;
            drop(attempt);
            fs::rename(&attempt_path, &piece_path)
                .await
                .map_err(DownloadEngineError::Io)?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            match fs::remove_file(&attempt_path).await {
                Ok(()) => {}
                Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {}
                Err(remove_error) => return Err(DownloadEngineError::Io(remove_error)),
            }
            return Err(error);
        }
    }

    Ok(())
}

async fn assemble_verified_chunks(
    chunk_dir: &Path,
    output_path: &Path,
    checksum: &MetalinkChunkChecksum,
) -> Result<PathBuf, DownloadEngineError> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .await
        .map_err(DownloadEngineError::Io)?;
    let result = async {
        for index in 0..checksum.hashes.len() {
            let mut piece = fs::File::open(chunk_dir.join(format!("{index:08}.piece")))
                .await
                .map_err(DownloadEngineError::Io)?;
            tokio::io::copy(&mut piece, &mut output)
                .await
                .map_err(DownloadEngineError::Io)?;
        }
        output.flush().await.map_err(DownloadEngineError::Io)?;
        Ok(())
    }
    .await;
    drop(output);
    if let Err(error) = result {
        let _ = fs::remove_file(output_path).await;
        return Err(error);
    }
    Ok(output_path.to_path_buf())
}

async fn verify_chunk_checksums(
    path: &Path,
    expected_size: Option<u64>,
    checksums: &[MetalinkChunkChecksum],
) -> Result<(), DownloadEngineError> {
    if checksums.is_empty() {
        return Ok(());
    }

    let file_size = fs::metadata(path)
        .await
        .map_err(DownloadEngineError::Io)?
        .len();
    if let Some(expected_size) = expected_size.filter(|expected| *expected != file_size) {
        return Err(DownloadEngineError::MetalinkError(format!(
            "downloaded file size {file_size} does not match manifest size {expected_size}"
        )));
    }
    validate_chunk_metadata(Some(file_size), checksums)?;

    let mut file = fs::File::open(path)
        .await
        .map_err(DownloadEngineError::Io)?;
    for checksum in checksums {
        for (index, expected) in checksum.hashes.iter().enumerate() {
            let offset = (index as u64)
                .checked_mul(checksum.chunk_length)
                .ok_or_else(|| {
                    DownloadEngineError::MetalinkError("chunk checksum offset overflow".to_owned())
                })?;
            let chunk_size = (file_size - offset).min(checksum.chunk_length);
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(DownloadEngineError::Io)?;
            let mut hasher = new_chunk_hasher(&checksum.algorithm)?;
            let mut remaining = chunk_size;
            let mut buffer = [0_u8; 16 * 1024];
            while remaining > 0 {
                let read_limit =
                    usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
                let read = file
                    .read(&mut buffer[..read_limit])
                    .await
                    .map_err(DownloadEngineError::Io)?;
                if read == 0 {
                    return Err(DownloadEngineError::MetalinkError(
                        "downloaded file ended inside a checksummed chunk".to_owned(),
                    ));
                }
                hasher.update(&buffer[..read]);
                remaining -= read as u64;
            }
            let actual = hex::encode(hasher.finalize());
            if actual != *expected {
                return Err(DownloadEngineError::MetalinkChunkChecksumMismatch {
                    algorithm: checksum.algorithm.clone(),
                    chunk_index: index,
                    expected: expected.clone(),
                    actual,
                });
            }
        }
    }
    Ok(())
}

fn new_chunk_hasher(
    algorithm: &str,
) -> Result<Box<dyn digest::DynDigest + Send>, DownloadEngineError> {
    match algorithm {
        "md5" => Ok(Box::new(Md5::new())),
        "sha-1" | "sha1" => Ok(Box::new(Sha1::new())),
        "sha-256" | "sha256" => Ok(Box::new(Sha256::new())),
        "sha-512" | "sha512" => Ok(Box::new(Sha512::new())),
        other => Err(DownloadEngineError::MetalinkError(format!(
            "unsupported checksum algorithm '{other}'"
        ))),
    }
}

/// Computes the named hash of `data`, returning lowercase hex.
///
/// Supported: `md5`, `sha-1`, `sha1`, `sha-256`, `sha-512`.
fn compute_hash(algorithm: &str, data: &[u8]) -> Result<String, DownloadEngineError> {
    match algorithm {
        "md5" => Ok(hex::encode(Md5::digest(data))),
        "sha-1" | "sha1" => Ok(hex::encode(Sha1::digest(data))),
        "sha-256" | "sha256" => Ok(hex::encode(Sha256::digest(data))),
        "sha-512" | "sha512" => Ok(hex::encode(Sha512::digest(data))),
        other => Err(DownloadEngineError::MetalinkError(format!(
            "unsupported checksum algorithm '{other}'"
        ))),
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn next_ts(current: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    Timestamp::now()
        .map_err(DownloadEngineError::Timestamp)
        .map(|now| now.max(current))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use digest::Digest;
    use fetchx_models::{
        DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use fetchx_storage::{Database, DownloadRepository};
    use tokio::sync::watch;

    use super::{MetalinkEngine, check_filename_safety, compute_hash, parse_metalink};
    use crate::{
        BandwidthControl, DirectHttpEngine, DownloadEngine, DownloadEngineError,
        ManagedDownloadControl, ManagedDownloadEngine,
    };

    /// Run an async test body in a thread with 8 MiB stack to avoid stack
    /// overflows caused by deeply nested futures in debug builds.
    #[allow(dead_code)]
    fn run_large_stack<F, T>(f: F) -> T
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(f)
            .expect("failed to spawn large-stack thread")
            .join()
            .expect("large-stack thread panicked")
    }

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
    }

    fn read_fixture(name: &str) -> String {
        std::fs::read_to_string(fixtures_dir().join(name))
            .unwrap_or_else(|_| panic!("fixture {name} not found"))
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn metalink_parse_valid_v3() {
        let xml = read_fixture("valid_v3.metalink");
        let files = parse_metalink(&xml).expect("valid v3 should parse");
        assert_eq!(files.len(), 1, "expected 1 file");
        let f = &files[0];
        assert_eq!(f.name, "test-file.bin");
        assert_eq!(f.size, Some(1024));
        assert_eq!(f.mirrors.len(), 2, "expected 2 mirrors");
        // Sorted by priority: priority 1 first.
        assert_eq!(f.mirrors[0].priority, 1);
        assert_eq!(f.mirrors[1].priority, 2);
        // Should have sha-256 and md5 hashes.
        assert!(
            f.hashes.iter().any(|(a, _)| a == "sha-256"),
            "expected sha-256 hash"
        );
        assert!(
            f.hashes.iter().any(|(a, _)| a == "md5"),
            "expected md5 hash"
        );
    }

    #[test]
    fn metalink_parse_valid_v4() {
        let xml = read_fixture("valid_v4.meta4");
        let files = parse_metalink(&xml).expect("valid v4 should parse");
        assert_eq!(files.len(), 2, "expected 2 files");
        let f = &files[0];
        assert_eq!(f.name, "test-v4-file.bin");
        assert_eq!(f.size, Some(512));
        assert_eq!(f.mirrors.len(), 2);
        assert!(f.hashes.iter().any(|(a, _)| a == "sha-256"));
        assert!(f.hashes.iter().any(|(a, _)| a == "sha-1"));
        let f2 = &files[1];
        assert_eq!(f2.name, "second-file.bin");
        assert_eq!(f2.mirrors.len(), 1);
    }

    #[test]
    fn metalink_parse_chunk_hashes_in_v3_and_v4() {
        let abc = compute_hash("sha-256", b"abc").unwrap();
        let def = compute_hash("sha-256", b"def").unwrap();
        let v4 = format!(
            r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="chunks.bin">
<pieces type="sha-256" length="3"><hash>{abc}</hash><hash>{def}</hash></pieces>
</file></metalink>"#
        );
        let parsed = parse_metalink(&v4).unwrap();
        assert_eq!(parsed[0].chunk_checksums.len(), 1);
        assert_eq!(parsed[0].chunk_checksums[0].chunk_length, 3);
        assert_eq!(
            parsed[0].chunk_checksums[0].hashes,
            [abc.clone(), def.clone()]
        );

        let v3 = format!(
            r#"<metalink xmlns="http://www.metalinker.org/"><files><file name="chunks.bin">
<verification><pieces type="sha-256" length="3"><hash>{abc}</hash><hash>{def}</hash></pieces></verification>
</file></files></metalink>"#
        );
        let parsed = parse_metalink(&v3).unwrap();
        assert_eq!(parsed[0].chunk_checksums.len(), 1);
        assert_eq!(parsed[0].chunk_checksums[0].hashes, [abc, def]);
    }

    #[test]
    fn metalink_rejects_malformed_chunk_hash_metadata() {
        let invalid = [
            r#"<pieces type="sha-256" length="0"><hash>00</hash></pieces>"#,
            r#"<pieces type="sha-256" length="2"><hash>not-hex</hash></pieces>"#,
            r#"<pieces type="blake3" length="2"><hash>00</hash></pieces>"#,
            r#"<pieces type="sha-256" length="2"></pieces>"#,
        ];
        for pieces in invalid {
            let xml = format!(
                r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="x">{pieces}</file></metalink>"#
            );
            assert!(matches!(
                parse_metalink(&xml),
                Err(DownloadEngineError::MetalinkError(_))
            ));
        }

        let boundary_mismatch = format!(
            r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="x"><size>5</size>
<pieces type="sha-256" length="3"><hash>{}</hash></pieces></file></metalink>"#,
            compute_hash("sha-256", b"abc").unwrap()
        );
        assert!(matches!(
            parse_metalink(&boundary_mismatch),
            Err(DownloadEngineError::MetalinkError(_))
        ));
    }

    #[test]
    fn metalink_malformed_xml() {
        let xml = read_fixture("malformed.xml");
        let result = parse_metalink(&xml);
        assert!(
            matches!(result, Err(DownloadEngineError::MetalinkError(_))),
            "malformed XML should return MetalinkError, got: {result:?}"
        );
    }

    #[test]
    fn metalink_missing_mirrors() {
        let xml = read_fixture("missing_mirrors.metalink");
        let files = parse_metalink(&xml).expect("parses ok");
        assert_eq!(files.len(), 1);
        assert!(files[0].mirrors.is_empty(), "expected no mirrors");
        // The engine itself enforces no-mirrors at runtime; the parser is lenient.
    }

    // ── Path traversal ────────────────────────────────────────────────────────

    #[test]
    fn metalink_path_traversal_rejected_by_check() {
        let dangerous = [
            "../../../etc/passwd",
            "/etc/passwd",
            "C:\\Windows\\System32",
            "..\\..\\secret",
            "subdir/../../secret",
        ];
        for name in dangerous {
            assert!(
                check_filename_safety(name).is_err(),
                "should reject '{name}'"
            );
        }
    }

    #[test]
    fn metalink_safe_filenames_accepted() {
        let safe = ["file.bin", "sub/file.bin", "a/b/c.txt"];
        for name in safe {
            assert!(
                check_filename_safety(name).is_ok(),
                "should accept '{name}'"
            );
        }
    }

    // ── Checksum computation ──────────────────────────────────────────────────

    #[test]
    fn compute_hash_md5() {
        // MD5 of empty string.
        let h = compute_hash("md5", b"").unwrap();
        assert_eq!(h, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn compute_hash_sha1() {
        let h = compute_hash("sha-1", b"").unwrap();
        assert_eq!(h, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn compute_hash_sha256() {
        let h = compute_hash("sha-256", b"abc").unwrap();
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn compute_hash_sha512() {
        let h = compute_hash("sha-512", b"").unwrap();
        assert_eq!(
            h,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        // Also verify sha512 alias.
        let h2 = compute_hash("sha512", b"").unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn compute_hash_unsupported_algo() {
        assert!(matches!(
            compute_hash("blake3", b"data"),
            Err(DownloadEngineError::MetalinkError(_))
        ));
    }

    #[tokio::test]
    async fn metalink_chunk_checksums_verify_all_supported_algorithms() -> Result<(), Box<dyn Error>>
    {
        let bytes = b"abcdefg";
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), bytes)?;
        for algorithm in ["md5", "sha-1", "sha-256", "sha-512"] {
            let checksums = super::MetalinkChunkChecksum {
                algorithm: algorithm.to_owned(),
                chunk_length: 3,
                hashes: [b"abc".as_slice(), b"def", b"g"]
                    .into_iter()
                    .map(|chunk| compute_hash(algorithm, chunk))
                    .collect::<Result<Vec<_>, _>>()?,
            };
            super::verify_chunk_checksums(file.path(), Some(bytes.len() as u64), &[checksums])
                .await?;
        }
        Ok(())
    }

    // ── Engine integration tests ──────────────────────────────────────────────

    async fn make_engine(
        db: &Database,
    ) -> (
        MetalinkEngine<fetchx_storage::SqliteDownloadRepository>,
        DirectHttpEngine<fetchx_storage::SqliteDownloadRepository>,
    ) {
        let repo = db.downloads();
        let http = DirectHttpEngine::new(repo.clone()).expect("http engine");
        let http_arc = std::sync::Arc::new(
            DirectHttpEngine::new(repo.clone()).expect("http engine for metalink"),
        );
        let metalink = MetalinkEngine::new(repo, http_arc);
        (metalink, http)
    }

    fn make_metalink_item(
        manifest_path: &std::path::Path,
        destination: &std::path::Path,
    ) -> Result<DownloadItem, Box<dyn Error>> {
        let url = format!("file://{}", manifest_path.display());
        Ok(DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(url, EngineKind::Metalink)?,
            FileName::new("manifest")?,
            destination.to_path_buf(),
            Timestamp::from_unix_millis(1),
        ))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_engine_mismatch_guard() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file.bin", EngineKind::DirectHttp)?,
            FileName::new("file.bin")?,
            PathBuf::from("/tmp/file.bin"),
            Timestamp::from_unix_millis(1),
        );
        let id = item.id();
        db.downloads().create(&item).await?;
        let (_, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(matches!(result, Err(DownloadEngineError::EngineMismatch)));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_engine_not_found() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let (_, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, DownloadId::new(), rx).await;
        assert!(matches!(result, Err(DownloadEngineError::NotFound(_))));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_path_traversal_in_manifest_rejected() -> Result<(), Box<dyn Error>> {
        // Build a manifest with a dangerous filename and point it at a local fixture.
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        // Write an in-memory manifest with traversal filename.
        let manifest_content = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="../../../etc/passwd">
    <size>5</size>
    <hash type="sha-256">2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824</hash>
    <url priority="1">http://127.0.0.1:1/nonexistent</url>
  </file>
</metalink>"#;
        let manifest_path = dir.path().join("traversal.meta4");
        std::fs::write(&manifest_path, manifest_content)?;

        let dest = dir.path().join("output");
        let item = make_metalink_item(&manifest_path, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(
            matches!(result, Err(DownloadEngineError::MetalinkError(_))),
            "expected MetalinkError for path traversal, got: {result:?}"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_missing_mirrors_returns_error() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;
        let fixture = fixtures_dir().join("missing_mirrors.metalink");
        let dest = dir.path().join("output");
        let item = make_metalink_item(&fixture, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(
            matches!(result, Err(DownloadEngineError::MetalinkError(_))),
            "expected MetalinkError for missing mirrors, got: {result:?}"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_checksum_success() -> Result<(), Box<dyn Error>> {
        use crate::tests::start_scripted_server;

        let body = b"hello".to_vec();
        // sha-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let expected_hash = hex::encode(sha2::Sha256::digest(&body));

        // Create a local HTTP server serving the body.
        let responses = vec![crate::tests::scripted_response(200, &[], &body)];
        let (url, server) = start_scripted_server(responses).await?;

        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        let manifest = format!(
            r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="hello.bin">
    <size>5</size>
    <hash type="sha-256">{expected_hash}</hash>
    <url priority="1">{url}</url>
  </file>
</metalink>"#
        );
        let manifest_path = dir.path().join("test.meta4");
        std::fs::write(&manifest_path, &manifest)?;

        let dest = dir.path().join("hello.bin");
        let item = make_metalink_item(&manifest_path, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(result.is_ok(), "expected success, got: {result:?}");
        let content = std::fs::read(&dest)?;
        assert_eq!(content, body);
        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_checksum_mismatch() -> Result<(), Box<dyn Error>> {
        use crate::tests::start_scripted_server;

        let body = b"hello".to_vec();
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let chunk_hash = compute_hash("sha-256", &body)?;

        let responses = vec![
            crate::tests::scripted_response(206, &[("Content-Range", "bytes 0-0/5")], &body[..1]),
            crate::tests::scripted_response(206, &[("Content-Range", "bytes 0-4/5")], &body),
        ];
        let (url, server) = start_scripted_server(responses).await?;

        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        let manifest = format!(
            r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="bad.bin">
    <size>5</size>
    <hash type="sha-256">{wrong_hash}</hash>
    <pieces type="sha-256" length="5"><hash>{chunk_hash}</hash></pieces>
    <url priority="1">{url}</url>
  </file>
</metalink>"#
        );
        let manifest_path = dir.path().join("bad.meta4");
        std::fs::write(&manifest_path, &manifest)?;

        let dest = dir.path().join("bad.bin");
        let item = make_metalink_item(&manifest_path, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(
            matches!(
                result,
                Err(DownloadEngineError::MetalinkChecksumMismatch { .. })
                    | Err(DownloadEngineError::MetalinkError(_))
            ),
            "expected MetalinkChecksumMismatch or MetalinkError, got: {result:?}"
        );
        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_corrupt_chunk_retries_next_mirror() -> Result<(), Box<dyn Error>> {
        use crate::tests::{scripted_response, start_scripted_server};

        let expected = b"abcdefgh";
        let corrupt = b"abcdEFgh";
        let chunk_hashes = expected
            .chunks(4)
            .map(|chunk| compute_hash("sha-256", chunk))
            .collect::<Result<Vec<_>, _>>()?;
        let (bad_url, bad_server) = start_scripted_server(vec![
            scripted_response(206, &[("Content-Range", "bytes 0-0/8")], &expected[..1]),
            scripted_response(206, &[("Content-Range", "bytes 0-3/8")], &expected[..4]),
            scripted_response(206, &[("Content-Range", "bytes 4-7/8")], &corrupt[4..]),
        ])
        .await?;
        let (good_url, good_server) = start_scripted_server(vec![
            scripted_response(206, &[("Content-Range", "bytes 0-0/8")], &expected[..1]),
            scripted_response(206, &[("Content-Range", "bytes 4-7/8")], &expected[4..]),
        ])
        .await?;

        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;
        let manifest = format!(
            r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="chunked.bin">
<size>8</size><pieces type="sha-256" length="4">
<hash>{}</hash><hash>{}</hash></pieces>
<url priority="1">{bad_url}</url><url priority="2">{good_url}</url>
</file></metalink>"#,
            chunk_hashes[0], chunk_hashes[1]
        );
        let manifest_path = dir.path().join("chunks.meta4");
        std::fs::write(&manifest_path, manifest)?;
        let destination = dir.path().join("chunked.bin");
        let item = make_metalink_item(&manifest_path, &destination)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, cancellation) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, cancellation).await;
        assert!(result.is_ok(), "expected mirror retry, got {result:?}");
        assert_eq!(std::fs::read(destination)?, expected);
        let bad_requests = bad_server.await?;
        let good_requests = good_server.await?;
        assert!(
            bad_requests[0]
                .to_ascii_lowercase()
                .contains("range: bytes=0-0")
        );
        assert!(
            bad_requests[1]
                .to_ascii_lowercase()
                .contains("range: bytes=0-3")
        );
        assert!(
            bad_requests[2]
                .to_ascii_lowercase()
                .contains("range: bytes=4-7")
        );
        assert!(
            good_requests[0]
                .to_ascii_lowercase()
                .contains("range: bytes=0-0")
        );
        assert!(
            good_requests[1]
                .to_ascii_lowercase()
                .contains("range: bytes=4-7")
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_invalid_ranges_are_rejected_before_next_mirror() -> Result<(), Box<dyn Error>>
    {
        use crate::tests::{scripted_response, start_scripted_server};

        let body = b"good";
        let hash = compute_hash("sha-256", body)?;
        for invalid_response in 0..5 {
            let bad_responses = match invalid_response {
                0 => vec![scripted_response(200, &[], body)],
                1 => vec![scripted_response(
                    206,
                    &[("Content-Range", "bytes 0-0/5")],
                    &body[..1],
                )],
                2 => vec![
                    scripted_response(206, &[("Content-Range", "bytes 0-0/4")], &body[..1]),
                    scripted_response(206, &[("Content-Range", "bytes 1-3/4")], &body[1..]),
                ],
                3 => vec![
                    scripted_response(206, &[("Content-Range", "bytes 0-0/4")], &body[..1]),
                    scripted_response(206, &[("Content-Range", "bytes 0-3/4")], &body[..2]),
                ],
                _ => vec![
                    scripted_response(206, &[("Content-Range", "bytes 0-0/4")], &body[..1]),
                    scripted_response(206, &[("Content-Range", "bytes 0-3/4")], b"good!"),
                ],
            };
            let (bad_url, bad_server) = start_scripted_server(bad_responses).await?;
            let (good_url, good_server) = start_scripted_server(vec![
                scripted_response(206, &[("Content-Range", "bytes 0-0/4")], &body[..1]),
                scripted_response(206, &[("Content-Range", "bytes 0-3/4")], body),
            ])
            .await?;
            let db = Database::connect("sqlite::memory:").await?;
            let (engine, _) = make_engine(&db).await;
            let dir = tempfile::tempdir()?;
            let manifest = format!(
                r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="range.bin">
<size>4</size><pieces type="sha-256" length="4"><hash>{hash}</hash></pieces>
<url priority="1">{bad_url}</url><url priority="2">{good_url}</url></file></metalink>"#
            );
            let manifest_path = dir.path().join("ranges.meta4");
            std::fs::write(&manifest_path, manifest)?;
            let destination = dir.path().join("range.bin");
            let item = make_metalink_item(&manifest_path, &destination)?;
            let id = item.id();
            db.downloads().create(&item).await?;

            let (_cancel_tx, cancellation) = watch::channel(false);
            let result = DownloadEngine::download(&engine, id, cancellation).await;
            assert!(
                result.is_ok(),
                "invalid range case {invalid_response} should fail over: {result:?}"
            );
            assert_eq!(std::fs::read(destination)?, body);
            let bad_requests = bad_server.await?;
            let good_requests = good_server.await?;
            assert!(
                bad_requests[0]
                    .to_ascii_lowercase()
                    .contains("range: bytes=0-0")
            );
            assert!(
                good_requests
                    .iter()
                    .all(|request| request.to_ascii_lowercase().contains("range:")),
                "good mirror should only receive range requests"
            );
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_ignored_range_uses_verified_whole_file_fallback() -> Result<(), Box<dyn Error>>
    {
        use crate::tests::{scripted_response, start_scripted_server};

        let body = b"fallback";
        let hash = compute_hash("sha-256", body)?;
        let chunk_hash = compute_hash("sha-256", body)?;
        let (url, server) = start_scripted_server(vec![
            scripted_response(200, &[], body),
            scripted_response(200, &[], body),
        ])
        .await?;
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;
        let manifest = format!(
            r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="fallback.bin">
<size>8</size><hash type="sha-256">{hash}</hash>
<pieces type="sha-256" length="8"><hash>{chunk_hash}</hash></pieces>
<url priority="1">{url}</url></file></metalink>"#
        );
        let manifest_path = dir.path().join("fallback.meta4");
        std::fs::write(&manifest_path, manifest)?;
        let destination = dir.path().join("fallback.bin");
        let item = make_metalink_item(&manifest_path, &destination)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, cancellation) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, cancellation).await;
        assert!(
            result.is_ok(),
            "expected safe whole-file fallback: {result:?}"
        );
        assert_eq!(std::fs::read(destination)?, body);
        let requests = server.await?;
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("range: bytes=0-0")
        );
        assert!(
            !requests[1].to_ascii_lowercase().contains("range:"),
            "fallback must be a whole-file request, not the ignored range response"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_chunk_mismatch_preserves_existing_destination() -> Result<(), Box<dyn Error>>
    {
        use crate::tests::{scripted_response, start_scripted_server};

        let expected = b"expected";
        let corrupt = b"corrupt!";
        let expected_hash = compute_hash("sha-256", expected)?;
        let (url1, server1) = start_scripted_server(vec![
            scripted_response(206, &[("Content-Range", "bytes 0-0/8")], &corrupt[..1]),
            scripted_response(206, &[("Content-Range", "bytes 0-7/8")], corrupt),
        ])
        .await?;
        let (url2, server2) = start_scripted_server(vec![
            scripted_response(206, &[("Content-Range", "bytes 0-0/8")], &corrupt[..1]),
            scripted_response(206, &[("Content-Range", "bytes 0-7/8")], corrupt),
        ])
        .await?;

        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;
        let manifest = format!(
            r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="chunked.bin">
<size>8</size><pieces type="sha-256" length="8"><hash>{expected_hash}</hash></pieces>
<url priority="1">{url1}</url><url priority="2">{url2}</url></file></metalink>"#
        );
        let manifest_path = dir.path().join("chunks.meta4");
        std::fs::write(&manifest_path, manifest)?;
        let destination = dir.path().join("chunked.bin");
        std::fs::write(&destination, b"preserve")?;
        let item = make_metalink_item(&manifest_path, &destination)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, cancellation) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, cancellation).await;
        assert!(
            matches!(
                result,
                Err(DownloadEngineError::MetalinkChunkChecksumMismatch { .. })
            ),
            "expected chunk checksum mismatch, got {result:?}"
        );
        assert_eq!(std::fs::read(destination)?, b"preserve");
        server1.await?;
        server2.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_mirror_failover() -> Result<(), Box<dyn Error>> {
        use crate::tests::{scripted_response, start_scripted_server};

        let body = b"world".to_vec();
        let expected_hash = hex::encode(sha2::Sha256::digest(&body));

        // First server returns 500.
        let responses_fail = vec![scripted_response(500, &[], b"error")];
        let (url_fail, server_fail) = start_scripted_server(responses_fail).await?;

        // Second server returns 200 with correct body.
        let responses_ok = vec![scripted_response(200, &[], &body)];
        let (url_ok, server_ok) = start_scripted_server(responses_ok).await?;

        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        let manifest = format!(
            r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="world.bin">
    <size>5</size>
    <hash type="sha-256">{expected_hash}</hash>
    <url priority="1">{url_fail}</url>
    <url priority="2">{url_ok}</url>
  </file>
</metalink>"#
        );
        let manifest_path = dir.path().join("failover.meta4");
        std::fs::write(&manifest_path, &manifest)?;

        let dest = dir.path().join("world.bin");
        let item = make_metalink_item(&manifest_path, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(
            result.is_ok(),
            "expected failover to succeed, got: {result:?}"
        );
        assert_eq!(std::fs::read(&dest)?, body);
        let _ = server_fail.await;
        let _ = server_ok.await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_multiple_files() -> Result<(), Box<dyn Error>> {
        use crate::tests::{scripted_response, start_scripted_server};

        let body1 = b"file one content".to_vec();
        let body2 = b"file two content".to_vec();
        let hash1 = hex::encode(sha2::Sha256::digest(&body1));
        let hash2 = hex::encode(sha2::Sha256::digest(&body2));

        let (url1, server1) =
            start_scripted_server(vec![scripted_response(200, &[], &body1)]).await?;
        let (url2, server2) =
            start_scripted_server(vec![scripted_response(200, &[], &body2)]).await?;

        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        let manifest = format!(
            r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="file1.bin">
    <size>{}</size>
    <hash type="sha-256">{hash1}</hash>
    <url priority="1">{url1}</url>
  </file>
  <file name="file2.bin">
    <size>{}</size>
    <hash type="sha-256">{hash2}</hash>
    <url priority="1">{url2}</url>
  </file>
</metalink>"#,
            body1.len(),
            body2.len()
        );
        let manifest_path = dir.path().join("multi.meta4");
        std::fs::write(&manifest_path, &manifest)?;

        let dest_dir = dir.path().join("output");
        let item = make_metalink_item(&manifest_path, &dest_dir)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (_cancel_tx, rx) = watch::channel(false);
        let result = DownloadEngine::download(&engine, id, rx).await;
        assert!(
            result.is_ok(),
            "expected multi-file success, got: {result:?}"
        );
        assert_eq!(std::fs::read(dest_dir.join("file1.bin"))?, body1);
        assert_eq!(std::fs::read(dest_dir.join("file2.bin"))?, body2);
        let _ = server1.await;
        let _ = server2.await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_cancel() -> Result<(), Box<dyn Error>> {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            sync::oneshot,
        };

        let body = b"helloworld".to_vec();
        let hash = compute_hash("sha-256", &body)?;
        let chunk_hashes = body
            .chunks(5)
            .map(|chunk| compute_hash("sha-256", chunk))
            .collect::<Result<Vec<_>, _>>()?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (piece_started_tx, piece_started_rx) = oneshot::channel();
        let (release_piece_tx, release_piece_rx) = oneshot::channel();
        let server_body = body.clone();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut piece_started_tx = Some(piece_started_tx);
            let mut release_piece_rx = Some(release_piece_rx);
            for request_index in 0..5 {
                let (mut stream, _) = listener.accept().await?;
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).await?;
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                if request_index == 2 {
                    if let Some(started) = piece_started_tx.take() {
                        let _ = started.send(());
                    }
                    if let Some(release) = release_piece_rx.take() {
                        let _ = release.await;
                    }
                }
                let (start, end, bytes) = match request_index {
                    0 | 3 => (0, 0, server_body[0..1].to_vec()),
                    1 => (0, 4, server_body[0..5].to_vec()),
                    _ => (5, 9, server_body[5..10].to_vec()),
                };
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/10\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(headers.as_bytes()).await;
                let _ = stream.write_all(&bytes).await;
                let _ = stream.flush().await;
            }
            Ok::<_, std::io::Error>(requests)
        });
        let url = format!("http://{address}/file");
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        let manifest = format!(
            r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="cancel.bin">
    <size>10</size>
    <hash type="sha-256">{hash}</hash>
    <pieces type="sha-256" length="5"><hash>{}</hash><hash>{}</hash></pieces>
    <url priority="1">{url}</url>
  </file>
</metalink>"#,
            chunk_hashes[0], chunk_hashes[1]
        );
        let manifest_path = dir.path().join("cancel.meta4");
        std::fs::write(&manifest_path, manifest)?;

        let dest = dir.path().join("cancel.bin");
        let item = make_metalink_item(&manifest_path, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (_pause_tx, pause) = watch::channel(false);
        let (_limit_tx, connection_limit) = watch::channel(1_usize);
        let control = ManagedDownloadControl {
            cancellation: cancel_rx,
            pause,
            connection_limit,
            resource_demand: None,
            bandwidth: BandwidthControl::default(),
        };
        let download = ManagedDownloadEngine::download_managed(&engine, id, control);
        tokio::pin!(download);
        tokio::select! {
            result = &mut download => panic!("download completed before cancellation: {result:?}"),
            started = piece_started_rx => {
                started?;
                cancel_tx.send(true)?;
            }
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), download)
            .await
            .map_err(|error| format!("cancellation timed out: {error}"))?;
        assert!(
            matches!(result, Err(DownloadEngineError::Cancelled)),
            "expected Cancelled, got: {result:?}"
        );
        assert!(!dest.exists(), "cancelled download must not publish output");
        let chunk_dir = std::env::temp_dir()
            .join(format!("fetchx-metalink-{id}"))
            .join(format!("{id}-cancel.bin-verified-chunks"));
        assert!(
            chunk_dir.join("00000000.piece").exists(),
            "first verified chunk should survive cancellation"
        );
        release_piece_tx
            .send(())
            .map_err(|_| "range test server stopped before resume")?;
        let (_cancel_tx, cancellation) = watch::channel(false);
        let resumed = DownloadEngine::download(&engine, id, cancellation).await;
        assert!(resumed.is_ok(), "expected clean restart, got {resumed:?}");
        assert_eq!(std::fs::read(dest)?, body);
        let requests = server.await??;
        assert!(
            requests[4]
                .to_ascii_lowercase()
                .contains("range: bytes=5-9")
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metalink_retry_resumes_unchecksummed_http_partial() -> Result<(), Box<dyn Error>> {
        use crate::tests::start_cancel_resume_server;

        let body = vec![0x5a; 1024 * 1024];
        let (url, notified, continue_server, server) =
            start_cancel_resume_server(body.clone()).await?;
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;
        let manifest_path = dir.path().join("resume.meta4");
        std::fs::write(
            &manifest_path,
            format!(
                r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="resume.bin">
    <size>{}</size>
    <url priority="1">{url}</url>
  </file>
</metalink>"#,
                body.len()
            ),
        )?;
        let destination = dir.path().join("resume.bin");
        let item = make_metalink_item(&manifest_path, &destination)?;
        let id = item.id();
        db.downloads().create(&item).await?;
        let temp_dir = std::env::temp_dir().join(format!("fetchx-metalink-{id}"));
        let staging_path = temp_dir.join(format!("{id}-resume.bin"));
        let partial_path = PathBuf::from(format!("{}.part", staging_path.display()));

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let download = DownloadEngine::download(&engine, id, cancel_rx);
        tokio::pin!(download);
        tokio::select! {
            result = &mut download => panic!("download completed before cancellation: {result:?}"),
            started = notified => {
                started?;
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        if tokio::fs::metadata(&partial_path)
                            .await
                            .is_ok_and(|metadata| metadata.len() > 0)
                        {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    }
                })
                .await?;
                cancel_tx.send(true)?;
            }
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), download)
            .await
            .map_err(|error| format!("cancellation timed out: {error}"))?;
        assert!(matches!(result, Err(DownloadEngineError::Cancelled)));

        let partial_size = tokio::fs::metadata(&partial_path).await?.len();
        assert!(partial_size > 0 && partial_size < body.len() as u64);

        continue_server
            .send(())
            .map_err(|_| "resume test server stopped before retry")?;
        let (_cancel_tx, cancellation) = watch::channel(false);
        let completed = DownloadEngine::download(&engine, id, cancellation).await?;
        let requests = server.await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(tokio::fs::read(destination).await?, body);
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains(&format!("range: bytes={partial_size}-"))
        );
        Ok(())
    }
}

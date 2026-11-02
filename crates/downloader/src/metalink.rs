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
    /// Mirrors sorted by ascending priority (priority 1 is highest).
    pub mirrors: Vec<MetalinkMirror>,
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

        // Create temp dir for staging.
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
                // Cleanup temp dir.
                let _ = fs::remove_dir_all(&temp_dir).await;
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() {
                let _ = fs::remove_dir_all(&temp_dir).await;
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
                let _ = fs::remove_dir_all(&temp_dir).await;
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
                    let _ = fs::remove_dir_all(&temp_dir).await;
                    return Err(DownloadEngineError::Cancelled);
                }
                Err(DownloadEngineError::Paused) => {
                    let _ = fs::remove_dir_all(&temp_dir).await;
                    return Err(DownloadEngineError::Paused);
                }
                Err(e) => {
                    let _ = fs::remove_dir_all(&temp_dir).await;
                    return Err(e);
                }
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
                    // Verify checksum of staged file.
                    let checksum_result = verify_checksums(&staging_path, &mf.hashes).await;
                    match checksum_result {
                        Err(DownloadEngineError::MetalinkChecksumMismatch {
                            algorithm,
                            expected,
                            actual,
                        }) => {
                            warn!(
                                "Metalink: checksum mismatch for '{}' via '{}': {} expected={} actual={}",
                                mf.name, mirror.url, algorithm, expected, actual
                            );
                            let _ = self.repository.delete(transient_id).await;
                            let _ = fs::remove_file(&staging_path).await;
                            last_error = Some(DownloadEngineError::MetalinkChecksumMismatch {
                                algorithm,
                                expected,
                                actual,
                            });
                            // Try next mirror.
                            continue;
                        }
                        Err(e) => {
                            let _ = self.repository.delete(transient_id).await;
                            let _ = fs::remove_file(&staging_path).await;
                            last_error = Some(e);
                            continue;
                        }
                        Ok(()) => {
                            // Checksum OK. Move staged file to final destination.
                            let _ = self.repository.delete(transient_id).await;
                            if let Err(e) = fs::rename(&staging_path, final_path).await {
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

    Ok(MetalinkFile {
        name,
        size,
        hashes,
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

        files.push(MetalinkFile {
            name,
            size,
            hashes,
            mirrors,
        });
    }

    Ok(files)
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
        DownloadId, DownloadItem, DownloadSource, EngineKind, FileName, Timestamp,
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

        let responses = vec![crate::tests::scripted_response(200, &[], &body)];
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
        // Create a manifest pointing to a non-existent URL; cancel immediately.
        let db = Database::connect("sqlite::memory:").await?;
        let (engine, _) = make_engine(&db).await;
        let dir = tempfile::tempdir()?;

        let manifest = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="cancel.bin">
    <size>5</size>
    <hash type="sha-256">2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824</hash>
    <url priority="1">http://127.0.0.1:1/cancel.bin</url>
  </file>
</metalink>"#;
        let manifest_path = dir.path().join("cancel.meta4");
        std::fs::write(&manifest_path, manifest)?;

        let dest = dir.path().join("cancel.bin");
        let item = make_metalink_item(&manifest_path, &dest)?;
        let id = item.id();
        db.downloads().create(&item).await?;

        // Signal cancellation before starting.
        let (cancel_tx, cancel_rx) = watch::channel(true);
        let (_, pause) = watch::channel(false);
        let (_, connection_limit) = watch::channel(1_usize);
        let control = ManagedDownloadControl {
            cancellation: cancel_rx,
            pause,
            connection_limit,
            resource_demand: None,
            bandwidth: BandwidthControl::default(),
        };
        let result = ManagedDownloadEngine::download_managed(&engine, id, control).await;
        drop(cancel_tx);
        assert!(
            matches!(result, Err(DownloadEngineError::Cancelled)),
            "expected Cancelled, got: {result:?}"
        );
        Ok(())
    }
}

//! Platform-independent service bootstrap for FetchX.
//!
//! The Tauri desktop application, Android bridge, CLI, and future headless
//! server share the same database, repository, engine runtime, queue, and scheduler.
//! This crate provides a single [`CoreServices`] type that wires them
//! together from explicit paths, without any Tauri or GUI dependency.
//!
//! # Usage
//!
//! ```no_run
//! # use fetchx_core_services::{CoreServices, CoreServicesConfig};
//! # use std::path::PathBuf;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = CoreServicesConfig {
//!     database_path: PathBuf::from("/data/fetchx.db"),
//!     downloads_dir: PathBuf::from("/downloads"),
//!     runtime_dir:   PathBuf::from("/tmp/fetchx-runtime"),
//!     bundled_engines_dir: None,
//!     defer_queued_on_start: false,
//! };
//! let services = CoreServices::start(config).await?;
//! // use services.queue(), services.scheduler(), etc.
//! services.shutdown().await?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};

use fetchx_downloader::{
    DirectHttpEngine, EngineDispatcher, FtpEngine, HlsEngine, MetalinkEngine,
    NativeTorrentConfig as DownloaderNativeTorrentConfig, NativeTorrentEngine, SftpEngine,
    YtDlpConfig as DownloaderYtDlpConfig, YtDlpEngine, YtDlpProcessRunner,
    YtDlpProcessRunnerAdapter, is_dash_url, is_hls_url, remove_http_multi_source_set,
    save_http_multi_source_set, validate_http_source_url,
};
use fetchx_media_extractor::{
    BrightcoveExtractor, CbcExtractor, ExtractOptions as MediaExtractOptions, ExtractionError,
    ExtractorRegistry, GenericExtractor, JwPlatformExtractor, KalturaExtractor,
    PolskieRadioExtractor, RadioFranceExtractor, UnsupportedExtractor, VimeoExtractor,
    YoutubeExtractor, YtDlpExtractor,
};
use fetchx_models::{
    ApplicationDiagnostics, DIAGNOSTICS_SCHEMA_VERSION, DiagnosticComponentStatus,
    DiagnosticHealth, DiagnosticsSnapshot, DownloadId, DownloadItem, DownloadPriority,
    DownloadRuntimeStatus, DownloadSource, DownloadStatus, DuplicateDownloadMatch,
    EngineDiagnostics, EngineKind, FileName, PluginDiagnostics, QueueDiagnostics, RpcDiagnostics,
    StorageDiagnostics, Timestamp, TorrentFileSelection, TorrentMetadata, YtDlpAudioFormat,
    global_diagnostics, parse_magnet_uri, parse_torrent_metainfo,
};
use fetchx_queue::{DownloadEventBus, EventRepository, QueueManager};
use fetchx_scheduler::Scheduler;
use fetchx_storage::{
    Database, DownloadRepository, SqliteDownloadRepository, TorrentMetadataRepository,
    TorrentMetadataRow, TorrentSeedState, TorrentSourceType,
};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    time::Duration,
};
use tracing::{error, info, warn};
use url::Url;

const MAX_HTTP_MULTI_SOURCE_URL_LENGTH: usize = 8192;

pub use config::{
    Aria2Config, ConfigError, ConfigService, DirectHttpConfig, EngineConfig, FtpConfig,
    MetalinkConfig, NativeTorrentConfig, SftpConfig, YtDlpConfig,
};
pub use engine_status::{
    EngineAvailability, EngineRuntimeState, EngineRuntimeStatus, EngineStatusEntry,
};

mod builtin;
pub mod config;
mod engine_status;
pub use builtin::{
    CoreBuiltinError, FilenameTemplateContext, FilenameTemplateDiagnostic, FilenameTemplateResult,
    extract_download_links, find_archive_link, is_duplicate_download_url, match_download_category,
    normalize_download_url, render_filename_template, resolve_core_url,
    sanitize_download_package_name, suggest_download_filename, suggest_download_package_name,
};

// ============================================================
// Repository type alias
// ============================================================

/// Concrete repository type: event-bus-wrapped SQLite repository.
pub type CoreRepository = EventRepository<SqliteDownloadRepository>;

// ============================================================
// Configuration
// ============================================================

/// Paths needed to start the core services.
#[derive(Clone, Debug)]
pub struct CoreServicesConfig {
    pub database_path: PathBuf,
    pub downloads_dir: PathBuf,
    pub runtime_dir: PathBuf,
    /// Optional directory containing installer-bundled desktop engine binaries.
    /// Used only to seed engine defaults when no engine config has been stored.
    pub bundled_engines_dir: Option<PathBuf>,
    /// Defers persisted runnable work while a short-lived host performs a CLI
    /// operation. Long-running app and server hosts should leave this false.
    pub defer_queued_on_start: bool,
}

// ============================================================
// CoreServices
// ============================================================

/// Owns the live service graph: database, event bus, engines, queue, scheduler.
///
/// Shared service owner used by desktop, Android, CLI, and headless hosts.
pub struct CoreServices {
    pub database: Database,
    pub repository: CoreRepository,
    pub event_bus: DownloadEventBus,
    pub config_service: ConfigService,
    pub engine_runtime: Arc<EngineRuntime>,
    pub scheduler: Scheduler<CoreRepository>,
    queue: QueueManager,
    pub downloads_dir: PathBuf,
    shutdown_complete: Mutex<bool>,
}

/// Parameters for [`CoreServices::create_torrent_metadata_row`].
/// Using a struct avoids the clippy `too_many_arguments` lint.
struct TorrentMetadataParams {
    id: DownloadId,
    source_type: TorrentSourceType,
    magnet_uri: Option<String>,
    local_torrent_path: Option<PathBuf>,
    info_hash: String,
    torrent_name: String,
    torrent_metadata: Option<TorrentMetadata>,
    destination: PathBuf,
    selected_file_indexes: Vec<u32>,
    selected_total_bytes: u64,
    selection_confirmed: bool,
    created_at: Timestamp,
}

/// One URL and its media-selection options for shared batch admission.
#[derive(Clone, Debug)]
pub struct BatchDownloadRequest {
    pub url: String,
    pub package_name: Option<String>,
    pub engine: EngineKind,
    pub format_id: Option<String>,
    pub extension: Option<String>,
    pub audio_format: Option<YtDlpAudioFormat>,
}

/// One URL with an adapter-resolved destination for shared batch admission.
///
/// The destination is an absolute file path chosen by the host. This is useful
/// for platform adapters that must stage files before handing them to the
/// shared queue. URL validation, engine/format validation, collision handling,
/// and queue admission remain in `CoreServices`.
#[derive(Clone, Debug)]
pub struct BatchDownloadDestinationRequest {
    pub url: String,
    pub filename: String,
    pub destination: PathBuf,
    pub engine: EngineKind,
    pub format_id: Option<String>,
    pub extension: Option<String>,
    pub audio_format: Option<YtDlpAudioFormat>,
    pub expected_sha256: Option<String>,
}

/// Ordered HTTP(S) sources for one logical DirectHTTP download.
///
/// Cross-source range assignment is enabled only with `expected_sha256` and
/// after every source passes the shared downloader's range/length probes.
#[derive(Clone, Debug)]
pub struct HttpMultiSourceRequest {
    pub urls: Vec<String>,
    pub package_name: Option<String>,
    /// Expected SHA-256; required before the downloader can combine byte ranges
    /// from multiple sources.
    pub expected_sha256: Option<String>,
}

struct BatchAdmissionRequest {
    request: BatchDownloadRequest,
    destination: Option<PathBuf>,
    filename: Option<String>,
    expected_sha256: Option<String>,
    require_exact_destination: bool,
}

/// Per-input result from [`CoreServices::enqueue_download_batch`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchDownloadOutcome {
    pub input_url: String,
    pub result: Result<DownloadId, BatchDownloadError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeMagnetFile {
    pub index: u32,
    pub path: Vec<String>,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeMagnetMetadata {
    pub metadata_state: String,
    pub info_hash: String,
    pub name: Option<String>,
    pub files: Vec<NativeMagnetFile>,
    pub selected_file_indexes: Vec<u32>,
    pub selected_total_bytes: u64,
    pub selection_confirmed: bool,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BatchDownloadError {
    #[error("duplicate URL in batch")]
    DuplicateUrl,
    #[error("URL must be a valid HTTP or HTTPS address")]
    InvalidUrl,
    #[error("package name must be a single safe directory name")]
    InvalidPackageName,
    #[error("media format selection is invalid")]
    InvalidFormat,
    #[error("expected SHA-256 must contain exactly 64 hexadecimal characters")]
    InvalidChecksum,
    #[error("HTTP multi-source request must contain between 2 and 16 distinct sources")]
    InvalidSourceSet,
    #[error("HTTP multi-source inputs must be direct HTTP(S) file URLs")]
    InvalidMultiSourceUrl,
    #[error("destination must be an absolute file path with a valid filename")]
    InvalidDestination,
    #[error("download engine is unavailable: {0:?}")]
    UnavailableEngine(EngineKind),
    #[error("could not prepare download destination: {0}")]
    Destination(String),
    #[error("could not store download checksum: {0}")]
    Storage(String),
    #[error("could not create download record: {0}")]
    Queue(String),
    #[error("this aria2 download is not eligible for native restart")]
    RestartNotEligible,
    #[error("the selected destination conflicts with existing download data")]
    DestinationCollision,
    #[error("the current source must be a valid v1 BitTorrent magnet or .torrent file")]
    InvalidTorrentRestartSource,
    #[error("the current source does not match the saved aria2 torrent info hash")]
    TorrentSourceMismatch,
    #[error("review and confirm at least one file from the current torrent")]
    InvalidTorrentRestartSelection,
    #[error("the selected torrent destination must be a new directory under downloads")]
    InvalidTorrentRestartDestination,
    #[error("the selected torrent destination overlaps existing download data")]
    TorrentDestinationCollision,
}

fn valid_batch_format_options(request: &BatchDownloadRequest) -> bool {
    match (
        request.engine,
        request.format_id.is_some(),
        request.audio_format,
    ) {
        (EngineKind::YtDlp, true, None) => request
            .extension
            .as_deref()
            .is_none_or(is_safe_batch_extension),
        (EngineKind::YtDlp, false, Some(audio_format)) => {
            request.extension.as_deref() == Some(audio_format.extension())
        }
        (_, false, None) => request.extension.is_none(),
        _ => false,
    }
}

fn is_safe_batch_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.len() <= 10
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_valid_batch_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn batch_destination_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn append_path_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn strict_destination_keys(path: &Path) -> Vec<String> {
    let mut keys = vec![batch_destination_key(path)];
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name())
        && let Ok(canonical_parent) = std::fs::canonicalize(parent)
    {
        keys.push(
            canonical_parent
                .join(file_name)
                .to_string_lossy()
                .to_lowercase(),
        );
    }
    keys
}

fn normalized_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().to_lowercase();
    #[cfg(windows)]
    let value = value.replace('/', "\\");
    value.trim_end_matches(std::path::MAIN_SEPARATOR).to_owned()
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = normalized_path_key(left);
    let right = normalized_path_key(right);
    left == right
        || left.starts_with(&format!("{right}{}", std::path::MAIN_SEPARATOR))
        || right.starts_with(&format!("{left}{}", std::path::MAIN_SEPARATOR))
}

async fn canonical_destination_identity(path: &Path) -> Result<PathBuf, String> {
    match fs::canonicalize(path).await {
        Ok(canonical) => Ok(canonical),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| "destination has no parent directory".to_owned())?;
            let file_name = path
                .file_name()
                .ok_or_else(|| "destination has no directory name".to_owned())?;
            Ok(fs::canonicalize(parent)
                .await
                .map_err(|error| error.to_string())?
                .join(file_name))
        }
        Err(error) => Err(error.to_string()),
    }
}

async fn verify_directory_writable(directory: &Path) -> Result<(), BatchDownloadError> {
    static PROBE_ID: AtomicU64 = AtomicU64::new(0);

    for _ in 0..16 {
        let id = PROBE_ID.fetch_add(1, Ordering::Relaxed);
        let probe = directory.join(format!(".fetchx-write-check-{}-{id}", std::process::id()));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .await
        {
            Ok(file) => {
                drop(file);
                tokio::fs::remove_file(probe)
                    .await
                    .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(BatchDownloadError::Destination(error.to_string())),
        }
    }

    Err(BatchDownloadError::Destination(
        "could not allocate a destination permission check".to_owned(),
    ))
}

fn is_media_download_filename(filename: &str) -> bool {
    Path::new(filename)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
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
                    | "ts"
                    | "wav"
                    | "webm"
                    | "wma"
                    | "wmv"
                    | "m3u8"
                    | "mpd"
            )
        })
}

fn is_media_http_url(url: &reqwest::Url) -> bool {
    url.path_segments()
        .and_then(Iterator::last)
        .is_some_and(is_media_download_filename)
}

async fn unique_batch_destination(
    destination: PathBuf,
    filename: FileName,
    reserved_destinations: &mut HashSet<String>,
) -> Result<(PathBuf, FileName), BatchDownloadError> {
    let path = Path::new(filename.as_str());
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or(BatchDownloadError::InvalidFormat)?;
    let extension = path.extension().and_then(std::ffi::OsStr::to_str);

    for suffix in 1..=10_000 {
        let candidate_name = if suffix == 1 {
            filename.as_str().to_owned()
        } else if let Some(extension) = extension {
            format!("{stem} ({suffix}).{extension}")
        } else {
            format!("{stem} ({suffix})")
        };
        let candidate_filename = FileName::new(candidate_name)
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        let candidate_path = destination.with_file_name(candidate_filename.as_str());
        let key = batch_destination_key(&candidate_path);
        let exists = tokio::fs::try_exists(&candidate_path)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        if !exists && reserved_destinations.insert(key) {
            return Ok((candidate_path, candidate_filename));
        }
    }

    Err(BatchDownloadError::Destination(
        "could not find an unused destination filename".to_owned(),
    ))
}

impl CoreServices {
    /// Initialises all services from the supplied paths.
    ///
    /// Optional engines that fail to start are marked `Unavailable`; the
    /// function only fails for unrecoverable errors (e.g. database I/O).
    pub async fn start(cfg: CoreServicesConfig) -> Result<Self, CoreServicesError> {
        Self::start_internal(cfg, None, None).await
    }

    /// Initializes services with a per-process engine configuration override.
    ///
    /// This is used by Android, where executable paths are private to the app
    /// installation and must not be persisted as desktop engine settings.
    pub async fn start_with_engine_config(
        cfg: CoreServicesConfig,
        engine_config_override: Option<EngineConfig>,
    ) -> Result<Self, CoreServicesError> {
        Self::start_internal(cfg, engine_config_override, None).await
    }

    pub async fn start_with_engine_config_and_ytdlp_runner(
        cfg: CoreServicesConfig,
        engine_config_override: Option<EngineConfig>,
        ytdlp_runner: Option<Arc<dyn YtDlpProcessRunner>>,
    ) -> Result<Self, CoreServicesError> {
        Self::start_internal(cfg, engine_config_override, ytdlp_runner).await
    }

    async fn start_internal(
        cfg: CoreServicesConfig,
        engine_config_override: Option<EngineConfig>,
        ytdlp_runner: Option<Arc<dyn YtDlpProcessRunner>>,
    ) -> Result<Self, CoreServicesError> {
        // Ensure directories exist.
        if let Some(parent) = cfg.database_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(CoreServicesError::Io)?;
        }
        fs::create_dir_all(&cfg.downloads_dir)
            .await
            .map_err(CoreServicesError::Io)?;
        fs::create_dir_all(&cfg.runtime_dir)
            .await
            .map_err(CoreServicesError::Io)?;

        // Database + repository
        let database = Database::connect_path(&cfg.database_path)
            .await
            .map_err(|e| CoreServicesError::Database(e.to_string()))?;
        let raw_repository = database.downloads();
        let event_bus = DownloadEventBus::new();
        let repository = EventRepository::new(raw_repository, event_bus.clone());

        // Engine configuration
        let config_service = ConfigService::new(database.config());
        let engine_config = match engine_config_override {
            Some(config) => config,
            None => {
                let defaults = bundled_engine_defaults(cfg.bundled_engines_dir.as_deref());
                config_service
                    .initialize_if_missing(&defaults)
                    .await
                    .map_err(CoreServicesError::Config)?;
                config_service
                    .load()
                    .await
                    .map_err(CoreServicesError::Config)?
            }
        };
        engine_config
            .validate()
            .map_err(CoreServicesError::Config)?;

        // Engine runtime
        let engine_runtime = EngineRuntime::initialize_with_ytdlp_runner(
            repository.clone(),
            &engine_config,
            &cfg.runtime_dir,
            ytdlp_runner,
            Some(database.torrent_metadata()),
        )
        .await?;

        // Queue + Scheduler
        let deferred = if cfg.defer_queued_on_start {
            repository
                .list()
                .await
                .map_err(|error| CoreServicesError::Database(error.to_string()))?
                .into_iter()
                .filter(|item| {
                    matches!(
                        item.status(),
                        DownloadStatus::Queued
                            | DownloadStatus::Preparing
                            | DownloadStatus::Downloading
                            | DownloadStatus::MetadataAcquiring
                            | DownloadStatus::Recovering
                            | DownloadStatus::Seeding
                    )
                })
                .map(|item| item.id())
                .collect()
        } else {
            std::collections::HashSet::new()
        };
        let scheduler = match Scheduler::start_with_resource_policy_and_deferred(
            repository.clone(),
            engine_runtime.dispatcher.clone(),
            engine_config.resource_policy.clone(),
            deferred,
        )
        .await
        {
            Ok(scheduler) => scheduler,
            Err(error) => {
                if let Err(cleanup_error) = engine_runtime.shutdown().await {
                    warn!(%cleanup_error, "engine shutdown failed after scheduler startup failure");
                }
                return Err(CoreServicesError::Queue(error.to_string()));
            }
        };
        let queue = scheduler.queue_manager();

        Ok(Self {
            database,
            repository,
            event_bus,
            config_service,
            engine_runtime: Arc::new(engine_runtime),
            scheduler,
            queue,
            downloads_dir: cfg.downloads_dir,
            shutdown_complete: Mutex::new(false),
        })
    }

    // ── Accessors ────────────────────────────────────────────────────────

    #[must_use]
    pub fn queue(&self) -> &QueueManager {
        &self.queue
    }

    /// Returns runtime engine availability without exposing executable paths.
    pub fn engine_runtime_statuses(&self) -> Vec<EngineStatusEntry> {
        self.engine_runtime.status.entries()
    }

    /// Prevents new queued downloads from starting without interrupting active transfers.
    pub async fn pause_queue_dispatch(&self) -> Result<(), CoreServicesError> {
        self.queue
            .pause_dispatch()
            .await
            .map_err(|error| CoreServicesError::Queue(error.to_string()))
    }

    /// Allows eligible queued downloads to start again.
    pub async fn resume_queue_dispatch(&self) -> Result<(), CoreServicesError> {
        self.queue
            .resume_dispatch()
            .await
            .map_err(|error| CoreServicesError::Queue(error.to_string()))
    }

    /// Reports whether new queue dispatch is currently held.
    pub async fn is_queue_dispatch_paused(&self) -> Result<bool, CoreServicesError> {
        self.queue
            .snapshot()
            .await
            .map(|snapshot| snapshot.dispatch_paused)
            .map_err(|error| CoreServicesError::Queue(error.to_string()))
    }

    #[must_use]
    pub fn scheduler(&self) -> &Scheduler<CoreRepository> {
        &self.scheduler
    }

    #[must_use]
    pub fn downloads_dir(&self) -> &std::path::Path {
        &self.downloads_dir
    }

    /// Admits a batch using a per-source package when no explicit package is set.
    ///
    /// This delegates validation, collision handling, partial results, and queue
    /// lifecycle to [`Self::enqueue_download_batch`].
    pub async fn enqueue_download_batch_by_source(
        &self,
        mut requests: Vec<BatchDownloadRequest>,
        priority: DownloadPriority,
    ) -> Vec<BatchDownloadOutcome> {
        for request in &mut requests {
            if request.package_name.is_none() {
                let filename = suggest_download_filename(&request.url);
                request.package_name = Some(suggest_download_package_name(&request.url, &filename));
            }
        }
        self.enqueue_download_batch(requests, priority).await
    }

    /// Returns only fields that change during download execution.
    ///
    /// Clients can poll this compact projection without transferring durable
    /// metadata such as source URLs, destinations, filenames, and priorities.
    pub async fn download_runtime_statuses(
        &self,
    ) -> Result<Vec<DownloadRuntimeStatus>, fetchx_storage::StorageError> {
        let items = self.repository.list().await?;
        Ok(items.iter().map(DownloadRuntimeStatus::from).collect())
    }

    pub async fn find_duplicate_download(
        &self,
        url: &str,
    ) -> Result<Option<DuplicateDownloadMatch>, fetchx_storage::StorageError> {
        let normalized = normalize_download_url(url);
        let items = self.repository.list().await?;
        Ok(items
            .iter()
            .find(|item| normalize_download_url(item.source().url()) == normalized)
            .map(DuplicateDownloadMatch::from))
    }

    /// Builds a bounded snapshot containing aggregate state and no user configuration.
    pub async fn diagnostics_snapshot(&self) -> DiagnosticsSnapshot {
        let metrics = global_diagnostics().metrics();
        let recent_events = global_diagnostics().recent_events();
        let queue_result = self.queue.snapshot().await;
        let storage_check_started = std::time::Instant::now();
        let storage_result = self.database.check().await;
        let database_check_latency_ms =
            u64::try_from(storage_check_started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let (queue, queue_health, engines) = match queue_result {
            Ok(snapshot) => {
                let mut queue = QueueDiagnostics {
                    health: DiagnosticHealth::Healthy,
                    total_downloads: snapshot.items.len() as u64,
                    active_downloads: snapshot.active.len() as u64,
                    queued_downloads: snapshot.queued.len() as u64,
                    paused_downloads: snapshot.paused.len() as u64,
                    failed_downloads: 0,
                    completed_downloads: 0,
                    cancelled_downloads: 0,
                    max_active_downloads: Some(snapshot.max_active_downloads as u64),
                    global_connection_limit: Some(snapshot.global_connection_limit as u64),
                    bytes_downloaded: 0,
                    bytes_uploaded: None,
                    current_speed_bytes_per_second: 0,
                    average_speed_bytes_per_second: 0,
                };
                let engine_kinds = [
                    (EngineKind::DirectHttp, "direct_http"),
                    (EngineKind::YtDlp, "yt_dlp"),
                    (EngineKind::Hls, "hls"),
                    (EngineKind::NativeTorrent, "native_torrent"),
                    (EngineKind::Ftp, "ftp"),
                    (EngineKind::Ftps, "ftps"),
                    (EngineKind::Sftp, "sftp"),
                    (EngineKind::Metalink, "metalink"),
                ];
                for item in &snapshot.items {
                    queue.bytes_downloaded = queue
                        .bytes_downloaded
                        .saturating_add(item.progress().downloaded_bytes());
                    if item.status() == DownloadStatus::Downloading {
                        queue.current_speed_bytes_per_second = queue
                            .current_speed_bytes_per_second
                            .saturating_add(item.progress().speed_bytes_per_second().unwrap_or(0));
                    }
                    match item.status() {
                        DownloadStatus::Failed => queue.failed_downloads += 1,
                        DownloadStatus::Completed | DownloadStatus::Seeding => {
                            queue.completed_downloads += 1;
                        }
                        DownloadStatus::Cancelled | DownloadStatus::Stopped => {
                            queue.cancelled_downloads += 1;
                        }
                        _ => {}
                    }
                }
                let mut uploaded_bytes = 0_u64;
                let mut has_upload_telemetry = false;
                for active in &snapshot.active {
                    if !matches!(active.item.source().engine(), EngineKind::NativeTorrent) {
                        continue;
                    }
                    match self.torrent_telemetry(active.item.id()).await {
                        Ok(Some(telemetry)) => {
                            if let Some(bytes) = telemetry.uploaded_bytes {
                                uploaded_bytes = uploaded_bytes.saturating_add(bytes);
                                has_upload_telemetry = true;
                            }
                        }
                        Ok(None) => {}
                        Err(_) => {
                            global_diagnostics().record(
                                fetchx_models::DiagnosticEvent::new(
                                    fetchx_models::DiagnosticComponent::Downloader,
                                    "torrent_telemetry_unavailable",
                                    fetchx_models::DiagnosticLevel::Warn,
                                )
                                .with_error_code("TORRENT_TELEMETRY_UNAVAILABLE"),
                            );
                        }
                    }
                }
                queue.bytes_uploaded = has_upload_telemetry.then_some(uploaded_bytes);
                let downloading_count = snapshot
                    .active
                    .iter()
                    .filter(|active| active.item.status() == DownloadStatus::Downloading)
                    .count() as u64;
                queue.average_speed_bytes_per_second = queue
                    .current_speed_bytes_per_second
                    .checked_div(downloading_count)
                    .unwrap_or_default();
                let engines = engine_kinds
                    .into_iter()
                    .map(|(kind, name)| {
                        let health = self.engine_health(kind);
                        EngineDiagnostics {
                            engine: name.to_owned(),
                            health,
                            active_downloads: snapshot
                                .active
                                .iter()
                                .filter(|active| active.item.source().engine() == kind)
                                .count() as u64,
                        }
                    })
                    .collect();
                (queue, DiagnosticHealth::Healthy, engines)
            }
            Err(_) => (
                QueueDiagnostics {
                    health: DiagnosticHealth::Unavailable,
                    total_downloads: 0,
                    active_downloads: 0,
                    queued_downloads: 0,
                    paused_downloads: 0,
                    failed_downloads: 0,
                    completed_downloads: 0,
                    cancelled_downloads: 0,
                    max_active_downloads: None,
                    global_connection_limit: None,
                    bytes_downloaded: 0,
                    bytes_uploaded: None,
                    current_speed_bytes_per_second: 0,
                    average_speed_bytes_per_second: 0,
                },
                DiagnosticHealth::Unavailable,
                Vec::new(),
            ),
        };
        let (storage, storage_health) = match storage_result {
            Ok(check) => {
                let health = if check.is_healthy() {
                    DiagnosticHealth::Healthy
                } else {
                    DiagnosticHealth::Unhealthy
                };
                (
                    StorageDiagnostics {
                        health,
                        schema_version: check.schema.database_schema_version,
                        database_check_latency_ms: Some(database_check_latency_ms),
                        migration_state: Some(format!("{:?}", check.schema.state)),
                        integrity: Some(format!("{:?}", check.sqlite_integrity)),
                        database_size_bytes: check.database_size_bytes,
                        error_code: (!check.is_healthy())
                            .then(|| "STORAGE_HEALTH_CHECK_FAILED".to_owned()),
                    },
                    health,
                )
            }
            Err(_) => (
                StorageDiagnostics {
                    health: DiagnosticHealth::Unavailable,
                    schema_version: None,
                    database_check_latency_ms: Some(database_check_latency_ms),
                    migration_state: None,
                    integrity: None,
                    database_size_bytes: None,
                    error_code: Some("STORAGE_UNAVAILABLE".to_owned()),
                },
                DiagnosticHealth::Unavailable,
            ),
        };
        let downloader_health =
            if self.engine_health(EngineKind::DirectHttp) == DiagnosticHealth::Healthy {
                DiagnosticHealth::Healthy
            } else {
                DiagnosticHealth::Unhealthy
            };
        let core_health = if queue_health == DiagnosticHealth::Healthy
            && storage_health == DiagnosticHealth::Healthy
            && downloader_health == DiagnosticHealth::Healthy
        {
            DiagnosticHealth::Healthy
        } else {
            DiagnosticHealth::Degraded
        };
        let components = vec![
            DiagnosticComponentStatus {
                health: core_health,
                error_code: None,
            },
            DiagnosticComponentStatus {
                health: queue_health,
                error_code: (queue_health != DiagnosticHealth::Healthy)
                    .then(|| "QUEUE_UNAVAILABLE".to_owned()),
            },
            DiagnosticComponentStatus {
                health: downloader_health,
                error_code: (downloader_health != DiagnosticHealth::Healthy)
                    .then(|| "DOWNLOAD_ENGINE_UNAVAILABLE".to_owned()),
            },
            DiagnosticComponentStatus {
                health: storage_health,
                error_code: storage.error_code.clone(),
            },
            DiagnosticComponentStatus {
                health: DiagnosticHealth::Unavailable,
                error_code: None,
            },
            DiagnosticComponentStatus {
                health: DiagnosticHealth::Unavailable,
                error_code: None,
            },
        ];
        let overall_health = if core_health == DiagnosticHealth::Healthy {
            DiagnosticHealth::Healthy
        } else {
            DiagnosticHealth::Degraded
        };

        DiagnosticsSnapshot {
            schema_version: DIAGNOSTICS_SCHEMA_VERSION,
            generated_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|duration| u64::try_from(duration.as_millis()).ok())
                .unwrap_or_default(),
            health: overall_health,
            application: ApplicationDiagnostics {
                name: "FetchX".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                protocol_version: fetchx_remote_protocol::PROTOCOL_VERSION.to_owned(),
                target_os: std::env::consts::OS.to_owned(),
                target_arch: std::env::consts::ARCH.to_owned(),
            },
            components,
            queue,
            engines,
            storage,
            rpc: RpcDiagnostics {
                health: DiagnosticHealth::Unavailable,
                active_connections: None,
                metrics,
            },
            plugins: PluginDiagnostics {
                health: DiagnosticHealth::Unavailable,
                discovered_plugins: None,
                running_plugins: None,
                metrics,
            },
            metrics,
            recent_events,
        }
    }

    fn engine_health(&self, engine: EngineKind) -> DiagnosticHealth {
        use EngineAvailability::{Available, Disabled, Unavailable};
        let availability = match engine {
            EngineKind::DirectHttp | EngineKind::Hls => return DiagnosticHealth::Healthy,
            EngineKind::Aria2 => return DiagnosticHealth::Unavailable,
            EngineKind::YtDlp => &self.engine_runtime.status.yt_dlp,
            EngineKind::NativeTorrent => &self.engine_runtime.status.native_torrent,
            EngineKind::Ftp | EngineKind::Ftps => &self.engine_runtime.status.ftp,
            EngineKind::Sftp => &self.engine_runtime.status.sftp,
            EngineKind::Metalink => &self.engine_runtime.status.metalink,
        };
        match availability {
            Available { .. } => DiagnosticHealth::Healthy,
            Disabled => DiagnosticHealth::Unavailable,
            Unavailable { .. } => DiagnosticHealth::Degraded,
        }
    }

    /// Extract media formats for `url` using the registered media extractor pipeline.
    ///
    /// Returns structured [`fetchx_media_extractor::MediaInfo`] on success,
    /// or a [`fetchx_media_extractor::ExtractionError`] on failure.
    ///
    /// The title in the returned `MediaInfo` is suitable for use as a download
    /// filename after passing through
    /// [`fetchx_media_extractor::sanitise_title_for_filename`].
    pub async fn extract_media(
        &self,
        url: &str,
        opts: MediaExtractOptions,
    ) -> Result<fetchx_media_extractor::MediaInfo, ExtractionError> {
        self.engine_runtime.media_registry.extract(url, opts).await
    }

    /// Admits one DirectHTTP job with ordered, whole-source failover candidates.
    ///
    /// The first URL remains the persisted source for backward compatibility;
    /// the ordered list is stored in a bounded sidecar next to the destination.
    /// No bytes from different URLs are combined.
    pub async fn enqueue_http_multi_source(
        &self,
        request: HttpMultiSourceRequest,
        priority: DownloadPriority,
    ) -> Result<DownloadId, BatchDownloadError> {
        if !self.engine_runtime.is_available(EngineKind::DirectHttp) {
            return Err(BatchDownloadError::UnavailableEngine(
                EngineKind::DirectHttp,
            ));
        }
        if !(2..=16).contains(&request.urls.len()) {
            return Err(BatchDownloadError::InvalidSourceSet);
        }
        let mut urls = Vec::with_capacity(request.urls.len());
        let mut seen = HashSet::new();
        for source in request.urls {
            if source.len() > MAX_HTTP_MULTI_SOURCE_URL_LENGTH {
                return Err(BatchDownloadError::InvalidMultiSourceUrl);
            }
            let canonical = validate_http_source_url(&source)
                .map_err(|_| BatchDownloadError::InvalidMultiSourceUrl)?;
            let normalized = normalize_download_url(&canonical);
            if !seen.insert(normalized) || is_hls_url(&canonical) || is_dash_url(&canonical) {
                return Err(BatchDownloadError::InvalidMultiSourceUrl);
            }
            urls.push(canonical);
        }
        if request
            .expected_sha256
            .as_deref()
            .is_some_and(|checksum| !is_valid_batch_sha256(checksum.trim()))
        {
            return Err(BatchDownloadError::InvalidChecksum);
        }

        let first_url = &urls[0];
        let filename = FileName::new(suggest_download_filename(first_url))
            .map_err(|_| BatchDownloadError::InvalidDestination)?;
        let mut destination_dir = self.downloads_dir.clone();
        if let Some(package_name) = request.package_name {
            destination_dir.push(
                sanitize_download_package_name(&package_name)
                    .ok_or(BatchDownloadError::InvalidPackageName)?,
            );
        }
        fs::create_dir_all(&destination_dir)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        let mut reserved_destinations = HashSet::new();
        let (destination, filename) = unique_batch_destination(
            destination_dir.join(filename.as_str()),
            filename,
            &mut reserved_destinations,
        )
        .await?;

        let temporary = append_path_suffix(&destination, ".part");
        let identity = append_path_suffix(&temporary, ".meta");
        let sources_path = append_path_suffix(&destination, ".fetchx.sources");
        let checksum_path = append_path_suffix(&destination, ".fetchx.sha256");
        for path in [&temporary, &identity, &sources_path, &checksum_path] {
            if fs::try_exists(path)
                .await
                .map_err(|error| BatchDownloadError::Destination(error.to_string()))?
            {
                return Err(BatchDownloadError::DestinationCollision);
            }
        }

        let created_at =
            Timestamp::now().map_err(|error| BatchDownloadError::Queue(error.to_string()))?;
        let source = DownloadSource::new(&urls[0], EngineKind::DirectHttp)
            .map_err(|_| BatchDownloadError::InvalidMultiSourceUrl)?;
        let mut item = DownloadItem::new(
            DownloadId::new(),
            source,
            filename,
            destination.clone(),
            created_at,
        );
        item.set_priority_at(priority, created_at)
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?;

        if let Some(checksum) = request.expected_sha256 {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&checksum_path)
                .await
                .map_err(|error| BatchDownloadError::Storage(error.to_string()))?;
            let write_result = async {
                file.write_all(checksum.trim().to_ascii_lowercase().as_bytes())
                    .await?;
                file.flush().await?;
                file.sync_all().await
            }
            .await;
            if let Err(error) = write_result {
                drop(file);
                let _ = fs::remove_file(&checksum_path).await;
                return Err(BatchDownloadError::Storage(error.to_string()));
            }
        }
        if let Err(error) = save_http_multi_source_set(&destination, &urls).await {
            if let Err(cleanup_error) = fs::remove_file(&checksum_path).await
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(BatchDownloadError::Storage(format!(
                    "source metadata write failed ({error}) and checksum cleanup failed ({cleanup_error})"
                )));
            }
            return Err(BatchDownloadError::Storage(error.to_string()));
        }
        let id = item.id();
        if let Err(error) = self.queue.enqueue(item).await {
            let source_cleanup = remove_http_multi_source_set(&destination).await;
            let checksum_cleanup = fs::remove_file(&checksum_path).await;
            if let Err(cleanup_error) = source_cleanup {
                return Err(BatchDownloadError::Storage(format!(
                    "queue admission failed ({error}) and source metadata cleanup failed: {cleanup_error}"
                )));
            }
            if let Err(cleanup_error) = checksum_cleanup
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(BatchDownloadError::Storage(format!(
                    "queue admission failed ({error}) and checksum cleanup failed: {cleanup_error}"
                )));
            }
            return Err(BatchDownloadError::Queue(error.to_string()));
        }
        Ok(id)
    }

    /// Validates and admits each URL independently, returning one result per input.
    ///
    /// The batch is intentionally not transactional: an invalid or duplicate
    /// item does not prevent later items from being queued. Dropping this
    /// future stops work that has not started, while items already accepted by
    /// the queue remain persisted and continue through the ordinary lifecycle.
    pub async fn enqueue_download_batch(
        &self,
        requests: Vec<BatchDownloadRequest>,
        priority: DownloadPriority,
    ) -> Vec<BatchDownloadOutcome> {
        self.enqueue_batch_items(
            requests
                .into_iter()
                .map(|request| BatchAdmissionRequest {
                    request,
                    destination: None,
                    filename: None,
                    expected_sha256: None,
                    require_exact_destination: false,
                })
                .collect(),
            priority,
        )
        .await
    }

    /// Admits independent requests with host-selected explicit destinations.
    ///
    /// This additive operation is intended for adapters that own destination
    /// policy or filesystem staging. It applies the same Rust URL, engine,
    /// format, duplicate, collision, and queue rules as
    /// [`Self::enqueue_download_batch`]. A failure for one input does not
    /// prevent other valid inputs from being queued.
    pub async fn enqueue_download_batch_with_destinations(
        &self,
        requests: Vec<BatchDownloadDestinationRequest>,
        priority: DownloadPriority,
    ) -> Vec<BatchDownloadOutcome> {
        self.enqueue_batch_items(
            requests
                .into_iter()
                .map(|request| BatchAdmissionRequest {
                    request: BatchDownloadRequest {
                        url: request.url,
                        package_name: None,
                        engine: request.engine,
                        format_id: request.format_id,
                        extension: request.extension,
                        audio_format: request.audio_format,
                    },
                    destination: Some(request.destination),
                    filename: Some(request.filename),
                    expected_sha256: request.expected_sha256,
                    require_exact_destination: false,
                })
                .collect(),
            priority,
        )
        .await
    }

    /// Creates a new native HTTP job from a user-confirmed current URL for an
    /// eligible persisted aria2 HTTP(S) download. The old item and its files
    /// are never changed or reused.
    pub async fn restart_aria2_http_with_native(
        &self,
        aria2_id: DownloadId,
        current_url: String,
        destination: PathBuf,
        confirmed_direct_file: bool,
    ) -> Result<DownloadId, BatchDownloadError> {
        if !confirmed_direct_file {
            return Err(BatchDownloadError::RestartNotEligible);
        }
        let original = self
            .repository
            .get(aria2_id)
            .await
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?
            .ok_or(BatchDownloadError::RestartNotEligible)?;
        let original_url = reqwest::Url::parse(original.source().url())
            .map_err(|_| BatchDownloadError::RestartNotEligible)?;
        if original.source().engine() != EngineKind::Aria2
            || !matches!(original_url.scheme(), "http" | "https")
            || matches!(
                original.status(),
                DownloadStatus::Completed | DownloadStatus::Seeding | DownloadStatus::Stopped
            )
            || is_media_download_filename(original.filename().as_str())
            || is_media_http_url(&original_url)
        {
            return Err(BatchDownloadError::RestartNotEligible);
        }

        let canonical_url =
            validate_http_source_url(&current_url).map_err(|_| BatchDownloadError::InvalidUrl)?;
        let parsed_url =
            reqwest::Url::parse(&canonical_url).map_err(|_| BatchDownloadError::InvalidUrl)?;
        if !matches!(parsed_url.scheme(), "http" | "https")
            || !parsed_url.username().is_empty()
            || parsed_url.password().is_some()
            || parsed_url.fragment().is_some()
            || parsed_url
                .path_segments()
                .and_then(Iterator::last)
                .is_some_and(is_media_download_filename)
        {
            return Err(BatchDownloadError::InvalidUrl);
        }
        if !destination.is_absolute() {
            return Err(BatchDownloadError::InvalidDestination);
        }
        let filename = destination
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(BatchDownloadError::InvalidDestination)?;
        let filename =
            FileName::new(filename).map_err(|_| BatchDownloadError::InvalidDestination)?;
        let destination_parent = destination
            .parent()
            .ok_or(BatchDownloadError::InvalidDestination)?;

        tokio::fs::create_dir_all(destination_parent)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        let items = self
            .repository
            .list()
            .await
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?;
        let candidate_paths = [
            destination.clone(),
            append_path_suffix(&destination, ".aria2"),
            append_path_suffix(&destination, ".part"),
        ];
        let candidate_keys = candidate_paths
            .iter()
            .flat_map(|path| strict_destination_keys(path))
            .collect::<HashSet<_>>();
        for item in &items {
            let existing_paths = [
                item.destination().to_path_buf(),
                append_path_suffix(item.destination(), ".aria2"),
                append_path_suffix(item.destination(), ".part"),
            ];
            if existing_paths.iter().any(|existing| {
                strict_destination_keys(existing)
                    .iter()
                    .any(|key| candidate_keys.contains(key))
            }) {
                return Err(BatchDownloadError::DestinationCollision);
            }
        }
        verify_directory_writable(destination_parent).await?;
        for path in &candidate_paths {
            if tokio::fs::try_exists(path)
                .await
                .map_err(|error| BatchDownloadError::Destination(error.to_string()))?
            {
                return Err(BatchDownloadError::DestinationCollision);
            }
        }

        let request = BatchAdmissionRequest {
            request: BatchDownloadRequest {
                url: canonical_url.clone(),
                package_name: None,
                engine: EngineKind::DirectHttp,
                format_id: None,
                extension: None,
                audio_format: None,
            },
            destination: Some(destination),
            filename: Some(filename.as_str().to_owned()),
            expected_sha256: None,
            require_exact_destination: true,
        };
        let mut reserved_destinations = items
            .iter()
            .map(|item| batch_destination_key(item.destination()))
            .collect();
        self.enqueue_batch_item(
            request,
            DownloadPriority::Normal,
            &mut HashSet::new(),
            &mut reserved_destinations,
        )
        .await
    }

    /// Creates a separate native torrent job from a user-supplied current magnet.
    ///
    /// The magnet's info hash must match persisted aria2 torrent metadata. File
    /// selection is deliberately left to the native job's normal metadata and
    /// confirmation flow; legacy selection and partial data are never reused.
    pub async fn restart_aria2_torrent_with_native(
        &self,
        aria2_id: DownloadId,
        current_magnet_uri: String,
        destination: PathBuf,
        confirmed_current_source: bool,
    ) -> Result<DownloadId, BatchDownloadError> {
        if !confirmed_current_source {
            return Err(BatchDownloadError::RestartNotEligible);
        }
        let legacy_info_hash = self.aria2_torrent_restart_info_hash(aria2_id).await?;

        let magnet = parse_magnet_uri(&current_magnet_uri)
            .map_err(|_| BatchDownloadError::InvalidTorrentRestartSource)?;
        if magnet.info_hash.len() != 40 || !magnet.info_hash.eq_ignore_ascii_case(&legacy_info_hash)
        {
            return Err(BatchDownloadError::TorrentSourceMismatch);
        }
        if !self.engine_runtime.is_available(EngineKind::NativeTorrent) {
            return Err(BatchDownloadError::UnavailableEngine(
                EngineKind::NativeTorrent,
            ));
        }
        self.validate_aria2_torrent_restart_destination(&destination)
            .await?;
        let native_magnet_uri =
            normalize_base32_btih_magnet(&current_magnet_uri, &magnet.info_hash)
                .map_err(|_| BatchDownloadError::InvalidTorrentRestartSource)?;

        self.enqueue_magnet(
            &native_magnet_uri,
            DownloadPriority::Normal,
            Some(destination),
        )
        .await
        .map_err(|error| match error {
            CoreServicesError::UnavailableEngine(engine) => {
                BatchDownloadError::UnavailableEngine(engine)
            }
            CoreServicesError::InvalidMagnet(_) => BatchDownloadError::InvalidTorrentRestartSource,
            CoreServicesError::Queue(message) | CoreServicesError::Database(message) => {
                BatchDownloadError::Queue(message)
            }
            error => BatchDownloadError::Destination(error.to_string()),
        })
    }

    /// Creates a separate native torrent job from a user-selected .torrent file.
    ///
    /// The file's v1 info hash is checked against the persisted aria2 hash, and
    /// only the explicitly reviewed file indexes are admitted. A private copy
    /// is retained as the native job's source; aria2 files are never reused.
    pub async fn restart_aria2_torrent_file_with_native(
        &self,
        aria2_id: DownloadId,
        current_torrent_path: PathBuf,
        destination: PathBuf,
        selected_file_indexes: Vec<u32>,
        confirmed_current_source: bool,
        confirmed_selection: bool,
    ) -> Result<DownloadId, BatchDownloadError> {
        if !confirmed_current_source {
            return Err(BatchDownloadError::RestartNotEligible);
        }
        if !confirmed_selection {
            return Err(BatchDownloadError::InvalidTorrentRestartSelection);
        }

        let legacy_info_hash = self.aria2_torrent_restart_info_hash(aria2_id).await?;
        let (canonical_current_path, source_metadata) = load_torrent_metainfo(current_torrent_path)
            .await
            .map_err(|_| BatchDownloadError::InvalidTorrentRestartSource)?;
        if !source_metadata
            .info_hash
            .eq_ignore_ascii_case(&legacy_info_hash)
        {
            return Err(BatchDownloadError::TorrentSourceMismatch);
        }
        let selection = TorrentFileSelection::from_indices(
            source_metadata.files.len(),
            selected_file_indexes
                .iter()
                .map(|index| usize::try_from(*index))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| BatchDownloadError::InvalidTorrentRestartSelection)?,
        )
        .map_err(|_| BatchDownloadError::InvalidTorrentRestartSelection)?;
        if selection.selected_count() == 0 {
            return Err(BatchDownloadError::InvalidTorrentRestartSelection);
        }
        self.validate_aria2_torrent_restart_destination(&destination)
            .await?;

        let source_directory = self.downloads_dir.join(".fetchx-native-restart-sources");
        fs::create_dir_all(&source_directory)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        let source_copy = source_directory.join(format!("{}.torrent", DownloadId::new()));
        create_restart_source_copy(&canonical_current_path, &source_copy).await?;
        let (_, copied_metadata) = match load_torrent_metainfo(source_copy.clone()).await {
            Ok(metadata) if metadata.1.info_hash.eq_ignore_ascii_case(&legacy_info_hash) => {
                metadata
            }
            Ok(_) => {
                remove_restart_source_copy(&source_copy).await?;
                return Err(BatchDownloadError::TorrentSourceMismatch);
            }
            Err(_) => {
                remove_restart_source_copy(&source_copy).await?;
                return Err(BatchDownloadError::InvalidTorrentRestartSource);
            }
        };
        let selected_file_indexes = selection
            .selected_indices()
            .map(u32::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| BatchDownloadError::InvalidTorrentRestartSelection)?;
        let admission = self
            .enqueue_torrent_file_with_selection_and_engine(
                source_copy.clone(),
                EngineKind::NativeTorrent,
                DownloadPriority::Normal,
                Some(destination),
                Some(selected_file_indexes),
            )
            .await;
        match admission {
            Ok(native_id) => {
                let native_metadata = self
                    .database
                    .torrent_metadata()
                    .get_torrent_metadata(native_id)
                    .await
                    .map_err(|error| BatchDownloadError::Queue(error.to_string()))?
                    .ok_or_else(|| {
                        BatchDownloadError::Queue(
                            "native torrent metadata was not persisted".to_owned(),
                        )
                    })?;
                if !native_metadata
                    .info_hash
                    .eq_ignore_ascii_case(&copied_metadata.info_hash)
                {
                    return Err(BatchDownloadError::Queue(
                        "native torrent metadata did not match the confirmed source".to_owned(),
                    ));
                }
                Ok(native_id)
            }
            Err(error) => {
                remove_restart_source_copy(&source_copy).await?;
                Err(map_torrent_restart_admission_error(error))
            }
        }
    }

    async fn aria2_torrent_restart_info_hash(
        &self,
        aria2_id: DownloadId,
    ) -> Result<String, BatchDownloadError> {
        let original = self
            .repository
            .get(aria2_id)
            .await
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?
            .ok_or(BatchDownloadError::RestartNotEligible)?;
        if original.source().engine() != EngineKind::Aria2
            || matches!(
                original.status(),
                DownloadStatus::Completed | DownloadStatus::Seeding | DownloadStatus::Stopped
            )
        {
            return Err(BatchDownloadError::RestartNotEligible);
        }
        let legacy_metadata = self
            .database
            .torrent_metadata()
            .get_torrent_metadata(aria2_id)
            .await
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?
            .ok_or(BatchDownloadError::RestartNotEligible)?;
        let source_matches = match legacy_metadata.source_type {
            TorrentSourceType::Magnet => original.source().is_magnet(),
            TorrentSourceType::TorrentFile => original.source().is_torrent_file(),
        };
        let info_hash = legacy_metadata.info_hash.to_ascii_lowercase();
        if !source_matches
            || info_hash.len() != 40
            || !info_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || legacy_metadata.destination != *original.destination()
            || legacy_metadata
                .torrent_metadata
                .as_ref()
                .is_some_and(|metadata| !metadata.info_hash.eq_ignore_ascii_case(&info_hash))
        {
            return Err(BatchDownloadError::RestartNotEligible);
        }
        Ok(info_hash)
    }

    async fn validate_aria2_torrent_restart_destination(
        &self,
        destination: &Path,
    ) -> Result<(), BatchDownloadError> {
        if !destination.is_absolute() {
            return Err(BatchDownloadError::InvalidTorrentRestartDestination);
        }
        let directory_name = destination
            .file_name()
            .ok_or(BatchDownloadError::InvalidTorrentRestartDestination)?;
        FileName::new(directory_name.to_string_lossy().into_owned())
            .map_err(|_| BatchDownloadError::InvalidTorrentRestartDestination)?;
        if fs::try_exists(destination)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?
        {
            return Err(BatchDownloadError::TorrentDestinationCollision);
        }
        let canonical_root = fs::canonicalize(&self.downloads_dir)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        let canonical_destination = canonical_destination_identity(destination)
            .await
            .map_err(BatchDownloadError::Destination)?;
        if !canonical_destination.starts_with(&canonical_root) {
            return Err(BatchDownloadError::InvalidTorrentRestartDestination);
        }
        let items = self
            .repository
            .list()
            .await
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?;
        for item in items {
            for existing in [
                item.destination().to_path_buf(),
                append_path_suffix(item.destination(), ".aria2"),
                append_path_suffix(item.destination(), ".part"),
            ] {
                let existing = canonical_destination_identity(&existing)
                    .await
                    .map_err(BatchDownloadError::Destination)?;
                if paths_overlap(&canonical_destination, &existing) {
                    return Err(BatchDownloadError::TorrentDestinationCollision);
                }
            }
        }
        Ok(())
    }

    async fn enqueue_batch_items(
        &self,
        requests: Vec<BatchAdmissionRequest>,
        priority: DownloadPriority,
    ) -> Vec<BatchDownloadOutcome> {
        if requests.is_empty() {
            return Vec::new();
        }

        let mut outcomes = Vec::with_capacity(requests.len());
        let mut seen_urls = HashSet::with_capacity(requests.len());
        let mut reserved_destinations = match self.repository.list().await {
            Ok(items) => items
                .into_iter()
                .map(|item| batch_destination_key(item.destination()))
                .collect(),
            Err(error) => {
                let message = error.to_string();
                return requests
                    .into_iter()
                    .map(|request| BatchDownloadOutcome {
                        input_url: request.request.url,
                        result: Err(BatchDownloadError::Queue(message.clone())),
                    })
                    .collect();
            }
        };

        for request in requests {
            let input_url = request.request.url.clone();
            let result = self
                .enqueue_batch_item(
                    request,
                    priority,
                    &mut seen_urls,
                    &mut reserved_destinations,
                )
                .await;
            outcomes.push(BatchDownloadOutcome { input_url, result });
            tokio::task::yield_now().await;
        }

        outcomes
    }

    async fn enqueue_batch_item(
        &self,
        admission: BatchAdmissionRequest,
        priority: DownloadPriority,
        seen_urls: &mut HashSet<String>,
        reserved_destinations: &mut HashSet<String>,
    ) -> Result<DownloadId, BatchDownloadError> {
        let BatchAdmissionRequest {
            request,
            destination,
            filename: explicit_filename,
            expected_sha256,
            require_exact_destination,
        } = admission;
        let canonical_url =
            validate_http_source_url(&request.url).map_err(|_| BatchDownloadError::InvalidUrl)?;
        if !seen_urls.insert(normalize_download_url(&canonical_url)) {
            return Err(BatchDownloadError::DuplicateUrl);
        }
        if !self.engine_runtime.is_available(request.engine) {
            return Err(BatchDownloadError::UnavailableEngine(request.engine));
        }
        if !valid_batch_format_options(&request) {
            return Err(BatchDownloadError::InvalidFormat);
        }
        if expected_sha256
            .as_deref()
            .is_some_and(|checksum| !is_valid_batch_sha256(checksum.trim()))
        {
            return Err(BatchDownloadError::InvalidChecksum);
        }

        let mut source = DownloadSource::new(&canonical_url, request.engine)
            .map_err(|_| BatchDownloadError::InvalidFormat)?;
        source = match (request.format_id, request.audio_format) {
            (Some(format_id), None) => source.with_yt_dlp_format(format_id),
            (None, Some(audio_format)) => source.with_yt_dlp_audio_format(audio_format),
            (None, None) => Ok(source),
            (Some(_), Some(_)) => Err(fetchx_models::DownloadSourceError::InvalidYtDlpAudioFormat),
        }
        .map_err(|_| BatchDownloadError::InvalidFormat)?;

        let (destination, filename) = if let Some(destination) = destination {
            if !destination.is_absolute() {
                return Err(BatchDownloadError::InvalidDestination);
            }
            let filename = explicit_filename.ok_or(BatchDownloadError::InvalidDestination)?;
            let filename =
                FileName::new(filename).map_err(|_| BatchDownloadError::InvalidDestination)?;
            let destination_extension = destination.extension().and_then(std::ffi::OsStr::to_str);
            let filename_extension = Path::new(filename.as_str())
                .extension()
                .and_then(std::ffi::OsStr::to_str);
            if request.audio_format.is_some_and(|audio_format| {
                destination_extension
                    .is_none_or(|actual| !actual.eq_ignore_ascii_case(audio_format.extension()))
                    || filename_extension
                        .is_none_or(|actual| !actual.eq_ignore_ascii_case(audio_format.extension()))
            }) {
                return Err(BatchDownloadError::InvalidFormat);
            }
            (destination, filename)
        } else {
            let mut filename = suggest_download_filename(&canonical_url);
            if let Some(extension) = request.extension {
                filename = Path::new(&filename)
                    .with_extension(extension)
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .ok_or(BatchDownloadError::InvalidFormat)?
                    .to_owned();
            }
            let filename =
                FileName::new(filename).map_err(|_| BatchDownloadError::InvalidFormat)?;
            let mut destination_dir = self.downloads_dir.clone();
            if let Some(package_name) = request.package_name {
                let package_name = sanitize_download_package_name(&package_name)
                    .ok_or(BatchDownloadError::InvalidPackageName)?;
                destination_dir.push(package_name);
            }
            (destination_dir.join(filename.as_str()), filename)
        };
        let destination_dir = destination
            .parent()
            .ok_or(BatchDownloadError::InvalidDestination)?;
        fs::create_dir_all(&destination_dir)
            .await
            .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
        let (destination, filename) = if require_exact_destination {
            let key = batch_destination_key(&destination);
            if destination.file_name() != Some(std::ffi::OsStr::new(filename.as_str()))
                || !reserved_destinations.insert(key)
            {
                return Err(BatchDownloadError::DestinationCollision);
            }
            (destination, filename)
        } else {
            unique_batch_destination(destination, filename, reserved_destinations).await?
        };

        let checksum_sidecar = expected_sha256.as_deref().map(|_| {
            let mut sidecar = destination.as_os_str().to_os_string();
            sidecar.push(".fetchx.sha256");
            PathBuf::from(sidecar)
        });
        let created_at =
            Timestamp::now().map_err(|error| BatchDownloadError::Queue(error.to_string()))?;
        let mut item =
            DownloadItem::new(DownloadId::new(), source, filename, destination, created_at);
        item.set_priority_at(priority, created_at)
            .map_err(|error| BatchDownloadError::Queue(error.to_string()))?;
        let item_id = item.id();
        if let (Some(checksum), Some(sidecar)) =
            (expected_sha256.as_deref(), checksum_sidecar.as_ref())
        {
            fs::write(sidecar, checksum.trim().to_ascii_lowercase())
                .await
                .map_err(|error| BatchDownloadError::Storage(error.to_string()))?;
        }
        if let Err(error) = self.queue.enqueue(item).await {
            if let Some(sidecar) = checksum_sidecar {
                fs::remove_file(sidecar).await.map_err(|cleanup_error| {
                    BatchDownloadError::Storage(format!(
                        "queue admission failed ({error}) and checksum cleanup failed: {cleanup_error}"
                    ))
                })?;
            }
            return Err(BatchDownloadError::Queue(error.to_string()));
        }
        Ok(item_id)
    }

    /// Applies FetchX's core page discovery. This is equivalent to the built-in
    /// page scraper but runs inside every shared-core host.
    pub async fn discover_archive_url(&self, url: &str) -> Option<String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .ok()?;
        let response = client.get(url).send().await.ok()?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type.contains("text/html") {
            return None;
        }
        let html = response.text().await.ok()?;
        find_archive_link(&html, url)
    }

    /// Resolves the core URL resolver providers. Unknown providers are left
    /// unchanged and return `Ok(None)`.
    pub async fn resolve_core_url(&self, url: &str) -> Result<Option<String>, CoreBuiltinError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| CoreBuiltinError::InvalidResponse)?;
        builtin::resolve_core_url(&client, url).await
    }

    /// Durable torrent metadata repository for aria2 magnet/.torrent downloads.
    #[must_use]
    pub fn torrent_metadata(&self) -> fetchx_storage::SqliteTorrentMetadataRepository {
        self.database.torrent_metadata()
    }

    /// Returns live torrent telemetry only when the native engine owns the task.
    pub async fn torrent_telemetry(
        &self,
        id: DownloadId,
    ) -> Result<Option<fetchx_models::TorrentTelemetry>, CoreServicesError> {
        // Determine which engine owns this download.
        let engine_kind = self
            .repository
            .get(id)
            .await
            .map_err(|e| CoreServicesError::Database(e.to_string()))?
            .map(|item| item.source().engine());

        match engine_kind {
            Some(EngineKind::NativeTorrent) => {
                if let Some(engine) = &self.engine_runtime.native_torrent_engine {
                    Ok(engine.torrent_telemetry(id).await)
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Creates a durable torrent metadata row after a torrent download is queued.
    async fn create_torrent_metadata_row(
        &self,
        params: TorrentMetadataParams,
    ) -> Result<(), CoreServicesError> {
        let TorrentMetadataParams {
            id,
            source_type,
            magnet_uri,
            local_torrent_path,
            info_hash,
            torrent_name,
            torrent_metadata,
            destination,
            selected_file_indexes,
            selected_total_bytes,
            selection_confirmed,
            created_at,
        } = params;
        let row = TorrentMetadataRow {
            download_id: id,
            source_type,
            magnet_uri,
            local_torrent_path,
            info_hash,
            torrent_name,
            torrent_metadata,
            destination,
            metadata_state: if source_type == TorrentSourceType::Magnet {
                "pending".to_owned()
            } else {
                "ready".to_owned()
            },
            selected_file_indexes,
            selected_total_bytes,
            selection_confirmed,
            aria2_gid: None,
            seed_state: TorrentSeedState::Downloading,
            uploaded_bytes: 0,
            upload_speed_bps: 0,
            peer_count: 0,
            seed_count: 0,
            created_at,
            updated_at: created_at,
        };
        self.database
            .torrent_metadata()
            .create_torrent_metadata(&row)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?;
        Ok(())
    }

    async fn enqueue_torrent_item(
        &self,
        item: DownloadItem,
        metadata: TorrentMetadataParams,
    ) -> Result<DownloadId, CoreServicesError> {
        let id = item.id();
        self.queue
            .enqueue_deferred(item)
            .await
            .map_err(|error| CoreServicesError::Queue(error.to_string()))?;
        if let Err(error) = self.create_torrent_metadata_row(metadata).await {
            if let Err(cleanup_error) = self.queue.cancel(id).await {
                warn!(download_id = %id, %cleanup_error, "could not cancel torrent after metadata persistence failure");
            }
            if let Err(cleanup_error) = self.queue.remove_download(id).await {
                warn!(download_id = %id, %cleanup_error, "could not remove incomplete torrent queue item");
            }
            return Err(error);
        }
        if let Err(error) = self.queue.release_download(id).await {
            if let Err(cleanup_error) = self
                .database
                .torrent_metadata()
                .delete_torrent_metadata(id)
                .await
            {
                warn!(download_id = %id, %cleanup_error, "could not remove torrent metadata after queue release failure");
            }
            if let Err(cleanup_error) = self.queue.cancel(id).await {
                warn!(download_id = %id, %cleanup_error, "could not cancel torrent after queue release failure");
            }
            return Err(CoreServicesError::Queue(error.to_string()));
        }
        Ok(id)
    }

    /// Validates and queues a magnet job through the native Rust torrent runtime.
    ///
    /// Magnet output is isolated in a per-job directory unless the caller
    /// explicitly supplies a destination directory. aria2 remains available
    /// through [`Self::enqueue_magnet_with_engine`].
    pub async fn enqueue_magnet(
        &self,
        uri: &str,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
    ) -> Result<DownloadId, CoreServicesError> {
        let magnet = parse_magnet_uri(uri)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        if !self.engine_runtime.is_available(EngineKind::NativeTorrent) {
            return Err(CoreServicesError::UnavailableEngine(
                EngineKind::NativeTorrent,
            ));
        }
        self.enqueue_magnet_with_metadata(
            uri,
            magnet,
            EngineKind::NativeTorrent,
            priority,
            destination,
        )
        .await
    }

    /// Validates and queues a magnet job with the explicitly selected backend.
    ///
    /// This keeps explicit platform engine selection authoritative while
    /// [`Self::enqueue_magnet`] uses native Rust for new downloads by default.
    pub async fn enqueue_magnet_with_engine(
        &self,
        uri: &str,
        engine: EngineKind,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
    ) -> Result<DownloadId, CoreServicesError> {
        let magnet = parse_magnet_uri(uri)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        if !matches!(engine, EngineKind::Aria2 | EngineKind::NativeTorrent) {
            return Err(CoreServicesError::InvalidMagnet(
                "unsupported torrent engine".to_owned(),
            ));
        }
        if !self.engine_runtime.is_available(engine) {
            return Err(CoreServicesError::UnavailableEngine(engine));
        }
        self.enqueue_magnet_with_metadata(uri, magnet, engine, priority, destination)
            .await
    }

    async fn enqueue_magnet_with_metadata(
        &self,
        uri: &str,
        magnet: fetchx_models::MagnetInfo,
        torrent_engine: EngineKind,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
    ) -> Result<DownloadId, CoreServicesError> {
        let id = DownloadId::new();
        let source = DownloadSource::new(uri, torrent_engine)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        let filename = FileName::new(magnet.suggested_filename())
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        let default_dest = self
            .downloads_dir
            .join(format!("torrent-{}-{id}", magnet.info_hash));
        let destination = destination.unwrap_or(default_dest);
        let destination = validate_torrent_destination(&destination, &self.downloads_dir)
            .await
            .map_err(|e| CoreServicesError::InvalidMagnet(e.to_string()))?;
        let metadata_directory = destination
            .parent()
            .ok_or_else(|| CoreServicesError::InvalidMagnet("invalid destination".to_owned()))?
            .join(format!(".fetchx-metadata-{id}"));
        let metadata_directory =
            validate_torrent_destination(&metadata_directory, &self.downloads_dir)
                .await
                .map_err(CoreServicesError::InvalidMagnet)?;
        let created_at = Timestamp::now()
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        let mut item = DownloadItem::new(id, source, filename, destination.clone(), created_at);
        item.set_priority_at(priority, created_at)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        self.enqueue_torrent_item(
            item,
            TorrentMetadataParams {
                id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(uri.to_owned()),
                info_hash: magnet.info_hash.clone(),
                torrent_name: magnet.suggested_filename(),
                torrent_metadata: None,
                local_torrent_path: Some(
                    metadata_directory.join(format!("{}.torrent", magnet.info_hash)),
                ),
                destination,
                selected_file_indexes: Vec::new(),
                selected_total_bytes: 0,
                selection_confirmed: false,
                created_at,
            },
        )
        .await
    }

    /// Validates and queues a local `.torrent` file through the selected torrent runtime.
    ///
    /// The download destination is a directory. If omitted, each torrent is
    /// isolated in a directory named from its info hash and download ID.
    pub async fn enqueue_torrent_file(
        &self,
        torrent_path: PathBuf,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
    ) -> Result<DownloadId, CoreServicesError> {
        self.enqueue_torrent_file_with_selection(torrent_path, priority, destination, None)
            .await
    }

    /// Parses a local torrent file for the shared file-selection UI.
    pub async fn inspect_torrent_file(
        &self,
        torrent_path: PathBuf,
    ) -> Result<TorrentMetadata, CoreServicesError> {
        load_torrent_metainfo(torrent_path)
            .await
            .map(|(_, metadata)| metadata)
    }

    /// Validates and queues a local `.torrent` file with a durable file selection.
    ///
    /// The selection uses zero-based file indexes from the parsed metainfo.
    pub async fn enqueue_torrent_file_with_selection(
        &self,
        torrent_path: PathBuf,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
        selected_file_indexes: Option<Vec<u32>>,
    ) -> Result<DownloadId, CoreServicesError> {
        self.enqueue_torrent_file_with_selection_inner(
            torrent_path,
            None,
            priority,
            destination,
            selected_file_indexes,
        )
        .await
    }

    /// Queues a local torrent file using the explicitly selected backend.
    ///
    /// The selection uses zero-based file indexes from the parsed metainfo.
    pub async fn enqueue_torrent_file_with_selection_and_engine(
        &self,
        torrent_path: PathBuf,
        engine: EngineKind,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
        selected_file_indexes: Option<Vec<u32>>,
    ) -> Result<DownloadId, CoreServicesError> {
        self.enqueue_torrent_file_with_selection_inner(
            torrent_path,
            Some(engine),
            priority,
            destination,
            selected_file_indexes,
        )
        .await
    }

    async fn enqueue_torrent_file_with_selection_inner(
        &self,
        torrent_path: PathBuf,
        engine: Option<EngineKind>,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
        selected_file_indexes: Option<Vec<u32>>,
    ) -> Result<DownloadId, CoreServicesError> {
        let (torrent_path, torrent) = load_torrent_metainfo(torrent_path).await?;
        // Validate destination BEFORE engine availability check so security
        // errors are reported even when aria2 is not running.
        let id = DownloadId::new();
        let default_dest = self
            .downloads_dir
            .join(format!("torrent-{}-{id}", torrent.info_hash));
        let destination = destination.unwrap_or(default_dest);
        let destination = validate_torrent_destination(&destination, &self.downloads_dir)
            .await
            .map_err(|e| CoreServicesError::InvalidTorrent(e.to_string()))?;
        let torrent_engine = match engine {
            Some(engine) if !matches!(engine, EngineKind::Aria2 | EngineKind::NativeTorrent) => {
                return Err(CoreServicesError::InvalidTorrent(
                    "unsupported torrent engine".to_owned(),
                ));
            }
            Some(engine) if !self.engine_runtime.is_available(engine) => {
                return Err(CoreServicesError::UnavailableEngine(engine));
            }
            Some(engine) => engine,
            None if !self.engine_runtime.is_available(EngineKind::NativeTorrent) => {
                return Err(CoreServicesError::UnavailableEngine(
                    EngineKind::NativeTorrent,
                ));
            }
            None => EngineKind::NativeTorrent,
        };
        let source = DownloadSource::new_torrent_file(&torrent_path, torrent_engine)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let torrent_name_for_meta = torrent.name.clone();
        let torrent_info_hash_for_meta = torrent.info_hash.clone();
        let selection = match selected_file_indexes {
            Some(indexes) => {
                let indexes = indexes
                    .into_iter()
                    .map(usize::try_from)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| CoreServicesError::InvalidTorrent("invalid file index".into()))?;
                TorrentFileSelection::from_indices(torrent.files.len(), indexes)
                    .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?
            }
            None => torrent.selection_all(),
        };
        if selection.selected_count() == 0 {
            return Err(CoreServicesError::InvalidTorrent(
                "at least one torrent file must be selected".to_owned(),
            ));
        }
        let selected_total_size = selection
            .selected_size(&torrent)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let filename = FileName::new(torrent.name.clone())
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let created_at = Timestamp::now()
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let mut item = DownloadItem::new(id, source, filename, destination.clone(), created_at);
        item.set_priority_at(priority, created_at)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let selected_indexes: Vec<u32> = selection
            .selected_indices()
            .map(u32::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                CoreServicesError::InvalidTorrent("torrent has too many files to index".to_owned())
            })?;
        self.enqueue_torrent_item(
            item,
            TorrentMetadataParams {
                id,
                source_type: TorrentSourceType::TorrentFile,
                magnet_uri: None,
                local_torrent_path: Some(torrent_path),
                info_hash: torrent_info_hash_for_meta,
                torrent_name: torrent_name_for_meta,
                torrent_metadata: Some(torrent.clone()),
                destination,
                selected_file_indexes: selected_indexes,
                selected_total_bytes: selected_total_size,
                selection_confirmed: true,
                created_at,
            },
        )
        .await
    }

    /// Stops torrent seeding for the given download, transitioning it to Stopped.
    pub async fn stop_torrent_seeding(&self, id: DownloadId) -> Result<(), CoreServicesError> {
        let item = self
            .repository
            .get(id)
            .await
            .map_err(|e| CoreServicesError::Queue(e.to_string()))?
            .ok_or_else(|| CoreServicesError::Queue(format!("download {id} not found")))?;
        if item.status() != fetchx_models::DownloadStatus::Seeding {
            return Err(CoreServicesError::Queue(format!(
                "download {id} is not seeding (status: {:?})",
                item.status()
            )));
        }
        match item.source().engine() {
            EngineKind::NativeTorrent => {
                if let Some(engine) = self.engine_runtime.native_torrent_engine.as_ref() {
                    engine
                        .stop_seeding(id)
                        .await
                        .map_err(|error| CoreServicesError::Engine(error.to_string()))?;
                }
                // If the engine is not available (e.g. session restart without
                // NativeTorrent), the handle is already gone — just transition.
            }
            EngineKind::Aria2 => {
                return Err(CoreServicesError::UnavailableEngine(EngineKind::Aria2));
            }
            engine => {
                return Err(CoreServicesError::Queue(format!(
                    "download {id} uses unsupported torrent engine {engine:?}"
                )));
            }
        }
        fetchx_core::DownloadStateMachine::new(self.repository.clone())
            .transition(id, fetchx_models::DownloadStatus::Stopped)
            .await
            .map_err(|e| CoreServicesError::Queue(e.to_string()))?;
        let mut meta_row = self
            .database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await
            .map_err(|e| CoreServicesError::Database(e.to_string()))?
            .ok_or_else(|| {
                CoreServicesError::Database(format!("torrent metadata for {id} was not found"))
            })?;
        meta_row.seed_state = fetchx_storage::TorrentSeedState::Stopped;
        meta_row.aria2_gid = None;
        meta_row.updated_at = Timestamp::now()
            .map_err(|error| CoreServicesError::Queue(error.to_string()))?
            .max(meta_row.updated_at);
        self.database
            .torrent_metadata()
            .update_torrent_metadata(&meta_row)
            .await
            .map_err(|e| CoreServicesError::Database(e.to_string()))?;
        Ok(())
    }

    /// Returns metadata and the persisted selection once aria2 has acquired a magnet's metainfo.
    pub async fn torrent_download_metadata(
        &self,
        id: DownloadId,
    ) -> Result<Option<fetchx_models::TorrentDownloadMetadata>, CoreServicesError> {
        let row = self
            .database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?;
        Ok(row.map(|row| fetchx_models::TorrentDownloadMetadata {
            metadata_state: row.metadata_state,
            metadata: row.torrent_metadata,
            selected_file_indexes: row.selected_file_indexes,
            selected_total_bytes: row.selected_total_bytes,
            selection_confirmed: row.selection_confirmed,
        }))
    }

    /// Returns remote-safe metadata for a native magnet without exposing its
    /// magnet URI, destination, or tracker URLs.
    pub async fn native_magnet_metadata(
        &self,
        id: DownloadId,
    ) -> Result<Option<NativeMagnetMetadata>, CoreServicesError> {
        let Some(item) = self
            .repository
            .get(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
        else {
            return Ok(None);
        };
        if item.source().engine() != EngineKind::NativeTorrent {
            return Err(CoreServicesError::InvalidTorrent(
                "download is not using the native torrent engine".to_owned(),
            ));
        }
        let row = self
            .database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| {
                CoreServicesError::InvalidTorrent("torrent metadata is unavailable".to_owned())
            })?;
        if row.source_type != TorrentSourceType::Magnet {
            return Err(CoreServicesError::InvalidTorrent(
                "download is not a magnet".to_owned(),
            ));
        }
        if row.metadata_state == "ready"
            && row
                .torrent_metadata
                .as_ref()
                .is_none_or(|metadata| !metadata.info_hash.eq_ignore_ascii_case(&row.info_hash))
        {
            return Err(CoreServicesError::StaleTorrentMetadata);
        }
        let (name, files) = match row.torrent_metadata {
            Some(metadata) => (
                Some(metadata.name),
                metadata
                    .files
                    .into_iter()
                    .enumerate()
                    .map(|(index, file)| {
                        u32::try_from(index)
                            .map(|index| NativeMagnetFile {
                                index,
                                path: file.path,
                                size: file.size,
                            })
                            .map_err(|_| {
                                CoreServicesError::InvalidTorrent(
                                    "torrent contains too many files".to_owned(),
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => (None, Vec::new()),
        };
        Ok(Some(NativeMagnetMetadata {
            metadata_state: row.metadata_state,
            info_hash: row.info_hash,
            name,
            files,
            selected_file_indexes: row.selected_file_indexes,
            selected_total_bytes: row.selected_total_bytes,
            selection_confirmed: row.selection_confirmed,
        }))
    }

    /// Confirms a native magnet file selection against the metadata version
    /// shown to the remote client.
    pub async fn select_native_magnet_files(
        &self,
        id: DownloadId,
        expected_info_hash: &str,
        selected_file_indexes: Vec<u32>,
    ) -> Result<(), CoreServicesError> {
        if expected_info_hash.len() != 40
            || !expected_info_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreServicesError::InvalidTorrent(
                "invalid torrent info hash".to_owned(),
            ));
        }
        let mut seen = HashSet::with_capacity(selected_file_indexes.len());
        if selected_file_indexes.is_empty()
            || selected_file_indexes
                .iter()
                .any(|index| !seen.insert(*index))
        {
            return Err(CoreServicesError::InvalidTorrent(
                "file selection must contain unique file indexes".to_owned(),
            ));
        }

        let Some(item) = self
            .repository
            .get(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
        else {
            return Err(CoreServicesError::TorrentDownloadNotFound);
        };
        if item.source().engine() != EngineKind::NativeTorrent {
            return Err(CoreServicesError::InvalidTorrent(
                "download is not using the native torrent engine".to_owned(),
            ));
        }
        if item.status() != DownloadStatus::MetadataAcquiring {
            return Err(CoreServicesError::TorrentSelectionConflict);
        }
        let row = self
            .database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or(CoreServicesError::TorrentDownloadNotFound)?;
        if row.source_type != TorrentSourceType::Magnet {
            return Err(CoreServicesError::InvalidTorrent(
                "download is not a magnet".to_owned(),
            ));
        }
        let metadata = row
            .torrent_metadata
            .as_ref()
            .filter(|_| row.metadata_state == "ready")
            .ok_or(CoreServicesError::TorrentSelectionConflict)?;
        if !row.info_hash.eq_ignore_ascii_case(expected_info_hash)
            || !metadata.info_hash.eq_ignore_ascii_case(expected_info_hash)
        {
            return Err(CoreServicesError::StaleTorrentMetadata);
        }
        let indexes = selected_file_indexes
            .iter()
            .map(|index| {
                usize::try_from(*index)
                    .map_err(|_| CoreServicesError::InvalidTorrent("invalid file index".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let selection = TorrentFileSelection::from_indices(metadata.files.len(), indexes)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        self.save_magnet_selection(id, selection, true, Some(expected_info_hash), true)
            .await
    }

    /// Persists a zero-based magnet file selection for the queued metadata-ready task.
    ///
    /// An empty selection deliberately leaves the task waiting; select one or more
    /// files later to begin the transfer.
    pub async fn select_torrent_files(
        &self,
        id: DownloadId,
        selected_file_indexes: Vec<u32>,
    ) -> Result<(), CoreServicesError> {
        let mut seen = std::collections::HashSet::with_capacity(selected_file_indexes.len());
        if selected_file_indexes
            .iter()
            .any(|index| !seen.insert(*index))
        {
            return Err(CoreServicesError::InvalidTorrent(
                "file selection contains duplicate indexes".to_owned(),
            ));
        }
        let metadata = self
            .database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .and_then(|row| row.torrent_metadata);
        let metadata = metadata.ok_or_else(|| {
            CoreServicesError::InvalidTorrent("torrent metadata is not ready".to_owned())
        })?;
        let indexes = selected_file_indexes
            .iter()
            .map(|index| {
                usize::try_from(*index)
                    .map_err(|_| CoreServicesError::InvalidTorrent("invalid file index".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let selection = TorrentFileSelection::from_indices(metadata.files.len(), indexes)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        self.save_magnet_selection(id, selection, true, None, false)
            .await
    }

    pub async fn set_torrent_selection_all(
        &self,
        id: DownloadId,
        selected: bool,
    ) -> Result<(), CoreServicesError> {
        let repository = self.database.torrent_metadata();
        let row = repository
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| CoreServicesError::InvalidTorrent("torrent not found".to_owned()))?;
        let metadata = row.torrent_metadata.as_ref().ok_or_else(|| {
            CoreServicesError::InvalidTorrent("torrent metadata is missing".into())
        })?;
        let selection = if selected {
            metadata.selection_all()
        } else {
            metadata.selection_none()
        };
        self.save_magnet_selection(id, selection, false, None, false)
            .await
    }

    pub async fn set_torrent_file_selected(
        &self,
        id: DownloadId,
        file_index: u32,
        selected: bool,
    ) -> Result<(), CoreServicesError> {
        let repository = self.database.torrent_metadata();
        let row = repository
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| CoreServicesError::InvalidTorrent("torrent not found".to_owned()))?;
        let metadata = row.torrent_metadata.as_ref().ok_or_else(|| {
            CoreServicesError::InvalidTorrent("torrent metadata is missing".into())
        })?;
        let indexes = row
            .selected_file_indexes
            .iter()
            .map(|index| {
                usize::try_from(*index)
                    .map_err(|_| CoreServicesError::InvalidTorrent("invalid file index".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut selection = TorrentFileSelection::from_indices(metadata.files.len(), indexes)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        selection
            .set_selected(
                usize::try_from(file_index)
                    .map_err(|_| CoreServicesError::InvalidTorrent("invalid file index".into()))?,
                selected,
            )
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        self.save_magnet_selection(id, selection, false, None, false)
            .await
    }

    pub async fn set_torrent_directory_selected(
        &self,
        id: DownloadId,
        directory: String,
        selected: bool,
    ) -> Result<(), CoreServicesError> {
        let repository = self.database.torrent_metadata();
        let row = repository
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| CoreServicesError::InvalidTorrent("torrent not found".to_owned()))?;
        let metadata = row.torrent_metadata.as_ref().ok_or_else(|| {
            CoreServicesError::InvalidTorrent("torrent metadata is missing".into())
        })?;
        let directory = directory
            .split(['/', '\\'])
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if directory.is_empty() || directory.iter().any(String::is_empty) {
            return Err(CoreServicesError::InvalidTorrent(
                "invalid torrent directory selection".to_owned(),
            ));
        }
        let indexes = row
            .selected_file_indexes
            .iter()
            .map(|index| {
                usize::try_from(*index)
                    .map_err(|_| CoreServicesError::InvalidTorrent("invalid file index".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut selection = TorrentFileSelection::from_indices(metadata.files.len(), indexes)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        selection
            .select_directory(metadata, &directory, selected)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        self.save_magnet_selection(id, selection, false, None, false)
            .await
    }

    async fn save_magnet_selection(
        &self,
        id: DownloadId,
        selection: TorrentFileSelection,
        confirmed: bool,
        expected_info_hash: Option<&str>,
        require_native_engine: bool,
    ) -> Result<(), CoreServicesError> {
        let repository = self.database.torrent_metadata();
        let mut row = repository
            .get_torrent_metadata(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| CoreServicesError::InvalidTorrent("torrent not found".to_owned()))?;
        if row.source_type != TorrentSourceType::Magnet
            || row.metadata_state != "ready"
            || row.torrent_metadata.is_none()
        {
            if require_native_engine {
                return Err(CoreServicesError::TorrentSelectionConflict);
            }
            return Err(CoreServicesError::InvalidTorrent(
                "magnet metadata is not ready".to_owned(),
            ));
        }
        if let Some(expected_info_hash) = expected_info_hash {
            let metadata = row
                .torrent_metadata
                .as_ref()
                .ok_or(CoreServicesError::TorrentSelectionConflict)?;
            if !row.info_hash.eq_ignore_ascii_case(expected_info_hash)
                || !metadata.info_hash.eq_ignore_ascii_case(expected_info_hash)
            {
                return Err(CoreServicesError::StaleTorrentMetadata);
            }
        }
        let item = self
            .repository
            .get(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| CoreServicesError::InvalidTorrent("download not found".to_owned()))?;
        if item.status() != fetchx_models::DownloadStatus::MetadataAcquiring {
            if require_native_engine {
                return Err(CoreServicesError::TorrentSelectionConflict);
            }
            return Err(CoreServicesError::InvalidTorrent(
                "file selection is only available while awaiting magnet selection".to_owned(),
            ));
        }
        if require_native_engine && item.source().engine() != EngineKind::NativeTorrent {
            return Err(CoreServicesError::InvalidTorrent(
                "download is not using the native torrent engine".to_owned(),
            ));
        }
        let metadata = row.torrent_metadata.as_ref().ok_or_else(|| {
            CoreServicesError::InvalidTorrent("torrent metadata is missing".into())
        })?;
        row.selected_total_bytes = selection
            .selected_size(metadata)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        row.selected_file_indexes = selection
            .selected_indices()
            .map(u32::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                CoreServicesError::InvalidTorrent("torrent contains too many files".to_owned())
            })?;
        row.selection_confirmed = confirmed;
        row.updated_at = Timestamp::now()
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?
            .max(row.updated_at);
        repository
            .update_torrent_metadata(&row)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))
    }

    // ── Shutdown ──────────────────────────────────────────────────────────

    /// Gracefully shuts down all services. Repeated and concurrent calls are safe.
    ///
    /// Order: scheduler/queue → optional engines. Storage closes when the service graph drops.
    pub async fn shutdown(&self) -> Result<(), CoreServicesError> {
        let mut shutdown_complete = self.shutdown_complete.lock().await;
        if *shutdown_complete {
            return Ok(());
        }
        let scheduler_result = self.scheduler.shutdown().await;
        let engine_result = self.engine_runtime.shutdown().await;
        *shutdown_complete = true;

        if let Err(error) = scheduler_result {
            return Err(CoreServicesError::Shutdown(error.to_string()));
        }
        if let Err(error) = engine_result {
            return Err(CoreServicesError::Shutdown(error.to_string()));
        }
        Ok(())
    }
}

// ============================================================
// Shared engine runtime
// ============================================================

pub struct EngineRuntime {
    pub dispatcher: Arc<EngineDispatcher<CoreRepository>>,
    pub native_torrent_engine: Option<Arc<NativeTorrentEngine<CoreRepository>>>,
    pub ftp_engine: Arc<FtpEngine<CoreRepository>>,
    pub sftp_engine: Arc<SftpEngine<CoreRepository>>,
    pub metalink_engine: Arc<MetalinkEngine<CoreRepository>>,
    pub status: EngineRuntimeStatus,
    pub media_registry: Arc<ExtractorRegistry>,
}

impl EngineRuntime {
    pub async fn initialize(
        repository: CoreRepository,
        config: &EngineConfig,
        runtime_dir: &std::path::Path,
    ) -> Result<Self, CoreServicesError> {
        Self::initialize_with_ytdlp_runner(repository, config, runtime_dir, None, None).await
    }

    async fn initialize_with_ytdlp_runner(
        repository: CoreRepository,
        config: &EngineConfig,
        runtime_dir: &std::path::Path,
        ytdlp_runner: Option<Arc<dyn YtDlpProcessRunner>>,
        torrent_metadata: Option<fetchx_storage::SqliteTorrentMetadataRepository>,
    ) -> Result<Self, CoreServicesError> {
        let direct_http = Arc::new(
            DirectHttpEngine::new(repository.clone())
                .map_err(|e| CoreServicesError::Engine(e.to_string()))?,
        );

        let hls_engine = Arc::new(
            HlsEngine::new(repository.clone())
                .map_err(|e| CoreServicesError::Engine(e.to_string()))?,
        );

        let native_torrent_metadata = torrent_metadata.clone();
        // Clone the runner before moving it into init_ytdlp so we can also
        // pass it to YtDlpProcessRunnerAdapter for the media extraction registry.
        let ytdlp_runner_for_registry = ytdlp_runner.clone();
        let (yt_dlp_engine, ytdlp_avail) =
            init_ytdlp(repository.clone(), config, runtime_dir, ytdlp_runner).await;
        let (native_torrent_engine, native_torrent_avail) =
            init_native_torrent(repository.clone(), config, native_torrent_metadata).await;

        log_avail("yt-dlp", &ytdlp_avail);
        log_avail("native-torrent", &native_torrent_avail);
        info!(engine = "hls", "engine ready");

        // FTP/FTPS engine — always available (pure Rust, no binary dep).
        let ftp_engine = Arc::new(FtpEngine::new(repository.clone()));
        let ftp_avail = if config.ftp.enabled {
            info!(engine = "ftp", version = "suppaftp/6.3.0", "engine ready");
            EngineAvailability::Available {
                version: "suppaftp/6.3.0".to_owned(),
            }
        } else {
            info!(engine = "ftp", "engine disabled");
            EngineAvailability::Disabled
        };

        // SFTP engine — always available (russh is pure Rust, no binary dep).
        let sftp_engine = Arc::new(SftpEngine::new(repository.clone()));
        let sftp_avail = if config.sftp.enabled {
            info!(engine = "sftp", version = "russh/0.63.1", "engine ready");
            EngineAvailability::Available {
                version: "russh/0.63.1".to_owned(),
            }
        } else {
            info!(engine = "sftp", "engine disabled");
            EngineAvailability::Disabled
        };

        // Metalink engine — always available (pure Rust, no binary dep).
        let metalink_engine = Arc::new(MetalinkEngine::new(
            repository.clone(),
            Arc::clone(&direct_http),
        ));
        let metalink_avail = if config.metalink.enabled {
            info!(
                engine = "metalink",
                version = "orchestrator",
                "engine ready"
            );
            EngineAvailability::Available {
                version: "orchestrator".to_owned(),
            }
        } else {
            info!(engine = "metalink", "engine disabled");
            EngineAvailability::Disabled
        };

        let mut dispatcher = EngineDispatcher::new(repository, direct_http)
            .with_hls(hls_engine)
            .with_ftp(Arc::clone(&ftp_engine))
            .with_sftp(Arc::clone(&sftp_engine))
            .with_metalink(Arc::clone(&metalink_engine));

        // Capture the resolved executable BEFORE moving the engine into the dispatcher.
        let ytdlp_executable: Option<std::path::PathBuf> =
            yt_dlp_engine.as_ref().map(|e| e.executable().to_path_buf());

        if let Some(engine) = yt_dlp_engine {
            dispatcher = dispatcher.with_yt_dlp(engine);
        }
        if let Some(ref engine) = native_torrent_engine {
            dispatcher = dispatcher.with_native_torrent(Arc::clone(engine));
        }

        // ── Media extraction registry ────────────────────────────────────
        // Build the ExtractorRegistry here in the composition root.
        // The adapter is built from the resolved executable path: when a runner
        // is injected (e.g. Android's in-process Chaquopy runner) it delegates
        // to that runner; otherwise it spawns yt-dlp itself (desktop/CLI).
        let media_registry = {
            let mut registry = ExtractorRegistry::new();
            registry.register(Arc::new(YoutubeExtractor));
            registry.register(Arc::new(JwPlatformExtractor));
            registry.register(Arc::new(BrightcoveExtractor));
            registry.register(Arc::new(CbcExtractor));
            registry.register(Arc::new(KalturaExtractor));
            registry.register(Arc::new(VimeoExtractor));
            registry.register(Arc::new(PolskieRadioExtractor));
            registry.register(Arc::new(RadioFranceExtractor));
            registry.register(Arc::new(GenericExtractor));
            if matches!(&ytdlp_avail, EngineAvailability::Available { .. }) {
                // Use the resolved executable path captured before the engine was moved.
                let executable = ytdlp_executable.unwrap_or_else(|| {
                    // Fallback: use config path (shouldn't reach here when Available).
                    if let Some(p) = &config.yt_dlp.executable_path {
                        p.clone()
                    } else if cfg!(target_os = "windows") {
                        "yt-dlp.exe".into()
                    } else {
                        "yt-dlp".into()
                    }
                });
                let adapter = Arc::new(if let Some(runner) = ytdlp_runner_for_registry {
                    YtDlpProcessRunnerAdapter::new(runner, executable.clone())
                } else {
                    YtDlpProcessRunnerAdapter::from_executable(executable.clone())
                });
                let extractor = Arc::new(YtDlpExtractor::new(adapter, executable));
                registry.register(extractor);
            } else {
                registry.register(Arc::new(UnsupportedExtractor));
            }
            Arc::new(registry)
        };

        Ok(Self {
            dispatcher: Arc::new(dispatcher),
            native_torrent_engine,
            ftp_engine,
            sftp_engine,
            metalink_engine,
            status: EngineRuntimeStatus {
                yt_dlp: ytdlp_avail,
                native_torrent: native_torrent_avail,
                ftp: ftp_avail,
                sftp: sftp_avail,
                metalink: metalink_avail,
            },
            media_registry,
        })
    }

    pub async fn shutdown(&self) -> Result<(), CoreServicesError> {
        if let Some(engine) = &self.native_torrent_engine {
            engine.session().stop().await;
            info!("native torrent session stopped");
        }
        Ok(())
    }

    /// `true` when `kind` is currently available at runtime.
    pub fn is_available(&self, kind: EngineKind) -> bool {
        match kind {
            EngineKind::DirectHttp | EngineKind::Hls => true,
            EngineKind::Aria2 => false,
            EngineKind::YtDlp => matches!(self.status.yt_dlp, EngineAvailability::Available { .. }),
            EngineKind::NativeTorrent => {
                matches!(
                    self.status.native_torrent,
                    EngineAvailability::Available { .. }
                )
            }
            EngineKind::Ftp | EngineKind::Ftps => {
                matches!(self.status.ftp, EngineAvailability::Available { .. })
            }
            EngineKind::Sftp => {
                matches!(self.status.sftp, EngineAvailability::Available { .. })
            }
            EngineKind::Metalink => {
                matches!(self.status.metalink, EngineAvailability::Available { .. })
            }
        }
    }
}

fn bundled_engine_defaults(directory: Option<&std::path::Path>) -> EngineConfig {
    let mut config = EngineConfig::default();
    let Some(directory) = directory else {
        return config;
    };

    let ytdlp = directory.join(if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    if ytdlp.is_file() {
        config.yt_dlp.enabled = true;
        config.yt_dlp.executable_path = Some(ytdlp);
    }

    config
}

fn log_avail(name: &str, avail: &EngineAvailability) {
    match avail {
        EngineAvailability::Available { version } => info!(engine = name, %version, "engine ready"),
        EngineAvailability::Unavailable { reason } => {
            warn!(engine = name, %reason, "engine unavailable")
        }
        EngineAvailability::Disabled => info!(engine = name, "engine disabled"),
    }
}

// ── yt-dlp init ──────────────────────────────────────────────────────────

async fn init_ytdlp(
    repository: CoreRepository,
    config: &EngineConfig,
    runtime_dir: &std::path::Path,
    runner: Option<Arc<dyn YtDlpProcessRunner>>,
) -> (Option<Arc<YtDlpEngine<CoreRepository>>>, EngineAvailability) {
    if !config.yt_dlp.enabled {
        return (None, EngineAvailability::Disabled);
    }
    let mut ytdlp_cfg = DownloaderYtDlpConfig::new();
    if let Some(p) = &config.yt_dlp.executable_path {
        ytdlp_cfg = ytdlp_cfg.with_executable(p.clone());
    }
    if runner.is_none() {
        ytdlp_cfg = ytdlp_cfg.with_version_cache_path(runtime_dir.join("yt-dlp-version.json"));
    }
    let initialized = if let Some(runner) = runner {
        YtDlpEngine::initialize_with_runner_and_executable(
            repository,
            ytdlp_cfg,
            "fetchx-embedded-yt-dlp",
            runner,
        )
        .await
    } else {
        YtDlpEngine::initialize(repository, ytdlp_cfg).await
    };
    match initialized {
        Ok(engine) => {
            let version = engine.version().to_owned();
            (
                Some(Arc::new(engine)),
                EngineAvailability::Available { version },
            )
        }
        Err(e) => (
            None,
            EngineAvailability::Unavailable {
                reason: e.to_string(),
            },
        ),
    }
}

// ── native-torrent init ───────────────────────────────────────────────────

async fn init_native_torrent(
    repository: CoreRepository,
    config: &EngineConfig,
    torrent_metadata: Option<fetchx_storage::SqliteTorrentMetadataRepository>,
) -> (
    Option<Arc<NativeTorrentEngine<CoreRepository>>>,
    EngineAvailability,
) {
    if !config.native_torrent.enabled {
        return (None, EngineAvailability::Disabled);
    }
    let native_cfg = DownloaderNativeTorrentConfig {
        listen_port: config.native_torrent.listen_port,
        max_connections_per_torrent: 20,
        session_dir: config.native_torrent.session_dir.clone(),
        disable_dht: false,
    };
    match NativeTorrentEngine::new(repository, native_cfg).await {
        Ok(engine) => {
            let engine = if let Some(torrent_metadata) = torrent_metadata {
                engine.with_torrent_metadata_repository(torrent_metadata)
            } else {
                engine
            };
            let engine = Arc::new(engine);
            info!("NativeTorrentEngine initialized successfully");
            (
                Some(engine),
                EngineAvailability::Available {
                    version: "9.0.1".to_owned(),
                },
            )
        }
        Err(e) => {
            error!("NativeTorrentEngine failed to initialize: {e}");
            (
                None,
                EngineAvailability::Unavailable {
                    reason: e.to_string(),
                },
            )
        }
    }
}

// ============================================================
// Filesystem security for torrent destinations
// ============================================================

/// Validates a torrent destination directory before aria2 submission.
///
/// Rejects:
/// - Paths that escape `downloads_dir` via traversal
/// - UNC paths (`\\server\share`) on Windows
/// - Symlinks and Windows junctions/reparse points
/// - Absolute paths that are not under `downloads_dir`
///
/// Creates the destination directory if it does not exist yet.
/// Returns the canonicalized destination path on success.
async fn load_torrent_metainfo(
    torrent_path: PathBuf,
) -> Result<(PathBuf, TorrentMetadata), CoreServicesError> {
    if !torrent_path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
    {
        return Err(CoreServicesError::InvalidTorrent(
            "source must have a .torrent extension".to_owned(),
        ));
    }
    let file_metadata = fs::symlink_metadata(&torrent_path)
        .await
        .map_err(CoreServicesError::Io)?;
    if !file_metadata.file_type().is_file() || file_metadata.len() > 8 * 1024 * 1024 {
        return Err(CoreServicesError::InvalidTorrent(
            "source must be a regular torrent file no larger than 8 MiB".to_owned(),
        ));
    }
    let mut torrent_bytes = Vec::new();
    fs::File::open(&torrent_path)
        .await
        .map_err(CoreServicesError::Io)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut torrent_bytes)
        .await
        .map_err(CoreServicesError::Io)?;
    if torrent_bytes.len() > 8 * 1024 * 1024 {
        return Err(CoreServicesError::InvalidTorrent(
            "torrent file exceeds 8 MiB".to_owned(),
        ));
    }
    let torrent = parse_torrent_metainfo(&torrent_bytes)
        .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
    let canonical_path = fs::canonicalize(torrent_path)
        .await
        .map_err(CoreServicesError::Io)?;
    Ok((canonical_path, torrent))
}

fn normalize_base32_btih_magnet(uri: &str, canonical_info_hash: &str) -> Result<String, ()> {
    let mut parsed = Url::parse(uri).map_err(|_| ())?;
    let pairs = parsed
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if !pairs.iter().any(|(key, value)| {
        key == "xt"
            && value
                .strip_prefix("urn:btih:")
                .is_some_and(|hash| hash.len() == 32)
    }) {
        return Ok(uri.to_owned());
    }

    parsed.set_query(None);
    {
        let mut query = parsed.query_pairs_mut();
        for (key, value) in pairs {
            if key == "xt" {
                query.append_pair("xt", &format!("urn:btih:{canonical_info_hash}"));
            } else {
                query.append_pair(&key, &value);
            }
        }
    }
    Ok(parsed.into())
}

async fn remove_restart_source_copy(path: &Path) -> Result<(), BatchDownloadError> {
    fs::remove_file(path)
        .await
        .map_err(|error| BatchDownloadError::Destination(error.to_string()))
}

async fn create_restart_source_copy(
    source: &Path,
    destination: &Path,
) -> Result<(), BatchDownloadError> {
    let mut bytes = Vec::new();
    fs::File::open(source)
        .await
        .map_err(|error| BatchDownloadError::Destination(error.to_string()))?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return Err(BatchDownloadError::InvalidTorrentRestartSource);
    }

    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|error| BatchDownloadError::Destination(error.to_string()))?;
    let result = async {
        output.write_all(&bytes).await?;
        output.sync_all().await
    }
    .await;
    if let Err(error) = result {
        drop(output);
        remove_restart_source_copy(destination).await?;
        return Err(BatchDownloadError::Destination(error.to_string()));
    }
    Ok(())
}

fn map_torrent_restart_admission_error(error: CoreServicesError) -> BatchDownloadError {
    match error {
        CoreServicesError::UnavailableEngine(engine) => {
            BatchDownloadError::UnavailableEngine(engine)
        }
        CoreServicesError::Queue(message) | CoreServicesError::Database(message) => {
            BatchDownloadError::Queue(message)
        }
        CoreServicesError::InvalidTorrent(_) => BatchDownloadError::InvalidTorrentRestartSource,
        error => BatchDownloadError::Destination(error.to_string()),
    }
}

async fn validate_torrent_destination(
    destination: &std::path::Path,
    downloads_dir: &std::path::Path,
) -> Result<PathBuf, String> {
    // Reject UNC paths on Windows (\\server\share).
    #[cfg(windows)]
    {
        if is_unc_path(destination) {
            return Err("UNC paths are not permitted as torrent destinations".to_owned());
        }
    }

    let canonical_root = tokio::fs::canonicalize(downloads_dir)
        .await
        .map_err(|e| format!("cannot canonicalize downloads dir: {e}"))?;

    // Validate every existing ancestor before creating any missing directories.
    // This prevents a rejected traversal or symlink from causing writes outside
    // the configured root.
    let mut ancestor = Some(destination);
    let mut found_root = false;
    // Collect path components that do not yet exist so we can re-verify them
    // after create_dir_all to close the TOCTOU window.
    let mut missing_components: Vec<std::path::PathBuf> = Vec::new();
    while let Some(path) = ancestor {
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) => {
                let canonical_ancestor = tokio::fs::canonicalize(path)
                    .await
                    .map_err(|e| format!("cannot canonicalize destination ancestor: {e}"))?;
                if !canonical_ancestor.starts_with(&canonical_root) {
                    return Err(format!(
                        "torrent destination '{}' must be under downloads directory '{}'",
                        canonical_ancestor.display(),
                        canonical_root.display()
                    ));
                }
                if canonical_ancestor == canonical_root {
                    found_root = true;
                    break;
                }
                if metadata.file_type().is_symlink() {
                    return Err("torrent destination must not contain a symlink".to_owned());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
                    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                        return Err(
                            "torrent destination must not contain a reparse point or junction"
                                .to_owned(),
                        );
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // This component does not exist yet — record it for post-creation check.
                missing_components.push(path.to_path_buf());
            }
            Err(error) => return Err(format!("destination stat error: {error}")),
        }
        ancestor = path.parent();
    }
    if !found_root {
        return Err(format!(
            "torrent destination '{}' must be under downloads directory '{}'",
            destination.display(),
            canonical_root.display()
        ));
    }

    tokio::fs::create_dir_all(destination)
        .await
        .map_err(|e| format!("cannot create torrent destination: {e}"))?;

    // Re-verify every component we just created to close the TOCTOU window.
    // An attacker could have planted a junction/symlink at a not-yet-existing
    // path component between the ancestor walk above and create_dir_all.
    for component in &missing_components {
        match tokio::fs::canonicalize(component).await {
            Ok(canonical) if canonical.starts_with(&canonical_root) => {}
            Ok(canonical) => {
                // Escape detected: clean up and reject.
                let _ = tokio::fs::remove_dir_all(destination).await;
                return Err(format!(
                    "torrent destination component '{}' escaped confinement root",
                    canonical.display()
                ));
            }
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(destination).await;
                return Err(format!("cannot verify created component: {e}"));
            }
        }
    }

    let destination_metadata = tokio::fs::symlink_metadata(destination)
        .await
        .map_err(|e| format!("destination stat error: {e}"))?;
    if !destination_metadata.is_dir() {
        return Err("torrent destination must be a directory".to_owned());
    }
    if destination_metadata.file_type().is_symlink() {
        return Err("torrent destination must not be a symlink".to_owned());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if destination_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("torrent destination must not be a reparse point or junction".to_owned());
        }
    }

    let canonical_dest = tokio::fs::canonicalize(destination)
        .await
        .map_err(|e| format!("cannot canonicalize destination: {e}"))?;
    if !canonical_dest.starts_with(&canonical_root) {
        return Err(format!(
            "torrent destination '{}' must be under downloads directory '{}'",
            canonical_dest.display(),
            canonical_root.display()
        ));
    }

    Ok(canonical_dest)
}

#[cfg(windows)]
fn is_unc_path(path: &std::path::Path) -> bool {
    use std::path::{Component, Prefix};

    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
}

// ============================================================
// Error type
// ============================================================

#[derive(Debug, Error)]
pub enum CoreServicesError {
    #[error("I/O error during service startup: {0}")]
    Io(#[source] std::io::Error),
    #[error("database error: {0}")]
    Database(String),
    #[error("engine initialization error: {0}")]
    Engine(String),
    #[error("invalid magnet download request: {0}")]
    InvalidMagnet(String),
    #[error("invalid torrent file request: {0}")]
    InvalidTorrent(String),
    #[error("torrent download was not found")]
    TorrentDownloadNotFound,
    #[error("torrent metadata changed; refresh it before confirming the selection")]
    StaleTorrentMetadata,
    #[error("torrent selection is no longer available for this download")]
    TorrentSelectionConflict,
    #[error("required download engine is unavailable: {0:?}")]
    UnavailableEngine(EngineKind),
    #[error("queue error: {0}")]
    Queue(String),
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("service shutdown error: {0}")]
    Shutdown(String),
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use fetchx_models::{
        DownloadId, DownloadItem, DownloadPriority, DownloadProgress, DownloadSource,
        DownloadStatus, EngineKind, FileName, Timestamp, TorrentFile, TorrentMetadata,
        YtDlpAudioFormat,
    };
    use fetchx_storage::{
        DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
        TorrentSourceType,
    };

    use super::{
        BatchDownloadDestinationRequest, BatchDownloadError, BatchDownloadRequest, ConfigService,
        CoreServices, CoreServicesConfig, CoreServicesError, EngineAvailability, EngineConfig,
        HttpMultiSourceRequest, NativeTorrentConfig, append_path_suffix,
        suggest_download_package_name,
    };
    use fetchx_core::DownloadStateMachine;
    use fetchx_storage::Database;
    use sha1::{Digest, Sha1};
    use std::{
        net::{Ipv4Addr, SocketAddr},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn config(root: &std::path::Path) -> CoreServicesConfig {
        CoreServicesConfig {
            database_path: root.join("fetchx.db"),
            downloads_dir: root.join("downloads"),
            runtime_dir: root.join("runtime"),
            bundled_engines_dir: None,
            defer_queued_on_start: false,
        }
    }

    async fn start_test_services(
        root: &std::path::Path,
    ) -> Result<CoreServices, Box<dyn std::error::Error>> {
        let mut engine_config = EngineConfig::default();
        engine_config.native_torrent.listen_port = 0;
        engine_config.native_torrent.session_dir = Some(root.join("runtime/native-torrent"));
        let services =
            CoreServices::start_with_engine_config(config(root), Some(engine_config)).await?;
        if let EngineAvailability::Unavailable { reason } =
            &services.engine_runtime.status.native_torrent
        {
            return Err(format!("native torrent test engine unavailable: {reason}").into());
        }
        Ok(services)
    }

    async fn persist_test_torrent_config(
        paths: &CoreServicesConfig,
        session_dir: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect_path(&paths.database_path).await?;
        ConfigService::new(database.config())
            .save(&EngineConfig {
                native_torrent: NativeTorrentConfig {
                    enabled: true,
                    listen_port: 0,
                    session_dir: Some(session_dir.to_path_buf()),
                },
                ..EngineConfig::default()
            })
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn http_multi_source_admission_rejects_ambiguous_source_sets()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        let too_few = services
            .enqueue_http_multi_source(
                HttpMultiSourceRequest {
                    urls: vec!["https://downloads.example/file.bin".to_owned()],
                    package_name: None,
                    expected_sha256: None,
                },
                DownloadPriority::Normal,
            )
            .await;
        assert_eq!(too_few, Err(BatchDownloadError::InvalidSourceSet));

        let duplicate = services
            .enqueue_http_multi_source(
                HttpMultiSourceRequest {
                    urls: vec![
                        "https://downloads.example/file.bin".to_owned(),
                        "https://downloads.example/file.bin".to_owned(),
                    ],
                    package_name: None,
                    expected_sha256: None,
                },
                DownloadPriority::Normal,
            )
            .await;
        assert_eq!(duplicate, Err(BatchDownloadError::InvalidMultiSourceUrl));
        services.shutdown().await?;
        Ok(())
    }

    #[test]
    fn normalizes_only_base32_magnets_for_native_v1_parsing() {
        let base32 = "magnet:?xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH&dn=sample";
        assert_eq!(
            super::normalize_base32_btih_magnet(base32, "0123456789abcdef0123456789abcdef01234567",),
            Ok(
                "magnet:?xt=urn%3Abtih%3A0123456789abcdef0123456789abcdef01234567&dn=sample"
                    .to_owned()
            )
        );
        let hex = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=sample";
        assert_eq!(
            super::normalize_base32_btih_magnet(hex, "0123456789abcdef0123456789abcdef01234567",),
            Ok(hex.to_owned())
        );
    }

    const QUEUE_RECOVERY_PHASE: &str = "FETCHX_QUEUE_RECOVERY_PHASE";

    struct QueueRecoveryWorker(std::process::Child);

    impl QueueRecoveryWorker {
        fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
            self.0.try_wait()
        }

        fn kill(&mut self) -> Result<(), std::io::Error> {
            self.0.kill()
        }

        fn wait(&mut self) -> Result<std::process::ExitStatus, std::io::Error> {
            self.0.wait()
        }
    }

    impl Drop for QueueRecoveryWorker {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    fn queue_recovery_paths(root: &Path) -> CoreServicesConfig {
        CoreServicesConfig {
            database_path: root.join("fetchx.sqlite"),
            downloads_dir: root.join("downloads"),
            runtime_dir: root.join("runtime"),
            bundled_engines_dir: None,
            defer_queued_on_start: false,
        }
    }

    async fn run_queue_recovery_worker() -> Result<(), Box<dyn std::error::Error>> {
        let phase = std::env::var(QUEUE_RECOVERY_PHASE)?;
        let root = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_ROOT")?);
        let torrent_path = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_TORRENT")?);
        let destination = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_DESTINATION")?);
        let id_path = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_ID_FILE")?);
        let progress_path = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_PROGRESS_FILE")?);
        let dispatch_path = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_DISPATCH_FILE")?);
        let complete_path = PathBuf::from(std::env::var("FETCHX_QUEUE_RECOVERY_COMPLETE_FILE")?);
        let paths = queue_recovery_paths(&root);

        match phase.as_str() {
            "admit" => {
                let services = CoreServices::start(paths).await?;
                let id = services
                    .enqueue_torrent_file_with_selection_and_engine(
                        torrent_path,
                        EngineKind::NativeTorrent,
                        DownloadPriority::default(),
                        Some(destination),
                        Some(vec![1]),
                    )
                    .await?;
                tokio::fs::write(&id_path, id.to_string()).await?;

                let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
                loop {
                    let item = services
                        .repository
                        .get(id)
                        .await?
                        .ok_or("CoreServices admission lost the native torrent record")?;
                    if item.status() == DownloadStatus::Failed {
                        return Err(
                            "native torrent failed before the process-death checkpoint".into()
                        );
                    }
                    if item.status() == DownloadStatus::Downloading
                        && item.progress().downloaded_bytes() >= 128 * 1024
                    {
                        tokio::fs::write(
                            &progress_path,
                            item.progress().downloaded_bytes().to_string(),
                        )
                        .await?;
                        std::future::pending::<()>().await;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err("CoreServices download did not reach partial progress".into());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            "restart" => {
                let id = DownloadId::parse(&tokio::fs::read_to_string(&id_path).await?)?;
                let services = CoreServices::start(paths).await?;
                let dispatch_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                loop {
                    let snapshot = services.queue().snapshot().await?;
                    if snapshot.active.iter().any(|active| active.item.id() == id) {
                        tokio::fs::write(&dispatch_path, "QueueManager active").await?;
                        break;
                    }
                    let item = services
                        .repository
                        .get(id)
                        .await?
                        .ok_or("QueueManager startup recovery lost the persisted record")?;
                    if item.status() == DownloadStatus::Failed {
                        return Err(
                            "QueueManager startup dispatch failed the native torrent".into()
                        );
                    }
                    if tokio::time::Instant::now() >= dispatch_deadline {
                        return Err(
                            "QueueManager did not dispatch the recovered native torrent".into()
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }

                let completion_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
                loop {
                    let item = services
                        .repository
                        .get(id)
                        .await?
                        .ok_or("recovered download disappeared before completion")?;
                    match item.status() {
                        DownloadStatus::Completed | DownloadStatus::Seeding => {
                            tokio::fs::write(&complete_path, format!("{:?}", item.status()))
                                .await?;
                            services.shutdown().await?;
                            return Ok(());
                        }
                        DownloadStatus::Failed => {
                            return Err("QueueManager-dispatched native torrent failed".into());
                        }
                        _ => {}
                    }
                    if tokio::time::Instant::now() >= completion_deadline {
                        return Err("QueueManager-dispatched torrent did not complete".into());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            _ => Err(format!("invalid queue recovery phase: {phase}").into()),
        }
    }

    fn spawn_queue_recovery_worker(
        phase: &str,
        root: &Path,
        torrent_path: &Path,
        destination: &Path,
    ) -> Result<QueueRecoveryWorker, Box<dyn std::error::Error>> {
        let executable = std::env::current_exe()?;
        Ok(QueueRecoveryWorker(
            Command::new(executable)
                .args([
                    "--exact",
                    "tests::core_services_queue_recovers_native_torrent_after_process_death",
                    "--nocapture",
                ])
                .env(QUEUE_RECOVERY_PHASE, phase)
                .env("FETCHX_QUEUE_RECOVERY_ROOT", root)
                .env("FETCHX_QUEUE_RECOVERY_TORRENT", torrent_path)
                .env("FETCHX_QUEUE_RECOVERY_DESTINATION", destination)
                .env(
                    "FETCHX_QUEUE_RECOVERY_ID_FILE",
                    root.join("download-id.txt"),
                )
                .env(
                    "FETCHX_QUEUE_RECOVERY_PROGRESS_FILE",
                    root.join("progress.txt"),
                )
                .env(
                    "FETCHX_QUEUE_RECOVERY_DISPATCH_FILE",
                    root.join("queue-dispatched.txt"),
                )
                .env(
                    "FETCHX_QUEUE_RECOVERY_COMPLETE_FILE",
                    root.join("completed.txt"),
                )
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()?,
        ))
    }

    fn bencode_bytes(bytes: &[u8], output: &mut Vec<u8>) {
        output.extend_from_slice(bytes.len().to_string().as_bytes());
        output.push(b':');
        output.extend_from_slice(bytes);
    }

    fn core_services_torrent_bytes(
        tracker_url: &str,
        unselected: &[u8],
        selected: &[u8],
    ) -> Vec<u8> {
        const PIECE_LENGTH: usize = 16_384;
        let mut pieces = Vec::new();
        for piece in unselected
            .chunks(PIECE_LENGTH)
            .chain(selected.chunks(PIECE_LENGTH))
        {
            pieces.extend_from_slice(&Sha1::digest(piece));
        }

        let mut info = b"d5:filesl".to_vec();
        for (name, payload) in [("unselected.bin", unselected), ("selected.bin", selected)] {
            info.extend_from_slice(b"d6:lengthi");
            info.extend_from_slice(payload.len().to_string().as_bytes());
            info.extend_from_slice(b"e4:pathl");
            bencode_bytes(name.as_bytes(), &mut info);
            info.extend_from_slice(b"ee");
        }
        info.extend_from_slice(b"e4:name6:bundle12:piece lengthi16384e6:pieces");
        bencode_bytes(&pieces, &mut info);
        info.extend_from_slice(b"7:privatei1ee");

        let mut torrent = b"d8:announce".to_vec();
        bencode_bytes(tracker_url.as_bytes(), &mut torrent);
        torrent.extend_from_slice(b"4:info");
        torrent.extend_from_slice(&info);
        torrent.push(b'e');
        torrent
    }

    async fn serve_loopback_tracker(
        listener: TcpListener,
        peer: SocketAddr,
    ) -> Result<(), std::io::Error> {
        loop {
            let (mut stream, _) = listener.accept().await?;
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let Ok(count) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }

                let mut body = b"d8:intervali60e5:peers6:".to_vec();
                body.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
                body.extend_from_slice(&peer.port().to_be_bytes());
                body.push(b'e');
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            });
        }
    }

    async fn wait_for_recovery_file(
        path: &Path,
        worker: &mut QueueRecoveryWorker,
        timeout: Duration,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if tokio::fs::try_exists(path).await? {
                return Ok(tokio::fs::read_to_string(path).await?);
            }
            if let Some(status) = worker.try_wait()? {
                return Err(format!("queue recovery worker exited early: {status}").into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("timed out waiting for queue recovery worker evidence".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn start_without_native_torrent(
        root: &std::path::Path,
    ) -> Result<CoreServices, Box<dyn std::error::Error>> {
        use super::{ConfigService, EngineConfig, NativeTorrentConfig};
        use fetchx_storage::Database;

        let paths = config(root);
        let database = Database::connect_path(&paths.database_path).await?;
        ConfigService::new(database.config())
            .save(&EngineConfig {
                native_torrent: NativeTorrentConfig {
                    enabled: false,
                    ..NativeTorrentConfig::default()
                },
                ..EngineConfig::default()
            })
            .await?;
        drop(database);
        Ok(CoreServices::start(paths).await?)
    }

    #[tokio::test]
    async fn starts_shared_graph_and_shutdown_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;

        assert!(services.engine_runtime.is_available(EngineKind::DirectHttp));
        assert!(services.engine_runtime.is_available(EngineKind::Hls));
        assert!(services.queue().snapshot().await?.items.is_empty());

        let (first, second) = tokio::join!(services.shutdown(), services.shutdown());
        first?;
        second?;
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn core_services_queue_recovers_native_torrent_after_process_death()
    -> Result<(), Box<dyn std::error::Error>> {
        if std::env::var_os(QUEUE_RECOVERY_PHASE).is_some() {
            run_queue_recovery_worker().await?;
            return Ok(());
        }

        use librqbit::{AddTorrent, AddTorrentOptions, ListenerOptions, Session, SessionOptions};
        use std::num::NonZeroU32;

        let root = tempdir()?;
        let root_path = root.path();
        let paths = queue_recovery_paths(root_path);
        let tracker_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let tracker_address = tracker_listener.local_addr()?;
        let tracker_url = format!("http://{tracker_address}/announce");
        let unselected = vec![b'u'; 16_384];
        let selected = vec![b's'; 4 * 1024 * 1024];
        let torrent_bytes = core_services_torrent_bytes(&tracker_url, &unselected, &selected);
        let torrent_metadata = fetchx_models::parse_torrent_metainfo(&torrent_bytes)?;
        assert_eq!(torrent_metadata.files.len(), 2);
        assert_eq!(torrent_metadata.trackers, vec![tracker_url]);

        let torrent_path = root_path.join("selected.torrent");
        let seed_output = root_path.join("seed-output");
        let destination = paths.downloads_dir.join("queue-recovery-output");
        tokio::fs::create_dir_all(&seed_output).await?;
        tokio::fs::write(&torrent_path, &torrent_bytes).await?;
        tokio::fs::write(seed_output.join("unselected.bin"), &unselected).await?;
        tokio::fs::write(seed_output.join("selected.bin"), &selected).await?;

        let seeder_session = Session::new_with_opts(
            root_path.join("seeder-session"),
            SessionOptions {
                dht: None,
                persistence: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ipv4_only: true,
                listen: Some(ListenerOptions {
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    ipv4_only: true,
                    ..ListenerOptions::default()
                }),
                ..SessionOptions::default()
            },
        )
        .await?;
        let seeder = seeder_session
            .add_torrent(
                AddTorrent::from_bytes(torrent_bytes),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(seed_output.to_string_lossy().into_owned()),
                    ratelimits: librqbit::limits::LimitsConfig {
                        upload_bps: NonZeroU32::new(512 * 1024),
                        download_bps: None,
                    },
                    ..AddTorrentOptions::default()
                }),
            )
            .await?
            .into_handle()
            .ok_or("loopback seeder did not return a managed torrent")?;
        tokio::time::timeout(Duration::from_secs(10), seeder.wait_until_initialized()).await??;
        assert!(
            seeder.stats().finished,
            "loopback seeder payload did not verify"
        );
        let peer = seeder_session
            .listen_addr()
            .ok_or("loopback seeder did not bind a peer listener")?;
        assert!(peer.ip().is_loopback());
        let tracker_task = tokio::spawn(serve_loopback_tracker(tracker_listener, peer));

        let database = Database::connect_path(&paths.database_path).await?;
        let engine_config = EngineConfig {
            native_torrent: NativeTorrentConfig {
                enabled: true,
                listen_port: 0,
                session_dir: Some(paths.runtime_dir.join("librqbit-session")),
            },
            ..EngineConfig::default()
        };
        ConfigService::new(database.config())
            .save(&engine_config)
            .await?;
        drop(database);

        let mut admission_worker =
            spawn_queue_recovery_worker("admit", root_path, &torrent_path, &destination)?;
        let progress_path = root_path.join("progress.txt");
        let progress = wait_for_recovery_file(
            &progress_path,
            &mut admission_worker,
            Duration::from_secs(45),
        )
        .await?;
        let progress_at_crash: u64 = progress.parse()?;
        assert!(progress_at_crash >= 128 * 1024);
        let id = DownloadId::parse(
            &tokio::fs::read_to_string(root_path.join("download-id.txt")).await?,
        )?;
        admission_worker.kill()?;
        let _ = admission_worker.wait()?;

        let database_after_crash = Database::connect_path(&paths.database_path).await?;
        let expected_destination = tokio::fs::canonicalize(&destination).await?;
        let interrupted = database_after_crash
            .downloads()
            .get(id)
            .await?
            .ok_or("CoreServices admission record disappeared after worker death")?;
        assert_eq!(interrupted.status(), DownloadStatus::Downloading);
        assert!(interrupted.progress().downloaded_bytes() >= progress_at_crash);
        assert_eq!(interrupted.source().engine(), EngineKind::NativeTorrent);
        assert_eq!(interrupted.destination(), expected_destination.as_path());
        let interrupted_metadata = database_after_crash
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await?
            .ok_or("CoreServices torrent metadata disappeared after worker death")?;
        assert_eq!(interrupted_metadata.info_hash, torrent_metadata.info_hash);
        assert_eq!(
            interrupted_metadata.destination,
            expected_destination.as_path()
        );
        assert_eq!(interrupted_metadata.selected_file_indexes, vec![1]);
        assert!(interrupted_metadata.selection_confirmed);
        assert_eq!(interrupted_metadata.aria2_gid, None);
        drop(database_after_crash);

        tokio::time::sleep(Duration::from_millis(250)).await;
        let uploaded_before_restart = seeder.stats().uploaded_bytes;
        let mut restart_worker =
            spawn_queue_recovery_worker("restart", root_path, &torrent_path, &destination)?;
        let dispatched = wait_for_recovery_file(
            &root_path.join("queue-dispatched.txt"),
            &mut restart_worker,
            Duration::from_secs(45),
        )
        .await?;
        assert_eq!(dispatched, "QueueManager active");
        let completion = wait_for_recovery_file(
            &root_path.join("completed.txt"),
            &mut restart_worker,
            Duration::from_secs(75),
        )
        .await?;
        assert!(completion == "Seeding" || completion == "Completed");
        let exit_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = restart_worker.try_wait()? {
                assert!(
                    status.success(),
                    "CoreServices restart worker failed: {status}"
                );
                break;
            }
            if tokio::time::Instant::now() >= exit_deadline {
                restart_worker.kill()?;
                let _ = restart_worker.wait()?;
                return Err("CoreServices restart worker did not exit after completion".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let database_after_restart = Database::connect_path(&paths.database_path).await?;
        let recovered = database_after_restart
            .downloads()
            .get(id)
            .await?
            .ok_or("QueueManager startup recovery removed the download record")?;
        assert_eq!(recovered.source().engine(), EngineKind::NativeTorrent);
        assert_eq!(recovered.destination(), expected_destination.as_path());
        assert!(
            recovered.status() == DownloadStatus::Seeding
                || recovered.status() == DownloadStatus::Completed
        );
        assert_eq!(
            recovered.progress().downloaded_bytes(),
            selected.len() as u64
        );
        let recovered_metadata = database_after_restart
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await?
            .ok_or("QueueManager startup recovery removed torrent metadata")?;
        assert_eq!(recovered_metadata.info_hash, torrent_metadata.info_hash);
        assert_eq!(
            recovered_metadata.destination,
            expected_destination.as_path()
        );
        assert_eq!(recovered_metadata.selected_file_indexes, vec![1]);
        assert!(recovered_metadata.selection_confirmed);
        assert_eq!(recovered_metadata.aria2_gid, None);
        drop(database_after_restart);

        let downloaded_selected = tokio::fs::read(destination.join("selected.bin")).await?;
        assert_eq!(downloaded_selected, selected);
        assert_eq!(
            Sha1::digest(&downloaded_selected),
            Sha1::digest(&selected),
            "QueueManager-recovered selected file has the wrong SHA-1"
        );
        let unselected_path = destination.join("unselected.bin");
        if tokio::fs::try_exists(&unselected_path).await? {
            assert_eq!(
                tokio::fs::metadata(unselected_path).await?.len(),
                0,
                "QueueManager recovery emitted bytes for the unselected torrent file"
            );
        }
        let uploaded_after_restart = seeder.stats().uploaded_bytes;
        eprintln!(
            "CoreServices queue recovery peer upload after restart: {} bytes for a {}-byte selected file",
            uploaded_after_restart.saturating_sub(uploaded_before_restart),
            selected.len()
        );

        tracker_task.abort();
        seeder_session.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn batch_admission_keeps_successes_when_other_inputs_are_invalid_or_duplicate()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;

        let results = services
            .enqueue_download_batch(
                vec![
                    BatchDownloadRequest {
                        url: "https://example.com/a.zip".to_owned(),
                        package_name: Some("media/pack".to_owned()),
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                    BatchDownloadRequest {
                        url: "https://EXAMPLE.com/a.zip#fragment".to_owned(),
                        package_name: None,
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                    BatchDownloadRequest {
                        url: "file:///tmp/not-http".to_owned(),
                        package_name: None,
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                    BatchDownloadRequest {
                        url: "https://example.net/audio.mp3".to_owned(),
                        package_name: None,
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: Some("mp3".to_owned()),
                        audio_format: Some(YtDlpAudioFormat::Mp3),
                    },
                    BatchDownloadRequest {
                        url: "https://example.net/invalid-package.zip".to_owned(),
                        package_name: Some("...".to_owned()),
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                    BatchDownloadRequest {
                        url: "https://example.net/aria2.zip".to_owned(),
                        package_name: None,
                        engine: EngineKind::Aria2,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                    BatchDownloadRequest {
                        url: "https://example.org/b.zip".to_owned(),
                        package_name: None,
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                    BatchDownloadRequest {
                        url: String::new(),
                        package_name: None,
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    },
                ],
                DownloadPriority::High,
            )
            .await;

        assert_eq!(results.len(), 8);
        assert!(results[0].result.is_ok());
        assert_eq!(results[1].result, Err(BatchDownloadError::DuplicateUrl));
        assert_eq!(results[2].result, Err(BatchDownloadError::InvalidUrl));
        assert_eq!(results[3].result, Err(BatchDownloadError::InvalidFormat));
        assert_eq!(
            results[4].result,
            Err(BatchDownloadError::InvalidPackageName)
        );
        assert_eq!(
            results[5].result,
            Err(BatchDownloadError::UnavailableEngine(EngineKind::Aria2))
        );
        assert!(results[6].result.is_ok());
        assert_eq!(results[7].result, Err(BatchDownloadError::InvalidUrl));

        let queued = services.repository.list().await?;
        assert_eq!(queued.len(), 2);
        assert!(
            queued
                .iter()
                .all(|item| item.priority() == DownloadPriority::High)
        );
        assert!(queued.iter().any(|item| {
            item.destination()
                .starts_with(root.path().join("downloads").join("media_pack"))
        }));

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn user_confirmed_aria2_http_restart_creates_a_separate_native_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;

        let legacy_destination = root.path().join("downloads/legacy.zip");
        tokio::fs::create_dir_all(legacy_destination.parent().expect("parent")).await?;
        tokio::fs::write(&legacy_destination, b"legacy partial bytes").await?;
        let aria2_sidecar = append_path_suffix(&legacy_destination, ".aria2");
        tokio::fs::write(&aria2_sidecar, b"legacy aria2 control data").await?;
        let created_at = Timestamp::now()?;
        let mut legacy = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://old.example/legacy.zip", EngineKind::Aria2)?,
            FileName::new("legacy.zip")?,
            legacy_destination.clone(),
            created_at,
        );
        legacy.set_status_at(DownloadStatus::Paused, created_at)?;
        let legacy_id = legacy.id();
        services.repository.create(&legacy).await?;

        let new_destination = root.path().join("downloads/legacy-native.zip");
        let new_id = services
            .restart_aria2_http_with_native(
                legacy_id,
                "https://current.example/fresh.zip".to_owned(),
                new_destination.clone(),
                true,
            )
            .await?;

        assert_ne!(new_id, legacy_id);
        let restarted = services
            .repository
            .get(new_id)
            .await?
            .expect("new native record is persisted");
        assert_eq!(restarted.source().engine(), EngineKind::DirectHttp);
        assert_eq!(
            restarted.source().url(),
            "https://current.example/fresh.zip"
        );
        assert_eq!(restarted.destination(), new_destination);
        assert_eq!(
            services.repository.get(legacy_id).await?,
            Some(legacy.clone())
        );
        assert_eq!(
            tokio::fs::read(&legacy_destination).await?,
            b"legacy partial bytes"
        );
        assert_eq!(
            tokio::fs::read(&aria2_sidecar).await?,
            b"legacy aria2 control data"
        );

        services.queue().shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn aria2_http_restart_rejects_invalid_sources_ineligible_rows_and_collisions()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;

        let legacy_destination = root.path().join("downloads/legacy.zip");
        tokio::fs::create_dir_all(legacy_destination.parent().expect("parent")).await?;
        let created_at = Timestamp::now()?;
        let legacy = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://old.example/legacy.zip", EngineKind::Aria2)?,
            FileName::new("legacy.zip")?,
            legacy_destination.clone(),
            created_at,
        );
        let legacy_id = legacy.id();
        services.repository.create(&legacy).await?;

        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "file:///unsafe".to_owned(),
                    root.path().join("downloads/fresh.zip"),
                    true,
                )
                .await,
            Err(BatchDownloadError::InvalidUrl)
        );
        for unsafe_url in [
            "https://user:password@current.example/fresh.zip",
            "https://current.example/fresh.zip#fragment",
            "https://current.example/movie.mp4",
        ] {
            assert_eq!(
                services
                    .restart_aria2_http_with_native(
                        legacy_id,
                        unsafe_url.to_owned(),
                        root.path().join("downloads/fresh-native.zip"),
                        true,
                    )
                    .await,
                Err(BatchDownloadError::InvalidUrl)
            );
        }
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    root.path().join("downloads/unconfirmed.zip"),
                    false,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    legacy_destination.clone(),
                    true,
                )
                .await,
            Err(BatchDownloadError::DestinationCollision)
        );
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    append_path_suffix(&legacy_destination, ".aria2"),
                    true,
                )
                .await,
            Err(BatchDownloadError::DestinationCollision)
        );
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    append_path_suffix(&legacy_destination, ".part"),
                    true,
                )
                .await,
            Err(BatchDownloadError::DestinationCollision)
        );
        let occupied_destination = root.path().join("downloads/already-present.zip");
        tokio::fs::write(&occupied_destination, b"preserve this output").await?;
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    occupied_destination.clone(),
                    true,
                )
                .await,
            Err(BatchDownloadError::DestinationCollision)
        );
        assert_eq!(
            tokio::fs::read(&occupied_destination).await?,
            b"preserve this output"
        );
        let occupied_partial = append_path_suffix(&occupied_destination, ".part");
        tokio::fs::write(&occupied_partial, b"preserve partial").await?;
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    occupied_destination.clone(),
                    true,
                )
                .await,
            Err(BatchDownloadError::DestinationCollision)
        );
        assert_eq!(
            tokio::fs::read(&occupied_partial).await?,
            b"preserve partial"
        );
        let occupied_sidecar = append_path_suffix(&occupied_destination, ".aria2");
        tokio::fs::write(&occupied_sidecar, b"preserve sidecar").await?;
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    legacy_id,
                    "https://current.example/fresh.zip".to_owned(),
                    occupied_destination.clone(),
                    true,
                )
                .await,
            Err(BatchDownloadError::DestinationCollision)
        );
        assert_eq!(
            tokio::fs::read(&occupied_sidecar).await?,
            b"preserve sidecar"
        );

        let mut completed = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://old.example/completed.zip", EngineKind::Aria2)?,
            FileName::new("completed.zip")?,
            root.path().join("downloads/completed.zip"),
            Timestamp::now()?,
        );
        completed.set_status_at(DownloadStatus::Completed, Timestamp::now()?)?;
        let completed_id = completed.id();
        services.repository.create(&completed).await?;
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    completed_id,
                    "https://current.example/fresh.zip".to_owned(),
                    root.path().join("downloads/completed-native.zip"),
                    true,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );

        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    DownloadId::new(),
                    "https://current.example/fresh.zip".to_owned(),
                    root.path().join("downloads/missing.zip"),
                    true,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );

        let magnet = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(
                "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
                EngineKind::Aria2,
            )?,
            FileName::new("torrent.zip")?,
            root.path().join("downloads/torrent.zip"),
            Timestamp::now()?,
        );
        services.repository.create(&magnet).await?;
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    magnet.id(),
                    "https://current.example/fresh.zip".to_owned(),
                    root.path().join("downloads/torrent-native.zip"),
                    true,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );

        let media = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://old.example/movie.mp4", EngineKind::Aria2)?,
            FileName::new("movie.mp4")?,
            root.path().join("downloads/movie.mp4"),
            Timestamp::now()?,
        );
        services.repository.create(&media).await?;
        assert_eq!(
            services
                .restart_aria2_http_with_native(
                    media.id(),
                    "https://current.example/movie.mp4".to_owned(),
                    root.path().join("downloads/movie-native.zip"),
                    true,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );

        services.queue().shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn user_confirmed_aria2_torrent_restart_creates_a_separate_native_job()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;

        let info_hash = "0123456789abcdef0123456789abcdef01234567";
        let legacy_destination = root.path().join("downloads/legacy-torrent");
        tokio::fs::create_dir_all(&legacy_destination).await?;
        let legacy_output = legacy_destination.join("selected.bin");
        tokio::fs::write(&legacy_output, b"legacy partial torrent data").await?;
        let legacy_sidecar = append_path_suffix(&legacy_output, ".aria2");
        tokio::fs::write(&legacy_sidecar, b"legacy aria2 control data").await?;

        let created_at = Timestamp::now()?;
        let mut legacy = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(
                format!("magnet:?xt=urn:btih:{info_hash}"),
                EngineKind::Aria2,
            )?,
            FileName::new("legacy-torrent")?,
            legacy_destination.clone(),
            created_at,
        );
        legacy.set_status_at(DownloadStatus::Paused, created_at)?;
        let legacy_id = legacy.id();
        services.repository.create(&legacy).await?;

        let legacy_metadata = TorrentMetadataRow {
            download_id: legacy_id,
            source_type: TorrentSourceType::Magnet,
            magnet_uri: Some(format!("magnet:?xt=urn:btih:{info_hash}")),
            local_torrent_path: None,
            info_hash: info_hash.to_owned(),
            torrent_name: "legacy-torrent".to_owned(),
            torrent_metadata: None,
            destination: legacy_destination.clone(),
            metadata_state: "pending".to_owned(),
            selected_file_indexes: vec![1],
            selected_total_bytes: 123,
            selection_confirmed: true,
            aria2_gid: Some("legacy-gid".to_owned()),
            seed_state: TorrentSeedState::Downloading,
            uploaded_bytes: 7,
            upload_speed_bps: 2,
            peer_count: 3,
            seed_count: 1,
            created_at,
            updated_at: created_at,
        };
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&legacy_metadata)
            .await?;

        let current_magnet = format!("magnet:?xt=urn:btih:{info_hash}&dn=current-copy");
        let new_destination = root.path().join("downloads/native-torrent");
        let native_id = services
            .restart_aria2_torrent_with_native(
                legacy_id,
                current_magnet.clone(),
                new_destination.clone(),
                true,
            )
            .await?;

        assert_ne!(native_id, legacy_id);
        let native = services
            .repository
            .get(native_id)
            .await?
            .expect("separate native download is persisted");
        assert_eq!(native.source().engine(), EngineKind::NativeTorrent);
        assert_eq!(native.source().url(), current_magnet);
        assert_eq!(
            native.destination(),
            tokio::fs::canonicalize(&new_destination).await?
        );
        let native_metadata = services
            .database
            .torrent_metadata()
            .get_torrent_metadata(native_id)
            .await?
            .expect("native torrent metadata is persisted");
        assert_eq!(native_metadata.info_hash, info_hash);
        assert!(native_metadata.selected_file_indexes.is_empty());
        assert!(!native_metadata.selection_confirmed);

        let base32_native_id = services
            .restart_aria2_torrent_with_native(
                legacy_id,
                "magnet:?xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH&dn=base32-copy".to_owned(),
                root.path().join("downloads/native-base32-torrent"),
                true,
            )
            .await?;
        let base32_native = services
            .repository
            .get(base32_native_id)
            .await?
            .expect("base32 source creates a separate native record");
        assert_eq!(
            fetchx_models::parse_magnet_uri(base32_native.source().url())?.info_hash,
            info_hash
        );
        assert!(base32_native.source().url().contains(info_hash));

        assert_eq!(services.repository.get(legacy_id).await?, Some(legacy));
        assert_eq!(
            services
                .database
                .torrent_metadata()
                .get_torrent_metadata(legacy_id)
                .await?,
            Some(legacy_metadata)
        );
        assert_eq!(
            tokio::fs::read(&legacy_output).await?,
            b"legacy partial torrent data"
        );
        assert_eq!(
            tokio::fs::read(&legacy_sidecar).await?,
            b"legacy aria2 control data"
        );

        services.queue().shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn aria2_torrent_restart_rejects_mismatches_and_destination_collisions()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;
        let info_hash = "0123456789abcdef0123456789abcdef01234567";
        let legacy_destination = root.path().join("downloads/legacy-torrent");
        tokio::fs::create_dir_all(&legacy_destination).await?;
        let created_at = Timestamp::now()?;
        let legacy = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(
                format!("magnet:?xt=urn:btih:{info_hash}"),
                EngineKind::Aria2,
            )?,
            FileName::new("legacy-torrent")?,
            legacy_destination.clone(),
            created_at,
        );
        let legacy_id = legacy.id();
        services.repository.create(&legacy).await?;
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: legacy_id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(format!("magnet:?xt=urn:btih:{info_hash}")),
                local_torrent_path: None,
                info_hash: info_hash.to_owned(),
                torrent_name: "legacy-torrent".to_owned(),
                torrent_metadata: None,
                destination: legacy_destination.clone(),
                metadata_state: "pending".to_owned(),
                selected_file_indexes: Vec::new(),
                selected_total_bytes: 0,
                selection_confirmed: false,
                aria2_gid: Some("legacy-gid".to_owned()),
                seed_state: TorrentSeedState::Downloading,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at,
                updated_at: created_at,
            })
            .await?;

        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    format!("magnet:?xt=urn:btih:{info_hash}"),
                    root.path().join("downloads/unconfirmed"),
                    false,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    "not-a-magnet".to_owned(),
                    root.path().join("downloads/invalid"),
                    true,
                )
                .await,
            Err(BatchDownloadError::InvalidTorrentRestartSource)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                    root.path().join("downloads/mismatched"),
                    true,
                )
                .await,
            Err(BatchDownloadError::TorrentSourceMismatch)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    format!("magnet:?xt=urn:btih:{info_hash}"),
                    legacy_destination.join("new-child"),
                    true,
                )
                .await,
            Err(BatchDownloadError::TorrentDestinationCollision)
        );
        tokio::fs::create_dir_all(root.path().join("outside")).await?;
        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    format!("magnet:?xt=urn:btih:{info_hash}"),
                    root.path().join("downloads"),
                    true,
                )
                .await,
            Err(BatchDownloadError::TorrentDestinationCollision)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    format!("magnet:?xt=urn:btih:{info_hash}"),
                    root.path().join("outside/native-torrent"),
                    true,
                )
                .await,
            Err(BatchDownloadError::InvalidTorrentRestartDestination)
        );

        let legacy_torrent_file = root.path().join("metadata/legacy.torrent");
        tokio::fs::create_dir_all(legacy_torrent_file.parent().expect("metadata parent")).await?;
        tokio::fs::write(&legacy_torrent_file, b"saved legacy metainfo").await?;
        let torrent_file_item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new_torrent_file(&legacy_torrent_file, EngineKind::Aria2)?,
            FileName::new("legacy-file-torrent")?,
            root.path().join("downloads/legacy-file-torrent"),
            created_at,
        );
        let torrent_file_id = torrent_file_item.id();
        services.repository.create(&torrent_file_item).await?;
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: torrent_file_id,
                source_type: TorrentSourceType::TorrentFile,
                magnet_uri: None,
                local_torrent_path: Some(legacy_torrent_file),
                info_hash: info_hash.to_owned(),
                torrent_name: "legacy-file-torrent".to_owned(),
                torrent_metadata: None,
                destination: torrent_file_item.destination().to_path_buf(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: vec![0],
                selected_total_bytes: 55,
                selection_confirmed: true,
                aria2_gid: Some("legacy-file-gid".to_owned()),
                seed_state: TorrentSeedState::Downloading,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at,
                updated_at: created_at,
            })
            .await?;
        let file_restart_id = services
            .restart_aria2_torrent_with_native(
                torrent_file_id,
                format!("magnet:?xt=urn:btih:{info_hash}"),
                root.path().join("downloads/native-from-file-torrent"),
                true,
            )
            .await?;
        assert_eq!(
            services
                .repository
                .get(file_restart_id)
                .await?
                .expect("native job from legacy torrent file exists")
                .source()
                .engine(),
            EngineKind::NativeTorrent
        );

        let mut completed = legacy.clone();
        completed.set_status_at(DownloadStatus::Completed, Timestamp::now()?)?;
        services.repository.update(&completed).await?;
        assert_eq!(
            services
                .restart_aria2_torrent_with_native(
                    legacy_id,
                    format!("magnet:?xt=urn:btih:{info_hash}"),
                    root.path().join("downloads/completed-restart"),
                    true,
                )
                .await,
            Err(BatchDownloadError::RestartNotEligible)
        );

        services.queue().shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn aria2_torrent_file_restart_checks_identity_selection_and_preserves_legacy_data()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;
        let tracker_url = "http://127.0.0.1:9/announce";
        let unselected = vec![b'u'; 16_384];
        let selected = vec![b's'; 16_384];
        let torrent_bytes = core_services_torrent_bytes(tracker_url, &unselected, &selected);
        let source_metadata = fetchx_models::parse_torrent_metainfo(&torrent_bytes)?;
        let torrent_path = root.path().join("current.torrent");
        tokio::fs::write(&torrent_path, &torrent_bytes).await?;

        let legacy_destination = root.path().join("downloads/legacy-torrent");
        tokio::fs::create_dir_all(&legacy_destination).await?;
        let legacy_output = legacy_destination.join("selected.bin");
        let legacy_sidecar = append_path_suffix(&legacy_output, ".aria2");
        let legacy_partial = append_path_suffix(&legacy_output, ".part");
        tokio::fs::write(&legacy_output, b"preserved legacy output").await?;
        tokio::fs::write(&legacy_sidecar, b"preserved legacy sidecar").await?;
        tokio::fs::write(&legacy_partial, b"preserved legacy partial bytes").await?;

        let created_at = Timestamp::now()?;
        let legacy = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(
                format!("magnet:?xt=urn:btih:{}", source_metadata.info_hash),
                EngineKind::Aria2,
            )?,
            FileName::new("legacy-torrent")?,
            legacy_destination.clone(),
            created_at,
        );
        let legacy_id = legacy.id();
        services.repository.create(&legacy).await?;
        let legacy_metadata = TorrentMetadataRow {
            download_id: legacy_id,
            source_type: TorrentSourceType::Magnet,
            magnet_uri: Some(legacy.source().url().to_owned()),
            local_torrent_path: None,
            info_hash: source_metadata.info_hash.clone(),
            torrent_name: source_metadata.name.clone(),
            torrent_metadata: Some(source_metadata.clone()),
            destination: legacy_destination,
            metadata_state: "ready".to_owned(),
            selected_file_indexes: vec![0],
            selected_total_bytes: u64::try_from(unselected.len())?,
            selection_confirmed: true,
            aria2_gid: Some("legacy-file-restart-gid".to_owned()),
            seed_state: TorrentSeedState::Downloading,
            uploaded_bytes: 0,
            upload_speed_bps: 0,
            peer_count: 0,
            seed_count: 0,
            created_at,
            updated_at: created_at,
        };
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&legacy_metadata)
            .await?;

        assert_eq!(
            services
                .restart_aria2_torrent_file_with_native(
                    legacy_id,
                    torrent_path.clone(),
                    root.path().join("downloads/unconfirmed-selection"),
                    vec![1],
                    true,
                    false,
                )
                .await,
            Err(BatchDownloadError::InvalidTorrentRestartSelection)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_file_with_native(
                    legacy_id,
                    torrent_path.clone(),
                    root.path().join("downloads/out-of-range-selection"),
                    vec![2],
                    true,
                    true,
                )
                .await,
            Err(BatchDownloadError::InvalidTorrentRestartSelection)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_file_with_native(
                    legacy_id,
                    torrent_path.clone(),
                    root.path().join("downloads/empty-selection"),
                    Vec::new(),
                    true,
                    true,
                )
                .await,
            Err(BatchDownloadError::InvalidTorrentRestartSelection)
        );
        assert_eq!(
            services
                .restart_aria2_torrent_file_with_native(
                    legacy_id,
                    torrent_path.clone(),
                    legacy_metadata.destination.join("new-child"),
                    vec![1],
                    true,
                    true,
                )
                .await,
            Err(BatchDownloadError::TorrentDestinationCollision)
        );

        let mismatch_bytes =
            core_services_torrent_bytes(tracker_url, &vec![b'x'; 16_384], &selected);
        let mismatch_path = root.path().join("mismatch.torrent");
        tokio::fs::write(&mismatch_path, mismatch_bytes).await?;
        assert_eq!(
            services
                .restart_aria2_torrent_file_with_native(
                    legacy_id,
                    mismatch_path,
                    root.path().join("downloads/hash-mismatch"),
                    vec![1],
                    true,
                    true,
                )
                .await,
            Err(BatchDownloadError::TorrentSourceMismatch)
        );
        let invalid_path = root.path().join("invalid.torrent");
        tokio::fs::write(&invalid_path, b"not bencoded torrent metadata").await?;
        assert_eq!(
            services
                .restart_aria2_torrent_file_with_native(
                    legacy_id,
                    invalid_path,
                    root.path().join("downloads/invalid-source"),
                    vec![1],
                    true,
                    true,
                )
                .await,
            Err(BatchDownloadError::InvalidTorrentRestartSource)
        );

        let native_id = services
            .restart_aria2_torrent_file_with_native(
                legacy_id,
                torrent_path,
                root.path().join("downloads/native-file-restart"),
                vec![1],
                true,
                true,
            )
            .await?;
        assert_ne!(native_id, legacy_id);
        let native = services
            .repository
            .get(native_id)
            .await?
            .expect("separate native record is persisted");
        assert_eq!(native.source().engine(), EngineKind::NativeTorrent);
        assert!(native.source().is_torrent_file());
        let native_metadata = services
            .database
            .torrent_metadata()
            .get_torrent_metadata(native_id)
            .await?
            .expect("native torrent metadata is persisted");
        assert_eq!(native_metadata.info_hash, source_metadata.info_hash);
        assert_eq!(native_metadata.selected_file_indexes, vec![1]);
        assert!(native_metadata.selection_confirmed);
        let native_source_path = native.source().torrent_file_path().expect("torrent path");
        assert_ne!(native_source_path, root.path().join("current.torrent"));
        assert_eq!(
            fetchx_models::parse_torrent_metainfo(&tokio::fs::read(native_source_path).await?)?
                .info_hash,
            source_metadata.info_hash
        );
        assert_eq!(services.repository.get(legacy_id).await?, Some(legacy));
        assert_eq!(
            services
                .database
                .torrent_metadata()
                .get_torrent_metadata(legacy_id)
                .await?,
            Some(legacy_metadata)
        );
        assert_eq!(
            tokio::fs::read(&legacy_output).await?,
            b"preserved legacy output"
        );
        assert_eq!(
            tokio::fs::read(&legacy_sidecar).await?,
            b"preserved legacy sidecar"
        );
        assert_eq!(
            tokio::fs::read(&legacy_partial).await?,
            b"preserved legacy partial bytes"
        );

        services.queue().shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn explicit_destination_batch_preserves_partial_results_and_resolves_collisions()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;
        let destination = root
            .path()
            .join("downloads")
            .join("package")
            .join("file.zip");
        let request = |url: &str, path: std::path::PathBuf, checksum: Option<&str>| {
            BatchDownloadDestinationRequest {
                url: url.to_owned(),
                filename: "file.zip".to_owned(),
                destination: path,
                engine: EngineKind::DirectHttp,
                format_id: None,
                extension: None,
                audio_format: None,
                expected_sha256: checksum.map(str::to_owned),
            }
        };

        let outcomes = services
            .enqueue_download_batch_with_destinations(
                vec![
                    request(
                        "https://example.com/first.zip",
                        destination.clone(),
                        Some(&"A".repeat(64)),
                    ),
                    request("file:///invalid", destination.clone(), None),
                    request("https://example.com/first.zip", destination.clone(), None),
                    request("https://example.com/second.zip", destination.clone(), None),
                    request(
                        "https://example.com/invalid-checksum.zip",
                        root.path().join("downloads").join("bad.zip"),
                        Some("not-a-checksum"),
                    ),
                ],
                DownloadPriority::High,
            )
            .await;

        assert_eq!(outcomes.len(), 5);
        assert!(outcomes[0].result.is_ok());
        assert_eq!(outcomes[1].result, Err(BatchDownloadError::InvalidUrl));
        assert_eq!(outcomes[2].result, Err(BatchDownloadError::DuplicateUrl));
        assert!(outcomes[3].result.is_ok());
        assert_eq!(outcomes[4].result, Err(BatchDownloadError::InvalidChecksum));

        let first = services
            .repository
            .get(outcomes[0].result.as_ref().unwrap().to_owned())
            .await?
            .expect("first admitted item");
        let second = services
            .repository
            .get(outcomes[3].result.as_ref().unwrap().to_owned())
            .await?
            .expect("second admitted item");
        assert_eq!(first.destination(), destination);
        assert_ne!(first.destination(), second.destination());
        assert_eq!(first.priority(), DownloadPriority::High);
        assert_eq!(second.priority(), DownloadPriority::High);
        let mut sidecar = first.destination().as_os_str().to_os_string();
        sidecar.push(".fetchx.sha256");
        assert_eq!(tokio::fs::read_to_string(sidecar).await?, "a".repeat(64));

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn batch_admission_keeps_output_names_unique() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;
        let filename = FileName::new("file")?;
        let existing = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://existing.example/file", EngineKind::DirectHttp)?,
            filename,
            root.path()
                .join("downloads")
                .join("same package")
                .join("file"),
            Timestamp::now()?,
        );
        services.repository.create(&existing).await?;

        let results = services
            .enqueue_download_batch(
                ["https://one.example/file", "https://two.example/file"]
                    .into_iter()
                    .map(|url| BatchDownloadRequest {
                        url: url.to_owned(),
                        package_name: Some("same package".to_owned()),
                        engine: EngineKind::DirectHttp,
                        format_id: None,
                        extension: None,
                        audio_format: None,
                    })
                    .collect(),
                DownloadPriority::Normal,
            )
            .await;

        assert!(results.iter().all(|result| result.result.is_ok()));
        let destinations = services
            .repository
            .list()
            .await?
            .into_iter()
            .map(|item| item.destination().to_path_buf())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(destinations.len(), 3);
        assert!(destinations.iter().any(|path| path.ends_with("file")));
        assert!(destinations.iter().any(|path| path.ends_with("file (2)")));
        assert!(destinations.iter().any(|path| path.ends_with("file (3)")));

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn batch_admission_groups_links_by_source_unless_package_is_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        services.pause_queue_dispatch().await?;
        let requests = [
            ("https://alpha.example.com/one.bin", None),
            ("https://alpha.example.com/two.bin", None),
            ("https://beta.example.org/three.bin", None),
            ("https://gamma.example.net/four.bin", Some("Manual package")),
        ]
        .into_iter()
        .map(|(url, package_name)| BatchDownloadRequest {
            url: url.to_owned(),
            package_name: package_name.map(str::to_owned),
            engine: EngineKind::DirectHttp,
            format_id: None,
            extension: None,
            audio_format: None,
        })
        .collect();

        let results = services
            .enqueue_download_batch_by_source(requests, DownloadPriority::Normal)
            .await;
        assert!(results.iter().all(|result| result.result.is_ok()));

        let downloads = services.repository.list().await?;
        for item in &downloads {
            let package = item
                .destination()
                .parent()
                .and_then(std::path::Path::file_name)
                .and_then(std::ffi::OsStr::to_str)
                .expect("package directory is present");
            let expected = match item.source().url() {
                "https://alpha.example.com/one.bin" | "https://alpha.example.com/two.bin" => {
                    suggest_download_package_name(item.source().url(), "")
                }
                "https://beta.example.org/three.bin" => {
                    suggest_download_package_name(item.source().url(), "")
                }
                "https://gamma.example.net/four.bin" => "Manual package".to_owned(),
                url => panic!("unexpected batch URL: {url}"),
            };
            assert_eq!(package, expected);
        }

        let alpha_package = suggest_download_package_name("https://alpha.example.com/one.bin", "");
        assert_eq!(
            downloads
                .iter()
                .filter(|item| item
                    .destination()
                    .starts_with(root.path().join("downloads").join(&alpha_package)))
                .count(),
            2
        );

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn empty_batch_returns_no_outcomes() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;

        assert!(
            services
                .enqueue_download_batch(Vec::new(), DownloadPriority::Normal)
                .await
                .is_empty()
        );

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_batch_admission_keeps_items_already_accepted_by_the_queue()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = std::sync::Arc::new(start_test_services(root.path()).await?);
        services.pause_queue_dispatch().await?;
        let requests = (0..500)
            .map(|index| BatchDownloadRequest {
                url: format!("https://example.com/{index}.bin"),
                package_name: None,
                engine: EngineKind::DirectHttp,
                format_id: None,
                extension: None,
                audio_format: None,
            })
            .collect();
        let batch_services = std::sync::Arc::clone(&services);
        let batch = tokio::spawn(async move {
            batch_services
                .enqueue_download_batch(requests, DownloadPriority::Normal)
                .await
        });

        loop {
            if !services.repository.list().await?.is_empty() {
                batch.abort();
                break;
            }
            tokio::task::yield_now().await;
        }

        let queued = services.repository.list().await?;
        assert!(!queued.is_empty());
        assert!(queued.len() < 500);
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn diagnostics_snapshot_reports_healthy_storage_without_exposing_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let database_path = root.path().join("fetchx.db");
        let services = start_test_services(root.path()).await?;

        let snapshot = services.diagnostics_snapshot().await;
        let serialized = serde_json::to_string(&snapshot)?;
        assert_eq!(
            snapshot.storage.health,
            fetchx_models::DiagnosticHealth::Healthy
        );
        assert!(snapshot.storage.database_check_latency_ms.is_some());
        assert_eq!(snapshot.queue.total_downloads, 0);
        assert!(!serialized.contains(&database_path.display().to_string()));

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn stopping_native_seeding_without_a_queue_worker_preserves_completed_file_state()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{ConfigService, EngineConfig, NativeTorrentConfig};
        use fetchx_storage::Database;

        let root = tempdir()?;
        let paths = config(root.path());
        let database = Database::connect_path(&paths.database_path).await?;
        ConfigService::new(database.config())
            .save(&EngineConfig {
                native_torrent: NativeTorrentConfig {
                    enabled: true,
                    listen_port: 0,
                    session_dir: Some(root.path().join("librqbit")),
                },
                ..EngineConfig::default()
            })
            .await?;
        drop(database);

        let services = CoreServices::start(paths).await?;
        let id = DownloadId::new();
        let now = Timestamp::from_unix_millis(1_700_000_000_000);
        let info_hash = "0123456789abcdef0123456789abcdef01234567";
        let magnet = format!("magnet:?xt=urn:btih:{info_hash}");
        let item = DownloadItem::new(
            id,
            DownloadSource::new(&magnet, EngineKind::NativeTorrent)?,
            FileName::new("completed.bin")?,
            root.path().join("downloads").join("completed.bin"),
            now,
        );
        services.repository.create(&item).await?;

        let state_machine = DownloadStateMachine::new(services.repository.clone());
        state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        state_machine
            .transition(id, DownloadStatus::Downloading)
            .await?;
        state_machine
            .transition(id, DownloadStatus::Seeding)
            .await?;

        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet),
                local_torrent_path: None,
                info_hash: info_hash.to_owned(),
                torrent_name: "completed.bin".to_owned(),
                torrent_metadata: None,
                destination: item.destination().to_path_buf(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: vec![0],
                selected_total_bytes: 1,
                selection_confirmed: true,
                aria2_gid: None,
                seed_state: TorrentSeedState::Seeding,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at: now,
                updated_at: now,
            })
            .await?;

        services.stop_torrent_seeding(id).await?;

        assert_eq!(
            services.repository.get(id).await?.map(|item| item.status()),
            Some(DownloadStatus::Stopped)
        );
        assert_eq!(
            services
                .database
                .torrent_metadata()
                .get_torrent_metadata(id)
                .await?
                .map(|metadata| metadata.seed_state),
            Some(TorrentSeedState::Stopped)
        );
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_service_graphs_can_start_and_stop_independently()
    -> Result<(), Box<dyn std::error::Error>> {
        let first_root = tempdir()?;
        let second_root = tempdir()?;
        let (first, second) = tokio::try_join!(
            start_test_services(first_root.path()),
            start_test_services(second_root.path())
        )?;

        let (first_result, second_result) = tokio::join!(first.shutdown(), second.shutdown());
        first_result?;
        second_result?;
        Ok(())
    }

    #[tokio::test]
    async fn persists_configuration_and_reports_optional_engine_start_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{Aria2Config, ConfigService, EngineConfig, NativeTorrentConfig, YtDlpConfig};
        use fetchx_storage::Database;

        let root = tempdir()?;
        let paths = config(root.path());
        let database = Database::connect_path(&paths.database_path).await?;
        let config_service = ConfigService::new(database.config());
        let engine_config = EngineConfig {
            aria2: Aria2Config {
                enabled: true,
                executable_path: Some(root.path().join("missing-aria2")),
                ..Aria2Config::default()
            },
            yt_dlp: YtDlpConfig {
                enabled: true,
                executable_path: Some(root.path().join("missing-yt-dlp")),
            },
            native_torrent: NativeTorrentConfig {
                enabled: true,
                listen_port: 0,
                session_dir: Some(root.path().join("runtime/native-torrent")),
            },
            ..EngineConfig::default()
        };
        config_service.save(&engine_config).await?;
        drop(database);

        let services = CoreServices::start(paths).await?;
        assert!(matches!(
            &services.engine_runtime.status.yt_dlp,
            EngineAvailability::Unavailable { .. }
        ));
        let persisted_config = services.config_service.load().await?;
        assert!(persisted_config.yt_dlp.enabled);
        assert!(persisted_config.aria2.enabled);

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn shared_magnet_enqueue_validates_before_engine_availability()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let services = start_without_native_torrent(dir.path()).await?;

        assert!(matches!(
            services
                .enqueue_magnet("magnet:?dn=missing-hash", DownloadPriority::Normal, None,)
                .await,
            Err(super::CoreServicesError::InvalidMagnet(_))
        ));
        assert!(matches!(
            services
                .enqueue_magnet(
                    "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                    DownloadPriority::Normal,
                    None,
                )
                .await,
            Err(super::CoreServicesError::UnavailableEngine(
                EngineKind::NativeTorrent
            ))
        ));
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn explicitly_selected_native_torrent_engine_is_persisted_for_magnet_and_file()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{ConfigService, EngineConfig, NativeTorrentConfig};
        use fetchx_storage::Database;

        let root = tempdir()?;
        let paths = config(root.path());
        let database = Database::connect_path(&paths.database_path).await?;
        ConfigService::new(database.config())
            .save(&EngineConfig {
                native_torrent: NativeTorrentConfig {
                    enabled: true,
                    listen_port: 0,
                    session_dir: Some(root.path().join("librqbit")),
                },
                ..EngineConfig::default()
            })
            .await?;
        drop(database);

        let services = CoreServices::start(paths).await?;
        services.pause_queue_dispatch().await?;

        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=native";
        assert!(matches!(
            services
                .enqueue_magnet_with_engine(
                    magnet,
                    EngineKind::DirectHttp,
                    DownloadPriority::Normal,
                    None,
                )
                .await,
            Err(super::CoreServicesError::InvalidMagnet(_))
        ));
        let magnet_id = services
            .enqueue_magnet_with_engine(
                magnet,
                EngineKind::NativeTorrent,
                DownloadPriority::Normal,
                None,
            )
            .await?;
        let magnet_item = services
            .repository
            .get(magnet_id)
            .await?
            .ok_or("native magnet item was not persisted")?;
        assert_eq!(magnet_item.source().engine(), EngineKind::NativeTorrent);

        let torrent_path = root.path().join("native.torrent");
        tokio::fs::write(
            &torrent_path,
            b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
              01234567890123456789ee",
        )
        .await?;
        assert!(matches!(
            services
                .enqueue_torrent_file_with_selection_and_engine(
                    torrent_path.clone(),
                    EngineKind::DirectHttp,
                    DownloadPriority::Normal,
                    None,
                    Some(vec![0]),
                )
                .await,
            Err(super::CoreServicesError::InvalidTorrent(_))
        ));
        let torrent_id = services
            .enqueue_torrent_file_with_selection_and_engine(
                torrent_path.clone(),
                EngineKind::NativeTorrent,
                DownloadPriority::Normal,
                None,
                Some(vec![0]),
            )
            .await?;
        let torrent_item = services
            .repository
            .get(torrent_id)
            .await?
            .ok_or("native torrent-file item was not persisted")?;
        assert_eq!(torrent_item.source().engine(), EngineKind::NativeTorrent);
        let metadata = services
            .torrent_download_metadata(torrent_id)
            .await?
            .ok_or("native torrent metadata was not persisted")?;
        assert_eq!(metadata.selected_file_indexes, [0]);
        assert!(metadata.selection_confirmed);

        let second_torrent_id = services
            .enqueue_torrent_file_with_selection_and_engine(
                torrent_path,
                EngineKind::NativeTorrent,
                DownloadPriority::Normal,
                None,
                Some(vec![0]),
            )
            .await?;
        let second_torrent_item = services
            .repository
            .get(second_torrent_id)
            .await?
            .ok_or("second native torrent item was not persisted")?;
        assert_ne!(
            torrent_item.destination(),
            second_torrent_item.destination()
        );

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn magnet_file_selection_uses_shared_model_and_persists_size_and_indexes()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        let id = DownloadId::new();
        let now = Timestamp::now()?;
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=sample";
        let item = DownloadItem::new(
            id,
            DownloadSource::new(magnet, EngineKind::Aria2)?,
            FileName::new("sample")?,
            services.downloads_dir().join("sample"),
            now,
        );
        services.repository.create(&item).await?;
        DownloadStateMachine::new(services.repository.clone())
            .transition(id, DownloadStatus::MetadataAcquiring)
            .await?;
        let torrent = TorrentMetadata {
            info_hash: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            name: "sample".to_owned(),
            total_size: 10,
            files: vec![
                TorrentFile {
                    path: vec!["root".to_owned(), "dir".to_owned(), "one.bin".to_owned()],
                    size: 3,
                },
                TorrentFile {
                    path: vec![
                        "root".to_owned(),
                        "dir".to_owned(),
                        "sub".to_owned(),
                        "two.bin".to_owned(),
                    ],
                    size: 5,
                },
                TorrentFile {
                    path: vec!["root".to_owned(), "other.bin".to_owned()],
                    size: 2,
                },
            ],
            trackers: vec!["https://tracker.test/announce".to_owned()],
            piece_length: 16_384,
            piece_count: 1,
        };
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet.to_owned()),
                local_torrent_path: Some(root.path().join("sample.torrent")),
                info_hash: torrent.info_hash.clone(),
                torrent_name: torrent.name.clone(),
                torrent_metadata: Some(torrent),
                destination: item.destination().to_path_buf(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: Vec::new(),
                selected_total_bytes: 0,
                selection_confirmed: false,
                aria2_gid: None,
                seed_state: TorrentSeedState::Downloading,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at: now,
                updated_at: now,
            })
            .await?;

        let exposed = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("torrent metadata missing")?;
        assert_eq!(exposed.metadata_state, "ready");
        assert_eq!(
            exposed.metadata.as_ref().map(|value| value.files.len()),
            Some(3)
        );
        assert!(!exposed.selection_confirmed);

        services
            .set_torrent_directory_selected(id, "root/dir".to_owned(), true)
            .await?;
        let selected = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("torrent metadata missing after selection")?;
        assert_eq!(selected.selected_file_indexes, [0, 1]);
        assert_eq!(selected.selected_total_bytes, 8);
        assert!(!selected.selection_confirmed);
        services.select_torrent_files(id, vec![0, 1]).await?;
        let confirmed = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("torrent metadata missing after selection confirmation")?;
        assert!(confirmed.selection_confirmed);

        services.set_torrent_file_selected(id, 0, false).await?;
        let selected = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("torrent metadata missing after file deselection")?;
        assert_eq!(selected.selected_file_indexes, [1]);
        assert_eq!(selected.selected_total_bytes, 5);

        services.set_torrent_selection_all(id, false).await?;
        let none = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("torrent metadata missing after deselect all")?;
        assert!(none.selected_file_indexes.is_empty());
        assert_eq!(none.selected_total_bytes, 0);
        assert!(services.select_torrent_files(id, vec![3]).await.is_err());
        assert!(services.select_torrent_files(id, vec![0, 0]).await.is_err());
        services.set_torrent_selection_all(id, true).await?;
        let all = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("torrent metadata missing after select all")?;
        assert_eq!(all.selected_file_indexes, [0, 1, 2]);
        assert_eq!(all.selected_total_bytes, 10);
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn native_magnet_remote_selection_revalidates_hash_and_persists_explicit_choice()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        let id = DownloadId::new();
        let now = Timestamp::now()?;
        let info_hash = "0123456789abcdef0123456789abcdef01234567";
        let magnet = format!("magnet:?xt=urn:btih:{info_hash}&dn=sample");
        let item = DownloadItem::new(
            id,
            DownloadSource::new(&magnet, EngineKind::NativeTorrent)?,
            FileName::new("sample")?,
            services.downloads_dir().join("sample"),
            now,
        );
        services.repository.create(&item).await?;
        DownloadStateMachine::new(services.repository.clone())
            .transition(id, DownloadStatus::MetadataAcquiring)
            .await?;
        let metadata = TorrentMetadata {
            info_hash: info_hash.to_owned(),
            name: "sample".to_owned(),
            total_size: 10,
            files: vec![
                TorrentFile {
                    path: vec!["root".to_owned(), "one.bin".to_owned()],
                    size: 4,
                },
                TorrentFile {
                    path: vec!["root".to_owned(), "two.bin".to_owned()],
                    size: 6,
                },
            ],
            trackers: vec!["https://secret-tracker.test/announce?token=hidden".to_owned()],
            piece_length: 16_384,
            piece_count: 1,
        };
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet),
                local_torrent_path: Some(root.path().join("sample.torrent")),
                info_hash: info_hash.to_owned(),
                torrent_name: metadata.name.clone(),
                torrent_metadata: Some(metadata),
                destination: item.destination().to_path_buf(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: Vec::new(),
                selected_total_bytes: 0,
                selection_confirmed: false,
                aria2_gid: None,
                seed_state: TorrentSeedState::Downloading,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at: now,
                updated_at: now,
            })
            .await?;

        let metadata_repository = services.torrent_metadata();
        let mut stored = metadata_repository
            .get_torrent_metadata(id)
            .await?
            .ok_or("torrent metadata should be persisted")?;
        stored.metadata_state = "pending".to_owned();
        metadata_repository.update_torrent_metadata(&stored).await?;
        assert!(matches!(
            services
                .select_native_magnet_files(id, info_hash, vec![0])
                .await,
            Err(CoreServicesError::TorrentSelectionConflict)
        ));
        stored.metadata_state = "ready".to_owned();
        metadata_repository.update_torrent_metadata(&stored).await?;

        let exposed = services
            .native_magnet_metadata(id)
            .await?
            .ok_or("native magnet metadata should be available")?;
        assert_eq!(exposed.info_hash, info_hash);
        assert_eq!(exposed.metadata_state, "ready");
        assert_eq!(exposed.files.len(), 2);
        assert_eq!(exposed.files[1].index, 1);
        assert_eq!(exposed.files[1].path, ["root", "two.bin"]);
        assert_eq!(exposed.files[1].size, 6);
        assert!(!exposed.selection_confirmed);

        assert!(matches!(
            services
                .select_native_magnet_files(id, info_hash, Vec::new())
                .await,
            Err(CoreServicesError::InvalidTorrent(_))
        ));
        assert!(matches!(
            services
                .select_native_magnet_files(id, info_hash, vec![2])
                .await,
            Err(CoreServicesError::InvalidTorrent(_))
        ));
        assert!(matches!(
            services
                .select_native_magnet_files(id, "abcdefabcdefabcdefabcdefabcdefabcdefabcd", vec![0])
                .await,
            Err(CoreServicesError::StaleTorrentMetadata)
        ));
        assert!(matches!(
            services
                .select_native_magnet_files(id, "not-an-info-hash", vec![0])
                .await,
            Err(CoreServicesError::InvalidTorrent(_))
        ));

        services
            .select_native_magnet_files(id, info_hash, vec![1])
            .await?;
        let persisted = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("persisted torrent selection should exist")?;
        assert_eq!(persisted.selected_file_indexes, [1]);
        assert_eq!(persisted.selected_total_bytes, 6);
        assert!(persisted.selection_confirmed);

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn shared_torrent_enqueue_validates_metainfo_before_engine_availability()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_without_native_torrent(root.path()).await?;
        let malformed = root.path().join("malformed.torrent");
        tokio::fs::write(&malformed, b"not bencode").await?;
        assert!(matches!(
            services
                .enqueue_torrent_file(malformed, DownloadPriority::Normal, None)
                .await,
            Err(super::CoreServicesError::InvalidTorrent(_))
        ));

        let valid = root.path().join("valid.torrent");
        tokio::fs::write(
            &valid,
            b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
              01234567890123456789ee",
        )
        .await?;
        assert!(matches!(
            services
                .enqueue_torrent_file(valid, DownloadPriority::Normal, None)
                .await,
            Err(super::CoreServicesError::UnavailableEngine(
                EngineKind::NativeTorrent
            ))
        ));
        assert!(services.queue().snapshot().await?.items.is_empty());
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn new_torrent_admissions_default_to_native_and_magnets_wait_for_selection()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{ConfigService, EngineConfig, NativeTorrentConfig};
        use fetchx_storage::Database;

        let root = tempdir()?;
        let paths = config(root.path());
        let database = Database::connect_path(&paths.database_path).await?;
        ConfigService::new(database.config())
            .save(&EngineConfig {
                native_torrent: NativeTorrentConfig {
                    enabled: true,
                    listen_port: 0,
                    session_dir: Some(root.path().join("native-torrent-session")),
                },
                ..EngineConfig::default()
            })
            .await?;
        drop(database);

        let services = CoreServices::start(paths).await?;
        services.pause_queue_dispatch().await?;
        let magnet =
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=native-default";
        let magnet_id = services
            .enqueue_magnet(magnet, DownloadPriority::Normal, None)
            .await?;
        let magnet_item = services
            .repository
            .get(magnet_id)
            .await?
            .ok_or("default magnet should be persisted")?;
        assert_eq!(magnet_item.source().engine(), EngineKind::NativeTorrent);
        assert_eq!(magnet_item.status(), DownloadStatus::Queued);
        let magnet_metadata = services
            .torrent_download_metadata(magnet_id)
            .await?
            .ok_or("default magnet metadata row should be persisted")?;
        assert_eq!(magnet_metadata.metadata_state, "pending");
        assert!(!magnet_metadata.selection_confirmed);
        assert!(magnet_metadata.selected_file_indexes.is_empty());

        let torrent_path = root.path().join("native-default.torrent");
        tokio::fs::write(
            &torrent_path,
            b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
              01234567890123456789ee",
        )
        .await?;
        let torrent_id = services
            .enqueue_torrent_file(torrent_path.clone(), DownloadPriority::Normal, None)
            .await?;
        let torrent_item = services
            .repository
            .get(torrent_id)
            .await?
            .ok_or("default torrent file should be persisted")?;
        assert_eq!(torrent_item.source().engine(), EngineKind::NativeTorrent);
        let torrent_metadata = services
            .torrent_download_metadata(torrent_id)
            .await?
            .ok_or("default torrent metadata row should be persisted")?;
        assert!(torrent_metadata.selection_confirmed);
        assert_eq!(torrent_metadata.selected_file_indexes, [0]);

        assert!(matches!(
            services
                .enqueue_torrent_file_with_selection_and_engine(
                    torrent_path,
                    EngineKind::Aria2,
                    DownloadPriority::Normal,
                    None,
                    Some(vec![0]),
                )
                .await,
            Err(super::CoreServicesError::UnavailableEngine(
                EngineKind::Aria2
            ))
        ));
        assert!(matches!(
            services
                .enqueue_magnet_with_engine(
                    "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234568",
                    EngineKind::Aria2,
                    DownloadPriority::Normal,
                    None,
                )
                .await,
            Err(super::CoreServicesError::UnavailableEngine(
                EngineKind::Aria2
            ))
        ));
        assert_eq!(services.queue().snapshot().await?.items.len(), 2);

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn local_torrent_inspection_returns_metadata_without_queueing()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        let torrent_path = root.path().join("inspect.torrent");
        tokio::fs::write(
            &torrent_path,
            b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
              01234567890123456789ee",
        )
        .await?;

        let metadata = services.inspect_torrent_file(torrent_path).await?;
        assert_eq!(metadata.name, "x");
        assert_eq!(metadata.total_size, 1);
        assert_eq!(metadata.files.len(), 1);
        assert_eq!(metadata.files[0].path, ["x"]);
        assert!(services.queue().snapshot().await?.items.is_empty());

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn torrent_destination_traversal_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;
        let valid_torrent = root.path().join("valid.torrent");
        tokio::fs::write(
            &valid_torrent,
            b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
              01234567890123456789ee",
        )
        .await?;

        // Traversal via ..
        let traversal_dest = root
            .path()
            .join("downloads")
            .join("..")
            .join("..")
            .join("escape");
        let result = services
            .enqueue_torrent_file(
                valid_torrent.clone(),
                DownloadPriority::Normal,
                Some(traversal_dest),
            )
            .await;
        assert!(
            matches!(&result, Err(super::CoreServicesError::InvalidTorrent(msg)) if msg.contains("must be under")),
            "expected traversal rejection, got: {result:?}"
        );

        let outside_destination = root.path().join("outside-downloads-root");
        let result =
            super::validate_torrent_destination(&outside_destination, services.downloads_dir())
                .await;
        assert!(result.is_err());
        assert!(
            !outside_destination.exists(),
            "rejected destinations must not be created outside the downloads root"
        );
        services.shutdown().await?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn torrent_destination_unc_check_allows_verbatim_drive_paths_only() {
        assert!(super::is_unc_path(std::path::Path::new(r"\\server\share")));
        assert!(super::is_unc_path(std::path::Path::new(
            r"\\?\UNC\server\share"
        )));
        assert!(!super::is_unc_path(std::path::Path::new(
            r"\\?\C:\downloads"
        )));
    }

    #[tokio::test]
    async fn magnet_destination_traversal_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = start_test_services(root.path()).await?;

        let traversal_dest = root
            .path()
            .join("downloads")
            .join("..")
            .join("..")
            .join("secret");
        let result = services
            .enqueue_magnet(
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                DownloadPriority::Normal,
                Some(traversal_dest),
            )
            .await;
        // This will fail with UnavailableEngine (aria2 not running) or traversal rejection.
        // Either is acceptable — what matters is it does NOT succeed.
        assert!(result.is_err(), "traversal path should be rejected");
        services.shutdown().await?;
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn torrent_destination_rejects_symlinks_but_accepts_configured_symlink_root()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = tempdir()?;
        let downloads_dir = root.path().join("downloads");
        let outside_dir = root.path().join("outside");
        let real_subdirectory = downloads_dir.join("real-subdirectory");
        tokio::fs::create_dir_all(&downloads_dir).await?;
        tokio::fs::create_dir_all(&outside_dir).await?;
        tokio::fs::create_dir_all(&real_subdirectory).await?;

        let linked_inside = downloads_dir.join("linked-inside");
        symlink(&real_subdirectory, &linked_inside)?;
        let linked_destination = linked_inside.join("torrent");
        assert!(
            super::validate_torrent_destination(&linked_destination, &downloads_dir)
                .await
                .is_err(),
            "symlinked destination ancestors inside the root must be rejected"
        );
        assert!(
            !real_subdirectory.join("torrent").exists(),
            "rejected symlink destinations must not create a path at their target"
        );

        let linked_outside = downloads_dir.join("linked-outside");
        symlink(&outside_dir, &linked_outside)?;
        assert!(
            super::validate_torrent_destination(&linked_outside, &downloads_dir)
                .await
                .is_err(),
            "destination symlinks escaping the root must be rejected"
        );

        let configured_root_link = root.path().join("downloads-link");
        symlink(&downloads_dir, &configured_root_link)?;
        let configured_destination = configured_root_link.join("torrent");
        let validated =
            super::validate_torrent_destination(&configured_destination, &configured_root_link)
                .await
                .map_err(std::io::Error::other)?;
        assert!(validated.starts_with(tokio::fs::canonicalize(&downloads_dir).await?));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn torrent_destination_junction_escape_is_caught_after_creation()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = tempdir()?;
        let downloads_dir = root.path().join("downloads");
        let outside_dir = root.path().join("outside");
        tokio::fs::create_dir_all(&downloads_dir).await?;
        tokio::fs::create_dir_all(&outside_dir).await?;

        // Simulate a TOCTOU attack: plant a symlink pointing outside the root
        // at a path component that was NotFound during the ancestor walk.
        // We do this by creating the symlink directly at a not-yet-existing
        // intermediate path inside the downloads dir so validate_torrent_destination
        // sees it as missing during the walk but then follows it on create_dir_all.
        let intermediate = downloads_dir.join("intermediate");
        // intermediate does not exist yet — this is what the ancestor walk would see
        symlink(&outside_dir, &intermediate)?;

        // The destination is nested under the symlinked intermediate.
        let destination = intermediate.join("malicious");

        let result = super::validate_torrent_destination(&destination, &downloads_dir).await;
        assert!(
            result.is_err(),
            "TOCTOU symlink escape must be caught after create_dir_all, got: {result:?}"
        );
        // The outside directory must not have been permanently polluted.
        assert!(
            !outside_dir.join("malicious").exists(),
            "escaped directory must have been cleaned up"
        );
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn torrent_destination_rejects_unc_and_drive_paths_outside_the_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let downloads_dir = root.path().join("downloads");
        tokio::fs::create_dir_all(&downloads_dir).await?;
        let unc = std::path::PathBuf::from(r"\\server\share\outside");
        assert!(
            super::validate_torrent_destination(&unc, &downloads_dir)
                .await
                .is_err()
        );

        let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
        let drive_path = std::path::PathBuf::from(format!(
            "{system_drive}\\fetchx-outside-downloads-{}",
            fetchx_models::DownloadId::new()
        ));
        assert!(
            super::validate_torrent_destination(&drive_path, &downloads_dir)
                .await
                .is_err()
        );
        assert!(!drive_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn restart_recovers_persisted_download_state() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let paths = config(root.path());
        persist_test_torrent_config(&paths, &root.path().join("runtime/native-torrent")).await?;
        let services = CoreServices::start(paths.clone()).await?;
        let now = Timestamp::now()?;
        let id = DownloadId::new();
        let item = DownloadItem::from_parts(
            id,
            DownloadSource::new("https://example.com/file.zip", EngineKind::DirectHttp)?,
            FileName::new("file.zip")?,
            paths.downloads_dir.join("file.zip"),
            DownloadStatus::Paused,
            DownloadPriority::default(),
            DownloadProgress::default(),
            now,
            now,
            None,
        )?;
        services.repository.create(&item).await?;
        services.shutdown().await?;

        let restarted = CoreServices::start(paths).await?;
        let recovered = restarted
            .repository
            .get(id)
            .await?
            .ok_or("persisted download missing after restart")?;
        assert_eq!(recovered.status(), DownloadStatus::Paused);
        restarted.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn legacy_aria2_rows_and_files_are_preserved_on_startup()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let paths = config(root.path());
        persist_test_torrent_config(&paths, &root.path().join("runtime/native-torrent")).await?;
        let services = CoreServices::start(paths.clone()).await?;
        let now = Timestamp::now()?;
        let id = DownloadId::new();
        let destination = root.path().join("downloads").join("partial.bin");
        let payload = b"existing partial payload";
        let control_data = b"aria2 control data";
        tokio::fs::write(&destination, payload).await?;
        let mut sidecar = destination.as_os_str().to_os_string();
        sidecar.push(".aria2");
        let sidecar = std::path::PathBuf::from(sidecar);
        tokio::fs::write(&sidecar, control_data).await?;

        let aria2_item = DownloadItem::from_parts(
            id,
            DownloadSource::new("https://example.test/partial.bin", EngineKind::Aria2)?,
            FileName::new("partial.bin")?,
            destination.clone(),
            DownloadStatus::Paused,
            DownloadPriority::default(),
            DownloadProgress::new(4, Some(100), None, None)?,
            now,
            now,
            None,
        )?;
        services.repository.create(&aria2_item).await?;
        services.shutdown().await?;

        let restarted = CoreServices::start(paths).await?;
        let after = restarted
            .repository
            .get(id)
            .await?
            .ok_or("missing legacy aria2 row after startup")?;
        assert_eq!(aria2_item, after);
        assert_eq!(tokio::fs::read(&destination).await?, payload);
        assert_eq!(tokio::fs::read(&sidecar).await?, control_data);
        restarted.shutdown().await?;
        Ok(())
    }
}

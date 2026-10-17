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
//! };
//! let services = CoreServices::start(config).await?;
//! // use services.queue(), services.scheduler(), etc.
//! services.shutdown().await?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

use std::{path::PathBuf, sync::Arc};

use fetchx_downloader::{
    Aria2Client, Aria2ClientConfig, Aria2Engine, Aria2Supervisor, Aria2SupervisorConfig,
    DirectHttpEngine, EngineDispatcher, HlsEngine, YtDlpConfig as DownloaderYtDlpConfig,
    YtDlpEngine, YtDlpProcessRunner,
};
use fetchx_models::EngineKind;
use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSource, FileName, Timestamp,
    parse_magnet_uri, parse_torrent_metainfo,
};
use fetchx_queue::{DownloadEventBus, EventRepository, QueueManager};
use fetchx_scheduler::Scheduler;
use fetchx_storage::{Database, SqliteDownloadRepository};
use thiserror::Error;
use tokio::{fs, io::AsyncReadExt, process::Command, sync::Mutex, time::Duration};
use tracing::{info, warn};

pub use config::{
    Aria2Config, ConfigError, ConfigService, DirectHttpConfig, EngineConfig, YtDlpConfig,
};
pub use engine_status::{EngineAvailability, EngineRuntimeStatus};

pub mod config;
mod engine_status;

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

impl CoreServices {
    /// Initialises all services from the supplied paths.
    ///
    /// Optional engines that fail to start are marked `Unavailable`; the
    /// function only fails for unrecoverable errors (e.g. database I/O).
    pub async fn start(cfg: CoreServicesConfig) -> Result<Self, CoreServicesError> {
        Self::start_with_engine_config(cfg, None).await
    }

    /// Initializes services with a per-process engine configuration override.
    ///
    /// This is used by Android, where executable paths are private to the app
    /// installation and must not be persisted as desktop engine settings.
    pub async fn start_with_engine_config(
        cfg: CoreServicesConfig,
        engine_config_override: Option<EngineConfig>,
    ) -> Result<Self, CoreServicesError> {
        Self::start_with_engine_config_and_ytdlp_runner(cfg, engine_config_override, None).await
    }

    pub async fn start_with_engine_config_and_ytdlp_runner(
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
            None => config_service
                .load()
                .await
                .map_err(CoreServicesError::Config)?,
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
        )
        .await?;

        // Queue + Scheduler
        let scheduler = match Scheduler::start_with_resource_policy(
            repository.clone(),
            engine_runtime.dispatcher.clone(),
            engine_config.resource_policy.clone(),
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

    #[must_use]
    pub fn scheduler(&self) -> &Scheduler<CoreRepository> {
        &self.scheduler
    }

    #[must_use]
    pub fn downloads_dir(&self) -> &std::path::Path {
        &self.downloads_dir
    }

    /// Validates and queues a magnet job through the shared aria2 runtime.
    ///
    /// Magnet output is isolated in a per-job directory unless the caller
    /// explicitly supplies a destination directory.
    pub async fn enqueue_magnet(
        &self,
        uri: &str,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
    ) -> Result<DownloadId, CoreServicesError> {
        let magnet = parse_magnet_uri(uri)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        if !self.engine_runtime.is_available(EngineKind::Aria2) {
            return Err(CoreServicesError::UnavailableEngine(EngineKind::Aria2));
        }
        let id = DownloadId::new();
        let source = DownloadSource::new(uri, EngineKind::Aria2)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        let filename = FileName::new(magnet.suggested_filename())
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        let destination = destination.unwrap_or_else(|| {
            self.downloads_dir
                .join(format!("torrent-{}-{id}", magnet.info_hash))
        });
        let created_at = Timestamp::now()
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        let mut item = DownloadItem::new(id, source, filename, destination, created_at);
        item.set_priority_at(priority, created_at)
            .map_err(|error| CoreServicesError::InvalidMagnet(error.to_string()))?;
        self.queue
            .enqueue(item)
            .await
            .map_err(|error| CoreServicesError::Queue(error.to_string()))?;
        Ok(id)
    }

    /// Validates and queues a local `.torrent` file through shared aria2.
    ///
    /// The download destination is a directory. If omitted, each torrent is
    /// isolated in a directory named from its info hash and download ID.
    pub async fn enqueue_torrent_file(
        &self,
        torrent_path: PathBuf,
        priority: DownloadPriority,
        destination: Option<PathBuf>,
    ) -> Result<DownloadId, CoreServicesError> {
        if !torrent_path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
        {
            return Err(CoreServicesError::InvalidTorrent(
                "source must have a .torrent extension".to_owned(),
            ));
        }
        let metadata = fs::symlink_metadata(&torrent_path)
            .await
            .map_err(CoreServicesError::Io)?;
        if !metadata.file_type().is_file() || metadata.len() > 8 * 1024 * 1024 {
            return Err(CoreServicesError::InvalidTorrent(
                "source must be a regular torrent file no larger than 8 MiB".to_owned(),
            ));
        }
        let torrent_file = fs::File::open(&torrent_path)
            .await
            .map_err(CoreServicesError::Io)?;
        let mut torrent_bytes = Vec::new();
        torrent_file
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
        if !self.engine_runtime.is_available(EngineKind::Aria2) {
            return Err(CoreServicesError::UnavailableEngine(EngineKind::Aria2));
        }
        let torrent_path = fs::canonicalize(torrent_path)
            .await
            .map_err(CoreServicesError::Io)?;
        let source = DownloadSource::new_torrent_file(&torrent_path, EngineKind::Aria2)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let filename = FileName::new(torrent.name)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let id = DownloadId::new();
        let destination = destination.unwrap_or_else(|| {
            self.downloads_dir
                .join(format!("torrent-{}-{id}", torrent.info_hash))
        });
        let created_at = Timestamp::now()
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        let mut item = DownloadItem::new(id, source, filename, destination, created_at);
        item.set_priority_at(priority, created_at)
            .map_err(|error| CoreServicesError::InvalidTorrent(error.to_string()))?;
        self.queue
            .enqueue(item)
            .await
            .map_err(|error| CoreServicesError::Queue(error.to_string()))?;
        Ok(id)
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
    pub aria2_supervisor: Option<Arc<Aria2Supervisor>>,
    pub status: EngineRuntimeStatus,
}

impl EngineRuntime {
    pub async fn initialize(
        repository: CoreRepository,
        config: &EngineConfig,
        runtime_dir: &std::path::Path,
    ) -> Result<Self, CoreServicesError> {
        Self::initialize_with_ytdlp_runner(repository, config, runtime_dir, None).await
    }

    async fn initialize_with_ytdlp_runner(
        repository: CoreRepository,
        config: &EngineConfig,
        runtime_dir: &std::path::Path,
        ytdlp_runner: Option<Arc<dyn YtDlpProcessRunner>>,
    ) -> Result<Self, CoreServicesError> {
        let direct_http = Arc::new(
            DirectHttpEngine::new(repository.clone())
                .map_err(|e| CoreServicesError::Engine(e.to_string()))?,
        );

        let hls_engine = Arc::new(
            HlsEngine::new(repository.clone())
                .map_err(|e| CoreServicesError::Engine(e.to_string()))?,
        );

        let (aria2_engine, aria2_supervisor, aria2_avail) =
            init_aria2(repository.clone(), config, runtime_dir).await;
        let (yt_dlp_engine, ytdlp_avail) =
            init_ytdlp(repository.clone(), config, ytdlp_runner).await;

        log_avail("aria2", &aria2_avail);
        log_avail("yt-dlp", &ytdlp_avail);
        info!(engine = "hls", "engine ready");

        let mut dispatcher =
            EngineDispatcher::new(repository, direct_http, aria2_engine).with_hls(hls_engine);
        if let Some(engine) = yt_dlp_engine {
            dispatcher = dispatcher.with_yt_dlp(engine);
        }

        Ok(Self {
            dispatcher: Arc::new(dispatcher),
            aria2_supervisor,
            status: EngineRuntimeStatus {
                aria2: aria2_avail,
                yt_dlp: ytdlp_avail,
            },
        })
    }

    pub async fn shutdown(&self) -> Result<(), CoreServicesError> {
        if let Some(sup) = &self.aria2_supervisor {
            sup.shutdown()
                .await
                .map_err(|error| CoreServicesError::Shutdown(error.to_string()))?;
            info!("aria2 supervisor shut down");
        }
        Ok(())
    }

    /// `true` when `kind` is currently available at runtime.
    pub fn is_available(&self, kind: EngineKind) -> bool {
        match kind {
            EngineKind::DirectHttp | EngineKind::Hls => true,
            EngineKind::Aria2 => matches!(self.status.aria2, EngineAvailability::Available { .. }),
            EngineKind::YtDlp => matches!(self.status.yt_dlp, EngineAvailability::Available { .. }),
        }
    }
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

// ── aria2 init ────────────────────────────────────────────────────────────

async fn init_aria2(
    repository: CoreRepository,
    config: &EngineConfig,
    runtime_dir: &std::path::Path,
) -> (
    Arc<Aria2Engine<CoreRepository>>,
    Option<Arc<Aria2Supervisor>>,
    EngineAvailability,
) {
    if !config.aria2.enabled {
        return (stub_aria2(repository), None, EngineAvailability::Disabled);
    }
    let executable: PathBuf = config.aria2.executable_path.clone().unwrap_or_else(|| {
        if cfg!(target_os = "windows") {
            "aria2c.exe".into()
        } else {
            "aria2c".into()
        }
    });
    let sup_cfg =
        Aria2SupervisorConfig::new(executable.clone()).with_runtime_directory(runtime_dir);
    match Aria2Supervisor::start(sup_cfg).await {
        Ok(sup) => {
            let version = detect_aria2_version(&executable).await;
            let engine = Arc::new(Aria2Engine::with_supervisor(repository, Arc::clone(&sup)));
            (engine, Some(sup), EngineAvailability::Available { version })
        }
        Err(e) => {
            warn!(error = %e, "aria2 supervisor failed to start");
            (
                stub_aria2(repository),
                None,
                EngineAvailability::Unavailable {
                    reason: e.to_string(),
                },
            )
        }
    }
}

async fn detect_aria2_version(executable: &std::path::Path) -> String {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(executable).arg("--version").output(),
    )
    .await;
    match result {
        Ok(Ok(out)) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("aria2")
            .trim()
            .to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn stub_aria2(repository: CoreRepository) -> Arc<Aria2Engine<CoreRepository>> {
    let client = Aria2Client::new(Aria2ClientConfig::new(
        "http://127.0.0.1:1/jsonrpc",
        None,
        Duration::from_millis(1),
    ))
    .expect("stub aria2 client is statically valid");
    Arc::new(Aria2Engine::new(repository, client))
}

// ── yt-dlp init ──────────────────────────────────────────────────────────

async fn init_ytdlp(
    repository: CoreRepository,
    config: &EngineConfig,
    runner: Option<Arc<dyn YtDlpProcessRunner>>,
) -> (Option<Arc<YtDlpEngine<CoreRepository>>>, EngineAvailability) {
    if !config.yt_dlp.enabled {
        return (None, EngineAvailability::Disabled);
    }
    let mut ytdlp_cfg = DownloaderYtDlpConfig::new();
    if let Some(p) = &config.yt_dlp.executable_path {
        ytdlp_cfg = ytdlp_cfg.with_executable(p.clone());
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
        DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use fetchx_storage::DownloadRepository;

    use super::{CoreServices, CoreServicesConfig, EngineAvailability};

    fn config(root: &std::path::Path) -> CoreServicesConfig {
        CoreServicesConfig {
            database_path: root.join("fetchx.db"),
            downloads_dir: root.join("downloads"),
            runtime_dir: root.join("runtime"),
        }
    }

    #[tokio::test]
    async fn starts_shared_graph_and_shutdown_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = CoreServices::start(config(root.path())).await?;

        assert!(services.engine_runtime.is_available(EngineKind::DirectHttp));
        assert!(services.engine_runtime.is_available(EngineKind::Hls));
        assert!(services.queue().snapshot().await?.items.is_empty());

        let (first, second) = tokio::join!(services.shutdown(), services.shutdown());
        first?;
        second?;
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_service_graphs_can_start_and_stop_independently()
    -> Result<(), Box<dyn std::error::Error>> {
        let first_root = tempdir()?;
        let second_root = tempdir()?;
        let (first, second) = tokio::try_join!(
            CoreServices::start(config(first_root.path())),
            CoreServices::start(config(second_root.path()))
        )?;

        let (first_result, second_result) = tokio::join!(first.shutdown(), second.shutdown());
        first_result?;
        second_result?;
        Ok(())
    }

    #[tokio::test]
    async fn persists_configuration_and_reports_optional_engine_start_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{Aria2Config, ConfigService, EngineConfig, YtDlpConfig};
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
            ..EngineConfig::default()
        };
        config_service.save(&engine_config).await?;
        drop(database);

        let services = CoreServices::start(paths).await?;
        assert!(matches!(
            &services.engine_runtime.status.yt_dlp,
            EngineAvailability::Unavailable { .. }
        ));
        assert!(matches!(
            &services.engine_runtime.status.aria2,
            EngineAvailability::Unavailable { .. }
        ));
        assert!(services.config_service.load().await?.yt_dlp.enabled);

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn shared_magnet_enqueue_validates_before_engine_availability()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let services = CoreServices::start(CoreServicesConfig {
            database_path: dir.path().join("fetchx.db"),
            downloads_dir: dir.path().join("downloads"),
            runtime_dir: dir.path().join("runtime"),
        })
        .await?;

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
                EngineKind::Aria2
            ))
        ));
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn shared_torrent_enqueue_validates_metainfo_before_engine_availability()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = CoreServices::start(config(root.path())).await?;
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
                EngineKind::Aria2
            ))
        ));
        assert!(services.queue().snapshot().await?.items.is_empty());
        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn restart_recovers_persisted_download_state() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let paths = config(root.path());
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
}

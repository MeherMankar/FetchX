//! Platform-independent service bootstrap for FetchX.
//!
//! Both the Tauri desktop application and the CLI share the same core
//! services: database, repository, engine runtime, queue, and scheduler.
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
use fetchx_queue::{DownloadEventBus, EventRepository, QueueConfig, QueueManager};
use fetchx_scheduler::Scheduler;
use fetchx_storage::{Database, SqliteDownloadRepository};
use thiserror::Error;
use tokio::{fs, process::Command, time::Duration};
use tracing::{info, warn};

pub use config::{Aria2Config, ConfigService, DirectHttpConfig, EngineConfig, YtDlpConfig};
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
/// Drop-in for both the desktop `AppState` and the CLI.
pub struct CoreServices {
    pub database: Database,
    pub repository: CoreRepository,
    pub event_bus: DownloadEventBus,
    pub config_service: ConfigService,
    pub engine_runtime: Arc<EngineRuntime>,
    pub scheduler: Scheduler<CoreRepository>,
    queue: QueueManager,
    pub downloads_dir: PathBuf,
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
            None => config_service.load().await.unwrap_or_default(),
        };

        // Engine runtime
        let engine_runtime = EngineRuntime::initialize_with_ytdlp_runner(
            repository.clone(),
            &engine_config,
            &cfg.runtime_dir,
            ytdlp_runner,
        )
        .await?;

        // Queue + Scheduler
        let scheduler = Scheduler::start(
            repository.clone(),
            engine_runtime.dispatcher.clone(),
            QueueConfig::default(),
        )
        .await
        .map_err(|e| CoreServicesError::Queue(e.to_string()))?;
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

    // ── Shutdown ──────────────────────────────────────────────────────────

    /// Gracefully shuts down all services.
    ///
    /// Order: scheduler → queue → optional engines.
    pub async fn shutdown(self) -> Result<(), CoreServicesError> {
        if let Err(e) = self.scheduler.shutdown().await {
            warn!(error = %e, "scheduler shutdown error");
        }
        self.engine_runtime.shutdown().await;
        Ok(())
    }
}

// ============================================================
// EngineRuntime (extracted from desktop engines.rs)
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

        let mut dispatcher = EngineDispatcher::new(repository, direct_http, aria2_engine)
            .with_hls(hls_engine);
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

    pub async fn shutdown(&self) {
        if let Some(sup) = &self.aria2_supervisor {
            match sup.shutdown().await {
                Ok(()) => info!("aria2 supervisor shut down"),
                Err(e) => warn!(error = %e, "aria2 shutdown error"),
            }
        }
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
    #[error("queue error: {0}")]
    Queue(String),
}

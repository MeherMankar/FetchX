//! Runtime engine initialisation and availability tracking.
//!
//! Reads persisted [`EngineConfig`], attempts to start each optional engine,
//! and produces a concrete [`EngineDispatcher`] for the desktop application.
//!
//! # Design contract
//! - Direct HTTP is always available and never fails initialisation.
//! - aria2 and yt-dlp are optional.  A startup failure marks the engine
//!   `Unavailable` but does not prevent the application from running.
//! - The [`Aria2Supervisor`] is owned here for clean shutdown.
//! - No RPC secrets or authentication tokens are logged or exposed.

use std::{path::PathBuf, sync::Arc, time::Duration};

use fetchx_downloader::{
    Aria2Client, Aria2ClientConfig, Aria2Engine, Aria2Supervisor, Aria2SupervisorConfig,
    DirectHttpEngine, EngineDispatcher, YtDlpConfig as DownloaderYtDlpConfig, YtDlpEngine,
};
use tokio::process::Command;
use tracing::{info, warn};

use crate::commands::AppRepository;
use crate::config::EngineConfig;

// ---------------------------------------------------------------------------
// Availability state
// ---------------------------------------------------------------------------

/// Runtime availability of an optional engine.
#[derive(Clone, Debug)]
pub enum EngineAvailability {
    /// Disabled in persisted configuration.
    Disabled,
    /// Initialised successfully; `version` is the executable's version string.
    Available { version: String },
    /// Enabled in configuration but could not be started.
    Unavailable { reason: String },
}

/// Runtime status of all engines, shared with Tauri commands.
#[derive(Clone, Debug)]
pub struct EngineRuntimeStatus {
    pub aria2: EngineAvailability,
    pub yt_dlp: EngineAvailability,
}

// ---------------------------------------------------------------------------
// EngineRuntime — owns supervisors and the dispatcher
// ---------------------------------------------------------------------------

/// Owns optional engine supervisors and the dispatcher used by the queue.
pub struct EngineRuntime {
    pub dispatcher: Arc<EngineDispatcher<AppRepository>>,
    /// Kept alive for graceful shutdown.  `None` if aria2 was not started.
    pub aria2_supervisor: Option<Arc<Aria2Supervisor>>,
    pub status: EngineRuntimeStatus,
}

impl EngineRuntime {
    /// Initialises all configured engines; optional engine failures are
    /// captured as `Unavailable` rather than being propagated.
    pub async fn initialize(
        repository: AppRepository,
        config: &EngineConfig,
        runtime_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Direct HTTP — always present.
        let direct_http = Arc::new(DirectHttpEngine::new(repository.clone())?);

        // aria2 — optional.
        let (aria2_engine, aria2_supervisor, aria2_avail) =
            init_aria2(repository.clone(), config, &runtime_dir).await;

        // yt-dlp — optional.
        let (yt_dlp_engine, ytdlp_avail) = init_ytdlp(repository.clone(), config).await;

        // Log summary (no secrets).
        log_availability("aria2", &aria2_avail);
        log_availability("yt-dlp", &ytdlp_avail);

        // Build dispatcher.
        let mut dispatcher = EngineDispatcher::new(repository, direct_http, aria2_engine);
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

    /// Shuts down supervised optional engines gracefully.
    /// Errors are logged but not propagated — the application must always exit.
    pub async fn shutdown(&self) {
        if let Some(supervisor) = &self.aria2_supervisor {
            match supervisor.shutdown().await {
                Ok(()) => info!("aria2 supervisor shut down cleanly"),
                Err(e) => warn!(error = %e, "aria2 supervisor shutdown encountered an error"),
            }
        }
    }

    /// Returns `true` when the given engine kind is currently available at runtime.
    /// Direct HTTP is always available.
    pub fn is_available(&self, kind: fetchx_models::EngineKind) -> bool {
        use fetchx_models::EngineKind;
        match kind {
            EngineKind::DirectHttp => true,
            EngineKind::Aria2 => matches!(self.status.aria2, EngineAvailability::Available { .. }),
            EngineKind::YtDlp => {
                matches!(self.status.yt_dlp, EngineAvailability::Available { .. })
            }
        }
    }
}

fn log_availability(name: &str, avail: &EngineAvailability) {
    match avail {
        EngineAvailability::Available { version } => {
            info!(engine = name, version = %version, "engine ready");
        }
        EngineAvailability::Unavailable { reason } => {
            warn!(engine = name, reason = %reason, "engine unavailable at startup");
        }
        EngineAvailability::Disabled => {
            info!(engine = name, "engine disabled");
        }
    }
}

// ---------------------------------------------------------------------------
// aria2 initialisation
// ---------------------------------------------------------------------------

async fn init_aria2(
    repository: AppRepository,
    config: &EngineConfig,
    runtime_dir: &std::path::Path,
) -> (
    Arc<Aria2Engine<AppRepository>>,
    Option<Arc<Aria2Supervisor>>,
    EngineAvailability,
) {
    if !config.aria2.enabled {
        return (
            stub_aria2_engine(repository),
            None,
            EngineAvailability::Disabled,
        );
    }

    let executable: PathBuf = match &config.aria2.executable_path {
        Some(p) => p.clone(),
        None => {
            if cfg!(target_os = "windows") {
                "aria2c.exe".into()
            } else {
                "aria2c".into()
            }
        }
    };

    let supervisor_config =
        Aria2SupervisorConfig::new(executable.clone()).with_runtime_directory(runtime_dir);

    match Aria2Supervisor::start(supervisor_config).await {
        Ok(supervisor) => {
            // Detect version with a structured --version call (no shell).
            let version = detect_aria2_version(&executable).await;
            let engine = Arc::new(Aria2Engine::with_supervisor(
                repository,
                Arc::clone(&supervisor),
            ));
            (
                engine,
                Some(supervisor),
                EngineAvailability::Available { version },
            )
        }
        Err(e) => {
            warn!(error = %e, "aria2 supervisor failed to start");
            (
                stub_aria2_engine(repository),
                None,
                EngineAvailability::Unavailable {
                    reason: e.to_string(),
                },
            )
        }
    }
}

/// Runs `aria2c --version` with structured arguments (no shell) and extracts
/// the first output line as the version string.
async fn detect_aria2_version(executable: &std::path::Path) -> String {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(executable).arg("--version").output(),
    )
    .await;

    match result {
        Ok(Ok(out)) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().next().unwrap_or("aria2").trim().to_owned()
        }
        _ => "unknown".to_owned(),
    }
}

/// Builds a stub Aria2Engine from a minimal, unreachable client.
/// The engine will return `EngineNotAvailable` if any Aria2 download is attempted
/// while the engine is disabled/unavailable — the same behaviour as before Phase 16B.
fn stub_aria2_engine(repository: AppRepository) -> Arc<Aria2Engine<AppRepository>> {
    // Port 1 is privileged and unreachable; this client is never used for real RPC.
    let client = Aria2Client::new(Aria2ClientConfig::new(
        "http://127.0.0.1:1/jsonrpc",
        None,
        Duration::from_millis(1),
    ))
    .expect("stub aria2 client configuration is statically valid");
    Arc::new(Aria2Engine::new(repository, client))
}

// ---------------------------------------------------------------------------
// yt-dlp initialisation
// ---------------------------------------------------------------------------

async fn init_ytdlp(
    repository: AppRepository,
    config: &EngineConfig,
) -> (Option<Arc<YtDlpEngine<AppRepository>>>, EngineAvailability) {
    if !config.yt_dlp.enabled {
        return (None, EngineAvailability::Disabled);
    }

    let mut ytdlp_config = DownloaderYtDlpConfig::new();
    if let Some(exec_path) = &config.yt_dlp.executable_path {
        ytdlp_config = ytdlp_config.with_executable(exec_path.clone());
    }

    match YtDlpEngine::initialize(repository, ytdlp_config).await {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use fetchx_storage::Database;

    use super::*;
    use crate::commands::AppRepository;
    use crate::config::{EngineConfig, YtDlpConfig as CfgYtDlp};
    use fetchx_queue::{DownloadEventBus, EventRepository};

    async fn make_repository() -> AppRepository {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let event_bus = DownloadEventBus::new();
        EventRepository::new(db.downloads(), event_bus)
    }

    #[tokio::test]
    async fn direct_http_always_initializes() {
        let repo = make_repository().await;
        let cfg = EngineConfig::default(); // aria2 + yt-dlp disabled
        let runtime = EngineRuntime::initialize(repo, &cfg, std::env::temp_dir())
            .await
            .expect("initialization with defaults must succeed");
        assert!(runtime.aria2_supervisor.is_none());
        assert!(matches!(runtime.status.aria2, EngineAvailability::Disabled));
        assert!(matches!(
            runtime.status.yt_dlp,
            EngineAvailability::Disabled
        ));
    }

    #[tokio::test]
    async fn aria2_disabled_produces_no_supervisor() {
        let repo = make_repository().await;
        let cfg = EngineConfig {
            aria2: crate::config::Aria2Config {
                enabled: false,
                ..Default::default()
            },
            ..EngineConfig::default()
        };
        let runtime = EngineRuntime::initialize(repo, &cfg, std::env::temp_dir())
            .await
            .unwrap();
        assert!(runtime.aria2_supervisor.is_none());
        assert!(matches!(runtime.status.aria2, EngineAvailability::Disabled));
    }

    #[tokio::test]
    async fn aria2_enabled_with_nonexistent_executable_is_unavailable_not_fatal() {
        let repo = make_repository().await;
        let cfg = EngineConfig {
            aria2: crate::config::Aria2Config {
                enabled: true,
                executable_path: Some("/nonexistent/path/to/aria2c".into()),
                rpc_port: 6800,
                rpc_secret: None,
            },
            ..EngineConfig::default()
        };
        let runtime = EngineRuntime::initialize(repo, &cfg, std::env::temp_dir())
            .await
            .expect("unavailable aria2 must not fail initialization");
        assert!(runtime.aria2_supervisor.is_none());
        assert!(
            matches!(runtime.status.aria2, EngineAvailability::Unavailable { .. }),
            "expected Unavailable, got {:?}",
            runtime.status.aria2
        );
    }

    #[tokio::test]
    async fn ytdlp_disabled_produces_no_engine() {
        let repo = make_repository().await;
        let cfg = EngineConfig {
            yt_dlp: CfgYtDlp {
                enabled: false,
                executable_path: None,
            },
            ..EngineConfig::default()
        };
        let runtime = EngineRuntime::initialize(repo, &cfg, std::env::temp_dir())
            .await
            .unwrap();
        assert!(matches!(
            runtime.status.yt_dlp,
            EngineAvailability::Disabled
        ));
    }

    #[tokio::test]
    async fn ytdlp_enabled_with_nonexistent_executable_is_unavailable_not_fatal() {
        let repo = make_repository().await;
        let cfg = EngineConfig {
            yt_dlp: CfgYtDlp {
                enabled: true,
                executable_path: Some("/nonexistent/path/to/yt-dlp".into()),
            },
            ..EngineConfig::default()
        };
        let runtime = EngineRuntime::initialize(repo, &cfg, std::env::temp_dir())
            .await
            .expect("unavailable yt-dlp must not fail initialization");
        assert!(
            matches!(
                runtime.status.yt_dlp,
                EngineAvailability::Unavailable { .. }
            ),
            "expected Unavailable, got {:?}",
            runtime.status.yt_dlp
        );
    }

    #[tokio::test]
    async fn engine_status_dto_does_not_leak_secrets() {
        // Verify via the engine_commands module logic.
        use crate::engine_commands::engine_instance_dto;
        let avail = EngineAvailability::Unavailable {
            reason: "some reason".to_owned(),
        };
        let dto = engine_instance_dto(&avail);
        let json = serde_json::to_string(&dto).unwrap();
        // The error field contains "some reason" but no secrets.
        assert!(!json.contains("secret"));
        assert!(!json.contains("token"));
        assert!(dto.error.is_some());
        assert!(!dto.available);
    }
}


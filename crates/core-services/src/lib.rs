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
    TorrentFileSelection, TorrentMetadata, parse_magnet_uri, parse_torrent_metainfo,
};
use fetchx_queue::{DownloadEventBus, EventRepository, QueueManager};
use fetchx_scheduler::Scheduler;
use fetchx_storage::{
    Database, DownloadRepository, SqliteDownloadRepository, TorrentMetadataRepository,
    TorrentMetadataRow, TorrentSeedState, TorrentSourceType,
};
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
            Some(database.torrent_metadata()),
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

    /// Durable torrent metadata repository for aria2 magnet/.torrent downloads.
    #[must_use]
    pub fn torrent_metadata(&self) -> fetchx_storage::SqliteTorrentMetadataRepository {
        self.database.torrent_metadata()
    }

    /// Queries current torrent telemetry from aria2 for a persisted torrent task.
    pub async fn torrent_telemetry(
        &self,
        id: DownloadId,
    ) -> Result<Option<fetchx_models::TorrentTelemetry>, CoreServicesError> {
        self.engine_runtime
            .aria2_engine
            .torrent_telemetry(id)
            .await
            .map_err(|error| CoreServicesError::Engine(error.to_string()))
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
        if !self.engine_runtime.is_available(EngineKind::Aria2) {
            return Err(CoreServicesError::UnavailableEngine(EngineKind::Aria2));
        }
        let source = DownloadSource::new_torrent_file(&torrent_path, EngineKind::Aria2)
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
    ///
    /// The download must be in `Seeding` state. After stopping seeding the
    /// underlying aria2 task is removed.
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
        self.queue
            .cancel(id)
            .await
            .map_err(|e| CoreServicesError::Queue(e.to_string()))?;
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
        self.save_magnet_selection(id, selection, true).await
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
        self.save_magnet_selection(id, selection, false).await
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
        self.save_magnet_selection(id, selection, false).await
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
        self.save_magnet_selection(id, selection, false).await
    }

    async fn save_magnet_selection(
        &self,
        id: DownloadId,
        selection: TorrentFileSelection,
        confirmed: bool,
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
            return Err(CoreServicesError::InvalidTorrent(
                "magnet metadata is not ready".to_owned(),
            ));
        }
        let item = self
            .repository
            .get(id)
            .await
            .map_err(|error| CoreServicesError::Database(error.to_string()))?
            .ok_or_else(|| CoreServicesError::InvalidTorrent("download not found".to_owned()))?;
        if item.status() != fetchx_models::DownloadStatus::MetadataAcquiring {
            return Err(CoreServicesError::InvalidTorrent(
                "file selection is only available while awaiting magnet selection".to_owned(),
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
    pub aria2_engine: Arc<Aria2Engine<CoreRepository>>,
    pub aria2_supervisor: Option<Arc<Aria2Supervisor>>,
    pub status: EngineRuntimeStatus,
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

        let (aria2_engine, aria2_supervisor, aria2_avail) =
            init_aria2(repository.clone(), config, runtime_dir, torrent_metadata).await;
        let (yt_dlp_engine, ytdlp_avail) =
            init_ytdlp(repository.clone(), config, ytdlp_runner).await;

        log_avail("aria2", &aria2_avail);
        log_avail("yt-dlp", &ytdlp_avail);
        info!(engine = "hls", "engine ready");

        let mut dispatcher =
            EngineDispatcher::new(repository, direct_http, Arc::clone(&aria2_engine))
                .with_hls(hls_engine);
        if let Some(engine) = yt_dlp_engine {
            dispatcher = dispatcher.with_yt_dlp(engine);
        }

        Ok(Self {
            dispatcher: Arc::new(dispatcher),
            aria2_engine,
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
    torrent_metadata: Option<fetchx_storage::SqliteTorrentMetadataRepository>,
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
            let mut engine = Aria2Engine::with_supervisor(repository, Arc::clone(&sup));
            if let Some(torrent_metadata) = torrent_metadata {
                engine = engine.with_torrent_metadata_repository(torrent_metadata);
            }
            let engine = Arc::new(engine);
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

async fn validate_torrent_destination(
    destination: &std::path::Path,
    downloads_dir: &std::path::Path,
) -> Result<PathBuf, String> {
    // Reject UNC paths on Windows (\\server\share).
    #[cfg(windows)]
    {
        let s = destination.to_string_lossy();
        if s.starts_with("\\\\") {
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
        DownloadStatus, EngineKind, FileName, Timestamp, TorrentFile, TorrentMetadata,
    };
    use fetchx_storage::{
        DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
        TorrentSourceType,
    };

    use super::{CoreServices, CoreServicesConfig, EngineAvailability};
    use fetchx_core::DownloadStateMachine;

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
    async fn magnet_file_selection_uses_shared_model_and_persists_size_and_indexes()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = CoreServices::start(config(root.path())).await?;
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
    async fn torrent_destination_traversal_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = CoreServices::start(config(root.path())).await?;
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

    #[tokio::test]
    async fn magnet_destination_traversal_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let services = CoreServices::start(config(root.path())).await?;

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

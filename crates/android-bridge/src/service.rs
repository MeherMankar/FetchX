use std::path::PathBuf;
use std::{
    ffi::OsString,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_downloader::{
    YtDlpError, YtDlpFormat, YtDlpProcessExit, YtDlpProcessRunner, YtDlpRunnerEvent,
    YtDlpRunnerFuture, is_hls_url, parse_format_output,
};
use fetchx_models::{
    DiagnosticComponent, DiagnosticEvent, DiagnosticLevel, DownloadId, DownloadItem,
    DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp, global_diagnostics,
};
use fetchx_plugin_api::{PluginPolicy, PluginTrustContext, PolicyEnvironment, parse_manifest};
use fetchx_storage::DownloadRepository as _;
use tokio::{
    fs,
    sync::{broadcast, mpsc, watch},
    time::sleep,
};
use tracing::warn;
use url::Url;

fn validate_sha256(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("SHA-256 must contain exactly 64 hexadecimal characters".to_owned());
    }
    Ok(())
}

fn checksum_sidecar_path(destination: &Path) -> PathBuf {
    let mut sidecar = destination.as_os_str().to_os_string();
    sidecar.push(".fetchx.sha256");
    PathBuf::from(sidecar)
}

use crate::dto::{
    CoreConfig, DownloadInfoDto, DownloadPriorityDto, DownloadRuntimeStatusDto,
    TorrentDownloadMetadataDto, TorrentFileDto, TorrentMetadataDto, TorrentTelemetryDto,
    YtDlpFormatDto, duplicate_match_to_dto, event_to_dto, item_to_dto, priority_from_dto,
    runtime_status_to_dto,
};
use crate::error::MobileApiError;
use crate::{DownloadEventListener, YtDlpRuntime};

const MAX_ANDROID_PROGRESS_LINE_BYTES: usize = 1024;
const MAX_PLUGIN_TRUST_CONTEXT_BYTES: usize = 16_384;

/// Android bridge for evaluating manifests with the shared plugin policy.
/// The current Android app has no plugin execution or privileged-operation
/// adapter, so its supported capability set is deliberately empty.
pub struct PluginSecurityService;

impl PluginSecurityService {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate_policy(
        &self,
        manifest_json: String,
        trust_context_json: String,
    ) -> Result<String, MobileApiError> {
        if trust_context_json.len() > MAX_PLUGIN_TRUST_CONTEXT_BYTES {
            return Err(MobileApiError::Validation(
                "plugin trust context exceeds the size limit".to_owned(),
            ));
        }
        let manifest = parse_manifest(manifest_json.as_bytes())
            .map_err(|error| MobileApiError::Validation(error.to_string()))?;
        let trust_context: PluginTrustContext =
            serde_json::from_str(&trust_context_json).map_err(|error| {
                MobileApiError::Validation(format!("invalid plugin trust context: {error}"))
            })?;
        let decision = PluginPolicy::evaluate(
            &manifest,
            &trust_context,
            &PolicyEnvironment::current(Vec::new()),
        )
        .map_err(|error| MobileApiError::Validation(error.to_string()))?;
        serde_json::to_string(&decision).map_err(|error| MobileApiError::Runtime(error.to_string()))
    }
}

impl Default for PluginSecurityService {
    fn default() -> Self {
        Self::new()
    }
}

// ── Inner state that lives only while the service is running ──────────────

struct ServiceInner {
    runtime: tokio::runtime::Runtime,
    core: CoreServices,
}

// ── Public DownloadService type exposed via UniFFI ────────────────────────

/// The central Android bridge service. Wraps all core services and
/// provides a synchronous (blocking) API for Kotlin callers via UniFFI.
pub struct DownloadService {
    /// `None` after `shutdown()` has been called.
    inner: Mutex<Option<ServiceInner>>,
}

/// Returns `true` for URLs that point directly to a media file and can be
/// downloaded without yt-dlp format extraction.
fn is_direct_media_url(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    let extension = url
        .path_segments()
        .and_then(Iterator::last)
        .and_then(|filename| filename.rsplit_once('.').map(|(_, ext)| ext))
        .unwrap_or_default();
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "mp4"
            | "m4v"
            | "webm"
            | "mkv"
            | "avi"
            | "mov"
            | "ts"
            | "m3u8"
            | "m4a"
            | "mp3"
            | "ogg"
            | "flac"
            | "aac"
            | "wav"
    )
}

fn engine_for_url(url: &str) -> EngineKind {
    match Url::parse(url)
        .ok()
        .map(|url| url.scheme().to_ascii_lowercase())
    {
        Some(scheme) if scheme == "ftp" => EngineKind::Ftp,
        Some(scheme) if scheme == "ftps" => EngineKind::Ftps,
        Some(scheme) if scheme == "sftp" => EngineKind::Sftp,
        _ => EngineKind::DirectHttp,
    }
}

impl DownloadService {
    pub fn extract_download_links(&self, text: String) -> Vec<String> {
        fetchx_core_services::extract_download_links(&text)
    }

    pub fn is_queue_dispatch_paused(&self) -> Result<bool, MobileApiError> {
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.is_queue_dispatch_paused())
                .map_err(|error| MobileApiError::Runtime(error.to_string()))
        })
    }

    pub fn pause_queue_dispatch(&self) -> Result<(), MobileApiError> {
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.pause_queue_dispatch())
                .map_err(|error| MobileApiError::Runtime(error.to_string()))
        })
    }

    pub fn resume_queue_dispatch(&self) -> Result<(), MobileApiError> {
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.resume_queue_dispatch())
                .map_err(|error| MobileApiError::Runtime(error.to_string()))
        })
    }

    pub fn suggest_download_filename(&self, url: String) -> String {
        fetchx_core_services::suggest_download_filename(&url)
    }

    pub fn suggest_download_package_name(&self, url: String, filename: String) -> String {
        fetchx_core_services::suggest_download_package_name(&url, &filename)
    }

    pub fn normalize_download_url(&self, url: String) -> String {
        fetchx_core_services::normalize_download_url(&url)
    }

    pub fn is_duplicate_download_url(&self, url: String, existing_urls: Vec<String>) -> bool {
        fetchx_core_services::is_duplicate_download_url(&url, &existing_urls)
    }

    pub fn match_download_category(
        &self,
        filename: String,
        url: Option<String>,
        page_url: Option<String>,
    ) -> Option<String> {
        fetchx_core_services::match_download_category(
            &filename,
            url.as_deref(),
            page_url.as_deref(),
        )
        .map(str::to_owned)
    }

    /// Creates the service, starts the Tokio runtime, and initialises CoreServices.
    ///
    /// This is the UniFFI constructor; called from Kotlin as `DownloadService(config)`.
    pub fn new(
        config: CoreConfig,
        yt_dlp_runtime: Option<Box<dyn YtDlpRuntime>>,
    ) -> Result<Self, MobileApiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| MobileApiError::Runtime(format!("failed to build Tokio runtime: {e}")))?;

        let runtime_dir = PathBuf::from(&config.runtime_dir);
        let core_config = CoreServicesConfig {
            database_path: PathBuf::from(&config.database_path),
            downloads_dir: PathBuf::from(&config.downloads_dir),
            runtime_dir: PathBuf::from(&config.runtime_dir),
            bundled_engines_dir: None,
            defer_queued_on_start: false,
        };
        let ytdlp_runner = yt_dlp_runtime.map(|runtime| {
            Arc::new(AndroidYtDlpProcessRunner {
                runtime: Arc::from(runtime),
                runtime_dir: runtime_dir.clone(),
            }) as Arc<dyn YtDlpProcessRunner>
        });
        let engine_config = {
            let mut engine_config = fetchx_core_services::EngineConfig::default();
            if let Some(path) = config.yt_dlp_executable_path {
                engine_config.yt_dlp.enabled = true;
                engine_config.yt_dlp.executable_path = Some(PathBuf::from(path));
            }
            if ytdlp_runner.is_some() {
                engine_config.yt_dlp.enabled = true;
            }
            if let Some(path) = config.aria2_executable_path {
                engine_config.aria2.enabled = true;
                engine_config.aria2.executable_path = Some(PathBuf::from(path));
            }
            // Enable native torrent engine on Android when no aria2 binary is
            // configured — aria2c cannot be bundled due to OOM on startup.
            if !engine_config.aria2.enabled {
                engine_config.native_torrent.enabled = true;
                // Set the session dir to the app's private files directory.
                // std::env::temp_dir() returns /data/local/tmp on Android which
                // is not writable by the app sandbox.
                let runtime_parent = PathBuf::from(&config.runtime_dir)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from(&config.runtime_dir));
                engine_config.native_torrent.session_dir =
                    Some(runtime_parent.join("native-torrent-session"));
            }
            engine_config
        };

        let core = runtime
            .block_on(CoreServices::start_with_engine_config_and_ytdlp_runner(
                core_config,
                Some(engine_config),
                ytdlp_runner,
            ))
            .map_err(MobileApiError::from)?;
        tracing::info!("Android native bridge initialized");
        global_diagnostics().record(DiagnosticEvent::new(
            DiagnosticComponent::Android,
            "native_bridge_initialized",
            DiagnosticLevel::Info,
        ));

        Ok(Self {
            inner: Mutex::new(Some(ServiceInner { runtime, core })),
        })
    }

    // ── Helper: lock inner and execute a blocking async call ──────────────

    fn with_inner<T, F>(&self, f: F) -> Result<T, MobileApiError>
    where
        F: FnOnce(&ServiceInner) -> Result<T, MobileApiError>,
    {
        let guard = self
            .inner
            .lock()
            .map_err(|_| MobileApiError::Runtime("service lock poisoned".to_string()))?;
        match guard.as_ref() {
            Some(inner) => f(inner),
            None => Err(MobileApiError::Runtime(
                "service has been shut down".to_string(),
            )),
        }
    }

    // ── List / Get ────────────────────────────────────────────────────────

    pub fn list_downloads(&self) -> Result<Vec<DownloadInfoDto>, MobileApiError> {
        self.with_inner(|inner| {
            let items = inner
                .runtime
                .block_on(inner.core.repository.list())
                .map_err(|e| MobileApiError::Storage(e.to_string()))?;
            Ok(items.iter().map(item_to_dto).collect())
        })
    }

    pub fn list_runtime_statuses(&self) -> Result<Vec<DownloadRuntimeStatusDto>, MobileApiError> {
        self.with_inner(|inner| {
            let statuses = inner
                .runtime
                .block_on(inner.core.download_runtime_statuses())
                .map_err(|error| MobileApiError::Storage(error.to_string()))?;
            Ok(statuses.iter().map(runtime_status_to_dto).collect())
        })
    }

    pub fn find_duplicate_download(
        &self,
        url: String,
    ) -> Result<Option<crate::DuplicateDownloadMatchDto>, MobileApiError> {
        self.with_inner(|inner| {
            let duplicate = inner
                .runtime
                .block_on(inner.core.find_duplicate_download(&url))
                .map_err(|error| MobileApiError::Storage(error.to_string()))?;
            Ok(duplicate.as_ref().map(duplicate_match_to_dto))
        })
    }

    pub fn diagnostics_snapshot(&self) -> Result<String, MobileApiError> {
        self.with_inner(|inner| {
            let snapshot = inner.runtime.block_on(inner.core.diagnostics_snapshot());
            serde_json::to_string(&snapshot).map_err(|_| {
                global_diagnostics().record(DiagnosticEvent::new(
                    DiagnosticComponent::Android,
                    "diagnostics_serialization_failed",
                    DiagnosticLevel::Error,
                ));
                MobileApiError::Runtime("diagnostics snapshot is unavailable".to_owned())
            })
        })
    }

    pub fn get_download(&self, id: String) -> Result<Option<DownloadInfoDto>, MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            let item = inner
                .runtime
                .block_on(inner.core.repository.get(download_id))
                .map_err(|e| MobileApiError::Storage(e.to_string()))?;
            Ok(item.as_ref().map(item_to_dto))
        })
    }

    // ── Add ────────────────────────────────────────────────────────────────

    pub fn add_download(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
    ) -> Result<String, MobileApiError> {
        let engine = engine_for_url(&url);
        self.add_download_for_engine(url, filename, destination, priority, engine, None)
    }

    pub fn add_download_with_sha256(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
        expected_sha256: String,
    ) -> Result<String, MobileApiError> {
        validate_sha256(&expected_sha256).map_err(|message| MobileApiError::Validation(message))?;
        let engine = engine_for_url(&url);
        let result = self.add_download_for_engine(
            url,
            filename,
            destination.clone(),
            priority,
            engine,
            None,
        )?;
        let checksum_path = checksum_sidecar_path(Path::new(&destination));
        std::fs::write(checksum_path, expected_sha256.trim().to_ascii_lowercase()).map_err(
            |error| MobileApiError::Storage(format!("could not store SHA-256: {error}")),
        )?;
        Ok(result)
    }

    pub fn add_magnet_download(
        &self,
        magnet_uri: String,
        priority: DownloadPriorityDto,
    ) -> Result<String, MobileApiError> {
        self.with_inner(|inner| {
            let id = inner
                .runtime
                .block_on(
                    inner
                        .core
                        .enqueue_magnet(&magnet_uri, priority_from_dto(priority), None),
                )
                .map_err(MobileApiError::from)?;
            Ok(id.to_string())
        })
    }

    pub fn add_torrent_file_download(
        &self,
        torrent_path: String,
        priority: DownloadPriorityDto,
    ) -> Result<String, MobileApiError> {
        self.with_inner(|inner| {
            let id = inner
                .runtime
                .block_on(inner.core.enqueue_torrent_file(
                    PathBuf::from(torrent_path),
                    priority_from_dto(priority),
                    None,
                ))
                .map_err(MobileApiError::from)?;
            Ok(id.to_string())
        })
    }

    pub fn inspect_torrent_file(
        &self,
        torrent_path: String,
    ) -> Result<TorrentMetadataDto, MobileApiError> {
        self.with_inner(|inner| {
            let metadata = inner
                .runtime
                .block_on(inner.core.inspect_torrent_file(PathBuf::from(torrent_path)))
                .map_err(MobileApiError::from)?;
            let files = metadata
                .files
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    Ok(TorrentFileDto {
                        index: u32::try_from(index).map_err(|_| {
                            MobileApiError::Validation("torrent has too many files".to_owned())
                        })?,
                        path: file.path.join("/"),
                        size_bytes: file.size,
                    })
                })
                .collect::<Result<Vec<_>, MobileApiError>>()?;
            Ok(TorrentMetadataDto {
                name: metadata.name,
                info_hash: metadata.info_hash,
                total_size_bytes: metadata.total_size,
                files,
                trackers: metadata.trackers,
                piece_length: metadata.piece_length,
                piece_count: u64::try_from(metadata.piece_count).map_err(|_| {
                    MobileApiError::Validation("torrent has too many pieces".to_owned())
                })?,
            })
        })
    }

    pub fn get_torrent_download_metadata(
        &self,
        id: String,
    ) -> Result<Option<TorrentDownloadMetadataDto>, MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            let metadata = inner
                .runtime
                .block_on(inner.core.torrent_download_metadata(download_id))
                .map_err(MobileApiError::from)?;
            metadata
                .map(|value| {
                    Ok(TorrentDownloadMetadataDto {
                        metadata_state: value.metadata_state,
                        metadata: value.metadata.map(torrent_metadata_to_dto).transpose()?,
                        selected_file_indexes: value.selected_file_indexes,
                        selected_total_bytes: value.selected_total_bytes,
                        selection_confirmed: value.selection_confirmed,
                    })
                })
                .transpose()
        })
    }

    pub fn select_torrent_files(
        &self,
        id: String,
        selected_file_indexes: Vec<u32>,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(
                    inner
                        .core
                        .select_torrent_files(download_id, selected_file_indexes),
                )
                .map_err(MobileApiError::from)
        })
    }

    pub fn set_torrent_file_selected(
        &self,
        id: String,
        file_index: u32,
        selected: bool,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(
                    inner
                        .core
                        .set_torrent_file_selected(download_id, file_index, selected),
                )
                .map_err(MobileApiError::from)
        })
    }

    pub fn set_torrent_selection_all(
        &self,
        id: String,
        selected: bool,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.set_torrent_selection_all(download_id, selected))
                .map_err(MobileApiError::from)
        })
    }

    pub fn set_torrent_directory_selected(
        &self,
        id: String,
        directory: String,
        selected: bool,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.set_torrent_directory_selected(
                    download_id,
                    directory,
                    selected,
                ))
                .map_err(MobileApiError::from)
        })
    }

    pub fn add_torrent_file_download_selected(
        &self,
        torrent_path: String,
        priority: DownloadPriorityDto,
        selected_file_indexes: Vec<u32>,
        destination: String,
    ) -> Result<String, MobileApiError> {
        self.with_inner(|inner| {
            let id = inner
                .runtime
                .block_on(inner.core.enqueue_torrent_file_with_selection(
                    PathBuf::from(torrent_path),
                    priority_from_dto(priority),
                    Some(PathBuf::from(destination)),
                    Some(selected_file_indexes),
                ))
                .map_err(MobileApiError::from)?;
            Ok(id.to_string())
        })
    }

    pub fn add_media_download(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
        format_id: Option<String>,
    ) -> Result<String, MobileApiError> {
        // For direct media URLs (mp4, m3u8, webm, etc.) use the appropriate native engine
        // instead of routing through yt-dlp which is unavailable via subprocess on Android.
        let engine = if format_id.is_none() && is_direct_media_url(&url) {
            if is_hls_url(&url) {
                EngineKind::Hls
            } else {
                EngineKind::DirectHttp
            }
        } else {
            EngineKind::YtDlp
        };
        self.add_download_for_engine(url, filename, destination, priority, engine, format_id)
    }

    pub fn discover_media_formats(
        &self,
        url: String,
    ) -> Result<Vec<YtDlpFormatDto>, MobileApiError> {
        if url.trim().is_empty() {
            return Err(MobileApiError::Validation(
                "URL must not be empty".to_string(),
            ));
        }
        self.with_inner(|inner| {
            let info = inner
                .runtime
                .block_on(
                    inner
                        .core
                        .extract_media(&url, fetchx_media_extractor::ExtractOptions::default()),
                )
                .map_err(|error| MobileApiError::Runtime(error.to_string()))?;
            Ok(info
                .formats
                .into_iter()
                .map(|f| YtDlpFormatDto {
                    format_id: f.id,
                    label: f.label,
                    extension: if f.ext.is_empty() { None } else { Some(f.ext) },
                    title: None,
                    platform: None,
                })
                .collect())
        })
    }

    fn add_download_for_engine(
        &self,
        url: String,
        filename: String,
        destination: String,
        priority: DownloadPriorityDto,
        engine: EngineKind,
        format_id: Option<String>,
    ) -> Result<String, MobileApiError> {
        // Validate inputs before locking the inner service.
        if url.trim().is_empty() {
            return Err(MobileApiError::Validation(
                "URL must not be empty".to_string(),
            ));
        }
        let fname =
            FileName::new(&filename).map_err(|e| MobileApiError::Validation(e.to_string()))?;
        let source = DownloadSource::new(&url, engine)
            .and_then(|source| match format_id {
                Some(format_id) if engine == EngineKind::YtDlp => {
                    source.with_yt_dlp_format(format_id)
                }
                _ => Ok(source),
            })
            .map_err(|e| MobileApiError::Validation(e.to_string()))?;
        let dest = PathBuf::from(&destination);
        let (dest, fname) = if dest.exists() {
            let unique_dest = unique_destination_path(&dest);
            let unique_fname = unique_dest
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| FileName::new(n).ok())
                .unwrap_or(fname);
            (unique_dest, unique_fname)
        } else {
            (dest, fname)
        };
        let rust_priority = priority_from_dto(priority);

        self.with_inner(|inner| {
            if engine != EngineKind::DirectHttp && !inner.core.engine_runtime.is_available(engine) {
                return Err(MobileApiError::UnavailableEngine(format!("{engine:?}")));
            }
            let created_at =
                Timestamp::now().map_err(|e| MobileApiError::Runtime(e.to_string()))?;
            let new_id = DownloadId::new();
            let mut item = DownloadItem::new(new_id, source, fname, dest, created_at);
            // Apply the requested priority (default is Normal).
            item.set_priority_at(rust_priority, created_at)
                .map_err(|e| MobileApiError::Runtime(e.to_string()))?;
            inner
                .runtime
                .block_on(inner.core.queue().enqueue(item))
                .map_err(MobileApiError::from)?;
            Ok(new_id.to_string())
        })
    }

    // ── Lifecycle controls ────────────────────────────────────────────────

    pub fn pause_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().pause(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn resume_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().resume(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn cancel_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().cancel(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn remove_download(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().remove_download(download_id))
                .map_err(MobileApiError::from)
        })
    }

    // ── Priority ──────────────────────────────────────────────────────────

    pub fn set_priority(
        &self,
        id: String,
        priority: DownloadPriorityDto,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        let rust_priority = priority_from_dto(priority);
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().set_priority(download_id, rust_priority))
                .map_err(MobileApiError::from)
        })
    }

    pub fn move_download_up_in_queue(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().move_up(download_id))
                .map_err(MobileApiError::from)
        })
    }

    pub fn move_download_down_in_queue(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.queue().move_down(download_id))
                .map_err(MobileApiError::from)
        })
    }

    // ── Schedule ──────────────────────────────────────────────────────────

    pub fn schedule_download(
        &self,
        id: String,
        start_at_millis: i64,
    ) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        let millis = u64::try_from(start_at_millis).map_err(|_| {
            MobileApiError::Validation("start_at_millis must be non-negative".to_string())
        })?;
        let ts = Timestamp::from_unix_millis(millis);
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.scheduler().schedule_download(download_id, ts))
                .map_err(MobileApiError::from)
        })
    }

    // ── Retry ──────────────────────────────────────────────────────────────

    pub fn retry_download(&self, id: String) -> Result<String, MobileApiError> {
        let old_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;

        self.with_inner(|inner| {
            // 1. Look up the old item.
            let item = inner
                .runtime
                .block_on(inner.core.repository.get(old_id))
                .map_err(|e| MobileApiError::Storage(e.to_string()))?
                .ok_or_else(|| MobileApiError::NotFound(format!("download {old_id} not found")))?;

            // 2. Validate that it is in a retryable state.
            if !matches!(
                item.status(),
                DownloadStatus::Failed | DownloadStatus::Cancelled
            ) {
                return Err(MobileApiError::InvalidStateTransition(format!(
                    "retry requires a failed or cancelled download (current: {:?})",
                    item.status()
                )));
            }

            // 3. Check engine availability.
            let engine = item.source().engine();
            if !inner.core.engine_runtime.is_available(engine) {
                return Err(MobileApiError::UnavailableEngine(format!(
                    "{engine:?} engine is not available"
                )));
            }

            // 4. Remove old record.
            inner
                .runtime
                .block_on(inner.core.queue().remove_download(old_id))
                .map_err(MobileApiError::from)?;

            // 5. Build a new item preserving source/filename/destination (or resolving collision if destination exists).
            let created_at =
                Timestamp::now().map_err(|e| MobileApiError::Runtime(e.to_string()))?;
            let new_id = DownloadId::new();
            let (destination, filename) = if item.destination().exists() {
                let unique_dest = unique_destination_path(item.destination());
                let unique_fname = unique_dest
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| FileName::new(n).ok())
                    .unwrap_or_else(|| item.filename().clone());
                (unique_dest, unique_fname)
            } else {
                (item.destination().to_path_buf(), item.filename().clone())
            };
            let new_item = DownloadItem::new(
                new_id,
                item.source().clone(),
                filename,
                destination,
                created_at,
            );

            // 6. Enqueue the new item.
            inner
                .runtime
                .block_on(inner.core.queue().enqueue(new_item))
                .map_err(MobileApiError::from)?;

            Ok(new_id.to_string())
        })
    }

    // ── Stop seeding ──────────────────────────────────────────────────────

    pub fn stop_torrent_seeding(&self, id: String) -> Result<(), MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            inner
                .runtime
                .block_on(inner.core.stop_torrent_seeding(download_id))
                .map_err(|e| MobileApiError::Runtime(e.to_string()))
        })
    }

    pub fn get_torrent_telemetry(
        &self,
        id: String,
    ) -> Result<Option<TorrentTelemetryDto>, MobileApiError> {
        let download_id = DownloadId::parse(&id)
            .map_err(|_| MobileApiError::Validation(format!("invalid download ID: {id}")))?;
        self.with_inner(|inner| {
            let telemetry = inner
                .runtime
                .block_on(inner.core.torrent_telemetry(download_id))
                .map_err(|error| MobileApiError::Runtime(error.to_string()))?;
            Ok(telemetry.map(|value| TorrentTelemetryDto {
                download_speed_bps: value.download_speed_bps,
                upload_speed_bps: value.upload_speed_bps,
                downloaded_bytes: value.downloaded_bytes,
                uploaded_bytes: value.uploaded_bytes,
                peer_count: value.peer_count,
                completed_peer_count: value.completed_peer_count,
                seed_count: value.seed_count,
                share_ratio: value.share_ratio,
                seeding: value.seeding,
                tracker_uris: value.tracker_uris,
            }))
        })
    }

    /// Dispatches a browser RPC request after the Android adapter has
    /// authenticated it as coming from the bundled GeckoView extension.
    pub fn dispatch_browser_rpc(&self, request_json: String) -> Result<String, MobileApiError> {
        self.with_inner(|inner| {
            let response = inner
                .runtime
                .block_on(fetchx_remote_server::dispatch_local_rpc(
                    &inner.core,
                    request_json.as_bytes(),
                ))
                .map_err(|error| MobileApiError::Runtime(error.to_string()))?;
            serde_json::to_string(&response)
                .map_err(|error| MobileApiError::Runtime(error.to_string()))
        })
    }

    // ── Event listener ────────────────────────────────────────────────────

    /// Registers a Kotlin-implemented event listener. Spawns a background task
    /// on the existing runtime that forwards events until the broadcast channel closes.
    pub fn add_event_listener(&self, listener: Box<dyn DownloadEventListener>) {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(inner) = guard.as_ref() else {
            return;
        };

        let mut rx: broadcast::Receiver<fetchx_queue::DownloadEvent> =
            inner.core.event_bus.subscribe();

        // UniFFI callback interface objects are Send + Sync, wrap in Arc for the async task.
        let listener_arc: Arc<dyn DownloadEventListener> = Arc::from(listener);

        inner.runtime.handle().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let dto = event_to_dto(event);
                        listener_arc.on_event(dto);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            skipped = n,
                            "event listener lagged; some events were skipped"
                        );
                        // Continue — do not break; catch up from next available event.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        });
    }

    // ── Shutdown ──────────────────────────────────────────────────────────

    /// Gracefully shuts down the core services and drops the Tokio runtime.
    ///
    /// Calling `shutdown()` more than once is safe; subsequent calls are no-ops.
    pub fn shutdown(&self) {
        let inner_opt = match self.inner.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        if let Some(inner) = inner_opt {
            let ServiceInner { runtime, core } = inner;
            runtime.block_on(async {
                if let Err(e) = core.shutdown().await {
                    warn!(error = %e, "error during CoreServices shutdown");
                }
            });
            // Dropping `runtime` here joins all spawned tasks.
            drop(runtime);
        }
    }
}

struct AndroidYtDlpProcessRunner {
    runtime: Arc<dyn YtDlpRuntime>,
    runtime_dir: PathBuf,
}

static NEXT_YTDLP_RUN_ID: AtomicU64 = AtomicU64::new(1);

impl YtDlpProcessRunner for AndroidYtDlpProcessRunner {
    fn report_diagnostic(&self, code: &'static str) {
        self.runtime.report_failure(code.to_owned());
    }

    fn report_diagnostic_details(&self, details: String) {
        self.runtime.report_failure(details);
    }

    fn validate_version<'a>(
        &'a self,
        _executable: &'a Path,
        command_timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
        Box::pin(async move {
            if command_timeout.is_zero() {
                return Err(YtDlpError::VersionTimeout);
            }
            let runtime = Arc::clone(&self.runtime);
            match tokio::time::timeout(
                command_timeout,
                tokio::task::spawn_blocking(move || runtime.validate_version()),
            )
            .await
            {
                Ok(result) => result.map_err(|_| YtDlpError::ProcessRunner),
                Err(_) => Err(YtDlpError::VersionTimeout),
            }
        })
    }

    fn run<'a>(
        &'a self,
        _executable: &'a Path,
        arguments: &'a [OsString],
        stop: watch::Receiver<bool>,
        events: mpsc::Sender<YtDlpRunnerEvent>,
    ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
        let arguments = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let runtime = Arc::clone(&self.runtime);
        let runtime_dir = self.runtime_dir.clone();
        Box::pin(async move {
            let run_id = NEXT_YTDLP_RUN_ID.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| YtDlpError::ProcessRunner)?
                .as_nanos();
            let run_dir = runtime_dir.join(format!("ytdlp-run-{timestamp}-{run_id}"));
            fs::create_dir(&run_dir)
                .await
                .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
            let progress_path = run_dir.join("progress");
            let cancel_path = run_dir.join("cancel");
            let progress_for_worker = progress_path.to_string_lossy().into_owned();
            let cancel_for_worker = cancel_path.to_string_lossy().into_owned();
            let mut worker = tokio::task::spawn_blocking(move || {
                runtime.run(arguments, progress_for_worker, cancel_for_worker)
            });
            let mut last_progress = String::new();
            let mut cancellation_requested = false;

            let result = loop {
                tokio::select! {
                    joined = &mut worker => {
                        break joined.map_err(|_| YtDlpError::ProcessRunner)?;
                    }
                    _ = sleep(Duration::from_millis(150)) => {
                        if *stop.borrow() && !cancellation_requested {
                            fs::write(&cancel_path, b"cancel")
                                .await
                                .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                            cancellation_requested = true;
                        }

                        match fs::read_to_string(&progress_path).await {
                            Ok(contents) if contents.ends_with('\n') => {
                                let line = contents.trim_end_matches(['\r', '\n']);
                                if line.len() > MAX_ANDROID_PROGRESS_LINE_BYTES {
                                    fs::write(&cancel_path, b"cancel")
                                        .await
                                        .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                                    worker.await.map_err(|_| YtDlpError::ProcessRunner)?;
                                    if let Err(error) = fs::remove_dir_all(&run_dir).await {
                                        warn!(%error, "yt-dlp control workspace cleanup failed");
                                    }
                                    return Err(YtDlpError::ProgressOutputTooLarge);
                                }
                                if !line.is_empty() && line != last_progress {
                                    if events.send(YtDlpRunnerEvent::ProgressLine(line.to_owned())).await.is_err() {
                                        fs::write(&cancel_path, b"cancel")
                                            .await
                                            .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                                        cancellation_requested = true;
                                    }
                                    last_progress = line.to_owned();
                                }
                            }
                            Ok(_) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(_) => {
                                fs::write(&cancel_path, b"cancel")
                                    .await
                                    .map_err(|_| YtDlpError::WorkspaceUnavailable)?;
                                worker.await.map_err(|_| YtDlpError::ProcessRunner)?;
                                if let Err(cleanup_error) = fs::remove_dir_all(&run_dir).await {
                                    warn!(%cleanup_error, "yt-dlp control workspace cleanup failed");
                                }
                                return Err(YtDlpError::ProgressOutputRead);
                            }
                        }
                    }
                }
            };
            if let Err(error) = fs::remove_dir_all(&run_dir).await {
                warn!(%error, "yt-dlp control workspace cleanup failed");
            }
            if cancellation_requested || *stop.borrow() {
                return Ok(YtDlpProcessExit {
                    success: false,
                    code: Some(130),
                });
            }
            let exit_code = if result.is_empty() { 0 } else { 1 };
            if !result.is_empty() {
                events
                    .send(YtDlpRunnerEvent::ProcessFailed(result))
                    .await
                    .map_err(|_| YtDlpError::ProgressOutputRead)?;
            }
            Ok(YtDlpProcessExit {
                success: exit_code == 0,
                code: Some(exit_code),
            })
        })
    }

    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        url: &'a str,
        timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
        let runtime = Arc::clone(&self.runtime);
        let url = url.to_owned();
        Box::pin(async move {
            let output = tokio::time::timeout(
                timeout,
                tokio::task::spawn_blocking(move || runtime.discover_formats(url)),
            )
            .await
            .map_err(|_| YtDlpError::FormatDiscoveryTimeout)?
            .map_err(|_| YtDlpError::ProcessRunner)?;
            if output.starts_with("ERROR:") {
                return Err(YtDlpError::InvalidFormatOutput);
            }
            parse_format_output(output.as_bytes())
        })
    }
}

fn torrent_metadata_to_dto(
    metadata: fetchx_models::TorrentMetadata,
) -> Result<TorrentMetadataDto, MobileApiError> {
    let files = metadata
        .files
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            Ok(TorrentFileDto {
                index: u32::try_from(index).map_err(|_| {
                    MobileApiError::Validation("torrent has too many files".to_owned())
                })?,
                path: file.path.join("/"),
                size_bytes: file.size,
            })
        })
        .collect::<Result<Vec<_>, MobileApiError>>()?;
    Ok(TorrentMetadataDto {
        name: metadata.name,
        info_hash: metadata.info_hash,
        total_size_bytes: metadata.total_size,
        files,
        trackers: metadata.trackers,
        piece_length: metadata.piece_length,
        piece_count: u64::try_from(metadata.piece_count)
            .map_err(|_| MobileApiError::Validation("too many torrent pieces".to_owned()))?,
    })
}

fn unique_destination_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = path.extension().and_then(|s| s.to_str());
    let mut counter = 1;
    loop {
        let new_name = match ext {
            Some(ext) => format!("{stem} ({counter}).{ext}"),
            None => format!("{stem} ({counter})"),
        };
        let candidate = parent.join(new_name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::thread;
    use std::{path::Path, sync::Arc, time::Duration};

    use tempfile::tempdir;

    use super::{AndroidYtDlpProcessRunner, DownloadService, engine_for_url, is_direct_media_url};
    use crate::YtDlpRuntime;
    use crate::dto::{CoreConfig, DownloadEventDto, DownloadPriorityDto};
    use crate::error::MobileApiError;
    use fetchx_downloader::{YtDlpError, YtDlpProcessRunner};
    use fetchx_models::EngineKind;

    fn make_config() -> (CoreConfig, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let config = CoreConfig {
            database_path: dir.path().join("test.db").to_string_lossy().into_owned(),
            downloads_dir: dir.path().join("downloads").to_string_lossy().into_owned(),
            runtime_dir: dir.path().join("runtime").to_string_lossy().into_owned(),
            yt_dlp_executable_path: None,
            aria2_executable_path: None,
        };
        (config, dir)
    }

    struct SlowVersionRuntime;

    impl YtDlpRuntime for SlowVersionRuntime {
        fn validate_version(&self) -> String {
            thread::sleep(Duration::from_millis(150));
            "test-version".to_owned()
        }

        fn run(
            &self,
            _arguments: Vec<String>,
            _progress_path: String,
            _cancel_path: String,
        ) -> String {
            String::new()
        }

        fn discover_formats(&self, _url: String) -> String {
            "[]".to_owned()
        }

        fn report_failure(&self, _diagnostic_code: String) {}
    }

    #[tokio::test]
    async fn android_version_validation_honors_timeout() {
        let directory = tempdir().expect("tempdir");
        let runner = AndroidYtDlpProcessRunner {
            runtime: Arc::new(SlowVersionRuntime),
            runtime_dir: directory.path().to_path_buf(),
        };
        let started = std::time::Instant::now();

        let result = runner
            .validate_version(Path::new("embedded"), Duration::from_millis(10))
            .await;

        assert!(matches!(result, Err(YtDlpError::VersionTimeout)));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn add_download_selects_engine_from_url_scheme() {
        assert_eq!(engine_for_url("ftp://example.test/file"), EngineKind::Ftp);
        assert_eq!(engine_for_url("FTPS://example.test/file"), EngineKind::Ftps);
        assert_eq!(engine_for_url("sftp://example.test/file"), EngineKind::Sftp);
        assert_eq!(
            engine_for_url("https://example.test/file"),
            EngineKind::DirectHttp
        );
    }

    #[test]
    fn direct_media_detection_uses_url_path_not_host_query_or_fragment() {
        assert!(is_direct_media_url(
            "https://cdn.example.test/video.mp4?token=abc#t=10"
        ));
        assert!(!is_direct_media_url("https://video.mp4/"));
        assert!(!is_direct_media_url(
            "https://example.test/watch?format=video.mp4"
        ));
        assert!(!is_direct_media_url("https://example.test/watch#video.mp4"));
    }

    #[test]
    fn init_and_shutdown() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        svc.shutdown();
    }

    #[test]
    fn double_shutdown_is_safe() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        svc.shutdown();
        svc.shutdown(); // second call must not panic
    }

    #[test]
    fn list_downloads_empty_initially() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let items = svc.list_downloads().expect("list failed");
        assert!(items.is_empty());
        svc.shutdown();
    }

    #[test]
    fn browser_rpc_dispatch_uses_fetchx_protocol() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let response = svc
            .dispatch_browser_rpc(
                r#"{"jsonrpc":"2.0","id":1,"method":"system.discover"}"#.to_owned(),
            )
            .expect("browser RPC dispatch failed");
        let response: serde_json::Value =
            serde_json::from_str(&response).expect("browser RPC response was not JSON");
        assert_eq!(response["result"]["protocol_version"], "fetchx.rpc.v1");
        let response = svc
            .dispatch_browser_rpc(r#"{"jsonrpc":"2.0","id":2,"method":"download.list"}"#.to_owned())
            .expect("browser download list failed");
        let response: serde_json::Value =
            serde_json::from_str(&response).expect("browser RPC response was not JSON");
        assert_eq!(response["result"]["downloads"], serde_json::json!([]));
        let response = svc
            .dispatch_browser_rpc(
                r#"{"jsonrpc":"2.0","id":3,"method":"download.pause","params":{"id":"00000000-0000-4000-8000-000000000000"}}"#.to_owned(),
            )
            .expect("browser RPC dispatch failed");
        let response: serde_json::Value =
            serde_json::from_str(&response).expect("browser RPC response was not JSON");
        assert_eq!(response["error"]["code"], -32601);
        svc.shutdown();
    }

    #[test]
    fn add_download_returns_uuid() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                "/tmp/file.zip".to_string(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");
        // Must parse as a UUID.
        uuid::Uuid::parse_str(&id).expect("returned ID is not a valid UUID");
        svc.shutdown();
    }

    #[test]
    fn magnet_add_validates_uri_before_reporting_aria2_availability() {
        let (mut config, dir) = make_config();
        config.aria2_executable_path = Some(
            dir.path()
                .join("missing-aria2")
                .to_string_lossy()
                .into_owned(),
        );
        let svc = DownloadService::new(config, None).expect("service creation failed");

        assert!(matches!(
            svc.add_magnet_download(
                "magnet:?dn=missing-hash".to_owned(),
                DownloadPriorityDto::Normal
            ),
            Err(MobileApiError::Validation(_))
        ));
        assert!(matches!(
            svc.add_magnet_download(
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_owned(),
                DownloadPriorityDto::Normal
            ),
            Err(MobileApiError::UnavailableEngine(_))
        ));
        svc.shutdown();
    }

    #[test]
    fn add_media_download_requires_an_available_ytdlp_engine() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let result = svc.add_media_download(
            "https://www.youtube.com/watch?v=example".to_string(),
            "video".to_string(),
            "/tmp/downloads".to_string(),
            DownloadPriorityDto::Normal,
            None,
        );
        assert!(matches!(result, Err(MobileApiError::UnavailableEngine(_))));
        svc.shutdown();
    }

    #[test]
    fn get_download_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let unknown_id = uuid::Uuid::new_v4().to_string();
        let result = svc.get_download(unknown_id).expect("get_download failed");
        assert!(result.is_none());
        svc.shutdown();
    }

    #[test]
    fn add_and_list_two_downloads() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        svc.add_download(
            "https://example.com/a.zip".to_string(),
            "a.zip".to_string(),
            "/tmp/a.zip".to_string(),
            DownloadPriorityDto::Low,
        )
        .expect("add first failed");
        svc.add_download(
            "https://example.com/b.zip".to_string(),
            "b.zip".to_string(),
            "/tmp/b.zip".to_string(),
            DownloadPriorityDto::High,
        )
        .expect("add second failed");
        let items = svc.list_downloads().expect("list failed");
        assert_eq!(items.len(), 2);
        svc.shutdown();
    }

    #[test]
    fn invalid_url_returns_validation_error() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let result = svc.add_download(
            "   ".to_string(),
            "file.zip".to_string(),
            "/tmp/file.zip".to_string(),
            DownloadPriorityDto::Normal,
        );
        assert!(
            matches!(result, Err(MobileApiError::Validation(_))),
            "expected Validation error, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn pause_nonexistent_returns_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = uuid::Uuid::new_v4().to_string();
        let result = svc.pause_download(id);
        assert!(
            matches!(result, Err(MobileApiError::NotFound(_))),
            "expected NotFound, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn cancel_nonexistent_returns_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = uuid::Uuid::new_v4().to_string();
        let result = svc.cancel_download(id);
        assert!(
            matches!(result, Err(MobileApiError::NotFound(_))),
            "expected NotFound, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn remove_nonexistent_returns_not_found() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = uuid::Uuid::new_v4().to_string();
        let result = svc.remove_download(id);
        assert!(
            matches!(result, Err(MobileApiError::NotFound(_))),
            "expected NotFound, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn priority_change_roundtrip() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                "/tmp/file.zip".to_string(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");

        svc.set_priority(id.clone(), DownloadPriorityDto::High)
            .expect("set_priority failed");

        let item = svc
            .get_download(id)
            .expect("get_download failed")
            .expect("item should exist");
        assert!(
            matches!(item.priority, DownloadPriorityDto::High),
            "expected High priority"
        );
        svc.shutdown();
    }

    #[test]
    fn retry_non_failed_returns_invalid_state_error() {
        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                "/tmp/file.zip".to_string(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");

        // The item is Queued, not Failed/Cancelled — retry should fail.
        let result = svc.retry_download(id);
        assert!(
            matches!(result, Err(MobileApiError::InvalidStateTransition(_))),
            "expected InvalidStateTransition, got {result:?}"
        );
        svc.shutdown();
    }

    #[test]
    fn concurrent_list_calls() {
        let (config, _dir) = make_config();
        let svc = Arc::new(DownloadService::new(config, None).expect("service creation failed"));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let svc = Arc::clone(&svc);
                thread::spawn(move || {
                    svc.list_downloads().expect("concurrent list failed");
                })
            })
            .collect();

        for h in handles {
            h.join().expect("thread panicked");
        }
        svc.shutdown();
    }

    #[test]
    fn event_listener_receives_created_event() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let (config, _dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");

        let received = Arc::new(AtomicBool::new(false));
        let received_clone = Arc::clone(&received);

        struct TestListener {
            received: Arc<AtomicBool>,
        }
        impl crate::DownloadEventListener for TestListener {
            fn on_event(&self, _event: DownloadEventDto) {
                self.received.store(true, Ordering::SeqCst);
            }
        }

        svc.add_event_listener(Box::new(TestListener {
            received: received_clone,
        }));

        svc.add_download(
            "https://example.com/file.zip".to_string(),
            "file.zip".to_string(),
            "/tmp/file.zip".to_string(),
            DownloadPriorityDto::Normal,
        )
        .expect("add_download failed");

        // Give the async event dispatch task time to fire.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !received.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            received.load(Ordering::SeqCst),
            "event listener did not receive Created event within 2 seconds"
        );
        svc.shutdown();
    }

    #[test]
    fn unique_destination_path_generates_numbered_copies() {
        let dir = tempdir().expect("tempdir");
        let initial = dir.path().join("video.mp4");
        assert_eq!(super::unique_destination_path(&initial), initial);

        std::fs::write(&initial, b"content").expect("write");
        let copy1 = super::unique_destination_path(&initial);
        assert_eq!(copy1, dir.path().join("video (1).mp4"));

        std::fs::write(&copy1, b"content").expect("write");
        let copy2 = super::unique_destination_path(&initial);
        assert_eq!(copy2, dir.path().join("video (2).mp4"));
    }

    #[test]
    fn add_download_auto_disambiguates_existing_destination() {
        let (config, dir) = make_config();
        let svc = DownloadService::new(config, None).expect("service creation failed");
        let existing_dest = dir.path().join("downloads").join("file.zip");
        std::fs::create_dir_all(existing_dest.parent().unwrap()).expect("mkdirs");
        std::fs::write(&existing_dest, b"existing").expect("write");

        let id = svc
            .add_download(
                "https://example.com/file.zip".to_string(),
                "file.zip".to_string(),
                existing_dest.to_string_lossy().into_owned(),
                DownloadPriorityDto::Normal,
            )
            .expect("add_download failed");

        let item = svc.get_download(id).expect("get").expect("found");
        assert_eq!(item.filename, "file (1).zip");
        assert!(item.destination.ends_with("file (1).zip"));
        svc.shutdown();
    }
}

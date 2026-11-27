use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicU8},
};

use fetchx_core::StateMachineError;
use fetchx_core_services::{
    BatchDownloadRequest, ConfigService, CoreRepository, CoreServices, EngineRuntime,
};
use fetchx_downloader::{
    DownloadEngineError, YtDlpError, YtDlpFormat, discover_hls_variants as fetch_hls_variants,
    is_dash_url, is_hls_url, validate_http_source_url,
};
use fetchx_media_extractor::MediaProtocol;
use fetchx_models::{
    DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadSource, DownloadStatus,
    EngineKind, FileName, HttpCredentials, Timestamp, YtDlpAudioFormat, parse_magnet_uri,
};
use fetchx_queue::{QueueError, QueueManager};
use fetchx_scheduler::{ScheduleError, Scheduler};
use fetchx_storage::{Database, DownloadRepository, TorrentMetadataRepository};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::events::{
    DOWNLOAD_REMOVED, DOWNLOAD_UPDATED, DownloadRemovedEvent, DownloadUpdatedEvent,
};

use crate::engine_commands::EngineConfigDto;

/// The concrete repository type used by the desktop application.
pub(crate) type AppRepository = CoreRepository;

pub struct AppState {
    pub(crate) core_services: Option<Arc<CoreServices>>,
    pub(crate) database: Database,
    pub(crate) repository: AppRepository,
    pub(crate) queue: QueueManager,
    pub(crate) scheduler: Scheduler<AppRepository>,
    downloads_dir: std::path::PathBuf,
    pub(crate) shutdown_state: Arc<AtomicU8>,
    pub(crate) config_service: ConfigService,
    /// Owns the optional engine supervisors and the dispatcher.
    pub(crate) engine_runtime: Arc<EngineRuntime>,
}

impl AppState {
    #[cfg(test)]
    pub(crate) fn new(
        database: Database,
        repository: AppRepository,
        scheduler: Scheduler<AppRepository>,
        queue: QueueManager,
        downloads_dir: std::path::PathBuf,
        config_service: ConfigService,
        engine_runtime: EngineRuntime,
    ) -> Self {
        Self {
            core_services: None,
            database,
            repository,
            queue,
            scheduler,
            downloads_dir,
            shutdown_state: Arc::new(AtomicU8::new(0)),
            config_service,
            engine_runtime: Arc::new(engine_runtime),
        }
    }

    pub(crate) fn from_core_services(core: CoreServices) -> Self {
        let core_services = Arc::new(core);
        Self {
            database: core_services.database.clone(),
            repository: core_services.repository.clone(),
            queue: core_services.queue().clone(),
            scheduler: core_services.scheduler.clone(),
            downloads_dir: core_services.downloads_dir.clone(),
            shutdown_state: Arc::new(AtomicU8::new(0)),
            config_service: core_services.config_service.clone(),
            engine_runtime: Arc::clone(&core_services.engine_runtime),
            core_services: Some(core_services),
        }
    }

    async fn list(&self) -> Result<Vec<DownloadDto>, IpcError> {
        self.repository
            .list()
            .await
            .map(|items| {
                items
                    .into_iter()
                    .enumerate()
                    .map(|(position, item)| {
                        DownloadDto::from_with_queue_position(item, Some(position as u64))
                    })
                    .collect()
            })
            .map_err(|_| IpcError::internal())
    }

    async fn add(&self, request: AddDownloadRequest) -> Result<DownloadDto, IpcError> {
        self.add_with_ytdlp_format(request, None, None, None).await
    }

    async fn add_with_ytdlp_format(
        &self,
        request: AddDownloadRequest,
        format_id: Option<String>,
        extension: Option<String>,
        audio_format: Option<YtDlpAudioFormat>,
    ) -> Result<DownloadDto, IpcError> {
        let magnet = if request.url.starts_with("magnet:") {
            Some(parse_magnet_uri(&request.url).map_err(|_| IpcError::invalid_request())?)
        } else {
            None
        };
        let torrent_path = (magnet.is_none()
            && !request.url.contains("://")
            && Path::new(&request.url)
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent")))
        .then(|| Path::new(&request.url).to_path_buf());
        let canonical_url = match (magnet.is_some(), torrent_path.is_some()) {
            (true, _) | (_, true) => request.url.clone(),
            (false, false) => validate_source_url(&request.url)?,
        };
        let parsed_url = if magnet.is_none() && torrent_path.is_none() {
            Some(Url::parse(&canonical_url).map_err(|_| IpcError::invalid_request())?)
        } else {
            None
        };
        if extension.is_some()
            && (format_id.is_none() && audio_format.is_none()
                || magnet.is_some()
                || torrent_path.is_some()
                || !extension_is_safe(extension.as_deref()))
        {
            return Err(IpcError::invalid_request());
        }
        if audio_format.is_some()
            && (format_id.is_some()
                || extension.as_deref() != audio_format.map(YtDlpAudioFormat::extension))
        {
            return Err(IpcError::invalid_request());
        }
        // Duplicate-URL check for HTTP(S) downloads (skip for magnet/torrent).
        if magnet.is_none()
            && torrent_path.is_none()
            && let Some(ref cs) = self.core_services
            && let Ok(Some(dup)) = cs.find_duplicate_download(&canonical_url).await
        {
            let msg_string = format!(
                "This URL is already in your downloads (status: {:?}, filename: {})",
                dup.status(),
                dup.filename()
            );
            // We need a 'static str; leak a small allocation so the
            // IpcError<'static> constraint is satisfied.
            let msg: &'static str = Box::leak(msg_string.into_boxed_str());
            return Err(IpcError::duplicate_url(msg));
        }
        // Resolve the requested engine from the source when no override is present.
        let engine_kind = match (
            magnet.is_some() || torrent_path.is_some(),
            request.engine_kind,
        ) {
            (true, None) => EngineKind::NativeTorrent,
            (true, Some(EngineKind::NativeTorrent)) => EngineKind::NativeTorrent,
            (true, Some(_)) => return Err(IpcError::invalid_request()),
            (false, Some(engine)) => {
                if !engine_matches_source(
                    engine,
                    parsed_url.as_ref().ok_or_else(IpcError::invalid_request)?,
                ) {
                    return Err(IpcError::invalid_request());
                }
                engine
            }
            (false, None) => EngineKind::from_source_url(&canonical_url),
        };

        // Backend validates availability — the frontend cannot bypass this check.
        // Direct HTTP is always available; optional engines must be running.
        if !self.engine_runtime.is_available(engine_kind) {
            return Err(IpcError::engine_unavailable(engine_kind));
        }

        // When the user selected a specific HLS variant in the quality picker,
        // replace the master playlist URL with the chosen variant playlist URL.
        let canonical_url = if engine_kind == EngineKind::Hls {
            if let Some(ref variant_url) = request.hls_variant_url {
                validate_source_url(variant_url)?
            } else {
                canonical_url
            }
        } else {
            canonical_url
        };

        if magnet.is_some() {
            let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
            // Append any extra tracker URLs as &tr= parameters to the magnet URI.
            let magnet_url = if let Some(ref trackers) = request.extra_trackers {
                let mut uri = canonical_url.clone();
                for tracker in trackers.iter().filter(|t| !t.is_empty()) {
                    uri.push_str("&tr=");
                    // Percent-encode the tracker URL for safe embedding in the magnet URI.
                    for byte in tracker.bytes() {
                        if byte.is_ascii_alphanumeric()
                            || matches!(
                                byte,
                                b'-' | b'_'
                                    | b'.'
                                    | b'~'
                                    | b':'
                                    | b'/'
                                    | b'?'
                                    | b'#'
                                    | b'['
                                    | b']'
                                    | b'@'
                                    | b'!'
                                    | b'$'
                                    | b'&'
                                    | b'\''
                                    | b'('
                                    | b')'
                                    | b'*'
                                    | b'+'
                                    | b','
                                    | b';'
                                    | b'='
                            )
                        {
                            uri.push(byte as char);
                        } else {
                            uri.push_str(&format!("%{:02X}", byte));
                        }
                    }
                }
                uri
            } else {
                canonical_url.clone()
            };
            let id = match request.engine_kind {
                Some(engine) => {
                    core_services
                        .enqueue_magnet_with_engine(
                            &magnet_url,
                            engine,
                            DownloadPriority::Normal,
                            None,
                        )
                        .await
                }
                None => {
                    core_services
                        .enqueue_magnet(&magnet_url, DownloadPriority::Normal, None)
                        .await
                }
            }
            .map_err(|_| IpcError::internal())?;
            return self.get(id).await;
        }

        if let Some(torrent_path) = torrent_path {
            let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
            let id = match request.engine_kind {
                Some(engine) => {
                    core_services
                        .enqueue_torrent_file_with_selection_and_engine(
                            torrent_path,
                            engine,
                            DownloadPriority::Normal,
                            None,
                            None,
                        )
                        .await
                }
                None => {
                    core_services
                        .enqueue_torrent_file(torrent_path, DownloadPriority::Normal, None)
                        .await
                }
            }
            .map_err(|_| IpcError::internal())?;
            return self.get(id).await;
        }

        let url_filename = parsed_url
            .as_ref()
            .map(filename_for_url)
            .ok_or_else(IpcError::invalid_request)?;
        let filename = if engine_kind == EngineKind::YtDlp {
            match self.core_services.as_ref() {
                Some(core_services) => {
                    core_services
                        .resolve_media_filename(&canonical_url, &url_filename)
                        .await
                }
                None => {
                    tracing::warn!(
                        "media title extraction is unavailable; using URL-derived filename"
                    );
                    url_filename
                }
            }
        } else {
            url_filename
        };
        let filename = match extension.as_deref() {
            Some(extension) if engine_kind == EngineKind::YtDlp => {
                format!("{filename}.{extension}")
            }
            Some(extension) => Path::new(&filename)
                .with_extension(extension)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(IpcError::invalid_request)?
                .to_owned(),
            None => filename,
        };
        let filename = FileName::new(filename).map_err(|_| IpcError::invalid_request())?;
        let id = DownloadId::new();
        let source = DownloadSource::new(canonical_url.clone(), engine_kind)
            .and_then(|source| match (format_id, audio_format) {
                (Some(format_id), None) => source.with_yt_dlp_format(format_id),
                (None, Some(audio_format)) => source.with_yt_dlp_audio_format(audio_format),
                (None, None) => Ok(source),
                (Some(_), Some(_)) => {
                    Err(fetchx_models::DownloadSourceError::InvalidYtDlpAudioFormat)
                }
            })
            .map(|source| {
                if let Some(ref proxy) = request.proxy_url {
                    source.with_proxy(proxy.clone())
                } else {
                    source
                }
            })
            .map(|mut source| {
                if engine_kind == EngineKind::YtDlp {
                    source.download_subtitles = request.download_subtitles;
                    source.subtitle_langs = request.subtitle_langs.clone();
                }
                source
            })
            .map_err(|_| IpcError::invalid_request())?;
        let destination = package_destination(&self.downloads_dir, request.package_name.as_deref())
            .join(filename.as_str());
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| IpcError::internal())?;
        }
        let mut context_headers = Vec::new();
        if let Some(ref referer) = request.referer {
            context_headers.push(("Referer".to_string(), referer.clone()));
        }
        if let Some(ref ua) = request.user_agent {
            context_headers.push(("User-Agent".to_string(), ua.clone()));
        }
        if let Some(ref cookies) = request.cookies {
            context_headers.push(("Cookie".to_string(), cookies.clone()));
        }
        if !context_headers.is_empty() {
            let _ = fetchx_downloader::save_initial_context_headers(
                &destination,
                &canonical_url,
                context_headers,
            )
            .await;
        }
        let created_at = Timestamp::now().map_err(|_| IpcError::internal())?;
        let item = DownloadItem::new(id, source, filename, destination, created_at);

        self.queue
            .enqueue(item)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    pub(crate) async fn extract_media_formats(
        &self,
        source_url: &str,
    ) -> Result<MediaInfoDto, IpcError> {
        let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
        core_services
            .extract_media(
                source_url,
                fetchx_media_extractor::ExtractOptions::default(),
            )
            .await
            .map(media_info_to_dto)
            .map_err(|_| IpcError::internal())
    }

    async fn discover_ytdlp_formats(&self, source_url: &str) -> Result<Vec<YtDlpFormat>, IpcError> {
        let core_services = self.core_services.as_ref().ok_or_else(IpcError::internal)?;
        core_services
            .discover_ytdlp_formats(source_url)
            .await
            .map_err(|error| match error {
                DownloadEngineError::EngineNotAvailable => {
                    IpcError::engine_unavailable(EngineKind::YtDlp)
                }
                DownloadEngineError::InvalidUrl
                | DownloadEngineError::YtDlp(YtDlpError::InvalidUrl) => IpcError::invalid_request(),
                error => {
                    tracing::warn!(%error, "yt-dlp format discovery failed");
                    IpcError::internal()
                }
            })
    }

    pub(crate) async fn add_media_format_from_browser(
        &self,
        url: String,
        format_id: String,
        extension: String,
    ) -> Result<DownloadDto, IpcError> {
        self.add_with_ytdlp_format(
            AddDownloadRequest {
                url,
                engine_kind: Some(EngineKind::YtDlp),
                ..Default::default()
            },
            Some(format_id),
            Some(extension),
            None,
        )
        .await
    }

    /// Adds a download from the browser integration IPC path.
    ///
    /// Shares exactly the same validation and queue path as the Tauri command,
    /// but is called from `browser_ipc` rather than through Tauri IPC.
    /// The browser may only supply a URL; engine, filename, and destination
    /// are determined by the application exactly as for the desktop UI.
    pub(crate) async fn add_from_browser(
        &self,
        request: AddDownloadRequest,
    ) -> Result<DownloadDto, IpcError> {
        self.add(request).await
    }

    /// Creates a lightweight clone suitable for the browser IPC service.
    ///
    /// `AppState` is not `Clone` because `_database` is not, but the browser
    /// IPC service only needs the queue, repository, engine_runtime, and
    /// downloads_dir — all of which are already Arc-backed or cheaply cloneable.
    pub(crate) fn clone_for_browser(&self) -> Self {
        Self {
            database: self.database.clone(),
            core_services: self.core_services.clone(),
            repository: self.repository.clone(),
            queue: self.queue.clone(),
            scheduler: self.scheduler.clone(),
            downloads_dir: self.downloads_dir.clone(),
            shutdown_state: Arc::clone(&self.shutdown_state),
            config_service: self.config_service.clone(),
            engine_runtime: Arc::clone(&self.engine_runtime),
        }
    }

    async fn get(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.repository
            .get(id)
            .await
            .map_err(|_| IpcError::internal())?
            .map(DownloadDto::from)
            .ok_or_else(IpcError::not_found)
    }

    async fn pause(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.queue.pause(id).await.map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn resume(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        let item = self
            .repository
            .get(id)
            .await
            .map_err(|_| IpcError::internal())?
            .ok_or_else(IpcError::not_found)?;

        if matches!(
            item.status(),
            DownloadStatus::Failed | DownloadStatus::Cancelled | DownloadStatus::Stopped
        ) {
            return self.retry(id).await;
        }

        self.queue.resume(id).await.map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn cancel(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        self.queue.cancel(id).await.map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn delete(&self, id: DownloadId) -> Result<(), IpcError> {
        self.queue
            .remove_download(id)
            .await
            .map_err(IpcError::from_queue)
    }

    /// Retry in place so the original engine can reuse its persisted partial data.
    async fn retry(&self, id: DownloadId) -> Result<DownloadDto, IpcError> {
        let item = self
            .repository
            .get(id)
            .await
            .map_err(|_| IpcError::internal())?
            .ok_or_else(IpcError::not_found)?;

        // Only failed or cancelled downloads can be retried.
        if !matches!(
            item.status(),
            DownloadStatus::Failed | DownloadStatus::Cancelled
        ) {
            return Err(IpcError::conflict());
        }

        let original_engine = item.source().engine();
        if !self.engine_runtime.is_available(original_engine) {
            return Err(IpcError::engine_unavailable(original_engine));
        }

        self.queue
            .retry_download(id)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn snapshot(&self) -> Result<QueueSnapshotDto, IpcError> {
        let snap = self.queue.snapshot().await.map_err(IpcError::from_queue)?;

        let active_count = snap.active.len();
        let used_connections: usize = snap
            .active
            .iter()
            .map(|a| a.active_connections.max(a.allocated_connections).max(1))
            .sum();

        Ok(QueueSnapshotDto {
            active_count,
            queued_count: snap.queued.len(),
            paused_count: snap.paused.len(),
            used_connections,
            total_connections: snap.global_connection_limit,
            dispatch_paused: snap.dispatch_paused,
            manual_dispatch_paused: snap.manual_dispatch_paused,
            schedule_dispatch_paused: snap.schedule_dispatch_paused,
        })
    }

    async fn set_queue_dispatch_paused(&self, paused: bool) -> Result<QueueSnapshotDto, IpcError> {
        if paused {
            self.queue.pause_dispatch().await
        } else {
            self.queue.resume_dispatch().await
        }
        .map_err(IpcError::from_queue)?;
        self.snapshot().await
    }

    async fn set_priority(
        &self,
        id: DownloadId,
        priority: DownloadPriority,
    ) -> Result<DownloadDto, IpcError> {
        self.queue
            .set_priority(id, priority)
            .await
            .map_err(IpcError::from_queue)?;
        self.get(id).await
    }

    async fn move_up_queue(&self, id: DownloadId) -> Result<(), IpcError> {
        self.queue.move_up(id).await.map_err(IpcError::from_queue)
    }

    async fn move_down_queue(&self, id: DownloadId) -> Result<(), IpcError> {
        self.queue.move_down(id).await.map_err(IpcError::from_queue)
    }

    async fn schedule(&self, id: DownloadId, scheduled_at_ms: u64) -> Result<(), IpcError> {
        let scheduled_at = Timestamp::from_unix_millis(scheduled_at_ms);
        self.scheduler
            .schedule_download(id, scheduled_at)
            .await
            .map_err(IpcError::from_schedule)
    }

    async fn remove_schedule(&self, id: DownloadId) -> Result<(), IpcError> {
        self.scheduler
            .remove_schedule(id)
            .await
            .map_err(IpcError::from_schedule)
    }

    async fn get_schedule(&self, id: DownloadId) -> Result<Option<ScheduleDto>, IpcError> {
        self.scheduler
            .get_schedule(id)
            .await
            .map_err(IpcError::from_schedule)
            .map(|opt| {
                opt.map(|schedule| ScheduleDto {
                    download_id: schedule.download_id().to_string(),
                    scheduled_at_ms: schedule.scheduled_at().as_unix_millis(),
                    enabled: schedule.enabled(),
                })
            })
    }
}

fn validate_source_url(source: &str) -> Result<String, IpcError> {
    let parsed = Url::parse(source).map_err(|_| IpcError::invalid_request())?;
    match parsed.scheme() {
        "http" | "https" => {
            validate_http_source_url(source).map_err(|_| IpcError::invalid_request())
        }
        "ftp" | "ftps" | "sftp" => {
            if source.split('/').any(|segment| segment == "..")
                || parsed.password().is_some()
                || parsed.host_str().is_none()
                || parsed.path().is_empty()
                || parsed.path() == "/"
            {
                return Err(IpcError::invalid_request());
            }
            Ok(parsed.to_string())
        }
        _ => Err(IpcError::invalid_request()),
    }
}

fn engine_matches_source(engine: EngineKind, source: &Url) -> bool {
    match engine {
        EngineKind::DirectHttp | EngineKind::YtDlp | EngineKind::Hls | EngineKind::Metalink => {
            matches!(source.scheme(), "http" | "https")
        }
        EngineKind::Ftp => source.scheme() == "ftp",
        EngineKind::Ftps => source.scheme() == "ftps",
        EngineKind::Sftp => source.scheme() == "sftp",
        EngineKind::NativeTorrent | EngineKind::Aria2 => false,
        EngineKind::Ed2k => source.scheme() == "ed2k",
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddDownloadRequest {
    pub url: String,
    #[serde(default)]
    pub package_name: Option<String>,
    /// Optional engine override. When absent, the backend selects an engine by source type.
    /// The backend validates availability; the frontend cannot bypass this check.
    #[serde(default)]
    pub engine_kind: Option<EngineKind>,
    /// Referer URL captured by the browser extension (forwarded as `Referer` header).
    #[serde(default)]
    pub referer: Option<String>,
    /// Browser User-Agent string captured by the extension.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Raw `Cookie` header value captured by the extension.
    #[serde(default)]
    pub cookies: Option<String>,
    /// Extra tracker announce URLs to add to the torrent (magnet or .torrent file).
    /// Ignored for non-torrent downloads.
    #[serde(default)]
    pub extra_trackers: Option<Vec<String>>,
    /// Optional per-download proxy URL that overrides the global desktop proxy.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Per-download subtitle download override. `None` defers to the global setting.
    #[serde(default)]
    pub download_subtitles: Option<bool>,
    /// Per-download subtitle languages (comma-separated, e.g. `"en,fr"`).
    /// Only used when `download_subtitles` is `Some(true)`.
    #[serde(default)]
    pub subtitle_langs: Option<String>,
    /// When set for an HLS download, use this specific variant playlist URL
    /// instead of the original master playlist URL.
    #[serde(default)]
    pub hls_variant_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BatchDownloadResultDto {
    pub input_url: String,
    pub download_id: Option<String>,
    pub error: Option<String>,
}

fn package_destination(downloads_dir: &Path, package_name: Option<&str>) -> PathBuf {
    let Some(package_name) = package_name else {
        return downloads_dir.to_path_buf();
    };
    fetchx_core_services::sanitize_download_package_name(package_name).map_or_else(
        || downloads_dir.to_path_buf(),
        |name| downloads_dir.join(name),
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetPriorityRequest {
    pub priority: DownloadPriority,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDownloadRequest {
    /// Unix timestamp in milliseconds at which the download should start.
    pub scheduled_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub ipc_version: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadDto {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub destination: String,
    pub status: DownloadStatus,
    pub priority: DownloadPriority,
    pub engine: EngineKind,
    pub progress: DownloadProgressDto,
    pub error: Option<DownloadErrorDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadRuntimeStatusDto {
    pub id: String,
    pub status: DownloadStatus,
    pub progress: DownloadProgressDto,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DuplicateDownloadMatchDto {
    pub id: String,
    pub filename: String,
    pub status: DownloadStatus,
    pub updated_at: u64,
}

impl From<&fetchx_models::DownloadProgress> for DownloadProgressDto {
    fn from(progress: &fetchx_models::DownloadProgress) -> Self {
        Self {
            downloaded_bytes: progress.downloaded_bytes(),
            total_bytes: progress.total_bytes(),
            speed_bytes_per_second: progress.speed_bytes_per_second(),
            estimated_remaining_seconds: progress.estimated_remaining_seconds(),
            percentage_basis_points: progress.percentage_basis_points(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadProgressDto {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: Option<u64>,
    pub estimated_remaining_seconds: Option<u64>,
    pub percentage_basis_points: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadErrorDto {
    pub code: DownloadErrorCode,
    pub message: String,
}

/// A single downloadable format, derived from [`fetchx_media_extractor::MediaFormat`].
#[derive(Clone, Debug, Serialize)]
pub struct MediaFormatDto {
    pub format_id: String,
    pub label: String,
    pub ext: String,
    pub protocol: String,
    pub url: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub bitrate: Option<u64>,
    pub filesize: Option<u64>,
}

/// Structured media information returned by the unified extraction pipeline.
#[derive(Clone, Debug, Serialize)]
pub struct MediaInfoDto {
    pub title: String,
    pub duration: Option<f64>,
    pub thumbnail_url: Option<String>,
    pub webpage_url: String,
    pub extractor_name: String,
    pub formats: Vec<MediaFormatDto>,
}

/// Transfer object for a persisted one-shot schedule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScheduleDto {
    pub download_id: String,
    /// Unix milliseconds at which the download is scheduled to start.
    pub scheduled_at_ms: u64,
    pub enabled: bool,
}

/// Live queue statistics for the status bar.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QueueSnapshotDto {
    pub active_count: usize,
    pub queued_count: usize,
    pub paused_count: usize,
    pub used_connections: usize,
    pub total_connections: usize,
    pub dispatch_paused: bool,
    pub manual_dispatch_paused: bool,
    pub schedule_dispatch_paused: bool,
}

impl DownloadDto {
    fn from_with_queue_position(item: DownloadItem, queue_position: Option<u64>) -> Self {
        let progress = item.progress();
        Self {
            id: item.id().to_string(),
            url: item.source().url().to_owned(),
            filename: item.filename().as_str().to_owned(),
            destination: item.destination().to_string_lossy().into_owned(),
            status: item.status(),
            priority: item.priority(),
            engine: item.source().engine(),
            progress: DownloadProgressDto {
                downloaded_bytes: progress.downloaded_bytes(),
                total_bytes: progress.total_bytes(),
                speed_bytes_per_second: progress.speed_bytes_per_second(),
                estimated_remaining_seconds: progress.estimated_remaining_seconds(),
                percentage_basis_points: progress.percentage_basis_points(),
            },
            error: item.error().map(|error| DownloadErrorDto {
                code: error.code(),
                message: error.message().to_owned(),
            }),
            queue_position,
            created_at: item.created_at().as_unix_millis(),
            updated_at: item.updated_at().as_unix_millis(),
        }
    }
}

impl From<DownloadItem> for DownloadDto {
    fn from(item: DownloadItem) -> Self {
        Self::from_with_queue_position(item, None)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorCode {
    InvalidRequest,
    NotFound,
    Conflict,
    DuplicateUrl,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: &'static str,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for IpcError {}

impl IpcError {
    fn invalid_request() -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message: "Enter a valid supported download URL without embedded passwords.",
        }
    }

    fn invalid_torrent_restart_source() -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message: "Select a valid v1 BitTorrent magnet URI or .torrent file.",
        }
    }

    fn torrent_restart_source_mismatch() -> Self {
        Self {
            code: IpcErrorCode::Conflict,
            message: "The current source does not match the saved aria2 torrent info hash.",
        }
    }

    fn invalid_torrent_restart_selection() -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message: "Inspect the current torrent and select at least one file.",
        }
    }

    fn not_found() -> Self {
        Self {
            code: IpcErrorCode::NotFound,
            message: "The download was not found.",
        }
    }

    fn conflict() -> Self {
        Self {
            code: IpcErrorCode::Conflict,
            message: "The download cannot perform that action in its current state.",
        }
    }

    fn duplicate_url(msg: &'static str) -> Self {
        Self {
            code: IpcErrorCode::DuplicateUrl,
            message: msg,
        }
    }

    fn destination_conflict() -> Self {
        Self {
            code: IpcErrorCode::Conflict,
            message: "Choose a new destination that does not exist or overlap another download.",
        }
    }

    fn destination_unavailable() -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message: "The selected destination could not be prepared. Check its path and permissions.",
        }
    }

    fn internal() -> Self {
        Self {
            code: IpcErrorCode::Internal,
            message: "The request could not be completed.",
        }
    }

    /// Returned when the user dismisses a settings export/import file dialog.
    fn settings_cancelled(message: &'static str) -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message,
        }
    }

    /// Returned when an imported settings file is missing, malformed, or unsupported.
    fn invalid_settings_file() -> Self {
        Self {
            code: IpcErrorCode::InvalidRequest,
            message: "The selected file is not a valid FetchX settings export.",
        }
    }

    fn from_queue(error: QueueError) -> Self {
        match error {
            QueueError::DownloadNotFound(_)
            | QueueError::StateMachine(StateMachineError::NotFound(_)) => Self::not_found(),
            QueueError::InvalidStatus { .. }
            | QueueError::StateMachine(
                StateMachineError::InvalidTransition { .. }
                | StateMachineError::ConcurrentTransition { .. },
            )
            | QueueError::UnsupportedEngineOperation { .. }
            | QueueError::PauseSuperseded
            | QueueError::ResourceLimitInUse => Self::conflict(),
            _ => Self::internal(),
        }
    }

    fn from_schedule(error: ScheduleError) -> Self {
        match error {
            ScheduleError::DownloadNotFound(_) | ScheduleError::ScheduleNotFound(_) => {
                Self::not_found()
            }
            ScheduleError::InvalidState { .. } | ScheduleError::InvalidSchedule(_) => {
                Self::conflict()
            }
            ScheduleError::Timestamp(_) => Self::internal(),
            _ => Self::internal(),
        }
    }

    /// Used when the engine selected for a source is not available at runtime.
    /// We do not silently fall back to a different engine.
    pub(crate) fn engine_unavailable(kind: EngineKind) -> Self {
        // Static messages per engine; do not expose process details or secrets.
        let message: &'static str = match kind {
            EngineKind::Aria2 => {
                "The external aria2 engine was removed. Use a native restart for supported legacy jobs."
            }
            EngineKind::YtDlp => {
                "The media download engine is not available. Check Settings → Download Engines to enable it."
            }
            EngineKind::NativeTorrent => {
                "The torrent engine is not available. Check Settings → Download Engines to enable it."
            }
            EngineKind::Ftp | EngineKind::Ftps => {
                "The FTP engine is not available. Check Settings → Download Engines to enable it."
            }
            EngineKind::Sftp => {
                "The SFTP engine is not available. Check Settings → Download Engines to enable it."
            }
            EngineKind::Metalink => "The Metalink engine is not available in this build.",
            EngineKind::Ed2k => {
                "The ed2k engine is not available. Check Settings → Download Engines to enable it."
            }
            EngineKind::DirectHttp | EngineKind::Hls => {
                // DirectHttp and Hls are always available; this branch should not be reached.
                "The requested download engine is not available."
            }
        };
        Self {
            code: IpcErrorCode::Conflict,
            message,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeleteDownloadResponse {
    pub download_id: String,
    pub removed: bool,
}

pub const APP_VERSION: &str = match option_env!("FETCHX_APP_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: "FetchX",
        version: APP_VERSION,
        ipc_version: 1,
    }
}

#[tauri::command]
pub async fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadDto>, IpcError> {
    state.list().await
}

#[tauri::command]
pub async fn list_runtime_statuses(
    state: State<'_, AppState>,
) -> Result<Vec<DownloadRuntimeStatusDto>, IpcError> {
    let statuses = if let Some(core) = state.core_services.as_ref() {
        core.download_runtime_statuses()
            .await
            .map_err(|_| IpcError::internal())?
    } else {
        state
            .repository
            .list()
            .await
            .map_err(|_| IpcError::internal())?
            .iter()
            .map(fetchx_models::DownloadRuntimeStatus::from)
            .collect()
    };

    Ok(statuses
        .iter()
        .map(|status| DownloadRuntimeStatusDto {
            id: status.id().to_string(),
            status: status.status(),
            progress: DownloadProgressDto::from(status.progress()),
            updated_at: status.updated_at().as_unix_millis(),
        })
        .collect())
}

#[tauri::command]
pub async fn find_duplicate_download(
    url: String,
    state: State<'_, AppState>,
) -> Result<Option<DuplicateDownloadMatchDto>, IpcError> {
    let core = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let duplicate = core
        .find_duplicate_download(&url)
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(duplicate.map(|item| DuplicateDownloadMatchDto {
        id: item.id().to_string(),
        filename: item.filename().to_owned(),
        status: item.status(),
        updated_at: item.updated_at().as_unix_millis(),
    }))
}

/// Checks whether a URL is already in the downloads list without attempting a
/// full add. Returns the matching download if one exists, or `None`.
#[tauri::command]
pub async fn check_duplicate_url(
    url: String,
    state: State<'_, AppState>,
) -> Result<Option<DuplicateDownloadMatchDto>, IpcError> {
    let core = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let duplicate = core
        .find_duplicate_download(&url)
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(duplicate.map(|item| DuplicateDownloadMatchDto {
        id: item.id().to_string(),
        filename: item.filename().to_owned(),
        status: item.status(),
        updated_at: item.updated_at().as_unix_millis(),
    }))
}

#[tauri::command]
pub fn extract_download_links(text: String) -> Vec<String> {
    fetchx_core_services::extract_download_links(&text)
}

/// Alias for `extract_download_links` used by the Batch Add dialog.
#[tauri::command]
pub fn extract_links_from_text(text: String) -> Vec<String> {
    fetchx_core_services::extract_download_links(&text)
}

#[tauri::command]
pub fn suggest_download_filename(url: String) -> String {
    fetchx_core_services::suggest_download_filename(&url)
}

#[tauri::command]
pub fn suggest_download_package_name(url: String, filename: String) -> String {
    fetchx_core_services::suggest_download_package_name(&url, &filename)
}

#[tauri::command]
pub fn normalize_download_url(url: String) -> String {
    fetchx_core_services::normalize_download_url(&url)
}

#[tauri::command]
pub fn is_duplicate_download_url(url: String, existing_urls: Vec<String>) -> bool {
    fetchx_core_services::is_duplicate_download_url(&url, &existing_urls)
}

#[tauri::command]
pub fn match_download_category(
    filename: String,
    url: Option<String>,
    page_url: Option<String>,
) -> Option<String> {
    fetchx_core_services::match_download_category(&filename, url.as_deref(), page_url.as_deref())
        .map(str::to_owned)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_download(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AddDownloadRequest,
) -> Result<DownloadDto, IpcError> {
    let download = state.add(request).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_torrent_download(
    app: AppHandle,
    state: State<'_, AppState>,
    torrent_path: String,
    selected_file_indexes: Option<Vec<u32>>,
    engine_kind: Option<fetchx_models::EngineKind>,
) -> Result<DownloadDto, IpcError> {
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let torrent_path = Path::new(&torrent_path).to_path_buf();
    let id = match engine_kind {
        Some(engine) => {
            core_services
                .enqueue_torrent_file_with_selection_and_engine(
                    torrent_path,
                    engine,
                    DownloadPriority::Normal,
                    None,
                    selected_file_indexes,
                )
                .await
        }
        None => {
            core_services
                .enqueue_torrent_file_with_selection(
                    torrent_path,
                    DownloadPriority::Normal,
                    None,
                    selected_file_indexes,
                )
                .await
        }
    }
    .map_err(|_| IpcError::internal())?;
    let download = state.get(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_media_download(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    format_id: Option<String>,
    extension: Option<String>,
    audio_format: Option<YtDlpAudioFormat>,
    download_subtitles: Option<bool>,
    subtitle_langs: Option<String>,
) -> Result<DownloadDto, IpcError> {
    let download = state
        .add_with_ytdlp_format(
            AddDownloadRequest {
                url,
                engine_kind: Some(EngineKind::YtDlp),
                download_subtitles,
                subtitle_langs,
                ..Default::default()
            },
            format_id,
            extension,
            audio_format,
        )
        .await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

// Keep the named arguments as the Tauri IPC payload contract.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn add_download_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    urls: Vec<String>,
    package_name: Option<String>,
    engine: EngineKind,
    priority: DownloadPriority,
    format_id: Option<String>,
    extension: Option<String>,
    audio_format: Option<YtDlpAudioFormat>,
) -> Result<Vec<BatchDownloadResultDto>, IpcError> {
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let requests = urls
        .iter()
        .map(|url| BatchDownloadRequest {
            url: url.clone(),
            package_name: package_name.clone(),
            engine: if engine == EngineKind::DirectHttp {
                EngineKind::from_source_url(url)
            } else {
                engine
            },
            format_id: format_id.clone(),
            extension: extension.clone(),
            audio_format,
        })
        .collect();
    let outcomes = if package_name.is_some() {
        core_services
            .enqueue_download_batch(requests, priority)
            .await
    } else {
        core_services
            .enqueue_download_batch_by_source(requests, priority)
            .await
    };
    let mut results = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let result = match outcome.result {
            Ok(id) => {
                let download_id = id.to_string();
                match state.get(id).await {
                    Ok(download) => {
                        emit_event(
                            &app,
                            DOWNLOAD_UPDATED,
                            &DownloadUpdatedEvent {
                                download: download.clone(),
                            },
                        );
                        BatchDownloadResultDto {
                            input_url: outcome.input_url,
                            download_id: Some(download_id),
                            error: None,
                        }
                    }
                    Err(_) => BatchDownloadResultDto {
                        input_url: outcome.input_url,
                        download_id: Some(download_id),
                        error: Some(
                            "Download was queued, but its details could not be loaded. Refresh the list."
                                .to_owned(),
                        ),
                    },
                }
            }
            Err(error) => BatchDownloadResultDto {
                input_url: outcome.input_url,
                download_id: None,
                error: Some(error.to_string()),
            },
        };
        results.push(result);
    }
    Ok(results)
}

fn extension_is_safe(extension: Option<&str>) -> bool {
    extension.is_some_and(|value| {
        !value.is_empty()
            && value.len() <= 10
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

/// Maps a [`fetchx_media_extractor::MediaInfo`] to the Tauri IPC [`MediaInfoDto`].
fn media_info_to_dto(info: fetchx_media_extractor::MediaInfo) -> MediaInfoDto {
    MediaInfoDto {
        title: info.title,
        duration: info.duration,
        thumbnail_url: info.thumbnail_url,
        webpage_url: info.webpage_url,
        extractor_name: info.extractor_name,
        formats: info
            .formats
            .into_iter()
            .map(|f| MediaFormatDto {
                format_id: f.id,
                label: f.label,
                ext: f.ext,
                protocol: match f.protocol {
                    MediaProtocol::Http => "http".to_owned(),
                    MediaProtocol::Hls => "hls".to_owned(),
                    MediaProtocol::Dash => "dash".to_owned(),
                    MediaProtocol::YtDlp => "ytdlp".to_owned(),
                    MediaProtocol::Other(s) => s,
                    _ => "other".to_owned(),
                },
                url: f.url,
                video_codec: f.video_codec,
                audio_codec: f.audio_codec,
                width: f.width,
                height: f.height,
                fps: f.fps,
                bitrate: f.bitrate,
                filesize: f.filesize,
            })
            .collect(),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn extract_media_formats(
    state: State<'_, AppState>,
    url: String,
) -> Result<MediaInfoDto, IpcError> {
    state.extract_media_formats(&url).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn discover_ytdlp_formats(
    state: State<'_, AppState>,
    url: String,
) -> Result<Vec<YtDlpFormat>, IpcError> {
    state.discover_ytdlp_formats(&url).await
}

/// Transfer object for a single HLS variant stream.
#[derive(Clone, Debug, Serialize)]
pub struct HlsVariantDto {
    /// Absolute URL of the variant media playlist.
    pub url: String,
    /// Peak bandwidth in bits per second.
    pub bandwidth_bps: u64,
    /// Resolution string, e.g. `"1920x1080"`.
    pub resolution: Option<String>,
    /// Codecs string from the manifest.
    pub codecs: Option<String>,
    /// Human-readable label, e.g. `"1080p · 5.2 Mbps"` or `"Audio only"`.
    pub label: String,
    /// True when the variant carries no video tracks (audio-only stream).
    pub is_audio_only: bool,
}

fn make_hls_label(
    resolution: Option<&str>,
    bandwidth_bps: u64,
    codecs: Option<&str>,
) -> (String, bool) {
    let mbps = bandwidth_bps as f64 / 1_000_000.0;
    let speed_str = format!("{mbps:.1} Mbps");

    // Heuristic: if codecs are present and contain no video codec, it is audio-only.
    let is_audio_only = match codecs {
        Some(c) => {
            let lower = c.to_lowercase();
            // Common video codec prefixes: avc, hevc, hvc, vp8, vp9, av01, dvh.
            let has_video = lower.contains("avc")
                || lower.contains("hvc")
                || lower.contains("hev")
                || lower.contains("vp8")
                || lower.contains("vp9")
                || lower.contains("av01")
                || lower.contains("dvh");
            !has_video
        }
        None => resolution.is_none(),
    };

    let label = if is_audio_only {
        format!("Audio only · {speed_str}")
    } else if let Some(res) = resolution {
        // Extract vertical dimension for a friendly label (e.g. "1080p").
        let height_label = res
            .split('x')
            .nth(1)
            .and_then(|h| h.parse::<u32>().ok())
            .map(|h| format!("{h}p"))
            .unwrap_or_else(|| res.to_owned());
        format!("{height_label} · {speed_str}")
    } else {
        speed_str
    };

    (label, is_audio_only)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn discover_hls_variants(url: String) -> Result<Vec<HlsVariantDto>, String> {
    if !is_hls_url(&url) {
        return Err("URL does not appear to be an HLS playlist".to_owned());
    }
    let mut variants = fetch_hls_variants(&url).await.map_err(|e| e.to_string())?;

    // Sort best-first by bandwidth descending.
    variants.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));

    let dtos = variants
        .into_iter()
        .map(|v| {
            let (label, is_audio_only) =
                make_hls_label(v.resolution.as_deref(), v.bandwidth, v.codecs.as_deref());
            HlsVariantDto {
                url: v.uri,
                bandwidth_bps: v.bandwidth,
                resolution: v.resolution,
                codecs: v.codecs,
                label,
                is_audio_only,
            }
        })
        .collect();

    Ok(dtos)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pause_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.pause(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn resume_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.resume(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.cancel(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DeleteDownloadResponse, IpcError> {
    let id = DownloadId::parse(&download_id).map_err(|_| IpcError::invalid_request())?;
    state.delete(id).await?;
    emit_event(
        &app,
        DOWNLOAD_REMOVED,
        &DownloadRemovedEvent {
            download_id: download_id.clone(),
        },
    );
    Ok(DeleteDownloadResponse {
        download_id,
        removed: true,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_download_priority(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
    request: SetPriorityRequest,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.set_priority(id, request.priority).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn move_download_up_in_queue(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<(), IpcError> {
    let id = parse_download_id(&download_id)?;
    state.move_up_queue(id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn move_download_down_in_queue(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<(), IpcError> {
    let id = parse_download_id(&download_id)?;
    state.move_down_queue(id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn schedule_download(
    state: State<'_, AppState>,
    download_id: String,
    request: ScheduleDownloadRequest,
) -> Result<ScheduleDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    state.schedule(id, request.scheduled_at_ms).await?;
    // Return the persisted schedule so the frontend has the confirmed value.
    state
        .get_schedule(id)
        .await?
        .ok_or_else(IpcError::not_found)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn remove_download_schedule(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<(), IpcError> {
    let id = parse_download_id(&download_id)?;
    state.remove_schedule(id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_download_schedule(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<Option<ScheduleDto>, IpcError> {
    let id = parse_download_id(&download_id)?;
    state.get_schedule(id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn retry_download(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let download = state.retry(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_download_url(
    app: AppHandle,
    state: State<'_, AppState>,
    download_id: String,
    new_url: String,
) -> Result<DownloadDto, IpcError> {
    let id = parse_download_id(&download_id)?;
    let validated_url = validate_source_url(&new_url)?;
    let core = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core.update_download_url(id, validated_url)
        .await
        .map_err(IpcError::from_queue)?;
    let download = state.get(id).await?;
    emit_event(
        &app,
        DOWNLOAD_UPDATED,
        &DownloadUpdatedEvent {
            download: download.clone(),
        },
    );
    Ok(download)
}

#[tauri::command]
pub async fn get_queue_snapshot(state: State<'_, AppState>) -> Result<QueueSnapshotDto, IpcError> {
    state.snapshot().await
}

#[tauri::command]
pub async fn pause_queue_dispatch(
    state: State<'_, AppState>,
) -> Result<QueueSnapshotDto, IpcError> {
    state.set_queue_dispatch_paused(true).await
}

#[tauri::command]
pub async fn resume_queue_dispatch(
    state: State<'_, AppState>,
) -> Result<QueueSnapshotDto, IpcError> {
    state.set_queue_dispatch_paused(false).await
}

// ---------------------------------------------------------------------------
// T4 — Pause all / Resume all
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn pause_all_downloads(state: State<'_, AppState>) -> Result<(), IpcError> {
    let snap = state.queue.snapshot().await.map_err(IpcError::from_queue)?;
    for active in &snap.active {
        let id = active.item.id();
        match state.queue.pause(id).await {
            Ok(()) => {}
            Err(QueueError::InvalidStatus { .. }) => {}
            Err(e) => return Err(IpcError::from_queue(e)),
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn resume_all_downloads(state: State<'_, AppState>) -> Result<(), IpcError> {
    let snap = state.queue.snapshot().await.map_err(IpcError::from_queue)?;
    for id in &snap.paused {
        match state.queue.resume(*id).await {
            Ok(()) => {}
            Err(QueueError::InvalidStatus { .. }) => {}
            Err(e) => return Err(IpcError::from_queue(e)),
        }
    }
    Ok(())
}

/// Torrent metadata transfer object for the desktop frontend.
#[derive(Clone, Debug, Serialize)]
pub struct TorrentMetadataDto {
    pub download_id: String,
    pub source_type: String,
    pub info_hash: String,
    pub torrent_name: String,
    pub destination: String,
    pub metadata_state: String,
    pub selected_file_indexes: Vec<u32>,
    pub selected_total_bytes: u64,
    pub aria2_gid: Option<String>,
    pub seed_state: String,
    pub uploaded_bytes: u64,
    pub upload_speed_bps: u64,
    pub peer_count: u64,
    pub seed_count: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TorrentFileDto {
    pub index: u32,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TorrentInspectionDto {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub files: Vec<TorrentFileDto>,
}

/// Torrent telemetry transfer object for the desktop frontend.
#[derive(Clone, Debug, Serialize)]
pub struct TorrentTelemetryDto {
    pub download_speed_bps: Option<u64>,
    pub upload_speed_bps: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub peer_count: Option<u64>,
    pub completed_peer_count: Option<u64>,
    pub seed_count: Option<u64>,
    pub share_ratio: Option<f64>,
    pub seeding: Option<bool>,
    pub tracker_uris: Option<Vec<String>>,
    pub connected_peers: Option<u32>,
    pub total_peers: Option<u32>,
}

#[tauri::command]
pub async fn get_torrent_metadata(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<TorrentMetadataDto>, IpcError> {
    let download_id = parse_download_id(&id)?;
    let row = state
        .database
        .torrent_metadata()
        .get_torrent_metadata(download_id)
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(row.map(|r| TorrentMetadataDto {
        download_id: r.download_id.to_string(),
        source_type: format!("{:?}", r.source_type),
        info_hash: r.info_hash,
        torrent_name: r.torrent_name,
        destination: r.destination.to_string_lossy().into_owned(),
        metadata_state: r.metadata_state,
        selected_file_indexes: r.selected_file_indexes,
        selected_total_bytes: r.selected_total_bytes,
        aria2_gid: r.aria2_gid,
        seed_state: format!("{:?}", r.seed_state),
        uploaded_bytes: r.uploaded_bytes,
        upload_speed_bps: r.upload_speed_bps,
        peer_count: r.peer_count,
        seed_count: r.seed_count,
    }))
}

#[tauri::command]
pub async fn get_torrent_download_metadata(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<fetchx_models::TorrentDownloadMetadata>, IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .torrent_download_metadata(download_id)
        .await
        .map_err(|_| IpcError::internal())
}

#[tauri::command]
pub async fn select_torrent_files(
    state: State<'_, AppState>,
    id: String,
    file_indexes: Vec<u32>,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .select_torrent_files(download_id, file_indexes)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command]
pub async fn set_torrent_file_selected(
    state: State<'_, AppState>,
    id: String,
    file_index: u32,
    selected: bool,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .set_torrent_file_selected(download_id, file_index, selected)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command]
pub async fn set_torrent_selection_all(
    state: State<'_, AppState>,
    id: String,
    selected: bool,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .set_torrent_selection_all(download_id, selected)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_torrent_directory_selected(
    state: State<'_, AppState>,
    id: String,
    directory: String,
    selected: bool,
) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .set_torrent_directory_selected(download_id, directory, selected)
        .await
        .map_err(|_| IpcError::invalid_request())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn inspect_torrent_file(
    state: State<'_, AppState>,
    torrent_path: String,
) -> Result<TorrentInspectionDto, IpcError> {
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let metadata = core_services
        .inspect_torrent_file(Path::new(&torrent_path).to_path_buf())
        .await
        .map_err(|_| IpcError::invalid_request())?;
    let files = metadata
        .files
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            Ok(TorrentFileDto {
                index: u32::try_from(index).map_err(|_| IpcError::internal())?,
                path: file.path.join("/"),
                size: file.size,
            })
        })
        .collect::<Result<Vec<_>, IpcError>>()?;
    Ok(TorrentInspectionDto {
        info_hash: metadata.info_hash,
        name: metadata.name,
        total_size: metadata.total_size,
        files,
    })
}

#[tauri::command]
pub async fn get_torrent_telemetry(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<TorrentTelemetryDto>, IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let telemetry = core_services
        .torrent_telemetry(download_id)
        .await
        .map_err(|_| IpcError::internal())?;
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
        connected_peers: value.connected_peers,
        total_peers: value.total_peers,
    }))
}

#[tauri::command]
pub async fn restart_aria2_http_with_native(
    state: State<'_, AppState>,
    id: String,
    current_url: String,
    destination: String,
    confirmed_direct_file: bool,
) -> Result<DownloadDto, IpcError> {
    let aria2_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let native_id = core_services
        .restart_aria2_http_with_native(
            aria2_id,
            current_url,
            PathBuf::from(destination),
            confirmed_direct_file,
        )
        .await
        .map_err(|error| match error {
            fetchx_core_services::BatchDownloadError::InvalidUrl => IpcError::invalid_request(),
            fetchx_core_services::BatchDownloadError::DestinationCollision => {
                IpcError::destination_conflict()
            }
            fetchx_core_services::BatchDownloadError::RestartNotEligible => IpcError::conflict(),
            fetchx_core_services::BatchDownloadError::InvalidDestination
            | fetchx_core_services::BatchDownloadError::Destination(_) => {
                IpcError::destination_unavailable()
            }
            _ => IpcError::internal(),
        })?;
    state.get(native_id).await
}

#[tauri::command]
pub async fn restart_aria2_torrent_with_native(
    state: State<'_, AppState>,
    id: String,
    current_magnet_uri: String,
    destination: String,
    confirmed_current_source: bool,
) -> Result<DownloadDto, IpcError> {
    let aria2_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let native_id = core_services
        .restart_aria2_torrent_with_native(
            aria2_id,
            current_magnet_uri,
            PathBuf::from(destination),
            confirmed_current_source,
        )
        .await
        .map_err(|error| match error {
            fetchx_core_services::BatchDownloadError::InvalidTorrentRestartSource => {
                IpcError::invalid_torrent_restart_source()
            }
            fetchx_core_services::BatchDownloadError::TorrentSourceMismatch => {
                IpcError::torrent_restart_source_mismatch()
            }
            fetchx_core_services::BatchDownloadError::RestartNotEligible => IpcError::conflict(),
            fetchx_core_services::BatchDownloadError::InvalidTorrentRestartDestination
            | fetchx_core_services::BatchDownloadError::Destination(_) => {
                IpcError::destination_unavailable()
            }
            fetchx_core_services::BatchDownloadError::TorrentDestinationCollision => {
                IpcError::destination_conflict()
            }
            fetchx_core_services::BatchDownloadError::UnavailableEngine(engine) => {
                IpcError::engine_unavailable(engine)
            }
            _ => IpcError::internal(),
        })?;
    state.get(native_id).await
}

#[tauri::command]
pub async fn restart_aria2_torrent_file_with_native(
    state: State<'_, AppState>,
    id: String,
    current_torrent_path: String,
    destination: String,
    selected_file_indexes: Vec<u32>,
    confirmed_current_source: bool,
    confirmed_selection: bool,
) -> Result<DownloadDto, IpcError> {
    let aria2_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    let native_id = core_services
        .restart_aria2_torrent_file_with_native(
            aria2_id,
            PathBuf::from(current_torrent_path),
            PathBuf::from(destination),
            selected_file_indexes,
            confirmed_current_source,
            confirmed_selection,
        )
        .await
        .map_err(|error| match error {
            fetchx_core_services::BatchDownloadError::InvalidTorrentRestartSource => {
                IpcError::invalid_torrent_restart_source()
            }
            fetchx_core_services::BatchDownloadError::TorrentSourceMismatch => {
                IpcError::torrent_restart_source_mismatch()
            }
            fetchx_core_services::BatchDownloadError::InvalidTorrentRestartSelection => {
                IpcError::invalid_torrent_restart_selection()
            }
            fetchx_core_services::BatchDownloadError::RestartNotEligible => IpcError::conflict(),
            fetchx_core_services::BatchDownloadError::InvalidTorrentRestartDestination
            | fetchx_core_services::BatchDownloadError::Destination(_) => {
                IpcError::destination_unavailable()
            }
            fetchx_core_services::BatchDownloadError::TorrentDestinationCollision => {
                IpcError::destination_conflict()
            }
            fetchx_core_services::BatchDownloadError::UnavailableEngine(engine) => {
                IpcError::engine_unavailable(engine)
            }
            _ => IpcError::internal(),
        })?;
    state.get(native_id).await
}

#[tauri::command]
pub async fn stop_torrent_seeding(state: State<'_, AppState>, id: String) -> Result<(), IpcError> {
    let download_id = parse_download_id(&id)?;
    let core_services = state
        .core_services
        .as_ref()
        .ok_or_else(IpcError::internal)?;
    core_services
        .stop_torrent_seeding(download_id)
        .await
        .map_err(|_| IpcError::internal())
}

/// Opens the parent directory of a download's destination in the OS file manager.
#[tauri::command(rename_all = "snake_case")]
pub async fn open_download_folder(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<(), IpcError> {
    let id = parse_download_id(&download_id)?;
    let item = state
        .repository
        .get(id)
        .await
        .map_err(|_| IpcError::internal())?
        .ok_or_else(IpcError::not_found)?;
    let folder = item
        .destination()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| item.destination().to_path_buf());
    // `open::that` is non-blocking (it returns once the OS launcher has been
    // spawned) and matches the post-download "OpenFolder" behaviour in app.rs.
    // The newer `that_detached` API does not exist in the pinned `open` 3.2.0.
    open::that(&folder).map_err(|_| IpcError::internal())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Checksum verification
// ---------------------------------------------------------------------------

/// Result returned by [`verify_download_checksum`].
#[derive(Clone, Debug, Serialize)]
pub struct ChecksumVerificationResult {
    /// Hex SHA-256 read from the `.fetchx.sha256` sidecar, or `None` if no sidecar exists.
    pub expected: Option<String>,
    /// Hex SHA-256 computed from the destination file, or `None` if the file is missing.
    pub actual: Option<String>,
    /// `Some(true)` when hashes match, `Some(false)` on mismatch, `None` when no expected hash.
    pub matches: Option<bool>,
    /// Size of the destination file in bytes, or `None` when the file is missing.
    pub file_size: Option<u64>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn verify_download_checksum(
    state: State<'_, AppState>,
    download_id: String,
) -> Result<ChecksumVerificationResult, String> {
    let id = DownloadId::parse(&download_id).map_err(|_| "Invalid download ID.".to_owned())?;
    let item = state
        .repository
        .get(id)
        .await
        .map_err(|_| "Could not load download.".to_owned())?
        .ok_or_else(|| "Download not found.".to_owned())?;

    let dest = item.destination().to_owned();

    // Read the destination file.
    let file_data = match tokio::fs::read(&dest).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(ChecksumVerificationResult {
                expected: None,
                actual: None,
                matches: None,
                file_size: None,
            });
        }
    };
    let file_size = file_data.len() as u64;

    // Compute the actual SHA-256.
    let hash = crate::app::sha256(&file_data);
    let actual_hex: String = hash.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    });

    // Look for the sidecar file.
    let sidecar = {
        let mut s = dest.as_os_str().to_owned();
        s.push(".fetchx.sha256");
        std::path::PathBuf::from(s)
    };
    let expected_hex = match tokio::fs::read_to_string(&sidecar).await {
        Ok(raw) => Some(raw.trim().to_ascii_lowercase()),
        Err(_) => None,
    };

    let matches = expected_hex
        .as_deref()
        .map(|exp| exp == actual_hex.as_str());

    Ok(ChecksumVerificationResult {
        expected: expected_hex,
        actual: Some(actual_hex),
        matches,
        file_size: Some(file_size),
    })
}

fn emit_event<T: Serialize + Clone>(app: &AppHandle, name: &str, payload: &T) {
    if let Err(error) = app.emit(name, payload.clone()) {
        tracing::warn!(%error, event = name, "could not emit desktop event");
    }
}

fn parse_download_id(value: &str) -> Result<DownloadId, IpcError> {
    DownloadId::parse(value).map_err(|_| IpcError::invalid_request())
}

fn filename_for_url(url: &Url) -> String {
    fetchx_core_services::suggest_download_filename(url.as_str())
}

#[tauri::command]
pub async fn register_browser_extensions(
    app: AppHandle,
) -> Result<crate::browser_extension_installer::ExtensionInstallSummary, String> {
    let app_data_dir = app
        .path()
        .app_local_data_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| crate::app::fallback_app_data_dir());
    let bundled_engines_dir = app
        .path()
        .resource_dir()
        .map(|p| p.join("engines"))
        .unwrap_or_else(|_| crate::app::fallback_bundled_engines_dir(&app_data_dir));

    tokio::task::spawn_blocking(move || {
        crate::browser_extension_installer::ensure_registered(&app_data_dir, &bundled_engines_dir)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_browser_extension_status(
    app: AppHandle,
) -> Result<crate::browser_extension_installer::ExtensionInstallStatus, String> {
    let app_data_dir = app
        .path()
        .app_local_data_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| crate::app::fallback_app_data_dir());
    let bundled_engines_dir = app
        .path()
        .resource_dir()
        .map(|p| p.join("engines"))
        .unwrap_or_else(|_| crate::app::fallback_bundled_engines_dir(&app_data_dir));

    tokio::task::spawn_blocking(move || {
        crate::browser_extension_installer::get_extension_status(
            &app_data_dir,
            &bundled_engines_dir,
        )
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_browser_extension_install_page(
    app: AppHandle,
    browser: String,
) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_local_data_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| crate::app::fallback_app_data_dir());
    let bundled_engines_dir = app
        .path()
        .resource_dir()
        .map(|path| path.join("engines"))
        .unwrap_or_else(|_| crate::app::fallback_bundled_engines_dir(&app_data_dir));

    tokio::task::spawn_blocking(move || {
        crate::browser_extension_installer::ensure_registered(&app_data_dir, &bundled_engines_dir)?;
        crate::browser_extension_installer::open_browser_extension_page(&browser)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn launch_browser_with_extension(app: AppHandle, browser: String) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_local_data_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| crate::app::fallback_app_data_dir());
    let bundled_engines_dir = app
        .path()
        .resource_dir()
        .map(|path| path.join("engines"))
        .unwrap_or_else(|_| crate::app::fallback_bundled_engines_dir(&app_data_dir));

    tokio::task::spawn_blocking(move || {
        crate::browser_extension_installer::launch_browser_with_extension(
            &browser,
            &app_data_dir,
            &bundled_engines_dir,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// T3 — Per-download speed limiter
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn set_download_bandwidth_limit(
    state: State<'_, AppState>,
    download_id: String,
    bytes_per_second: Option<u64>,
) -> Result<(), IpcError> {
    let id = DownloadId::parse(&download_id).map_err(|_| IpcError::invalid_request())?;
    if let Some(bps) = bytes_per_second
        && (bps == 0 || bps > 1_000_000_000_000)
    {
        return Err(IpcError::invalid_request());
    }
    state
        .queue
        .set_bandwidth_limit(id, bytes_per_second)
        .await
        .map_err(IpcError::from_queue)
}

// ---------------------------------------------------------------------------
// T17 — History
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_completed_downloads(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<DownloadDto>, IpcError> {
    let limit = limit.unwrap_or(200).min(1000);
    if let Some(ref cs) = state.core_services {
        cs.list_completed(limit)
            .await
            .map(|items| items.into_iter().map(DownloadDto::from).collect())
            .map_err(|_| IpcError::internal())
    } else {
        state
            .repository
            .list_completed(limit)
            .await
            .map(|items| items.into_iter().map(DownloadDto::from).collect())
            .map_err(|_| IpcError::internal())
    }
}

#[tauri::command]
pub async fn clear_download_history(state: State<'_, AppState>) -> Result<u64, IpcError> {
    // Delete completed downloads that no longer have an active part file.
    // Simple implementation: delete all completed records.
    let completed = state
        .repository
        .list_completed(10_000)
        .await
        .map_err(|_| IpcError::internal())?;
    let mut deleted = 0u64;
    for item in completed {
        if state.repository.delete(item.id()).await.unwrap_or(false) {
            deleted += 1;
        }
    }
    Ok(deleted)
}

// ---------------------------------------------------------------------------
// T13 — HTTP credentials
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn provide_download_credentials(
    state: State<'_, AppState>,
    download_id: String,
    username: String,
    password: String,
    /// Optional Windows domain for NTLM authentication.
    domain: Option<String>,
) -> Result<DownloadDto, IpcError> {
    let id = DownloadId::parse(&download_id).map_err(|_| IpcError::invalid_request())?;
    if username.trim().is_empty() || username.len() > 4096 || password.len() > 4096 {
        return Err(IpcError::invalid_request());
    }
    if domain.as_deref().is_some_and(|d| d.len() > 256) {
        return Err(IpcError::invalid_request());
    }
    let item = state
        .repository
        .get(id)
        .await
        .map_err(|_| IpcError::internal())?
        .ok_or_else(IpcError::not_found)?;
    if item.source().engine() != EngineKind::DirectHttp
        || is_hls_url(item.source().url())
        || is_dash_url(item.source().url())
    {
        return Err(IpcError::invalid_request());
    }
    let mut credentials = HttpCredentials::new(username, password);
    if let Some(d) = domain.filter(|d| !d.trim().is_empty()) {
        credentials = credentials.with_domain(d);
    }
    state
        .engine_runtime
        .dispatcher
        .provide_http_credentials_for_retry(id, credentials)
        .map_err(|_| IpcError::internal())?;
    if state.queue.retry_download(id).await.is_err() {
        state
            .engine_runtime
            .dispatcher
            .clear_http_credentials_for_retry(id)
            .map_err(|_| IpcError::internal())?;
        return Err(IpcError::internal());
    }
    state
        .repository
        .get(id)
        .await
        .map_err(|_| IpcError::internal())?
        .ok_or_else(IpcError::not_found)
        .map(DownloadDto::from)
}

// ---------------------------------------------------------------------------
// T9 — Post-queue action
// ---------------------------------------------------------------------------

/// Shared state for the pending post-queue-action countdown.
/// Holds a sender that can be used to cancel the pending action.
/// When `None` there is no pending countdown.
pub struct PostQueueActionState {
    pub cancel_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl Default for PostQueueActionState {
    fn default() -> Self {
        Self {
            cancel_tx: std::sync::Mutex::new(None),
        }
    }
}

#[tauri::command]
pub async fn get_post_queue_action(state: State<'_, AppState>) -> Result<String, IpcError> {
    if let Some(ref cs) = state.core_services {
        let action = cs.get_post_queue_action().await;
        Ok(format!("{action:?}"))
    } else {
        Ok("Nothing".to_owned())
    }
}

/// Saves the post-queue action to the resource policy.
#[tauri::command]
pub async fn set_post_queue_action(
    state: State<'_, AppState>,
    action: String,
) -> Result<(), IpcError> {
    use fetchx_models::PostQueueAction;
    let model = match action.as_str() {
        "nothing" | "Nothing" => PostQueueAction::Nothing,
        "shutdown" | "Shutdown" => PostQueueAction::Shutdown,
        "sleep" | "Sleep" => PostQueueAction::Sleep,
        "hibernate" | "Hibernate" => PostQueueAction::Hibernate,
        _ => return Err(IpcError::invalid_request()),
    };
    let mut config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    config.resource_policy.post_queue_action = model;
    state
        .config_service
        .save(&config)
        .await
        .map_err(|_| IpcError::internal())
}

/// Cancels a pending post-queue-action countdown (if any).
#[tauri::command]
pub async fn cancel_post_queue_action(
    pq_state: State<'_, PostQueueActionState>,
) -> Result<(), IpcError> {
    let maybe_tx = {
        let mut guard = pq_state
            .cancel_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take()
    };
    if let Some(tx) = maybe_tx {
        let _ = tx.send(());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Post-download action
// ---------------------------------------------------------------------------

/// Simple string-based DTO for the post-download action setting.
///
/// The `kind` field mirrors the serde tag of `PostDownloadAction`:
/// `"nothing"`, `"open_file"`, `"open_folder"`, `"run_command"`, or
/// `"extract_archive"`. When `kind` is `"run_command"` the `command` field
/// holds the shell string. When `kind` is `"extract_archive"` the
/// `delete_after` field controls archive deletion after extraction.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PostDownloadActionDto {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_after: Option<bool>,
}

impl From<fetchx_models::PostDownloadAction> for PostDownloadActionDto {
    fn from(action: fetchx_models::PostDownloadAction) -> Self {
        match action {
            fetchx_models::PostDownloadAction::Nothing => Self {
                kind: "nothing".to_owned(),
                command: None,
                delete_after: None,
            },
            fetchx_models::PostDownloadAction::OpenFile => Self {
                kind: "open_file".to_owned(),
                command: None,
                delete_after: None,
            },
            fetchx_models::PostDownloadAction::OpenFolder => Self {
                kind: "open_folder".to_owned(),
                command: None,
                delete_after: None,
            },
            fetchx_models::PostDownloadAction::RunCommand { command } => Self {
                kind: "run_command".to_owned(),
                command: Some(command),
                delete_after: None,
            },
            fetchx_models::PostDownloadAction::ExtractArchive { delete_after } => Self {
                kind: "extract_archive".to_owned(),
                command: None,
                delete_after: Some(delete_after),
            },
        }
    }
}

impl TryFrom<PostDownloadActionDto> for fetchx_models::PostDownloadAction {
    type Error = IpcError;

    fn try_from(dto: PostDownloadActionDto) -> Result<Self, Self::Error> {
        match dto.kind.as_str() {
            "nothing" => Ok(Self::Nothing),
            "open_file" => Ok(Self::OpenFile),
            "open_folder" => Ok(Self::OpenFolder),
            "run_command" => {
                let command = dto.command.unwrap_or_default();
                if command.is_empty() {
                    return Err(IpcError::invalid_request());
                }
                Ok(Self::RunCommand { command })
            }
            "extract_archive" => Ok(Self::ExtractArchive {
                delete_after: dto.delete_after.unwrap_or(false),
            }),
            _ => Err(IpcError::invalid_request()),
        }
    }
}

#[tauri::command]
pub async fn get_post_download_action(
    state: State<'_, AppState>,
) -> Result<PostDownloadActionDto, IpcError> {
    let config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(PostDownloadActionDto::from(
        config.default_post_download_action,
    ))
}

#[tauri::command]
pub async fn set_post_download_action(
    state: State<'_, AppState>,
    action: PostDownloadActionDto,
) -> Result<(), IpcError> {
    let model = fetchx_models::PostDownloadAction::try_from(action)?;
    let mut config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    config.default_post_download_action = model;
    state
        .config_service
        .save(&config)
        .await
        .map_err(|_| IpcError::internal())
}

// ---------------------------------------------------------------------------
// Folder categories
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CategoryRuleDto {
    pub folder: String,
    pub extensions: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FolderCategoriesConfigDto {
    pub enabled: bool,
    pub compressed: CategoryRuleDto,
    pub documents: CategoryRuleDto,
    pub music: CategoryRuleDto,
    pub programs: CategoryRuleDto,
    pub video: CategoryRuleDto,
    pub images: CategoryRuleDto,
    pub custom: String,
}

impl From<fetchx_models::CategoryRule> for CategoryRuleDto {
    fn from(rule: fetchx_models::CategoryRule) -> Self {
        Self {
            folder: rule.folder,
            extensions: rule.extensions,
        }
    }
}

impl From<CategoryRuleDto> for fetchx_models::CategoryRule {
    fn from(dto: CategoryRuleDto) -> Self {
        Self {
            folder: dto.folder,
            extensions: dto.extensions,
        }
    }
}

impl From<fetchx_models::FolderCategoriesConfig> for FolderCategoriesConfigDto {
    fn from(cfg: fetchx_models::FolderCategoriesConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            compressed: CategoryRuleDto::from(cfg.compressed),
            documents: CategoryRuleDto::from(cfg.documents),
            music: CategoryRuleDto::from(cfg.music),
            programs: CategoryRuleDto::from(cfg.programs),
            video: CategoryRuleDto::from(cfg.video),
            images: CategoryRuleDto::from(cfg.images),
            custom: cfg.custom,
        }
    }
}

impl From<FolderCategoriesConfigDto> for fetchx_models::FolderCategoriesConfig {
    fn from(dto: FolderCategoriesConfigDto) -> Self {
        Self {
            enabled: dto.enabled,
            compressed: fetchx_models::CategoryRule::from(dto.compressed),
            documents: fetchx_models::CategoryRule::from(dto.documents),
            music: fetchx_models::CategoryRule::from(dto.music),
            programs: fetchx_models::CategoryRule::from(dto.programs),
            video: fetchx_models::CategoryRule::from(dto.video),
            images: fetchx_models::CategoryRule::from(dto.images),
            custom: dto.custom,
        }
    }
}

#[tauri::command]
pub async fn get_folder_categories_config(
    state: State<'_, AppState>,
) -> Result<FolderCategoriesConfigDto, IpcError> {
    let config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(FolderCategoriesConfigDto::from(config.folder_categories))
}

#[tauri::command]
pub async fn set_folder_categories_config(
    config: FolderCategoriesConfigDto,
    state: State<'_, AppState>,
) -> Result<(), IpcError> {
    let model = fetchx_models::FolderCategoriesConfig::from(config);
    let mut engine_config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    engine_config.folder_categories = model;
    state
        .config_service
        .save(&engine_config)
        .await
        .map_err(|_| IpcError::internal())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct UpdateCheckResult {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub release_notes: String,
}

#[tauri::command]
pub async fn check_for_update() -> Result<UpdateCheckResult, IpcError> {
    let current = env!("CARGO_PKG_VERSION").to_owned();
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/repos/MeherMankar/FetchX/releases/latest")
        .header("User-Agent", format!("FetchX/{}", current))
        .send()
        .await
        .map_err(|_| IpcError::internal())?;
    let body: serde_json::Value = resp.json().await.map_err(|_| IpcError::internal())?;
    let tag = body["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_owned();
    let html_url = body["html_url"].as_str().unwrap_or("").to_owned();
    let notes = body["body"].as_str().unwrap_or("").to_owned();
    let available = !tag.is_empty() && tag != current;
    Ok(UpdateCheckResult {
        available,
        current_version: current,
        latest_version: tag,
        release_url: html_url,
        release_notes: notes,
    })
}

// ---------------------------------------------------------------------------
// Settings export / import
// ---------------------------------------------------------------------------

/// Envelope written by [`export_settings`] and read by [`import_settings`].
///
/// `exported_at` is ignored on import; only `version` and `config` are used.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct SettingsExportEnvelope {
    version: u32,
    exported_at: String,
    config: EngineConfigDto,
}

/// Opens a native save dialog and writes the current engine configuration to a
/// `.json` file as `{ "version": 1, "exported_at": "<ISO timestamp>", "config": <EngineConfigDto> }`.
#[tauri::command(rename_all = "snake_case")]
pub async fn export_settings(app: AppHandle, state: State<'_, AppState>) -> Result<(), IpcError> {
    let chosen = app
        .dialog()
        .file()
        .add_filter("FetchX settings", &["json"])
        .set_file_name("fetchx-settings.json")
        .blocking_save_file();
    let Some(selected) = chosen else {
        // The user dismissed the save dialog without choosing a destination.
        return Err(IpcError::settings_cancelled("Export cancelled."));
    };
    let path = file_path_to_pathbuf(selected)?;

    let config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    let envelope = SettingsExportEnvelope {
        version: 1,
        exported_at: iso_utc_now(),
        config: EngineConfigDto::from(&config),
    };
    let json = serde_json::to_string_pretty(&envelope).map_err(|_| IpcError::internal())?;

    tokio::fs::write(&path, json)
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(())
}

/// Opens a native open dialog for `.json` files, reads and parses the file,
/// validates `version == 1`, applies the imported user-facing settings onto the
/// current configuration, persists it, and returns the updated `EngineConfigDto`.
#[tauri::command(rename_all = "snake_case")]
pub async fn import_settings(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EngineConfigDto, IpcError> {
    let chosen = app
        .dialog()
        .file()
        .add_filter("FetchX settings", &["json"])
        .blocking_pick_file();
    let Some(selected) = chosen else {
        // The user dismissed the open dialog without choosing a file.
        return Err(IpcError::settings_cancelled("Import cancelled."));
    };
    let path = file_path_to_pathbuf(selected)?;

    let contents = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| IpcError::internal())?;
    let envelope: SettingsExportEnvelope =
        serde_json::from_str(&contents).map_err(|_| IpcError::invalid_settings_file())?;
    if envelope.version != 1 {
        return Err(IpcError::invalid_settings_file());
    }

    // The exported `config` is a user-facing subset of the full EngineConfig.
    // Overlay it onto the current (machine-specific) configuration so platform
    // settings such as the aria2 executable path or the torrent session
    // directory are preserved instead of being clobbered with foreign values.
    let mut config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    let imported = envelope.config;
    config.direct_http.enabled = imported.direct_http.enabled;
    config.yt_dlp.enabled = imported.yt_dlp.enabled;
    config.resource_policy = imported.resource_policy;
    config.weekly_queue_schedule = imported.weekly_queue_schedule;
    config.native_builtins = imported.native_builtins;
    state
        .config_service
        .save(&config)
        .await
        .map_err(|_| IpcError::internal())?;

    Ok(EngineConfigDto::from(&config))
}

fn file_path_to_pathbuf(file_path: FilePath) -> Result<PathBuf, IpcError> {
    match file_path {
        FilePath::Path(path) => Ok(path),
        FilePath::Url(url) => url.to_file_path().map_err(|_| IpcError::internal()),
    }
}

/// Formats the current UTC time as an ISO 8601 timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
fn iso_utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

// ---------------------------------------------------------------------------
// Webhook configuration
// ---------------------------------------------------------------------------

/// Transfer object for webhook configuration, mirroring [`fetchx_core_services::WebhookConfig`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WebhookConfigDto {
    pub enabled: bool,
    pub url: String,
    pub on_complete: bool,
    pub on_fail: bool,
    pub secret: String,
}

impl From<fetchx_core_services::WebhookConfig> for WebhookConfigDto {
    fn from(cfg: fetchx_core_services::WebhookConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            url: cfg.url,
            on_complete: cfg.on_complete,
            on_fail: cfg.on_fail,
            secret: cfg.secret,
        }
    }
}

impl From<WebhookConfigDto> for fetchx_core_services::WebhookConfig {
    fn from(dto: WebhookConfigDto) -> Self {
        Self {
            enabled: dto.enabled,
            url: dto.url,
            on_complete: dto.on_complete,
            on_fail: dto.on_fail,
            secret: dto.secret,
        }
    }
}

#[tauri::command]
pub async fn get_webhook_config(state: State<'_, AppState>) -> Result<WebhookConfigDto, IpcError> {
    let config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    Ok(WebhookConfigDto::from(config.webhook))
}

#[tauri::command]
pub async fn set_webhook_config(
    state: State<'_, AppState>,
    config: WebhookConfigDto,
) -> Result<(), IpcError> {
    let webhook = fetchx_core_services::WebhookConfig::from(config);
    let mut engine_config = state
        .config_service
        .load()
        .await
        .map_err(|_| IpcError::internal())?;
    engine_config.webhook = webhook;
    state
        .config_service
        .save(&engine_config)
        .await
        .map_err(|_| IpcError::internal())
}

/// Converts a count of days since the Unix epoch (1970-01-01) to a proleptic
/// Gregorian UTC date, using Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

    use fetchx_core::DownloadStateMachine;
    use fetchx_downloader::{
        DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    };
    use fetchx_models::{DownloadId, DownloadItem, DownloadStatus};
    use fetchx_queue::{DownloadEventBus, EventRepository, QueueConfig};
    use fetchx_scheduler::Scheduler;
    use fetchx_storage::{Database, DownloadRepository, SqliteDownloadRepository};
    use tokio::{sync::watch, time::timeout};

    use super::{
        AddDownloadRequest, AppState, DownloadDto, EngineKind, IpcError, IpcErrorCode,
        engine_matches_source, extension_is_safe, filename_for_url, parse_download_id,
        validate_http_source_url, validate_source_url,
    };
    use fetchx_core_services::{ConfigService, EngineConfig, EngineRuntime};

    struct WaitingEngine {
        repository: SqliteDownloadRepository,
    }

    impl DownloadEngine for WaitingEngine {
        async fn download(
            &self,
            _id: DownloadId,
            mut cancellation: watch::Receiver<bool>,
        ) -> Result<DownloadItem, DownloadEngineError> {
            let _ = cancellation.changed().await;
            Err(DownloadEngineError::Cancelled)
        }
    }

    impl ManagedDownloadEngine for WaitingEngine {
        async fn download_managed(
            &self,
            id: DownloadId,
            mut control: ManagedDownloadControl,
        ) -> Result<DownloadItem, DownloadEngineError> {
            let state_machine = DownloadStateMachine::new(self.repository.clone());
            let item = self
                .repository
                .get(id)
                .await?
                .ok_or(DownloadEngineError::NotFound(id))?;
            if item.status() == DownloadStatus::Queued {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
            }
            state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;

            loop {
                tokio::select! {
                    result = control.cancellation.changed() => {
                        if result.is_err() || *control.cancellation.borrow() {
                            state_machine.transition(id, DownloadStatus::Cancelled).await?;
                            return Err(DownloadEngineError::Cancelled);
                        }
                    }
                    result = control.pause.changed() => {
                        if result.is_err() || *control.pause.borrow() {
                            state_machine.transition(id, DownloadStatus::Paused).await?;
                            return Err(DownloadEngineError::Paused);
                        }
                    }
                }
            }
        }
    }

    async fn test_state() -> Result<AppState, Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let inner_repository = database.downloads();
        let event_bus = DownloadEventBus::new();
        let repository = EventRepository::new(inner_repository.clone(), event_bus);
        let config_service = ConfigService::new(database.config());

        // Keep optional engines disabled in this isolated adapter harness; the
        // test scheduler uses WaitingEngine to preserve lifecycle assertions.
        let mut engine_config = EngineConfig::default();
        engine_config.native_torrent.enabled = false;
        let engine_runtime =
            EngineRuntime::initialize(repository.clone(), &engine_config, &std::env::temp_dir())
                .await?;

        // Build scheduler with WaitingEngine so tests can observe lifecycle transitions.
        let scheduler = Scheduler::start(
            repository.clone(),
            Arc::new(WaitingEngine {
                repository: inner_repository,
            }),
            QueueConfig::default(),
        )
        .await?;
        let queue = scheduler.queue_manager();
        Ok(AppState::new(
            database,
            repository,
            scheduler,
            queue,
            PathBuf::from("test-downloads"),
            config_service,
            engine_runtime,
        ))
    }

    async fn wait_for_status(
        state: &AppState,
        id: DownloadId,
        status: DownloadStatus,
    ) -> Result<(), Box<dyn Error>> {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(item) = state.repository.get(id).await?
                    && item.status() == status
                {
                    return Ok::<(), fetchx_storage::StorageError>(());
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await??;
        Ok(())
    }

    #[test]
    fn add_request_rejects_unknown_fields_during_deserialization() -> Result<(), serde_json::Error>
    {
        assert!(
            serde_json::from_str::<AddDownloadRequest>(
                r#"{"url":"https://example.test/file","destination":"C:\\unsafe"}"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn download_ids_and_direct_http_urls_are_validated() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            parse_download_id("not-an-id").err().map(|error| error.code),
            Some(IpcErrorCode::InvalidRequest)
        );
        assert!(validate_http_source_url("file:///etc/passwd").is_err());
        let credential_url = format!(
            "https://{}:{}@example.test/file",
            "test-user", "test-password"
        );
        assert!(validate_http_source_url(&credential_url).is_err());
        assert_eq!(
            reqwest::Url::parse(&validate_http_source_url("https://example.test/file")?)?.scheme(),
            "https"
        );
        Ok(())
    }

    #[test]
    fn automatic_engine_selection_covers_supported_source_types() -> Result<(), Box<dyn Error>> {
        for (source, expected) in [
            ("https://example.test/video.mp4", EngineKind::DirectHttp),
            ("https://example.test/watch", EngineKind::YtDlp),
            ("https://www.youtube.com/watch?v=abc", EngineKind::YtDlp),
            ("https://example.test/playlist.m3u8", EngineKind::Hls),
            ("https://example.test/manifest.mpd", EngineKind::YtDlp),
            ("https://example.test/file.meta4", EngineKind::Metalink),
            ("ftp://example.test/file.zip", EngineKind::Ftp),
            ("ftps://example.test/file.zip", EngineKind::Ftps),
            ("sftp://example.test/file.zip", EngineKind::Sftp),
        ] {
            assert_eq!(EngineKind::from_source_url(source), expected, "{source}");
        }
        Ok(())
    }

    #[test]
    fn source_url_validation_accepts_supported_protocols_only() {
        assert!(validate_source_url("ftp://example.test/file.zip").is_ok());
        assert!(validate_source_url("ftps://example.test/file.zip").is_ok());
        assert!(validate_source_url("sftp://example.test/file.zip").is_ok());
        assert!(validate_source_url("sftp://user:password@example.test/file.zip").is_err());
        assert!(validate_source_url("file:///etc/passwd").is_err());
        assert!(!engine_matches_source(
            EngineKind::Ftp,
            &reqwest::Url::parse("https://example.test/file.zip").expect("valid URL")
        ));
    }

    #[test]
    fn filename_uses_only_a_safe_url_path_component() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            filename_for_url(&reqwest::Url::parse("https://example.test/a%20file.zip")?),
            "a file.zip"
        );
        assert_eq!(
            filename_for_url(&reqwest::Url::parse(
                "https://example.test/Idiocracy%20%282006%29%201080p%20BRrip.mkv"
            )?),
            "Idiocracy (2006) 1080p BRrip.mkv"
        );
        assert_eq!(
            filename_for_url(&reqwest::Url::parse("https://example.test/%2Fsecret.zip")?),
            "_secret.zip"
        );
        assert_eq!(
            filename_for_url(&reqwest::Url::parse("https://example.test/")?),
            "download"
        );
        Ok(())
    }

    #[test]
    fn selected_media_extension_is_safe_for_a_destination_name() {
        assert!(extension_is_safe(Some("webm")));
        assert!(extension_is_safe(Some("m4a")));
        assert!(!extension_is_safe(None));
        assert!(!extension_is_safe(Some("../mp4")));
        assert!(!extension_is_safe(Some(".mp4")));
        assert!(!extension_is_safe(Some("x".repeat(11).as_str())));
    }

    #[tokio::test]
    async fn add_and_list_download_use_the_queue_and_repository() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/file.zip".to_owned(),
                ..Default::default()
            })
            .await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;
        let listed = state.list().await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, download.id);
        assert_eq!(listed[0].status, DownloadStatus::Downloading);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn pause_resume_cancel_commands_route_through_queue() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/control.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;

        let paused = state.pause(DownloadId::parse(&download.id)?).await?;
        assert_eq!(paused.status, DownloadStatus::Paused);
        let resumed = state.resume(DownloadId::parse(&download.id)?).await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;
        assert!(matches!(
            resumed.status,
            DownloadStatus::Queued | DownloadStatus::Preparing | DownloadStatus::Downloading
        ));
        let cancelled = state.cancel(DownloadId::parse(&download.id)?).await?;
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn delete_routes_through_queue_and_unknown_ids_are_mapped() -> Result<(), Box<dyn Error>>
    {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/delete.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;
        assert_eq!(state.delete(id).await, Ok(()));
        assert!(state.repository.get(id).await?.is_none());
        assert_eq!(
            state.delete(id).await.err().map(|error| error.code),
            Some(IpcErrorCode::NotFound)
        );
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn ipc_errors_and_event_payloads_have_stable_serialization() -> Result<(), Box<dyn Error>> {
        let error = super::IpcError {
            code: IpcErrorCode::NotFound,
            message: "The download was not found.",
        };
        assert_eq!(
            serde_json::to_string(&error)?,
            r#"{"code":"not_found","message":"The download was not found."}"#
        );
        let event = crate::events::DownloadRemovedEvent {
            download_id: "00000000-0000-4000-8000-000000000000".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&event)?,
            r#"{"download_id":"00000000-0000-4000-8000-000000000000"}"#
        );
        let updated = crate::events::DownloadUpdatedEvent {
            download: DownloadDto {
                id: "00000000-0000-4000-8000-000000000000".to_owned(),
                url: "https://example.test/file".to_owned(),
                filename: "file".to_owned(),
                destination: "downloads/file".to_owned(),
                status: DownloadStatus::Queued,
                priority: fetchx_models::DownloadPriority::Normal,
                engine: fetchx_models::EngineKind::DirectHttp,
                progress: super::DownloadProgressDto {
                    downloaded_bytes: 0,
                    total_bytes: None,
                    speed_bytes_per_second: None,
                    estimated_remaining_seconds: None,
                    percentage_basis_points: None,
                },
                error: None,
                created_at: 1,
                updated_at: 1,
                queue_position: None,
            },
        };
        let serialized = serde_json::to_string(&updated)?;
        assert!(serialized.contains(r#""download":{"id":"00000000-0000-4000-8000-000000000000""#));
        assert!(serialized.contains(r#""status":"queued""#));
        Ok(())
    }

    #[test]
    fn new_request_types_reject_unknown_fields() -> Result<(), serde_json::Error> {
        // SetPriorityRequest must not accept extra fields.
        assert!(
            serde_json::from_str::<super::SetPriorityRequest>(
                r#"{"priority":"high","extra":true}"#
            )
            .is_err()
        );
        // ScheduleDownloadRequest must not accept extra fields.
        assert!(
            serde_json::from_str::<super::ScheduleDownloadRequest>(
                r#"{"scheduled_at_ms":1000000,"extra":true}"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn schedule_dto_serializes_stably() -> Result<(), serde_json::Error> {
        let dto = super::ScheduleDto {
            download_id: "00000000-0000-4000-8000-000000000000".to_owned(),
            scheduled_at_ms: 1_725_000_000_000,
            enabled: true,
        };
        let serialized = serde_json::to_string(&dto)?;
        assert!(serialized.contains(r#""scheduled_at_ms":1725000000000"#));
        assert!(serialized.contains(r#""enabled":true"#));
        Ok(())
    }

    #[tokio::test]
    async fn set_priority_changes_persisted_value() -> Result<(), Box<dyn Error>> {
        use fetchx_models::DownloadPriority;

        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/priority.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        assert_eq!(download.priority, DownloadPriority::Normal);

        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;

        let updated = state.set_priority(id, DownloadPriority::High).await?;
        assert_eq!(updated.priority, DownloadPriority::High);
        assert_eq!(updated.id, download.id);

        // Confirm it is persisted.
        let listed = state.list().await?;
        assert_eq!(listed[0].priority, DownloadPriority::High);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn set_priority_on_missing_download_returns_not_found() -> Result<(), Box<dyn Error>> {
        use fetchx_models::DownloadPriority;

        let state = test_state().await?;
        let result = state
            .set_priority(DownloadId::new(), DownloadPriority::High)
            .await;
        assert_eq!(result.err().map(|e| e.code), Some(IpcErrorCode::NotFound));
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn schedule_and_remove_schedule_round_trip() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/scheduled.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        // The download must be in Queued or Paused state for scheduling.
        // Pause it immediately so the engine cannot start it first.
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;
        let paused = state.pause(id).await?;
        assert_eq!(paused.status, DownloadStatus::Paused);

        // Schedule one minute into the future so the download stays deferred.
        let future_ms = fetchx_models::Timestamp::now()?.as_unix_millis() + 60_000;
        state.schedule(id, future_ms).await?;

        let schedule = state.get_schedule(id).await?;
        assert!(schedule.is_some());
        let schedule = schedule.unwrap();
        assert_eq!(schedule.scheduled_at_ms, future_ms);
        assert!(schedule.enabled);

        // Remove the schedule — the download is released back to the queue.
        state.remove_schedule(id).await?;
        let after_remove = state.get_schedule(id).await?;
        assert!(after_remove.is_none());

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn schedule_on_missing_download_returns_not_found() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let future_ms = fetchx_models::Timestamp::now()?.as_unix_millis() + 60_000;
        let result = state.schedule(DownloadId::new(), future_ms).await;
        assert_eq!(result.err().map(|e| e.code), Some(IpcErrorCode::NotFound));
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn get_schedule_returns_none_for_unscheduled_download() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/nosched.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        let schedule = state.get_schedule(id).await?;
        assert!(schedule.is_none());
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn snapshot_reflects_queue_state() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        // Before adding anything, counts should be zero.
        let snap = state.snapshot().await?;
        assert_eq!(snap.queued_count, 0);
        assert_eq!(snap.active_count, 0);

        // Add a download — it starts, so active count becomes non-zero.
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/snap.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        wait_for_status(
            &state,
            DownloadId::parse(&download.id)?,
            DownloadStatus::Downloading,
        )
        .await?;

        let snap = state.snapshot().await?;
        assert_eq!(snap.active_count, 1);
        assert!(snap.total_connections > 0);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_failed_reuses_existing_download() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/retry.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;

        // Cancel it so we have a terminal download.
        let cancelled = state.cancel(id).await?;
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);

        // Retry should retain the record so the engine can reuse partial data.
        let retried = state.retry(id).await?;
        assert!(
            matches!(
                retried.status,
                DownloadStatus::Queued | DownloadStatus::Preparing | DownloadStatus::Downloading
            ),
            "unexpected retry status: {:?}",
            retried.status
        );
        assert_eq!(retried.id, download.id, "retry must retain the existing ID");
        assert!(state.repository.get(id).await?.is_some());

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_on_active_download_returns_conflict() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                engine_kind: None,
                url: "https://example.test/activeretry.bin".to_owned(),
                ..Default::default()
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;

        // Cannot retry a download that's still active.
        let result = state.retry(id).await;
        assert_eq!(result.err().map(|e| e.code), Some(IpcErrorCode::Conflict));

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn queue_snapshot_dto_serializes_stably() -> Result<(), serde_json::Error> {
        let dto = super::QueueSnapshotDto {
            active_count: 2,
            queued_count: 5,
            paused_count: 1,
            used_connections: 8,
            total_connections: 16,
            dispatch_paused: true,
            manual_dispatch_paused: true,
            schedule_dispatch_paused: false,
        };
        let s = serde_json::to_string(&dto)?;
        assert!(s.contains(r#""active_count":2"#));
        assert!(s.contains(r#""total_connections":16"#));
        assert!(s.contains(r#""dispatch_paused":true"#));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Phase 16C engine-selection tests
    // -----------------------------------------------------------------------

    #[test]
    fn add_request_accepts_no_engine_kind() -> Result<(), serde_json::Error> {
        // engine_kind is optional with a default of None.
        let req: AddDownloadRequest =
            serde_json::from_str(r#"{"url":"https://example.test/file.zip"}"#)?;
        assert_eq!(req.engine_kind, None);
        Ok(())
    }

    #[test]
    fn add_request_accepts_explicit_direct_http() -> Result<(), serde_json::Error> {
        let req: AddDownloadRequest = serde_json::from_str(
            r#"{"url":"https://example.test/file.zip","engine_kind":"direct_http"}"#,
        )?;
        assert_eq!(req.engine_kind, Some(fetchx_models::EngineKind::DirectHttp));
        Ok(())
    }

    #[test]
    fn add_request_accepts_aria2_engine_kind() -> Result<(), serde_json::Error> {
        let req: AddDownloadRequest = serde_json::from_str(
            r#"{"url":"https://example.test/file.zip","engine_kind":"aria2"}"#,
        )?;
        assert_eq!(req.engine_kind, Some(fetchx_models::EngineKind::Aria2));
        Ok(())
    }

    #[test]
    fn add_request_rejects_unknown_engine_kind() -> Result<(), serde_json::Error> {
        // An unrecognised value must fail deserialization.
        let result: Result<AddDownloadRequest, _> = serde_json::from_str(
            r#"{"url":"https://example.test/file.zip","engine_kind":"unknown_engine"}"#,
        );
        assert!(
            result.is_err(),
            "unknown engine_kind value must be rejected"
        );
        Ok(())
    }

    #[tokio::test]
    async fn add_download_none_engine_uses_direct_http() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                url: "https://example.test/engine-default.zip".to_owned(),
                engine_kind: None,
                ..Default::default()
            })
            .await?;
        assert_eq!(download.engine, fetchx_models::EngineKind::DirectHttp);
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_download_explicit_direct_http_is_accepted() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let download = state
            .add(AddDownloadRequest {
                url: "https://example.test/explicit-http.zip".to_owned(),
                engine_kind: Some(fetchx_models::EngineKind::DirectHttp),
                ..Default::default()
            })
            .await?;
        assert_eq!(download.engine, fetchx_models::EngineKind::DirectHttp);
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_download_rejects_removed_aria2_engine() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        let result = state
            .add(AddDownloadRequest {
                url: "https://example.test/aria2-attempt.zip".to_owned(),
                engine_kind: Some(fetchx_models::EngineKind::Aria2),
                ..Default::default()
            })
            .await;
        assert_eq!(
            result.err().map(|e| e.code),
            Some(IpcErrorCode::InvalidRequest),
            "removed aria2 must not be accepted for HTTP sources"
        );
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_download_unavailable_ytdlp_is_rejected() -> Result<(), Box<dyn Error>> {
        // Default EngineConfig has yt-dlp disabled → unavailable.
        let state = test_state().await?;
        let result = state
            .add(AddDownloadRequest {
                url: "https://example.test/ytdlp-attempt.zip".to_owned(),
                engine_kind: Some(fetchx_models::EngineKind::YtDlp),
                ..Default::default()
            })
            .await;
        assert_eq!(
            result.err().map(|e| e.code),
            Some(IpcErrorCode::Conflict),
            "disabled yt-dlp must be rejected with Conflict"
        );
        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retry_preserves_engine_kind_for_direct_http() -> Result<(), Box<dyn Error>> {
        let state = test_state().await?;
        // Add a DirectHttp download and cancel it.
        let download = state
            .add(AddDownloadRequest {
                url: "https://example.test/retry-engine.zip".to_owned(),
                engine_kind: None, // → DirectHttp
                ..Default::default()
            })
            .await?;
        let id = DownloadId::parse(&download.id)?;
        wait_for_status(&state, id, DownloadStatus::Downloading).await?;
        let cancelled = state.cancel(id).await?;
        assert_eq!(cancelled.status, DownloadStatus::Cancelled);

        // Retry must preserve the engine and existing download ID.
        let retried = state.retry(id).await?;
        assert_eq!(
            retried.engine,
            fetchx_models::EngineKind::DirectHttp,
            "retry must preserve DirectHttp engine assignment"
        );
        assert_eq!(retried.id, download.id);

        state.scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn engine_unavailable_error_contains_no_secrets() {
        for kind in [
            fetchx_models::EngineKind::Aria2,
            fetchx_models::EngineKind::YtDlp,
            fetchx_models::EngineKind::DirectHttp,
            fetchx_models::EngineKind::NativeTorrent,
        ] {
            let err = IpcError::engine_unavailable(kind);
            let json = serde_json::to_string(&err).unwrap();
            assert!(!json.contains("secret"), "error must not leak secrets");
            assert!(!json.contains("token"), "error must not leak tokens");
            assert!(!json.contains("rpc"), "error must not expose RPC details");
            assert_eq!(err.code, IpcErrorCode::Conflict);
        }
    }
}

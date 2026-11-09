//! Native BitTorrent engine backed by librqbit.
//!
//! This engine handles magnet URIs and `.torrent` files on platforms where
//! aria2c cannot be bundled (primarily Android). It integrates into the
//! existing `EngineDispatcher` as `EngineKind::NativeTorrent` and drives
//! downloads through the same `DownloadStateMachine` used by every other
//! engine.
//!
//! The engine holds a single `librqbit::Session` (one per `NativeTorrentEngine`
//! instance) and maps librqbit's progress API onto FetchX progress and status
//! transitions.

use std::{net::SocketAddr, num::NonZeroU32, sync::Arc, time::Duration};

use fetchx_models::{
    DownloadId, DownloadItem, DownloadStatus, EngineKind, Timestamp, TorrentMetadata,
    parse_torrent_metainfo,
};
use fetchx_storage::{
    DownloadRepository, SqliteTorrentMetadataRepository, TorrentMetadataRepository,
    TorrentMetadataRow, TorrentSourceType,
};
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, ListenerOptions, ManagedTorrent, Session, SessionOptions,
    TorrentStatsState,
};
use tokio::sync::{Mutex, watch};
use tracing::{debug, info, warn};

use crate::{DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine};

// How often to poll librqbit for progress updates.
const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Engine-level configuration for the native torrent engine.
#[derive(Clone, Debug)]
pub struct NativeTorrentConfig {
    /// TCP listen port for incoming BitTorrent connections.
    pub listen_port: u16,
    /// Maximum number of connected peers per torrent. Caps socket usage.
    pub max_connections_per_torrent: usize,
    /// Directory for librqbit session working files. Must be writable.
    /// Defaults to `std::env::temp_dir()/fetchx-librqbit-session` but
    /// on Android this must be set to the app's private files directory
    /// (e.g. `context.filesDir`) because `/data/local/tmp` is not writable
    /// by app sandboxes.
    pub session_dir: Option<std::path::PathBuf>,
    /// Disables DHT discovery. Useful for isolated environments and deterministic peers.
    pub disable_dht: bool,
}

impl Default for NativeTorrentConfig {
    fn default() -> Self {
        Self {
            listen_port: 6881,
            max_connections_per_torrent: 20,
            session_dir: None,
            disable_dht: false,
        }
    }
}

/// A thin adapter over `librqbit::Session` that implements the FetchX engine
/// traits. One session is shared across all active native torrent downloads.
pub struct NativeTorrentEngine<R> {
    session: Arc<Session>,
    repository: R,
    torrent_metadata: Option<SqliteTorrentMetadataRepository>,
    /// Retains handles for active transfers and completed torrents that are
    /// still seeding, so controls can reach the librqbit session.
    handles: Mutex<std::collections::HashMap<DownloadId, Arc<ManagedTorrent>>>,
}

impl<R> NativeTorrentEngine<R>
where
    R: DownloadRepository + Clone,
{
    /// Creates the engine and starts the underlying librqbit session.
    ///
    /// This performs no network I/O until [`ManagedDownloadEngine::download_managed`]
    /// is called. DHT bootstrapping is lazy.
    pub async fn new(
        repository: R,
        config: NativeTorrentConfig,
    ) -> Result<Self, DownloadEngineError> {
        // Use a temporary directory as the session's output folder — each
        // torrent supplies its own output folder via `AddTorrentOptions`, so
        // this is never used for actual writes.
        // Use the configured session dir, or fall back to temp_dir.
        // On Android, std::env::temp_dir() is /data/local/tmp which is NOT
        // writable by the app sandbox — callers must set session_dir explicitly.
        let placeholder_dir = config
            .session_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir)
            .join("fetchx-librqbit-session");
        tokio::fs::create_dir_all(&placeholder_dir)
            .await
            .map_err(DownloadEngineError::Io)?;

        let listen_addr: std::net::SocketAddr = format!("0.0.0.0:{}", config.listen_port)
            .parse()
            .map_err(|_| {
            DownloadEngineError::NativeTorrent(format!(
                "invalid listen port: {}",
                config.listen_port
            ))
        })?;

        let session_opts = SessionOptions {
            listen: Some(ListenerOptions {
                listen_addr,
                ..ListenerOptions::default()
            }),
            // Cap peers per torrent to stay within Android's fd sandbox limit.
            peer_limit: Some(config.max_connections_per_torrent),
            // Disable disk-backed session persistence — FetchX manages state.
            persistence: None,
            // Ephemeral DHT (no disk writes).
            dht: (!config.disable_dht).then_some(librqbit::DhtSessionConfig {
                persistence: None,
                ..Default::default()
            }),
            disable_local_service_discovery: true,
            ..SessionOptions::default()
        };

        let session = Session::new_with_opts(placeholder_dir, session_opts)
            .await
            .map_err(|e| DownloadEngineError::NativeTorrent(e.to_string()))?;

        info!("NativeTorrentEngine: librqbit session started");

        Ok(Self {
            session,
            repository,
            torrent_metadata: None,
            handles: Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn with_torrent_metadata_repository(
        mut self,
        repository: SqliteTorrentMetadataRepository,
    ) -> Self {
        self.torrent_metadata = Some(repository);
        self
    }

    /// Acquires and validates a magnet's metainfo without downloading payload files.
    ///
    /// The temporary torrent uses an empty file selection and is removed after
    /// metadata is extracted. Callers must persist a confirmed selection before
    /// admitting the corresponding download to the shared queue.
    pub async fn acquire_magnet_metadata(
        &self,
        magnet_uri: &str,
    ) -> Result<TorrentMetadata, DownloadEngineError> {
        self.acquire_magnet_metadata_with_options(magnet_uri, None, None)
            .await
    }

    async fn acquire_magnet_metadata_with_options(
        &self,
        magnet_uri: &str,
        initial_peers: Option<Vec<SocketAddr>>,
        control: Option<&ManagedDownloadControl>,
    ) -> Result<TorrentMetadata, DownloadEngineError> {
        let magnet = fetchx_models::parse_magnet_uri(magnet_uri).map_err(|error| {
            DownloadEngineError::NativeTorrent(format!("invalid magnet URI: {error}"))
        })?;
        let add_torrent = self.session.add_torrent(
            AddTorrent::from_url(magnet_uri),
            Some(AddTorrentOptions {
                only_files: Some(Vec::new()),
                initial_peers,
                ..AddTorrentOptions::default()
            }),
        );
        let add_result = tokio::time::timeout(Duration::from_secs(120), async {
            if let Some(control) = control {
                let mut cancellation = control.cancellation.clone();
                let mut pause = control.pause.clone();
                tokio::select! {
                    biased;
                    () = wait_for_signal(&mut cancellation) => Err(DownloadEngineError::Cancelled),
                    () = wait_for_signal(&mut pause) => Err(DownloadEngineError::Paused),
                    result = add_torrent => result
                        .map_err(|error| DownloadEngineError::NativeTorrent(error.to_string())),
                }
            } else {
                add_torrent
                    .await
                    .map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))
            }
        })
        .await
        .map_err(|_| {
            DownloadEngineError::NativeTorrent(
                "timed out while acquiring magnet metadata".to_owned(),
            )
        })?;
        let response = add_result?;
        let handle = response.into_handle().ok_or_else(|| {
            DownloadEngineError::NativeTorrent(
                "magnet metadata acquisition returned no managed torrent".to_owned(),
            )
        })?;

        let acquisition = tokio::time::timeout(Duration::from_secs(120), async {
            let initialized = if let Some(control) = control {
                let mut cancellation = control.cancellation.clone();
                let mut pause = control.pause.clone();
                tokio::select! {
                    result = handle.wait_until_initialized() => {
                        result.map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))?;
                    }
                    () = wait_for_signal(&mut cancellation) => {
                        return Err(DownloadEngineError::Cancelled);
                    }
                    () = wait_for_signal(&mut pause) => {
                        return Err(DownloadEngineError::Paused);
                    }
                }
                Ok(())
            } else {
                handle
                    .wait_until_initialized()
                    .await
                    .map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))
            };
            initialized?;

            let metadata = handle
                .with_metadata(|metadata| parse_torrent_info_bytes(&metadata.info_bytes))
                .map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))?
                .map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))?;
            if !metadata.info_hash.eq_ignore_ascii_case(&magnet.info_hash) {
                return Err(DownloadEngineError::NativeTorrent(
                    "acquired metainfo does not match the magnet info hash".to_owned(),
                ));
            }
            Ok(metadata)
        })
        .await
        .map_err(|_| {
            DownloadEngineError::NativeTorrent(
                "timed out while acquiring magnet metadata".to_owned(),
            )
        })
        .and_then(|result| result);

        self.finish_magnet_metadata_acquisition(&magnet.info_hash, &handle, acquisition)
            .await
    }

    async fn finish_magnet_metadata_acquisition(
        &self,
        info_hash: &str,
        handle: &ManagedTorrent,
        acquisition: Result<TorrentMetadata, DownloadEngineError>,
    ) -> Result<TorrentMetadata, DownloadEngineError> {
        let cleanup = self
            .session
            .delete(librqbit::api::TorrentIdOrHash::Id(handle.id()), false)
            .await
            .map_err(|error| {
                warn!(
                    %info_hash,
                    %error,
                    "NativeTorrentEngine: failed to remove metadata-only torrent"
                );
                DownloadEngineError::NativeTorrent(format!(
                    "could not clean up metadata-only torrent: {error}"
                ))
            });
        match (acquisition, cleanup) {
            (Ok(metadata), Ok(())) => Ok(metadata),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(error), Err(cleanup_error)) => Err(DownloadEngineError::NativeTorrent(format!(
                "{error}; additionally, {cleanup_error}"
            ))),
        }
    }

    async fn wait_for_magnet_selection(
        &self,
        id: DownloadId,
        control: &mut ManagedDownloadControl,
    ) -> Result<TorrentMetadataRow, DownloadEngineError> {
        let Some(repository) = self.torrent_metadata.as_ref() else {
            return Err(DownloadEngineError::NativeTorrent(
                "torrent metadata repository is unavailable".to_owned(),
            ));
        };

        loop {
            if *control.cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() {
                return Err(DownloadEngineError::Paused);
            }

            let metadata = repository
                .get_torrent_metadata(id)
                .await
                .map_err(DownloadEngineError::Storage)?
                .ok_or_else(|| {
                    DownloadEngineError::NativeTorrent(format!(
                        "torrent metadata for download {id} was not found"
                    ))
                })?;

            if metadata.metadata_state == "ready"
                && metadata.torrent_metadata.is_some()
                && metadata.selection_confirmed
                && !metadata.selected_file_indexes.is_empty()
            {
                return Ok(metadata);
            }

            tokio::select! {
                biased;
                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                changed = control.pause.changed() => {
                    if changed.is_err() || *control.pause.borrow_and_update() {
                        return Err(DownloadEngineError::Paused);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
        }
    }

    async fn ensure_magnet_metadata_ready(
        &self,
        id: DownloadId,
        magnet_uri: &str,
        control: &mut ManagedDownloadControl,
        initial_peers: Option<Vec<SocketAddr>>,
    ) -> Result<TorrentMetadataRow, DownloadEngineError> {
        let Some(repository) = self.torrent_metadata.as_ref() else {
            return Err(DownloadEngineError::NativeTorrent(
                "torrent metadata repository is unavailable".to_owned(),
            ));
        };

        let mut metadata = repository
            .get_torrent_metadata(id)
            .await
            .map_err(DownloadEngineError::Storage)?
            .ok_or_else(|| {
                DownloadEngineError::NativeTorrent(format!(
                    "torrent metadata for download {id} was not found"
                ))
            })?;

        if metadata.metadata_state == "ready" && metadata.torrent_metadata.is_some() {
            return self.wait_for_magnet_selection(id, control).await;
        }

        let acquired = self
            .acquire_magnet_metadata_with_options(magnet_uri, initial_peers, Some(control))
            .await?;
        metadata.torrent_name = acquired.name.clone();
        metadata.torrent_metadata = Some(acquired);
        metadata.metadata_state = "ready".to_owned();
        metadata.updated_at = Timestamp::now()?.max(metadata.updated_at);
        repository
            .update_torrent_metadata(&metadata)
            .await
            .map_err(DownloadEngineError::Storage)?;

        self.wait_for_magnet_selection(id, control).await
    }

    /// Stops an in-session torrent without deleting its downloaded files.
    ///
    /// A missing handle means the torrent is not being seeded by this session,
    /// which is expected after an app restart because session persistence is disabled.
    pub async fn stop_seeding(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        self.remove_managed_torrent(id).await
    }

    /// Returns the underlying librqbit session for shutdown / telemetry.
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// Queries live torrent statistics for the given download.
    ///
    /// Returns `None` when the torrent is not currently managed by this engine.
    pub async fn torrent_telemetry(
        &self,
        id: DownloadId,
    ) -> Option<fetchx_models::TorrentTelemetry> {
        let handles = self.handles.lock().await;
        let handle: &Arc<ManagedTorrent> = handles.get(&id)?;
        let stats = handle.stats();
        let live = stats.live.as_ref();
        // Convert librqbit's Speed (megabits/s) to bytes/s using as_bytes().
        let down_bps = live.map(|l| l.download_speed.as_bytes());
        let up_bps = live.map(|l| l.upload_speed.as_bytes());
        let peer_count = live.map(|l| u64::from(l.snapshot.peer_stats.live));
        Some(fetchx_models::TorrentTelemetry {
            download_speed_bps: down_bps,
            upload_speed_bps: up_bps,
            downloaded_bytes: Some(stats.progress_bytes),
            uploaded_bytes: Some(stats.uploaded_bytes),
            peer_count,
            completed_peer_count: None,
            seed_count: None,
            share_ratio: None,
            seeding: None,
            tracker_uris: None,
        })
    }

    /// Polls librqbit progress on a 500 ms interval and mirrors it into the
    /// FetchX repository. Returns `Ok(())` when the torrent finishes, or an
    /// error on cancellation or librqbit failure.
    async fn drive_to_completion(
        &self,
        id: DownloadId,
        handle: &Arc<ManagedTorrent>,
        control: ManagedDownloadControl,
    ) -> Result<(), DownloadEngineError> {
        let mut ticker = tokio::time::interval(PROGRESS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut cancel_watch = control.cancellation.clone();

        loop {
            let stats = handle.stats();
            if stats.finished {
                self.persist_progress(id, &stats).await?;
                info!(download_id = %id, "NativeTorrentEngine: torrent download complete");
                return Ok(());
            }

            tokio::select! {
                _ = ticker.tick() => {}
                _ = cancel_watch.changed() => {
                    if *cancel_watch.borrow() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
            }

            if *control.pause.borrow() {
                return Err(DownloadEngineError::Paused);
            }

            self.persist_progress(id, &stats).await?;

            // Check librqbit error state.
            if matches!(stats.state, TorrentStatsState::Error) {
                let msg = stats
                    .error
                    .unwrap_or_else(|| "librqbit reported error state".to_owned());
                return Err(DownloadEngineError::NativeTorrent(msg));
            }
        }
    }

    async fn persist_progress(
        &self,
        id: DownloadId,
        stats: &librqbit::TorrentStats,
    ) -> Result<(), DownloadEngineError> {
        if stats.total_bytes == 0 {
            return Ok(());
        }
        let Some(mut item) = self.repository.get(id).await? else {
            return Err(DownloadEngineError::NotFound(id));
        };
        if item.status() != DownloadStatus::Downloading {
            return Ok(());
        }

        let expected_updated_at = item.updated_at();
        let updated_at = fetchx_models::Timestamp::now()?.max(expected_updated_at);
        let progress = fetchx_models::DownloadProgress::new(
            stats.progress_bytes,
            Some(stats.total_bytes),
            stats
                .live
                .as_ref()
                .map(|live| live.download_speed.as_bytes()),
            None,
        )?;
        item.set_progress_at(progress, updated_at)?;
        self.repository
            .update_if_status_and_timestamp_current(
                &item,
                DownloadStatus::Downloading,
                expected_updated_at,
            )
            .await?;
        Ok(())
    }

    async fn remove_managed_torrent(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        let handle = self.handles.lock().await.get(&id).cloned();
        if let Some(handle) = handle {
            if let Err(error) = self
                .session
                .delete(librqbit::api::TorrentIdOrHash::Id(handle.id()), false)
                .await
            {
                if let Err(pause_error) = self.session.pause(&handle).await {
                    warn!(
                        download_id = %id,
                        error = %pause_error,
                        "NativeTorrentEngine: torrent cleanup fallback pause failed"
                    );
                }
                return Err(DownloadEngineError::NativeTorrent(error.to_string()));
            }
            self.handles.lock().await.remove(&id);
        }
        Ok(())
    }

    async fn run_torrent(
        &self,
        id: DownloadId,
        item: DownloadItem,
        control: ManagedDownloadControl,
        initial_peers: Option<Vec<SocketAddr>>,
    ) -> Result<(), DownloadEngineError> {
        let source_url = item.source().url().to_owned();
        let destination = item.destination().to_path_buf();
        let is_magnet = source_url.starts_with("magnet:");
        let state_machine = fetchx_core::DownloadStateMachine::new(self.repository.clone());
        let mut control = control;

        state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        if is_magnet {
            state_machine
                .transition(id, DownloadStatus::MetadataAcquiring)
                .await?;
        }

        let (add_source, local_torrent_metadata, magnet_info_hash) = if is_magnet {
            let magnet = fetchx_models::parse_magnet_uri(&source_url).map_err(|error| {
                DownloadEngineError::NativeTorrent(format!("invalid magnet URI: {error}"))
            })?;
            (
                AddTorrent::from_url(source_url.as_str()),
                None,
                Some(magnet.info_hash),
            )
        } else if source_url.starts_with("torrent+file:") {
            let file_path = item
                .source()
                .torrent_file_path()
                .ok_or(DownloadEngineError::InvalidUrl)?;
            let torrent_bytes = tokio::fs::read(file_path)
                .await
                .map_err(DownloadEngineError::Io)?;
            let metadata = parse_torrent_metainfo(&torrent_bytes).map_err(|error| {
                DownloadEngineError::NativeTorrent(format!(
                    "could not validate local torrent metadata: {error}"
                ))
            })?;
            (AddTorrent::from_bytes(torrent_bytes), Some(metadata), None)
        } else {
            return Err(DownloadEngineError::UnsupportedScheme);
        };

        let rate_bps: Option<NonZeroU32> = control
            .bandwidth
            .per_download
            .as_ref()
            .map(|limit| limit.bytes_per_second())
            .and_then(|bps| NonZeroU32::new(bps.min(u64::from(u32::MAX)) as u32));
        let persisted_metadata = if is_magnet {
            self.ensure_magnet_metadata_ready(id, &source_url, &mut control, initial_peers.clone())
                .await?;
            match &self.torrent_metadata {
                Some(repository) => repository
                    .get_torrent_metadata(id)
                    .await
                    .map_err(DownloadEngineError::Storage)?,
                None => None,
            }
        } else {
            match &self.torrent_metadata {
                Some(repository) => repository
                    .get_torrent_metadata(id)
                    .await
                    .map_err(DownloadEngineError::Storage)?,
                None => None,
            }
        };
        let selected_files = selected_files_from_metadata(
            persisted_metadata,
            local_torrent_metadata.as_ref(),
            magnet_info_hash.as_deref(),
        )?;
        let add_opts = AddTorrentOptions {
            overwrite: true,
            output_folder: Some(destination.to_string_lossy().into_owned()),
            only_files: selected_files,
            initial_peers,
            ratelimits: LimitsConfig {
                download_bps: rate_bps,
                upload_bps: None,
            },
            peer_limit: Some(*control.connection_limit.borrow()),
            ..AddTorrentOptions::default()
        };

        let mut cancellation = control.cancellation.clone();
        let mut pause = control.pause.clone();
        let response = tokio::select! {
            response = self.session.add_torrent(add_source, Some(add_opts)) => {
                response.map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))?
            }
            () = wait_for_signal(&mut cancellation) => {
                return Err(DownloadEngineError::Cancelled);
            }
            () = wait_for_signal(&mut pause) => {
                return Err(DownloadEngineError::Paused);
            }
        };

        let handle: Arc<ManagedTorrent> = response.into_handle().ok_or_else(|| {
            DownloadEngineError::NativeTorrent(
                "torrent add returned a list-only response unexpectedly".to_owned(),
            )
        })?;
        self.handles.lock().await.insert(id, Arc::clone(&handle));

        if is_magnet {
            debug!(download_id = %id, "NativeTorrentEngine: awaiting magnet metadata");
            let mut cancellation = control.cancellation.clone();
            let mut pause = control.pause.clone();
            tokio::select! {
                result = handle.wait_until_initialized() => {
                    result.map_err(|error| DownloadEngineError::NativeTorrent(error.to_string()))?;
                }
                () = wait_for_signal(&mut cancellation) => {
                    return Err(DownloadEngineError::Cancelled);
                }
                () = wait_for_signal(&mut pause) => {
                    return Err(DownloadEngineError::Paused);
                }
            }
            debug!(download_id = %id, "NativeTorrentEngine: magnet metadata acquired");
        }

        state_machine
            .transition(id, DownloadStatus::Downloading)
            .await?;
        self.drive_to_completion(id, &handle, control).await?;
        state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        state_machine
            .transition(id, DownloadStatus::Seeding)
            .await?;
        Ok(())
    }
}

impl<R> DownloadEngine for NativeTorrentEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: tokio::sync::watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let pause = tokio::sync::watch::channel(false).1;
        let active_count = self
            .handles
            .lock()
            .await
            .values()
            .filter(|handle| !handle.stats().finished)
            .count();
        let connection_limit = tokio::sync::watch::channel(active_count).1;
        let control = ManagedDownloadControl {
            cancellation,
            pause,
            connection_limit,
            resource_demand: None,
            bandwidth: crate::BandwidthControl::default(),
        };
        self.download_managed(id, control).await
    }
}

impl<R> ManagedDownloadEngine for NativeTorrentEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        self.download_managed_with_peers(id, control, None).await
    }

    fn supports_active_pause(&self, _engine: EngineKind) -> bool {
        true
    }

    #[allow(clippy::manual_async_fn)]
    fn cancel_inactive(
        &self,
        id: DownloadId,
    ) -> impl std::future::Future<Output = Result<(), DownloadEngineError>> + Send {
        async move { self.remove_managed_torrent(id).await }
    }
}

impl<R> NativeTorrentEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed_with_peers(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
        initial_peers: Option<Vec<SocketAddr>>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;

        if item.source().engine() != EngineKind::NativeTorrent {
            return Err(DownloadEngineError::EngineMismatch);
        }

        match self.run_torrent(id, item, control, initial_peers).await {
            Ok(()) => self
                .repository
                .get(id)
                .await?
                .ok_or(DownloadEngineError::NotFound(id)),
            Err(error @ (DownloadEngineError::Cancelled | DownloadEngineError::Paused)) => {
                self.remove_managed_torrent(id).await?;
                let status = if error.is_cancelled() {
                    DownloadStatus::Cancelled
                } else {
                    DownloadStatus::Paused
                };
                let state_machine = fetchx_core::DownloadStateMachine::new(self.repository.clone());
                let current = self
                    .repository
                    .get(id)
                    .await?
                    .ok_or(DownloadEngineError::NotFound(id))?;
                if current.status() != status {
                    state_machine.transition(id, status).await?;
                }
                Err(error)
            }
            Err(error) => {
                self.remove_managed_torrent(id).await?;
                let state_machine = fetchx_core::DownloadStateMachine::new(self.repository.clone());
                let failure = error.persisted_error()?;
                if let Err(state) = state_machine
                    .transition_with_error(id, DownloadStatus::Failed, failure)
                    .await
                {
                    return Err(DownloadEngineError::FailurePersistence {
                        operation: Box::new(error),
                        state,
                    });
                }
                Err(error)
            }
        }
    }
}

fn selected_files_from_metadata(
    metadata: Option<TorrentMetadataRow>,
    local_torrent_metadata: Option<&TorrentMetadata>,
    magnet_info_hash: Option<&str>,
) -> Result<Option<Vec<usize>>, DownloadEngineError> {
    let Some(metadata) = metadata else {
        if magnet_info_hash.is_some() {
            return Err(DownloadEngineError::NativeTorrent(
                "magnet file selection is not confirmed".to_owned(),
            ));
        }
        return Ok(None);
    };

    if !metadata.selection_confirmed {
        return Err(DownloadEngineError::NativeTorrent(
            "torrent file selection is not confirmed".to_owned(),
        ));
    }
    if metadata.selected_file_indexes.is_empty() {
        return Err(DownloadEngineError::NativeTorrent(
            "confirmed torrent file selection contains no files".to_owned(),
        ));
    }
    let file_count = if let Some(magnet_info_hash) = magnet_info_hash {
        if metadata.source_type != TorrentSourceType::Magnet {
            return Err(DownloadEngineError::NativeTorrent(
                "persisted torrent source does not match the magnet".to_owned(),
            ));
        }
        let stored = metadata.torrent_metadata.as_ref().ok_or_else(|| {
            DownloadEngineError::NativeTorrent("magnet metadata is unavailable".to_owned())
        })?;
        if !metadata.info_hash.eq_ignore_ascii_case(magnet_info_hash)
            || !stored.info_hash.eq_ignore_ascii_case(magnet_info_hash)
        {
            return Err(DownloadEngineError::NativeTorrent(
                "magnet metadata changed after its selection was saved".to_owned(),
            ));
        }
        stored.files.len()
    } else {
        if metadata.source_type != TorrentSourceType::TorrentFile {
            return Err(DownloadEngineError::NativeTorrent(
                "persisted torrent source does not match the local torrent file".to_owned(),
            ));
        }
        let local_torrent_metadata = local_torrent_metadata.ok_or_else(|| {
            DownloadEngineError::NativeTorrent("local torrent metadata is unavailable".to_owned())
        })?;
        if !metadata
            .info_hash
            .eq_ignore_ascii_case(&local_torrent_metadata.info_hash)
            || metadata.torrent_metadata.as_ref().is_some_and(|stored| {
                !stored
                    .info_hash
                    .eq_ignore_ascii_case(&local_torrent_metadata.info_hash)
            })
        {
            return Err(DownloadEngineError::NativeTorrent(
                "local torrent file changed after its selection was saved".to_owned(),
            ));
        }
        local_torrent_metadata.files.len()
    };

    let selected_files = metadata
        .selected_file_indexes
        .into_iter()
        .map(usize::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DownloadEngineError::NativeTorrent(format!(
                "selected torrent file index is unsupported on this platform: {error}"
            ))
        })?;
    if let Some(index) = selected_files.iter().find(|index| **index >= file_count) {
        return Err(DownloadEngineError::NativeTorrent(format!(
            "selected torrent file index {index} is out of range"
        )));
    }
    Ok(Some(selected_files))
}

fn parse_torrent_info_bytes(
    info_bytes: &[u8],
) -> Result<TorrentMetadata, fetchx_models::TorrentMetadataError> {
    let mut metainfo = Vec::with_capacity(info_bytes.len() + 8);
    metainfo.extend_from_slice(b"d4:info");
    metainfo.extend_from_slice(info_bytes);
    metainfo.push(b'e');
    parse_torrent_metainfo(&metainfo)
}

async fn wait_for_signal(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    //! Deterministic unit tests for the native torrent engine.
    //!
    //! Most tests use in-memory fixtures; the controlled peer test binds only
    //! loopback sockets and never contacts a public tracker or swarm.

    use super::{
        DownloadEngineError, NativeTorrentConfig, NativeTorrentEngine, selected_files_from_metadata,
    };
    use crate::ManagedDownloadControl;
    use fetchx_models::{
        DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, Timestamp,
        TorrentFile, TorrentMetadata, parse_magnet_uri,
    };
    use fetchx_storage::{
        DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
        TorrentSourceType,
    };
    use sha1::{Digest, Sha1};
    use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

    fn local_torrent_metadata() -> TorrentMetadata {
        TorrentMetadata {
            info_hash: "0123456789012345678901234567890123456789".to_owned(),
            name: "sample".to_owned(),
            total_size: 4,
            files: (0..4)
                .map(|index| TorrentFile {
                    path: vec![format!("file-{index}.bin")],
                    size: 1,
                })
                .collect(),
            trackers: Vec::new(),
            piece_length: 16_384,
            piece_count: 1,
        }
    }

    fn torrent_metadata_row(
        source_type: TorrentSourceType,
        selection_confirmed: bool,
        selected_file_indexes: Vec<u32>,
    ) -> TorrentMetadataRow {
        let now = fetchx_models::Timestamp::from_unix_millis(1);
        let torrent_metadata =
            (source_type == TorrentSourceType::TorrentFile).then(local_torrent_metadata);
        TorrentMetadataRow {
            download_id: fetchx_models::DownloadId::new(),
            source_type,
            magnet_uri: None,
            local_torrent_path: None,
            info_hash: "0123456789012345678901234567890123456789".to_owned(),
            torrent_name: "sample".to_owned(),
            torrent_metadata,
            destination: PathBuf::from("downloads"),
            metadata_state: "ready".to_owned(),
            selected_file_indexes,
            selected_total_bytes: 0,
            selection_confirmed,
            aria2_gid: None,
            seed_state: TorrentSeedState::Downloading,
            uploaded_bytes: 0,
            upload_speed_bps: 0,
            peer_count: 0,
            seed_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn confirmed_local_torrent_selection_is_passed_as_native_file_indexes() {
        let torrent_metadata = local_torrent_metadata();
        let selected = selected_files_from_metadata(
            Some(torrent_metadata_row(
                TorrentSourceType::TorrentFile,
                true,
                vec![1, 3],
            )),
            Some(&torrent_metadata),
            None,
        )
        .expect("valid persisted indexes should convert");

        assert_eq!(selected, Some(vec![1, 3]));
    }

    #[test]
    fn native_engine_requires_confirmed_torrent_selection() {
        assert!(matches!(
            selected_files_from_metadata(
                Some(torrent_metadata_row(
                    TorrentSourceType::TorrentFile,
                    false,
                    vec![1],
                )),
                Some(&local_torrent_metadata()),
                None
            ),
            Err(DownloadEngineError::NativeTorrent(_))
        ));
        let mut magnet = torrent_metadata_row(TorrentSourceType::Magnet, true, vec![1]);
        magnet.torrent_metadata = Some(local_torrent_metadata());
        assert!(matches!(
            selected_files_from_metadata(
                Some(magnet),
                None,
                Some(&local_torrent_metadata().info_hash)
            ),
            Ok(Some(files)) if files == vec![1]
        ));
        assert!(matches!(
            selected_files_from_metadata(None, Some(&local_torrent_metadata()), None),
            Ok(None)
        ));
    }

    #[test]
    fn native_engine_rejects_empty_stale_and_out_of_range_magnet_selection() {
        let info_hash = local_torrent_metadata().info_hash;
        let mut row = torrent_metadata_row(TorrentSourceType::Magnet, true, Vec::new());
        row.torrent_metadata = Some(local_torrent_metadata());
        assert!(matches!(
            selected_files_from_metadata(Some(row.clone()), None, Some(&info_hash)),
            Err(DownloadEngineError::NativeTorrent(_))
        ));

        row.selected_file_indexes = vec![4];
        assert!(matches!(
            selected_files_from_metadata(Some(row.clone()), None, Some(&info_hash)),
            Err(DownloadEngineError::NativeTorrent(_))
        ));

        row.selected_file_indexes = vec![1];
        row.info_hash = "abcdefabcdefabcdefabcdefabcdefabcdefabcd".to_owned();
        assert!(matches!(
            selected_files_from_metadata(Some(row), None, Some(&info_hash)),
            Err(DownloadEngineError::NativeTorrent(_))
        ));
    }

    #[tokio::test]
    async fn native_engine_waits_until_magnet_selection_is_confirmed()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let database = fetchx_storage::Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let id = DownloadId::new();
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=sample";
        let now = Timestamp::now()?;
        let item = DownloadItem::new(
            id,
            DownloadSource::new(magnet, EngineKind::NativeTorrent)?,
            fetchx_models::FileName::new("sample")?,
            directory.path().join("downloads"),
            now,
        );
        repository.create(&item).await?;

        let metadata = local_torrent_metadata();
        database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet.to_owned()),
                local_torrent_path: Some(directory.path().join("sample.torrent")),
                info_hash: metadata.info_hash.clone(),
                torrent_name: metadata.name.clone(),
                torrent_metadata: Some(metadata.clone()),
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

        let engine = NativeTorrentEngine::new(
            repository.clone(),
            NativeTorrentConfig {
                listen_port: 0,
                session_dir: Some(directory.path().join("librqbit-session")),
                ..NativeTorrentConfig::default()
            },
        )
        .await?
        .with_torrent_metadata_repository(database.torrent_metadata());

        let (selection_tx, selection_rx) = tokio::sync::oneshot::channel();
        tokio::spawn({
            let repository = database.torrent_metadata();
            async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let mut row = repository
                    .get_torrent_metadata(id)
                    .await?
                    .ok_or("magnet row missing")?;
                row.selected_file_indexes = vec![0];
                row.selected_total_bytes = metadata.files[0].size;
                row.selection_confirmed = true;
                row.updated_at = Timestamp::now()?.max(row.updated_at);
                repository.update_torrent_metadata(&row).await?;
                selection_tx
                    .send(())
                    .map_err(|_| "selection notification dropped")?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
            }
        });

        let (_cancel_tx, cancellation) = tokio::sync::watch::channel(false);
        let (_pause_tx, pause) = tokio::sync::watch::channel(false);
        let (_limit_tx, connection_limit) = tokio::sync::watch::channel(1usize);
        let mut control = ManagedDownloadControl {
            cancellation,
            pause,
            connection_limit,
            resource_demand: None,
            bandwidth: crate::BandwidthControl::default(),
        };
        let row = engine.wait_for_magnet_selection(id, &mut control).await?;
        assert!(row.selection_confirmed);
        assert_eq!(row.selected_file_indexes, vec![0]);
        let _ = selection_rx.await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn native_magnet_acquires_metadata_waits_for_selection_and_recovers_after_restart()
    -> Result<(), Box<dyn std::error::Error>> {
        use fetchx_storage::{DownloadRepository, TorrentMetadataRepository};
        use librqbit::{AddTorrent, AddTorrentOptions, ListenerOptions, Session, SessionOptions};
        use std::{net::SocketAddr, sync::Arc};

        let directory = tempfile::tempdir()?;
        let first_payload = vec![b'u'; 16_384];
        let selected_payload = vec![b's'; 4 * 1024 * 1024];
        let torrent_bytes = selected_files_torrent_bytes(&first_payload, &selected_payload);
        let metadata = fetchx_models::parse_torrent_metainfo(&torrent_bytes)?;

        let seed_output = directory.path().join("seed-output");
        tokio::fs::create_dir_all(&seed_output).await?;
        tokio::fs::write(seed_output.join("first.bin"), &first_payload).await?;
        tokio::fs::write(seed_output.join("selected.bin"), &selected_payload).await?;
        let seeder_session = Session::new_with_opts(
            directory.path().join("seed-session"),
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
                AddTorrent::from_bytes(torrent_bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(seed_output.to_string_lossy().into_owned()),
                    ..AddTorrentOptions::default()
                }),
            )
            .await?
            .into_handle()
            .ok_or("seeder add did not return a managed torrent")?;
        tokio::time::timeout(Duration::from_secs(10), seeder.wait_until_initialized()).await??;
        assert!(
            seeder.stats().finished,
            "local seeder payload did not verify"
        );
        let seed_address: SocketAddr = seeder_session
            .listen_addr()
            .ok_or("seeder did not bind a peer listener")?;

        let database = fetchx_storage::Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let torrent_metadata_repository = database.torrent_metadata();
        let id = DownloadId::new();
        let now = Timestamp::now()?;
        let magnet_uri = format!("magnet:?xt=urn:btih:{}&dn=bundle", metadata.info_hash);
        let destination = directory.path().join("magnet-download");
        let item = DownloadItem::new(
            id,
            DownloadSource::new(&magnet_uri, EngineKind::NativeTorrent)?,
            fetchx_models::FileName::new("bundle")?,
            destination.clone(),
            now,
        );
        repository.create(&item).await?;
        torrent_metadata_repository
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet_uri.clone()),
                local_torrent_path: Some(directory.path().join("magnet.torrent")),
                info_hash: metadata.info_hash.clone(),
                torrent_name: "bundle".to_owned(),
                torrent_metadata: None,
                destination: destination.clone(),
                metadata_state: "pending".to_owned(),
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

        let make_engine = || async {
            NativeTorrentEngine::new(
                repository.clone(),
                NativeTorrentConfig {
                    listen_port: 0,
                    session_dir: Some(directory.path().join("download-session")),
                    disable_dht: true,
                    ..NativeTorrentConfig::default()
                },
            )
            .await
            .map(|engine| {
                Arc::new(
                    engine.with_torrent_metadata_repository(torrent_metadata_repository.clone()),
                )
            })
        };
        let engine = make_engine().await?;
        let selector_repository = torrent_metadata_repository.clone();
        let selector_metadata = metadata.clone();
        let selector = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let mut row = selector_repository
                        .get_torrent_metadata(id)
                        .await
                        .expect("magnet metadata lookup should succeed")
                        .expect("magnet metadata row should exist");
                    if row.metadata_state == "ready" && row.torrent_metadata.is_some() {
                        assert_eq!(
                            row.torrent_metadata.as_ref().map(|value| &value.info_hash),
                            Some(&selector_metadata.info_hash)
                        );
                        row.selected_file_indexes = vec![1];
                        row.selected_total_bytes = selector_metadata.files[1].size as u64;
                        row.selection_confirmed = true;
                        row.updated_at = Timestamp::now()
                            .expect("selection timestamp should be available")
                            .max(row.updated_at);
                        selector_repository
                            .update_torrent_metadata(&row)
                            .await
                            .expect("confirmed magnet selection should persist");
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("native engine should acquire magnet metadata");
        });

        let (pause_tx, pause_rx) = tokio::sync::watch::channel(false);
        let (_cancel_tx, cancellation) = tokio::sync::watch::channel(false);
        let (_limit_tx, connection_limit) = tokio::sync::watch::channel(1usize);
        let control = ManagedDownloadControl {
            cancellation,
            pause: pause_rx,
            connection_limit,
            resource_demand: None,
            bandwidth: crate::BandwidthControl {
                per_download: Some(Arc::new(crate::BandwidthLimiter::new(1024 * 1024)?)),
                ..crate::BandwidthControl::default()
            },
        };
        let worker_engine = Arc::clone(&engine);
        let mut worker = tokio::spawn(async move {
            worker_engine
                .download_managed_with_peers(id, control, Some(vec![seed_address]))
                .await
        });
        let progress_wait = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let item = repository
                    .get(id)
                    .await?
                    .ok_or("magnet download should exist")?;
                if item.status() == DownloadStatus::Failed {
                    return Err(format!("native magnet failed: {:?}", item.error()).into());
                }
                if item.status() == DownloadStatus::Downloading
                    && item.progress().downloaded_bytes() > 0
                {
                    return Ok::<u64, Box<dyn std::error::Error>>(
                        item.progress().downloaded_bytes(),
                    );
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        let progress_result = match progress_wait {
            Ok(result) => result,
            Err(error) => return Err(Box::new(error) as Box<dyn std::error::Error>),
        };
        let partial_progress = progress_result?;
        selector.await?;
        pause_tx.send(true)?;
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), &mut worker).await??,
            Err(DownloadEngineError::Paused)
        ));
        assert!(partial_progress > 0 && partial_progress < selected_payload.len() as u64);
        assert_eq!(
            repository.get(id).await?.unwrap().status(),
            DownloadStatus::Paused
        );
        assert_eq!(
            engine.session().with_torrents(|torrents| torrents.count()),
            0
        );
        drop(engine);

        fetchx_core::DownloadStateMachine::new(repository.clone())
            .transition(id, DownloadStatus::Queued)
            .await?;
        let restarted_engine = make_engine().await?;
        let (_cancel_tx, cancellation) = tokio::sync::watch::channel(false);
        let (_pause_tx, pause) = tokio::sync::watch::channel(false);
        let (_limit_tx, connection_limit) = tokio::sync::watch::channel(1usize);
        let completed = tokio::time::timeout(
            Duration::from_secs(20),
            restarted_engine.download_managed_with_peers(
                id,
                ManagedDownloadControl {
                    cancellation,
                    pause,
                    connection_limit,
                    resource_demand: None,
                    bandwidth: crate::BandwidthControl::default(),
                },
                Some(vec![seed_address]),
            ),
        )
        .await??;
        assert_eq!(completed.status(), DownloadStatus::Seeding);
        assert!(completed.progress().downloaded_bytes() >= partial_progress);
        assert_eq!(
            tokio::fs::read(destination.join("selected.bin")).await?,
            selected_payload
        );
        match tokio::fs::read(destination.join("first.bin")).await {
            Ok(bytes) => assert!(
                bytes.is_empty(),
                "unselected magnet file must not contain payload bytes"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Box::new(error)),
        }

        let interrupted_id = DownloadId::new();
        let interrupted_at = Timestamp::now()?;
        let interrupted_destination = directory.path().join("interrupted-magnet");
        let interrupted_item = DownloadItem::new(
            interrupted_id,
            DownloadSource::new(&magnet_uri, EngineKind::NativeTorrent)?,
            fetchx_models::FileName::new("bundle")?,
            interrupted_destination.clone(),
            interrupted_at,
        );
        repository.create(&interrupted_item).await?;
        torrent_metadata_repository
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: interrupted_id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet_uri.clone()),
                local_torrent_path: Some(directory.path().join("interrupted-magnet.torrent")),
                info_hash: metadata.info_hash.clone(),
                torrent_name: metadata.name.clone(),
                torrent_metadata: Some(metadata.clone()),
                destination: interrupted_destination.clone(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: vec![1],
                selected_total_bytes: selected_payload.len() as u64,
                selection_confirmed: true,
                aria2_gid: None,
                seed_state: TorrentSeedState::Downloading,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at: interrupted_at,
                updated_at: interrupted_at,
            })
            .await?;
        let cancel_engine = Arc::new(
            NativeTorrentEngine::new(
                repository.clone(),
                NativeTorrentConfig {
                    listen_port: 0,
                    session_dir: Some(directory.path().join("cancel-session")),
                    disable_dht: true,
                    ..NativeTorrentConfig::default()
                },
            )
            .await?
            .with_torrent_metadata_repository(torrent_metadata_repository.clone()),
        );
        let (cancel_tx, cancellation) = tokio::sync::watch::channel(false);
        let (_pause_tx, pause) = tokio::sync::watch::channel(false);
        let (_limit_tx, connection_limit) = tokio::sync::watch::channel(1usize);
        let cancel_control = ManagedDownloadControl {
            cancellation,
            pause,
            connection_limit,
            resource_demand: None,
            bandwidth: crate::BandwidthControl {
                per_download: Some(Arc::new(crate::BandwidthLimiter::new(1024 * 1024)?)),
                ..crate::BandwidthControl::default()
            },
        };
        let cancel_worker_engine = Arc::clone(&cancel_engine);
        let mut cancel_worker = tokio::spawn(async move {
            cancel_worker_engine
                .download_managed_with_peers(
                    interrupted_id,
                    cancel_control,
                    Some(vec![seed_address]),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let item = repository
                    .get(interrupted_id)
                    .await?
                    .ok_or("interrupted magnet download should exist")?;
                if item.status() == DownloadStatus::Failed {
                    return Err(format!("native magnet failed: {:?}", item.error()).into());
                }
                if item.status() == DownloadStatus::Downloading
                    && item.progress().downloaded_bytes() > 0
                {
                    return Ok::<(), Box<dyn std::error::Error>>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await??;
        seeder_session
            .delete(librqbit::api::TorrentIdOrHash::Id(seeder.id()), false)
            .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel_tx.send(true)?;
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), &mut cancel_worker).await??,
            Err(DownloadEngineError::Cancelled)
        ));
        assert_eq!(
            repository
                .get(interrupted_id)
                .await?
                .ok_or("interrupted magnet row should remain")?
                .status(),
            DownloadStatus::Cancelled
        );
        assert_eq!(
            cancel_engine
                .session()
                .with_torrents(|torrents| torrents.count()),
            0
        );

        let retry_seeder = seeder_session
            .add_torrent(
                AddTorrent::from_bytes(torrent_bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(seed_output.to_string_lossy().into_owned()),
                    ..AddTorrentOptions::default()
                }),
            )
            .await?
            .into_handle()
            .ok_or("retry seeder add did not return a managed torrent")?;
        tokio::time::timeout(
            Duration::from_secs(10),
            retry_seeder.wait_until_initialized(),
        )
        .await??;
        assert!(retry_seeder.stats().finished);
        let retry_seed_address: SocketAddr = seeder_session
            .listen_addr()
            .ok_or("retry seeder did not bind a peer listener")?;

        let (_retry_cancel_tx, cancellation) = tokio::sync::watch::channel(false);
        let (_retry_pause_tx, pause) = tokio::sync::watch::channel(false);
        let (_retry_limit_tx, connection_limit) = tokio::sync::watch::channel(1usize);
        let retried = tokio::time::timeout(
            Duration::from_secs(20),
            cancel_engine.download_managed_with_peers(
                interrupted_id,
                ManagedDownloadControl {
                    cancellation,
                    pause,
                    connection_limit,
                    resource_demand: None,
                    bandwidth: crate::BandwidthControl::default(),
                },
                Some(vec![retry_seed_address]),
            ),
        )
        .await??;
        assert_eq!(retried.status(), DownloadStatus::Seeding);
        assert_eq!(
            tokio::fs::read(interrupted_destination.join("selected.bin")).await?,
            selected_payload
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn native_magnet_metadata_cancellation_removes_temporary_torrent()
    -> Result<(), Box<dyn std::error::Error>> {
        use fetchx_storage::Database;

        let directory = tempfile::tempdir()?;
        let database = Database::connect("sqlite::memory:").await?;
        let engine = NativeTorrentEngine::new(
            database.downloads(),
            NativeTorrentConfig {
                listen_port: 0,
                session_dir: Some(directory.path().join("session")),
                disable_dht: true,
                ..NativeTorrentConfig::default()
            },
        )
        .await?;
        let (cancellation_tx, cancellation) = tokio::sync::watch::channel(false);
        let (_pause_tx, pause) = tokio::sync::watch::channel(false);
        let (_limit_tx, connection_limit) = tokio::sync::watch::channel(1usize);
        let control = ManagedDownloadControl {
            cancellation,
            pause,
            connection_limit,
            resource_demand: None,
            bandwidth: crate::BandwidthControl::default(),
        };
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=sample";
        cancellation_tx.send(true)?;
        let result = engine
            .acquire_magnet_metadata_with_options(magnet, None, Some(&control))
            .await;
        assert!(
            matches!(result, Err(DownloadEngineError::Cancelled)),
            "unexpected metadata acquisition result: {result:?}"
        );
        assert_eq!(
            engine.session().with_torrents(|torrents| torrents.count()),
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn native_magnet_metadata_failure_cleans_up_acquired_handle()
    -> Result<(), Box<dyn std::error::Error>> {
        use fetchx_storage::Database;
        use librqbit::{AddTorrent, AddTorrentOptions};

        let directory = tempfile::tempdir()?;
        let database = Database::connect("sqlite::memory:").await?;
        let engine = NativeTorrentEngine::new(
            database.downloads(),
            NativeTorrentConfig {
                listen_port: 0,
                session_dir: Some(directory.path().join("session")),
                disable_dht: true,
                ..NativeTorrentConfig::default()
            },
        )
        .await?;
        let bytes = selected_files_torrent_bytes(&[b'a'; 16_384], &[b'b'; 16_384]);
        let metadata = fetchx_models::parse_torrent_metainfo(&bytes)?;
        let handle = engine
            .session()
            .add_torrent(
                AddTorrent::from_bytes(bytes),
                Some(AddTorrentOptions {
                    only_files: Some(Vec::new()),
                    ..AddTorrentOptions::default()
                }),
            )
            .await?
            .into_handle()
            .ok_or("metadata fixture did not return a managed torrent")?;
        tokio::time::timeout(Duration::from_secs(5), handle.wait_until_initialized()).await??;
        assert_eq!(
            engine.session().with_torrents(|torrents| torrents.count()),
            1
        );

        let result = engine
            .finish_magnet_metadata_acquisition(
                &metadata.info_hash,
                &handle,
                Err(DownloadEngineError::NativeTorrent(
                    "injected metadata parse failure".to_owned(),
                )),
            )
            .await;
        assert!(matches!(
            result,
            Err(DownloadEngineError::NativeTorrent(message))
                if message == "injected metadata parse failure"
        ));
        assert_eq!(
            engine.session().with_torrents(|torrents| torrents.count()),
            0
        );
        Ok(())
    }

    #[test]
    fn native_engine_rejects_empty_confirmed_local_torrent_selection() {
        assert!(matches!(
            selected_files_from_metadata(
                Some(torrent_metadata_row(
                    TorrentSourceType::TorrentFile,
                    true,
                    Vec::new(),
                )),
                Some(&local_torrent_metadata()),
                None
            ),
            Err(DownloadEngineError::NativeTorrent(_))
        ));
    }

    #[test]
    fn native_engine_rejects_stale_local_torrent_selections() {
        let metadata = local_torrent_metadata();
        assert!(matches!(
            selected_files_from_metadata(
                Some(torrent_metadata_row(
                    TorrentSourceType::TorrentFile,
                    true,
                    vec![4],
                )),
                Some(&metadata),
                None
            ),
            Err(DownloadEngineError::NativeTorrent(_))
        ));

        let mut changed_torrent = metadata;
        changed_torrent.info_hash = "abcdefabcdefabcdefabcdefabcdefabcdefabcd".to_owned();
        assert!(matches!(
            selected_files_from_metadata(
                Some(torrent_metadata_row(
                    TorrentSourceType::TorrentFile,
                    true,
                    vec![1],
                )),
                Some(&changed_torrent),
                None
            ),
            Err(DownloadEngineError::NativeTorrent(_))
        ));
    }

    fn bencode_string(value: &[u8], output: &mut Vec<u8>) {
        output.extend_from_slice(format!("{}:", value.len()).as_bytes());
        output.extend_from_slice(value);
    }

    fn selected_files_torrent_bytes(first_file: &[u8], selected_file: &[u8]) -> Vec<u8> {
        let mut bytes = b"d4:infod5:filesl".to_vec();
        for (name, payload) in [("first.bin", first_file), ("selected.bin", selected_file)] {
            bytes.extend_from_slice(b"d6:lengthi");
            bytes.extend_from_slice(payload.len().to_string().as_bytes());
            bytes.extend_from_slice(b"e4:pathl");
            bencode_string(name.as_bytes(), &mut bytes);
            bytes.extend_from_slice(b"ee");
        }
        bytes.extend_from_slice(b"e4:name6:bundle12:piece lengthi16384e6:pieces");
        let mut piece_hashes = Vec::new();
        for piece in first_file
            .chunks(16_384)
            .chain(selected_file.chunks(16_384))
        {
            piece_hashes.extend_from_slice(&Sha1::digest(piece));
        }
        bencode_string(&piece_hashes, &mut bytes);
        bytes.extend_from_slice(b"ee");
        bytes
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_only_selected_file_from_controlled_loopback_peer()
    -> Result<(), Box<dyn std::error::Error>> {
        use fetchx_storage::{Database, DownloadRepository, TorrentMetadataRepository};
        use librqbit::{AddTorrent, AddTorrentOptions, ListenerOptions, Session, SessionOptions};
        use tokio::sync::watch;

        let directory = tempfile::tempdir()?;
        let first_payload = vec![b'a'; 16_384];
        let selected_payload = vec![b's'; 16_384];
        let torrent_bytes = selected_files_torrent_bytes(&first_payload, &selected_payload);
        let metadata = fetchx_models::parse_torrent_metainfo(&torrent_bytes)?;
        assert_eq!(metadata.files.len(), 2);

        let seed_output = directory.path().join("seed-output");
        tokio::fs::create_dir_all(&seed_output).await?;
        tokio::fs::write(seed_output.join("first.bin"), &first_payload).await?;
        tokio::fs::write(seed_output.join("selected.bin"), &selected_payload).await?;
        let seeder_session = Session::new_with_opts(
            directory.path().join("seed-session"),
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
                AddTorrent::from_bytes(torrent_bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(seed_output.to_string_lossy().into_owned()),
                    ..AddTorrentOptions::default()
                }),
            )
            .await?
            .into_handle()
            .ok_or("seeder add did not return a managed torrent")?;
        tokio::time::timeout(Duration::from_secs(10), seeder.wait_until_initialized()).await??;
        assert!(
            seeder.stats().finished,
            "local seeder payload did not verify"
        );
        let seed_address = seeder_session
            .listen_addr()
            .ok_or("seeder did not bind a peer listener")?;

        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let torrent_path = directory.path().join("selected.torrent");
        tokio::fs::write(&torrent_path, &torrent_bytes).await?;
        let id = fetchx_models::DownloadId::new();
        let now = fetchx_models::Timestamp::now()?;
        let destination = directory.path().join("download");
        let item = fetchx_models::DownloadItem::new(
            id,
            fetchx_models::DownloadSource::new_torrent_file(
                &torrent_path,
                fetchx_models::EngineKind::NativeTorrent,
            )?,
            fetchx_models::FileName::new("bundle")?,
            destination.clone(),
            now,
        );
        repository.create(&item).await?;
        database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::TorrentFile,
                magnet_uri: None,
                local_torrent_path: Some(torrent_path),
                info_hash: metadata.info_hash.clone(),
                torrent_name: metadata.name.clone(),
                torrent_metadata: Some(metadata),
                destination: destination.clone(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: vec![1],
                selected_total_bytes: selected_payload.len() as u64,
                selection_confirmed: true,
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

        let engine = NativeTorrentEngine::new(
            repository.clone(),
            NativeTorrentConfig {
                listen_port: 0,
                session_dir: Some(directory.path().join("download-session")),
                ..NativeTorrentConfig::default()
            },
        )
        .await?
        .with_torrent_metadata_repository(database.torrent_metadata());
        let cancellation_rx = watch::channel(false).1;
        let pause_rx = watch::channel(false).1;
        let control = ManagedDownloadControl {
            cancellation: cancellation_rx,
            pause: pause_rx,
            connection_limit: watch::channel(1).1,
            resource_demand: None,
            bandwidth: crate::BandwidthControl::default(),
        };
        engine
            .run_torrent(id, item, control, Some(vec![seed_address]))
            .await?;

        let completed = repository
            .get(id)
            .await?
            .ok_or("native download disappeared after completion")?;
        assert_eq!(completed.status(), DownloadStatus::Seeding);
        assert_eq!(
            completed.progress().downloaded_bytes(),
            selected_payload.len() as u64
        );
        assert_eq!(
            completed.progress().total_bytes(),
            Some(selected_payload.len() as u64)
        );
        assert_eq!(
            tokio::fs::read(destination.join("selected.bin")).await?,
            selected_payload
        );
        let unselected_path = destination.join("first.bin");
        if tokio::fs::try_exists(&unselected_path).await? {
            assert_eq!(
                tokio::fs::metadata(unselected_path).await?.len(),
                0,
                "native selected-file download wrote bytes for the unselected torrent file"
            );
        }

        engine.stop_seeding(id).await?;
        seeder_session
            .delete(librqbit::api::TorrentIdOrHash::Id(seeder.id()), false)
            .await?;
        Ok(())
    }

    // ── Bencode test fixtures ─────────────────────────────────────────────
    //
    // A minimal valid single-file torrent encoded as bencode:
    //   d 4:info d
    //       6:length i5e
    //       4:name   9:hello.txt
    //       12:piece length i262144e
    //       6:pieces 20:<20 zero bytes>
    //   e e

    fn single_file_torrent_bytes() -> Vec<u8> {
        let pieces = vec![0u8; 20]; // one placeholder SHA-1 hash
        let mut buf = Vec::new();
        buf.extend_from_slice(b"d"); // outer dict
        buf.extend_from_slice(b"4:info"); // "info" key
        buf.extend_from_slice(b"d"); // info dict
        buf.extend_from_slice(b"6:lengthi5e");
        buf.extend_from_slice(b"4:name9:hello.txt");
        buf.extend_from_slice(b"12:piece lengthi262144e");
        buf.extend_from_slice(b"6:pieces20:");
        buf.extend_from_slice(&pieces);
        buf.extend_from_slice(b"e"); // end info dict
        buf.extend_from_slice(b"e"); // end outer dict
        buf
    }

    fn multi_file_torrent_bytes() -> Vec<u8> {
        let pieces = vec![0u8; 20];
        let mut buf = Vec::new();
        buf.extend_from_slice(b"d4:infod");
        buf.extend_from_slice(b"5:filesld");
        buf.extend_from_slice(b"6:lengthi3e");
        buf.extend_from_slice(b"4:pathl3:dir5:a.txte");
        buf.extend_from_slice(b"ed6:lengthi7e4:pathl3:dir5:b.txtee");
        buf.extend_from_slice(b"e"); // end files list
        buf.extend_from_slice(b"4:name8:test.dir");
        buf.extend_from_slice(b"12:piece lengthi262144e");
        buf.extend_from_slice(b"6:pieces20:");
        buf.extend_from_slice(&pieces);
        buf.extend_from_slice(b"e"); // end info dict
        buf.extend_from_slice(b"e"); // end outer dict
        buf
    }

    // ── Test 1 ─────────────────────────────────────────────────────────────

    #[test]
    fn parses_valid_single_file_torrent_and_extracts_metadata() {
        let bytes = single_file_torrent_bytes();
        let result = fetchx_models::parse_torrent_metainfo(&bytes);
        assert!(
            result.is_ok(),
            "single-file torrent bencode parsing failed: {result:?}"
        );
        let meta = result.unwrap();
        assert_eq!(meta.name, "hello.txt");
        assert_eq!(meta.total_size, 5);
        assert_eq!(meta.files.len(), 1);
        assert_eq!(meta.files[0].size, 5);
    }

    // ── Test 2 ─────────────────────────────────────────────────────────────

    #[test]
    fn parses_valid_multi_file_torrent_and_lists_files() {
        let bytes = multi_file_torrent_bytes();
        let result = fetchx_models::parse_torrent_metainfo(&bytes);
        assert!(
            result.is_ok(),
            "multi-file torrent bencode parsing failed: {result:?}"
        );
        let meta = result.unwrap();
        assert_eq!(meta.name, "test.dir");
        assert_eq!(meta.files.len(), 2);
        assert_eq!(meta.total_size, 10);
    }

    // ── Test 3 ─────────────────────────────────────────────────────────────

    #[test]
    fn rejects_malformed_bencode() {
        let malformed: &[u8] = b"d4:infothis_is_not_bencode_garbage!!!";
        let result = fetchx_models::parse_torrent_metainfo(malformed);
        assert!(
            result.is_err(),
            "malformed bencode should have been rejected"
        );
    }

    // ── Test 4 ─────────────────────────────────────────────────────────────

    #[test]
    fn rejects_path_traversal_in_torrent_files() {
        // Torrent with a ".." path component — should be rejected.
        let pieces = vec![0u8; 20];
        let mut buf = Vec::new();
        buf.extend_from_slice(b"d4:infod");
        buf.extend_from_slice(b"5:filesld");
        buf.extend_from_slice(b"6:lengthi3e4:pathl2:..8:evil.txte");
        buf.extend_from_slice(b"e"); // end file entry
        buf.extend_from_slice(b"e"); // end files list
        buf.extend_from_slice(b"4:name8:safe.dir");
        buf.extend_from_slice(b"12:piece lengthi262144e");
        buf.extend_from_slice(b"6:pieces20:");
        buf.extend_from_slice(&pieces);
        buf.extend_from_slice(b"e"); // end info dict
        buf.extend_from_slice(b"e"); // end outer dict

        let result = fetchx_models::parse_torrent_metainfo(&buf);
        match result {
            Err(_) => {} // correctly rejected at parse time
            Ok(meta) => {
                // If the parser accepted it, no file path must expose "..".
                for file in &meta.files {
                    for component in &file.path {
                        assert_ne!(
                            component, "..",
                            "path traversal component '..' found in parsed torrent files"
                        );
                    }
                }
            }
        }
    }

    // ── Test 5 ─────────────────────────────────────────────────────────────

    #[test]
    fn magnet_uri_parsed_correctly() {
        let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567\
                   &dn=Sample+File\
                   &tr=https%3A%2F%2Ftracker.example.com%2Fannounce";
        let info = parse_magnet_uri(uri).expect("valid magnet should parse");
        assert_eq!(info.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(info.display_name.as_deref(), Some("Sample File"));
        assert_eq!(info.trackers.len(), 1);
        assert_eq!(info.trackers[0], "https://tracker.example.com/announce");

        // Confirm DownloadSource accepts the same URI with NativeTorrent engine.
        let source = DownloadSource::new(uri, EngineKind::NativeTorrent);
        assert!(
            source.is_ok(),
            "NativeTorrent should accept a valid magnet URI"
        );
        assert!(source.unwrap().is_magnet());
    }

    // ── Test 6 ─────────────────────────────────────────────────────────────

    #[test]
    fn invalid_magnet_uri_rejected() {
        for bad_uri in [
            "magnet:?dn=no-hash-at-all",
            "magnet:?xt=urn:btih:tooshort",
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567\
             &tr=file:///etc/passwd",
        ] {
            let result = DownloadSource::new(bad_uri, EngineKind::NativeTorrent);
            assert!(
                result.is_err(),
                "expected rejection of bad magnet URI: {bad_uri}"
            );
        }
        // A non-magnet URL with a wrong engine is also rejected for magnet
        // path (the validator only fires on the magnet: scheme).
        // Confirm a valid non-torrent URL does NOT get a NativeTorrent source
        // for a plain HTTP URL — NativeTorrent only accepts magnet/torrent+file.
        // (plain URLs are accepted generically, so we test the magnet-specific guard only)
    }

    // ── Extra: config default ──────────────────────────────────────────────

    #[test]
    fn native_torrent_config_default_listen_port() {
        assert_eq!(NativeTorrentConfig::default().listen_port, 6881);
    }

    // ── Extra: EngineBandwidthLimits round-trip with new field ────────────

    #[test]
    fn engine_bandwidth_limits_round_trips_with_native_torrent_field()
    -> Result<(), serde_json::Error> {
        use fetchx_models::EngineBandwidthLimits;
        let limits = EngineBandwidthLimits {
            native_torrent_bytes_per_second: Some(1_000_000),
            aria2_bytes_per_second: Some(500_000),
            ..EngineBandwidthLimits::default()
        };
        let json = serde_json::to_string(&limits)?;
        let restored: EngineBandwidthLimits = serde_json::from_str(&json)?;
        assert_eq!(limits, restored);
        assert_eq!(restored.native_torrent_bytes_per_second, Some(1_000_000));
        Ok(())
    }
}

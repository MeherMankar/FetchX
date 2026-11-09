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

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use fetchx_models::{DownloadId, DownloadItem, DownloadStatus, EngineKind};
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
}

impl Default for NativeTorrentConfig {
    fn default() -> Self {
        Self {
            listen_port: 6881,
            max_connections_per_torrent: 20,
            session_dir: None,
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
            dht: Some(librqbit::DhtSessionConfig {
                persistence: None,
                ..Default::default()
            }),
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
    ) -> Result<(), DownloadEngineError> {
        let source_url = item.source().url().to_owned();
        let destination = item.destination().to_path_buf();
        let is_magnet = source_url.starts_with("magnet:");
        let state_machine = fetchx_core::DownloadStateMachine::new(self.repository.clone());

        state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        if is_magnet {
            state_machine
                .transition(id, DownloadStatus::MetadataAcquiring)
                .await?;
        }

        let add_source = if is_magnet {
            AddTorrent::from_url(source_url.as_str())
        } else if source_url.starts_with("torrent+file:") {
            let file_path = item
                .source()
                .torrent_file_path()
                .ok_or(DownloadEngineError::InvalidUrl)?;
            let torrent_bytes = tokio::fs::read(file_path)
                .await
                .map_err(DownloadEngineError::Io)?;
            AddTorrent::from_bytes(torrent_bytes)
        } else {
            return Err(DownloadEngineError::UnsupportedScheme);
        };

        let rate_bps: Option<NonZeroU32> = control
            .bandwidth
            .per_download
            .as_ref()
            .map(|limit| limit.bytes_per_second())
            .and_then(|bps| NonZeroU32::new(bps.min(u64::from(u32::MAX)) as u32));
        let selected_files = match &self.torrent_metadata {
            Some(repository) => selected_files_from_metadata(
                repository
                    .get_torrent_metadata(id)
                    .await
                    .map_err(DownloadEngineError::Storage)?,
            )?,
            None => None,
        };
        let add_opts = AddTorrentOptions {
            overwrite: true,
            output_folder: Some(destination.to_string_lossy().into_owned()),
            only_files: selected_files,
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
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;

        if item.source().engine() != EngineKind::NativeTorrent {
            return Err(DownloadEngineError::EngineMismatch);
        }

        match self.run_torrent(id, item, control).await {
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

fn selected_files_from_metadata(
    metadata: Option<TorrentMetadataRow>,
) -> Result<Option<Vec<usize>>, DownloadEngineError> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };

    if metadata.source_type != TorrentSourceType::TorrentFile {
        return Ok(None);
    }
    if !metadata.selection_confirmed {
        return Err(DownloadEngineError::NativeTorrent(
            "local torrent file selection is not confirmed".to_owned(),
        ));
    }
    if metadata.selected_file_indexes.is_empty() {
        return Err(DownloadEngineError::NativeTorrent(
            "confirmed local torrent selection contains no files".to_owned(),
        ));
    }

    metadata
        .selected_file_indexes
        .into_iter()
        .map(usize::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
        .map_err(|error| {
            DownloadEngineError::NativeTorrent(format!(
                "selected torrent file index is unsupported on this platform: {error}"
            ))
        })
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
    //! These tests do NOT open network connections. They verify parsing,
    //! security invariants, and engine wiring using in-memory fixtures.

    use super::{DownloadEngineError, NativeTorrentConfig, selected_files_from_metadata};
    use fetchx_models::{DownloadSource, EngineKind, parse_magnet_uri};
    use fetchx_storage::{TorrentMetadataRow, TorrentSeedState, TorrentSourceType};
    use std::path::PathBuf;

    fn torrent_metadata_row(
        source_type: TorrentSourceType,
        selection_confirmed: bool,
        selected_file_indexes: Vec<u32>,
    ) -> TorrentMetadataRow {
        let now = fetchx_models::Timestamp::from_unix_millis(1);
        TorrentMetadataRow {
            download_id: fetchx_models::DownloadId::new(),
            source_type,
            magnet_uri: None,
            local_torrent_path: None,
            info_hash: "0123456789012345678901234567890123456789".to_owned(),
            torrent_name: "sample".to_owned(),
            torrent_metadata: None,
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
        let selected = selected_files_from_metadata(Some(torrent_metadata_row(
            TorrentSourceType::TorrentFile,
            true,
            vec![1, 3],
        )))
        .expect("valid persisted indexes should convert");

        assert_eq!(selected, Some(vec![1, 3]));
    }

    #[test]
    fn native_engine_requires_confirmed_local_selection_and_ignores_magnet_selection() {
        assert!(matches!(
            selected_files_from_metadata(Some(torrent_metadata_row(
                TorrentSourceType::TorrentFile,
                false,
                vec![1],
            ))),
            Err(DownloadEngineError::NativeTorrent(_))
        ));
        assert!(matches!(
            selected_files_from_metadata(Some(torrent_metadata_row(
                TorrentSourceType::Magnet,
                true,
                vec![1],
            ))),
            Ok(None)
        ));
        assert!(matches!(selected_files_from_metadata(None), Ok(None)));
    }

    #[test]
    fn native_engine_rejects_empty_confirmed_local_torrent_selection() {
        assert!(matches!(
            selected_files_from_metadata(Some(torrent_metadata_row(
                TorrentSourceType::TorrentFile,
                true,
                Vec::new(),
            ))),
            Err(DownloadEngineError::NativeTorrent(_))
        ));
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

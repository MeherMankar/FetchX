use std::{future::Future, path::PathBuf};

use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSchedule, DownloadStatus, Timestamp,
};

use crate::StorageError;

/// Async operations for persisted download domain objects.
pub trait DownloadRepository: Send + Sync {
    /// Inserts a new item. A duplicate identifier is reported as a database error.
    fn create(&self, item: &DownloadItem) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Finds one item, returning `None` when the identifier does not exist.
    fn get(
        &self,
        id: DownloadId,
    ) -> impl Future<Output = Result<Option<DownloadItem>, StorageError>> + Send;

    /// Lists all items in persisted queue order, using identifiers to break ties.
    fn list(&self) -> impl Future<Output = Result<Vec<DownloadItem>, StorageError>> + Send;

    /// Moves an existing item to the end of the persistent FIFO order.
    fn move_to_queue_tail(
        &self,
        id: DownloadId,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Updates priority without replacing progress or lifecycle fields.
    fn set_priority(
        &self,
        id: DownloadId,
        priority: DownloadPriority,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Inserts or replaces one-shot scheduling metadata for an existing download.
    fn save_schedule(
        &self,
        schedule: &DownloadSchedule,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Retrieves one schedule by download identifier.
    fn get_schedule(
        &self,
        id: DownloadId,
    ) -> impl Future<Output = Result<Option<DownloadSchedule>, StorageError>> + Send;

    /// Lists persisted schedules in scheduled-time and identifier order.
    fn list_schedules(
        &self,
    ) -> impl Future<Output = Result<Vec<DownloadSchedule>, StorageError>> + Send;

    /// Removes a schedule and reports whether one existed.
    fn delete_schedule(
        &self,
        id: DownloadId,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Replaces the stored item with the same identifier, or returns `NotFound`.
    fn update(&self, item: &DownloadItem) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Atomically replaces an item only if its persisted status matches.
    ///
    /// Returns `false` if the row is missing or another operation changed its state.
    fn update_if_status_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Atomically replaces an item only if both its status and timestamp still match.
    ///
    /// Use this for same-status snapshots such as progress checkpoints, where a
    /// status-only guard would allow an older snapshot to overwrite newer data.
    fn update_if_status_and_timestamp_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
        expected_updated_at: Timestamp,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Deletes an item and returns whether a row was removed.
    fn delete(&self, id: DownloadId) -> impl Future<Output = Result<bool, StorageError>> + Send;
}

/// Durable torrent-specific metadata for aria2 magnet/.torrent downloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentMetadataRow {
    pub download_id: DownloadId,
    pub source_type: TorrentSourceType,
    pub magnet_uri: Option<String>,
    pub local_torrent_path: Option<PathBuf>,
    pub info_hash: String,
    pub torrent_name: String,
    pub destination: PathBuf,
    pub metadata_state: String,
    pub selected_file_indexes: Vec<u32>,
    pub selected_total_bytes: u64,
    pub aria2_gid: Option<String>,
    pub seed_state: TorrentSeedState,
    pub uploaded_bytes: u64,
    pub upload_speed_bps: u64,
    pub peer_count: u64,
    pub seed_count: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Source of a torrent download.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TorrentSourceType {
    Magnet,
    TorrentFile,
}

/// Seeding lifecycle of a torrent task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TorrentSeedState {
    Downloading,
    Seeding,
    Stopped,
}

/// Async operations for persisted torrent metadata.
pub trait TorrentMetadataRepository: Send + Sync {
    /// Inserts a new torrent metadata row. A duplicate identifier is a database error.
    fn create_torrent_metadata(
        &self,
        row: &TorrentMetadataRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Replaces the stored torrent metadata for the same download identifier.
    fn update_torrent_metadata(
        &self,
        row: &TorrentMetadataRow,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Finds one torrent metadata row by download identifier.
    fn get_torrent_metadata(
        &self,
        id: DownloadId,
    ) -> impl Future<Output = Result<Option<TorrentMetadataRow>, StorageError>> + Send;

    /// Finds one torrent metadata row by the persisted aria2 GID.
    fn get_torrent_metadata_by_gid(
        &self,
        gid: &str,
    ) -> impl Future<Output = Result<Option<TorrentMetadataRow>, StorageError>> + Send;

    /// Lists torrent metadata rows for downloads that may still need aria2 reconciliation.
    ///
    /// A row is active when its seed state is not `Stopped` and its download has not
    /// reached a terminal non-seeding state.
    fn list_active_torrent_downloads(
        &self,
    ) -> impl Future<Output = Result<Vec<TorrentMetadataRow>, StorageError>> + Send;

    /// Removes a torrent metadata row and reports whether one existed.
    fn delete_torrent_metadata(
        &self,
        id: DownloadId,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;
}

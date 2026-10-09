use std::future::Future;

use mydownloader_models::{
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

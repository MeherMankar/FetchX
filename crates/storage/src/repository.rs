use mydownloader_models::{DownloadId, DownloadItem, DownloadStatus, Timestamp};

use crate::StorageError;

/// Async operations for persisted download domain objects.
#[allow(async_fn_in_trait)]
pub trait DownloadRepository: Send + Sync {
    /// Inserts a new item. A duplicate identifier is reported as a database error.
    async fn create(&self, item: &DownloadItem) -> Result<(), StorageError>;

    /// Finds one item, returning `None` when the identifier does not exist.
    async fn get(&self, id: DownloadId) -> Result<Option<DownloadItem>, StorageError>;

    /// Lists all items in stable creation-time and identifier order.
    async fn list(&self) -> Result<Vec<DownloadItem>, StorageError>;

    /// Replaces the stored item with the same identifier, or returns `NotFound`.
    async fn update(&self, item: &DownloadItem) -> Result<(), StorageError>;

    /// Atomically replaces an item only if its persisted status matches.
    ///
    /// Returns `false` if the row is missing or another operation changed its state.
    async fn update_if_status_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
    ) -> Result<bool, StorageError>;

    /// Atomically replaces an item only if both its status and timestamp still match.
    ///
    /// Use this for same-status snapshots such as progress checkpoints, where a
    /// status-only guard would allow an older snapshot to overwrite newer data.
    async fn update_if_status_and_timestamp_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
        expected_updated_at: Timestamp,
    ) -> Result<bool, StorageError>;

    /// Deletes an item and returns whether a row was removed.
    async fn delete(&self, id: DownloadId) -> Result<bool, StorageError>;
}

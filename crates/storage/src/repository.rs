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

    /// Atomically changes only status and updated timestamp if the current status matches.
    ///
    /// Returns `false` if the row is missing or another state transition won the race.
    async fn update_status_if_current(
        &self,
        id: DownloadId,
        expected_status: DownloadStatus,
        new_status: DownloadStatus,
        updated_at: Timestamp,
    ) -> Result<bool, StorageError>;

    /// Deletes an item and returns whether a row was removed.
    async fn delete(&self, id: DownloadId) -> Result<bool, StorageError>;
}

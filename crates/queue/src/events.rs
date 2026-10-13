use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSchedule, DownloadStatus, Timestamp,
};
use fetchx_storage::{DownloadRepository, StorageError};
use tokio::sync::broadcast;

const EVENT_BUFFER_CAPACITY: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadEvent {
    Created { revision: u64, item: DownloadItem },
    Updated { revision: u64, item: DownloadItem },
    Removed { revision: u64, id: DownloadId },
}

impl DownloadEvent {
    #[must_use]
    pub const fn revision(&self) -> u64 {
        match self {
            Self::Created { revision, .. }
            | Self::Updated { revision, .. }
            | Self::Removed { revision, .. } => *revision,
        }
    }
}

#[derive(Clone)]
pub struct DownloadEventBus {
    sender: broadcast::Sender<DownloadEvent>,
    next_revision: Arc<AtomicU64>,
}

impl Default for DownloadEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadEventBus {
    #[must_use]
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_BUFFER_CAPACITY);
        Self {
            sender,
            next_revision: Arc::new(AtomicU64::new(1)),
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.sender.subscribe()
    }

    fn publish(&self, event: impl FnOnce(u64) -> DownloadEvent) {
        let revision = self.next_revision.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = self.sender.send(event(revision)) {
            tracing::warn!(%error, "download event had no active subscribers");
        }
    }
}

/// Emits bounded lifecycle and persisted progress events from repository writes.
///
/// Use the same wrapper for QueueManager and its managed engine so their committed
/// snapshots enter one ordered event stream.
#[derive(Clone)]
pub struct EventRepository<R> {
    inner: R,
    events: DownloadEventBus,
}

impl<R> EventRepository<R> {
    #[must_use]
    pub const fn new(inner: R, events: DownloadEventBus) -> Self {
        Self { inner, events }
    }

    #[must_use]
    pub const fn inner(&self) -> &R {
        &self.inner
    }

    #[must_use]
    pub const fn event_bus(&self) -> &DownloadEventBus {
        &self.events
    }

    fn publish_updated(&self, item: DownloadItem) {
        self.events
            .publish(|revision| DownloadEvent::Updated { revision, item });
    }
}

impl<R> DownloadRepository for EventRepository<R>
where
    R: DownloadRepository,
{
    async fn create(&self, item: &DownloadItem) -> Result<(), StorageError> {
        self.inner.create(item).await?;
        let item = item.clone();
        self.events
            .publish(|revision| DownloadEvent::Created { revision, item });
        Ok(())
    }

    async fn get(&self, id: DownloadId) -> Result<Option<DownloadItem>, StorageError> {
        self.inner.get(id).await
    }

    async fn list(&self) -> Result<Vec<DownloadItem>, StorageError> {
        self.inner.list().await
    }

    async fn move_to_queue_tail(&self, id: DownloadId) -> Result<(), StorageError> {
        self.inner.move_to_queue_tail(id).await
    }

    async fn set_priority(
        &self,
        id: DownloadId,
        priority: DownloadPriority,
    ) -> Result<bool, StorageError> {
        let updated = self.inner.set_priority(id, priority).await?;
        if updated {
            match self.inner.get(id).await {
                Ok(Some(item)) => self.publish_updated(item),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, download_id = %id, "could not read updated download for event")
                }
            }
        }
        Ok(updated)
    }

    async fn save_schedule(&self, schedule: &DownloadSchedule) -> Result<(), StorageError> {
        self.inner.save_schedule(schedule).await
    }

    async fn get_schedule(&self, id: DownloadId) -> Result<Option<DownloadSchedule>, StorageError> {
        self.inner.get_schedule(id).await
    }

    async fn list_schedules(&self) -> Result<Vec<DownloadSchedule>, StorageError> {
        self.inner.list_schedules().await
    }

    async fn delete_schedule(&self, id: DownloadId) -> Result<bool, StorageError> {
        self.inner.delete_schedule(id).await
    }

    async fn update(&self, item: &DownloadItem) -> Result<(), StorageError> {
        self.inner.update(item).await?;
        self.publish_updated(item.clone());
        Ok(())
    }

    async fn update_if_status_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
    ) -> Result<bool, StorageError> {
        let updated = self
            .inner
            .update_if_status_current(item, expected_status)
            .await?;
        if updated {
            self.publish_updated(item.clone());
        }
        Ok(updated)
    }

    async fn update_if_status_and_timestamp_current(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
        expected_updated_at: Timestamp,
    ) -> Result<bool, StorageError> {
        let updated = self
            .inner
            .update_if_status_and_timestamp_current(item, expected_status, expected_updated_at)
            .await?;
        if updated {
            self.publish_updated(item.clone());
        }
        Ok(updated)
    }

    async fn delete(&self, id: DownloadId) -> Result<bool, StorageError> {
        let deleted = self.inner.delete(id).await?;
        if deleted {
            self.events
                .publish(|revision| DownloadEvent::Removed { revision, id });
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fetchx_models::{
        DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use fetchx_storage::{Database, DownloadRepository};
    use tokio::sync::broadcast;

    use super::{DownloadEvent, DownloadEventBus, EventRepository};

    fn new_item() -> DownloadItem {
        DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file", EngineKind::DirectHttp).unwrap(),
            FileName::new("file.bin").unwrap(),
            PathBuf::from("downloads/file.bin"),
            Timestamp::from_unix_millis(1),
        )
    }

    async fn event_repo() -> (
        EventRepository<fetchx_storage::SqliteDownloadRepository>,
        broadcast::Receiver<DownloadEvent>,
    ) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let inner = db.downloads();
        let bus = DownloadEventBus::new();
        let rx = bus.subscribe();
        (EventRepository::new(inner, bus), rx)
    }

    #[tokio::test]
    async fn create_emits_created_event() {
        let (repo, mut rx) = event_repo().await;
        let item = new_item();
        repo.create(&item).await.unwrap();
        let event = rx.try_recv().expect("created event should be emitted");
        assert!(matches!(event, DownloadEvent::Created { .. }));
        if let DownloadEvent::Created { item: emitted, .. } = event {
            assert_eq!(emitted.id(), item.id());
            assert_eq!(emitted.status(), DownloadStatus::Queued);
        }
    }

    #[tokio::test]
    async fn update_emits_updated_event() {
        let (repo, mut rx) = event_repo().await;
        let item = new_item();
        repo.create(&item).await.unwrap();
        let _ = rx.try_recv(); // consume created
        repo.update(&item).await.unwrap();
        let event = rx.try_recv().expect("updated event should be emitted");
        assert!(matches!(event, DownloadEvent::Updated { .. }));
    }

    #[tokio::test]
    async fn delete_emits_removed_event() {
        let (repo, mut rx) = event_repo().await;
        let item = new_item();
        repo.create(&item).await.unwrap();
        let _ = rx.try_recv(); // consume created
        let removed = repo.delete(item.id()).await.unwrap();
        assert!(removed);
        let event = rx.try_recv().expect("removed event should be emitted");
        assert!(matches!(event, DownloadEvent::Removed { id, .. } if id == item.id()));
    }

    #[tokio::test]
    async fn revisions_are_strictly_monotone() {
        let (repo, mut rx) = event_repo().await;
        let item1 = new_item();
        let item2 = new_item();
        repo.create(&item1).await.unwrap();
        repo.create(&item2).await.unwrap();
        let e1 = rx.try_recv().unwrap();
        let e2 = rx.try_recv().unwrap();
        assert!(e1.revision() < e2.revision());
    }

    #[tokio::test]
    async fn no_event_when_delete_finds_nothing() {
        let (repo, mut rx) = event_repo().await;
        let removed = repo.delete(DownloadId::new()).await.unwrap();
        assert!(!removed);
        assert!(rx.try_recv().is_err(), "no event expected for missing id");
    }

    #[tokio::test]
    async fn bus_is_cheaply_cloneable_and_independent_subscribers_each_receive_events() {
        let bus = DownloadEventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let inner = db.downloads();
        let repo = EventRepository::new(inner, bus);
        let item = new_item();
        repo.create(&item).await.unwrap();
        assert!(matches!(rx1.try_recv(), Ok(DownloadEvent::Created { .. })));
        assert!(matches!(rx2.try_recv(), Ok(DownloadEvent::Created { .. })));
    }

    #[tokio::test]
    async fn bus_handles_no_subscribers_without_panic() {
        // Publish with zero active subscribers - should warn but not panic.
        let bus = DownloadEventBus::new();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let inner = db.downloads();
        let repo = EventRepository::new(inner, bus);
        let item = new_item();
        // Should not panic even with no receiver.
        repo.create(&item).await.unwrap();
    }
}


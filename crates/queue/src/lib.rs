#![forbid(unsafe_code)]
mod events;
mod manager;
pub use events::{DownloadEvent, DownloadEventBus, EventRepository};
pub use manager::{ActiveDownload, QueueConfig, QueueError, QueueManager, QueueSnapshot};

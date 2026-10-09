#![forbid(unsafe_code)]

mod manager;

pub use manager::{ActiveDownload, QueueConfig, QueueError, QueueManager, QueueSnapshot};

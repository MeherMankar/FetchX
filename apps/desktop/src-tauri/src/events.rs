use serde::Serialize;

use crate::commands::DownloadDto;

pub const DOWNLOAD_UPDATED: &str = "download_updated";
pub const DOWNLOAD_REMOVED: &str = "download_removed";

#[derive(Clone, Debug, Serialize)]
pub struct DownloadUpdatedEvent {
    pub download: DownloadDto,
}

#[derive(Clone, Debug, Serialize)]
pub struct DownloadRemovedEvent {
    pub download_id: String,
}

pub const QUEUE_STATS_UPDATED: &str = "queue_stats_updated";
pub const QUEUE_IDLE: &str = "queue_idle";
pub const CLIPBOARD_URLS_DETECTED: &str = "clipboard_urls_detected";

#[derive(Clone, Debug, serde::Serialize)]
pub struct QueueStatsEvent {
    pub speed_bps: u64,
    pub active_count: u32,
    pub queued_count: u32,
    pub estimated_completion_seconds: Option<u64>,
}

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

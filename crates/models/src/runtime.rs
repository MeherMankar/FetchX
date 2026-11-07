use serde::{Deserialize, Serialize};

use crate::{DownloadId, DownloadItem, DownloadProgress, DownloadStatus, Timestamp};

/// Lightweight, frequently changing state for download progress clients.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DownloadRuntimeStatus {
    id: DownloadId,
    status: DownloadStatus,
    progress: DownloadProgress,
    updated_at: Timestamp,
}

impl DownloadRuntimeStatus {
    #[must_use]
    pub const fn id(&self) -> &DownloadId {
        &self.id
    }

    #[must_use]
    pub const fn status(&self) -> DownloadStatus {
        self.status
    }

    #[must_use]
    pub const fn progress(&self) -> &DownloadProgress {
        &self.progress
    }

    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }
}

impl From<&DownloadItem> for DownloadRuntimeStatus {
    fn from(item: &DownloadItem) -> Self {
        Self {
            id: item.id().clone(),
            status: item.status(),
            progress: item.progress().clone(),
            updated_at: item.updated_at(),
        }
    }
}

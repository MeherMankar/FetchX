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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{DownloadPriority, DownloadSource, EngineKind, FileName};

    #[test]
    fn projection_preserves_every_lifecycle_status() -> Result<(), Box<dyn std::error::Error>> {
        let statuses = [
            DownloadStatus::Queued,
            DownloadStatus::Preparing,
            DownloadStatus::Downloading,
            DownloadStatus::Paused,
            DownloadStatus::Completed,
            DownloadStatus::Failed,
            DownloadStatus::Cancelled,
            DownloadStatus::MetadataAcquiring,
            DownloadStatus::Seeding,
            DownloadStatus::Recovering,
            DownloadStatus::Stopped,
        ];

        for expected_status in statuses {
            let created_at = Timestamp::from_unix_millis(10);
            let item = DownloadItem::from_parts(
                DownloadId::new(),
                DownloadSource::new("https://example.test/file.bin", EngineKind::DirectHttp)?,
                FileName::new("file.bin")?,
                PathBuf::from("file.bin"),
                expected_status,
                DownloadPriority::default(),
                DownloadProgress::default(),
                created_at,
                Timestamp::from_unix_millis(20),
                None,
            )?;
            let status = DownloadRuntimeStatus::from(&item);

            assert_eq!(status.status(), expected_status);
            assert_eq!(status.id(), &item.id());
            assert_eq!(status.progress(), item.progress());
            assert_eq!(status.updated_at(), item.updated_at());
        }

        Ok(())
    }
}

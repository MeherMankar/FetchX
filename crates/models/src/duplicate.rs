use serde::{Deserialize, Serialize};

use crate::{DownloadId, DownloadItem, DownloadStatus, Timestamp};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DuplicateDownloadMatch {
    id: DownloadId,
    filename: String,
    status: DownloadStatus,
    updated_at: Timestamp,
}

impl DuplicateDownloadMatch {
    #[must_use]
    pub const fn id(&self) -> &DownloadId {
        &self.id
    }

    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    #[must_use]
    pub const fn status(&self) -> DownloadStatus {
        self.status
    }

    #[must_use]
    pub const fn updated_at(&self) -> &Timestamp {
        &self.updated_at
    }
}

impl From<&DownloadItem> for DuplicateDownloadMatch {
    fn from(item: &DownloadItem) -> Self {
        Self {
            id: item.id().clone(),
            filename: item.filename().as_str().to_owned(),
            status: item.status(),
            updated_at: item.updated_at(),
        }
    }
}

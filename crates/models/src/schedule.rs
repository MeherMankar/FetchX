use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{DownloadId, Timestamp};

/// One-shot start time for a download, persisted independently from lifecycle state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DownloadSchedule {
    download_id: DownloadId,
    scheduled_at: Timestamp,
    enabled: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl DownloadSchedule {
    #[must_use]
    pub const fn new(
        download_id: DownloadId,
        scheduled_at: Timestamp,
        enabled: bool,
        created_at: Timestamp,
    ) -> Self {
        Self {
            download_id,
            scheduled_at,
            enabled,
            created_at,
            updated_at: created_at,
        }
    }

    pub fn from_parts(
        download_id: DownloadId,
        scheduled_at: Timestamp,
        enabled: bool,
        created_at: Timestamp,
        updated_at: Timestamp,
    ) -> Result<Self, DownloadScheduleError> {
        if updated_at < created_at {
            return Err(DownloadScheduleError::UpdatedBeforeCreated);
        }
        Ok(Self {
            download_id,
            scheduled_at,
            enabled,
            created_at,
            updated_at,
        })
    }

    #[must_use]
    pub const fn download_id(self) -> DownloadId {
        self.download_id
    }

    #[must_use]
    pub const fn scheduled_at(self) -> Timestamp {
        self.scheduled_at
    }

    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn created_at(self) -> Timestamp {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(self) -> Timestamp {
        self.updated_at
    }

    pub fn set_scheduled_at(
        &mut self,
        scheduled_at: Timestamp,
        updated_at: Timestamp,
    ) -> Result<(), DownloadScheduleError> {
        self.validate_update_time(updated_at)?;
        self.scheduled_at = scheduled_at;
        self.updated_at = updated_at;
        Ok(())
    }

    pub fn set_enabled(
        &mut self,
        enabled: bool,
        updated_at: Timestamp,
    ) -> Result<(), DownloadScheduleError> {
        self.validate_update_time(updated_at)?;
        self.enabled = enabled;
        self.updated_at = updated_at;
        Ok(())
    }

    fn validate_update_time(&self, updated_at: Timestamp) -> Result<(), DownloadScheduleError> {
        if updated_at < self.updated_at {
            return Err(DownloadScheduleError::UpdatedBeforePreviousUpdate);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DownloadScheduleError {
    #[error("schedule updated timestamp must not precede its creation timestamp")]
    UpdatedBeforeCreated,
    #[error("schedule updated timestamp must not move backwards")]
    UpdatedBeforePreviousUpdate,
}

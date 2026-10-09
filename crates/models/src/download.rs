use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{DownloadError, DownloadId, DownloadPriority, DownloadProgress, EngineKind, Timestamp};

/// Source locator and selected engine; it deliberately contains no credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "DownloadSourceRepr")]
pub struct DownloadSource {
    url: String,
    engine: EngineKind,
}

impl DownloadSource {
    pub fn new(url: impl Into<String>, engine: EngineKind) -> Result<Self, DownloadSourceError> {
        let url = url.into();
        if url.trim().is_empty() {
            return Err(DownloadSourceError::EmptyUrl);
        }

        Ok(Self { url, engine })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub const fn engine(&self) -> EngineKind {
        self.engine
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DownloadSourceError {
    #[error("download source URL must not be empty")]
    EmptyUrl,
}

#[derive(Deserialize)]
struct DownloadSourceRepr {
    url: String,
    engine: EngineKind,
}

impl TryFrom<DownloadSourceRepr> for DownloadSource {
    type Error = DownloadSourceError;

    fn try_from(value: DownloadSourceRepr) -> Result<Self, Self::Error> {
        Self::new(value.url, value.engine)
    }
}

/// Suggested filename as supplied by the user or source, kept OS-agnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FileName(String);

impl FileName {
    pub fn new(value: impl Into<String>) -> Result<Self, FileNameError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(FileNameError::Empty);
        }

        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FileName {
    type Error = FileNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for FileName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FileNameError {
    #[error("filename must not be empty")]
    Empty,
}

/// The central domain record for a download.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "DownloadItemRepr")]
pub struct DownloadItem {
    id: DownloadId,
    source: DownloadSource,
    filename: FileName,
    destination: PathBuf,
    status: DownloadStatus,
    priority: DownloadPriority,
    progress: DownloadProgress,
    created_at: Timestamp,
    updated_at: Timestamp,
    error: Option<DownloadError>,
}

impl DownloadItem {
    /// Creates a queued download with normal priority and zero progress.
    pub fn new(
        id: DownloadId,
        source: DownloadSource,
        filename: FileName,
        destination: PathBuf,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            source,
            filename,
            destination,
            status: DownloadStatus::Queued,
            priority: DownloadPriority::default(),
            progress: DownloadProgress::default(),
            created_at,
            updated_at: created_at,
            error: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        id: DownloadId,
        source: DownloadSource,
        filename: FileName,
        destination: PathBuf,
        status: DownloadStatus,
        priority: DownloadPriority,
        progress: DownloadProgress,
        created_at: Timestamp,
        updated_at: Timestamp,
        error: Option<DownloadError>,
    ) -> Result<Self, DownloadItemError> {
        if updated_at < created_at {
            return Err(DownloadItemError::UpdatedBeforeCreated);
        }

        Ok(Self {
            id,
            source,
            filename,
            destination,
            status,
            priority,
            progress,
            created_at,
            updated_at,
            error,
        })
    }

    #[must_use]
    pub const fn id(&self) -> DownloadId {
        self.id
    }

    #[must_use]
    pub fn source(&self) -> &DownloadSource {
        &self.source
    }

    #[must_use]
    pub fn filename(&self) -> &FileName {
        &self.filename
    }

    #[must_use]
    pub fn destination(&self) -> &std::path::Path {
        &self.destination
    }

    #[must_use]
    pub const fn status(&self) -> DownloadStatus {
        self.status
    }

    #[must_use]
    pub const fn priority(&self) -> DownloadPriority {
        self.priority
    }

    #[must_use]
    pub const fn progress(&self) -> &DownloadProgress {
        &self.progress
    }

    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    #[must_use]
    pub const fn error(&self) -> Option<&DownloadError> {
        self.error.as_ref()
    }

    /// Updates only the lifecycle state and modification timestamp.
    ///
    /// Transition legality is enforced by the application state machine.
    pub fn set_status_at(
        &mut self,
        status: DownloadStatus,
        updated_at: Timestamp,
    ) -> Result<(), DownloadItemError> {
        self.validate_update_time(updated_at)?;
        self.status = status;
        self.updated_at = updated_at;
        Ok(())
    }

    /// Replaces progress without changing lifecycle state.
    pub fn set_progress_at(
        &mut self,
        progress: DownloadProgress,
        updated_at: Timestamp,
    ) -> Result<(), DownloadItemError> {
        self.validate_update_time(updated_at)?;
        self.progress = progress;
        self.updated_at = updated_at;
        Ok(())
    }

    /// Replaces the optional diagnostic associated with the item.
    pub fn set_error_at(
        &mut self,
        error: Option<DownloadError>,
        updated_at: Timestamp,
    ) -> Result<(), DownloadItemError> {
        self.validate_update_time(updated_at)?;
        self.error = error;
        self.updated_at = updated_at;
        Ok(())
    }

    fn validate_update_time(&self, updated_at: Timestamp) -> Result<(), DownloadItemError> {
        if updated_at < self.updated_at {
            return Err(DownloadItemError::UpdatedBeforePreviousUpdate);
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DownloadItemError {
    #[error("download updated timestamp must not precede its creation timestamp")]
    UpdatedBeforeCreated,
    #[error("download updated timestamp must not move backwards")]
    UpdatedBeforePreviousUpdate,
}

#[derive(Deserialize)]
struct DownloadItemRepr {
    id: DownloadId,
    source: DownloadSource,
    filename: FileName,
    destination: PathBuf,
    status: DownloadStatus,
    priority: DownloadPriority,
    progress: DownloadProgress,
    created_at: Timestamp,
    updated_at: Timestamp,
    error: Option<DownloadError>,
}

impl TryFrom<DownloadItemRepr> for DownloadItem {
    type Error = DownloadItemError;

    fn try_from(value: DownloadItemRepr) -> Result<Self, Self::Error> {
        Self::from_parts(
            value.id,
            value.source,
            value.filename,
            value.destination,
            value.status,
            value.priority,
            value.progress,
            value.created_at,
            value.updated_at,
            value.error,
        )
    }
}

/// Lifecycle state of a download.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DownloadStatus {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "preparing")]
    Preparing,
    #[serde(rename = "downloading")]
    Downloading,
    #[serde(rename = "paused")]
    Paused,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DownloadItem, DownloadItemError, DownloadSource, DownloadStatus, FileName};
    use crate::{DownloadId, DownloadPriority, DownloadProgress, EngineKind, Timestamp};

    fn source() -> Result<DownloadSource, super::DownloadSourceError> {
        DownloadSource::new("https://example.com/file", EngineKind::DirectHttp)
    }

    #[test]
    fn status_variants_have_stable_names_and_round_trip() -> Result<(), serde_json::Error> {
        for (status, expected) in [
            (DownloadStatus::Queued, "\"queued\""),
            (DownloadStatus::Preparing, "\"preparing\""),
            (DownloadStatus::Downloading, "\"downloading\""),
            (DownloadStatus::Paused, "\"paused\""),
            (DownloadStatus::Completed, "\"completed\""),
            (DownloadStatus::Failed, "\"failed\""),
            (DownloadStatus::Cancelled, "\"cancelled\""),
        ] {
            let serialized = serde_json::to_string(&status)?;
            assert_eq!(serialized, expected);
            let deserialized: DownloadStatus = serde_json::from_str(&serialized)?;
            assert_eq!(status, deserialized);
        }
        Ok(())
    }

    #[test]
    fn source_requires_nonempty_url_and_deserializes_through_validation() {
        assert!(DownloadSource::new("  ", EngineKind::DirectHttp).is_err());
        assert!(
            serde_json::from_str::<DownloadSource>(r#"{"url":" ","engine":"direct_http"}"#)
                .is_err()
        );
    }

    #[test]
    fn filename_must_not_be_empty() {
        assert!(FileName::new(" ").is_err());
        assert!(serde_json::from_str::<FileName>(r#""""#).is_err());
    }

    #[test]
    fn item_defaults_and_serialization_are_stable() -> Result<(), Box<dyn std::error::Error>> {
        let timestamp = Timestamp::from_unix_millis(1_725_000_000_123);
        let item = DownloadItem::new(
            DownloadId::new(),
            source()?,
            FileName::new("archive.zip")?,
            PathBuf::from("downloads/archive.zip"),
            timestamp,
        );

        assert_eq!(item.status(), DownloadStatus::Queued);
        assert_eq!(item.priority(), DownloadPriority::Normal);
        assert_eq!(item.progress().downloaded_bytes(), 0);
        assert_eq!(item.updated_at(), timestamp);
        assert_eq!(item.error(), None);

        let serialized = serde_json::to_string(&item)?;
        let deserialized: DownloadItem = serde_json::from_str(&serialized)?;
        assert_eq!(item, deserialized);
        Ok(())
    }

    #[test]
    fn item_rejects_update_time_before_creation_time() -> Result<(), Box<dyn std::error::Error>> {
        let result = DownloadItem::from_parts(
            DownloadId::new(),
            source()?,
            FileName::new("archive.zip")?,
            PathBuf::from("archive.zip"),
            DownloadStatus::Queued,
            DownloadPriority::Normal,
            DownloadProgress::new(0, None, None, None)?,
            Timestamp::from_unix_millis(2),
            Timestamp::from_unix_millis(1),
            None,
        );

        assert_eq!(result, Err(DownloadItemError::UpdatedBeforeCreated));
        Ok(())
    }

    #[test]
    fn item_deserialization_enforces_timestamp_invariant() -> Result<(), serde_json::Error> {
        let item = DownloadItem::new(
            DownloadId::new(),
            source().map_err(serde::de::Error::custom)?,
            FileName::new("archive.zip").map_err(serde::de::Error::custom)?,
            PathBuf::from("archive.zip"),
            Timestamp::from_unix_millis(2),
        );
        let serialized = serde_json::to_string(&item)?;
        let mut value: serde_json::Value = serde_json::from_str(&serialized)?;
        value["updated_at"] = serde_json::json!(1);

        assert!(serde_json::from_value::<DownloadItem>(value).is_err());
        Ok(())
    }
}

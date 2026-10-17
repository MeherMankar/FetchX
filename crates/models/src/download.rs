use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{DownloadError, DownloadId, DownloadPriority, DownloadProgress, EngineKind, Timestamp};

/// Source locator and selected engine; it deliberately contains no credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "DownloadSourceRepr")]
pub struct DownloadSource {
    url: String,
    engine: EngineKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    yt_dlp_format: Option<String>,
}

impl DownloadSource {
    pub fn new(url: impl Into<String>, engine: EngineKind) -> Result<Self, DownloadSourceError> {
        let url = url.into();
        if url.trim().is_empty() {
            return Err(DownloadSourceError::EmptyUrl);
        }
        if url.starts_with("magnet:")
            && (engine != EngineKind::Aria2 || parse_magnet_uri(&url).is_err())
        {
            return Err(DownloadSourceError::InvalidMagnetUri);
        }
        if url.starts_with("torrent+file:")
            && (engine != EngineKind::Aria2 || torrent_file_path(&url).is_none())
        {
            return Err(DownloadSourceError::InvalidTorrentFile);
        }

        Ok(Self {
            url,
            engine,
            yt_dlp_format: None,
        })
    }

    pub fn new_torrent_file(
        path: &std::path::Path,
        engine: EngineKind,
    ) -> Result<Self, DownloadSourceError> {
        if engine != EngineKind::Aria2
            || !path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
        {
            return Err(DownloadSourceError::InvalidTorrentFile);
        }
        let file_url =
            Url::from_file_path(path).map_err(|_| DownloadSourceError::InvalidTorrentFile)?;
        Self::new(format!("torrent+{file_url}"), engine)
    }

    pub fn with_yt_dlp_format(
        mut self,
        format_id: impl Into<String>,
    ) -> Result<Self, DownloadSourceError> {
        let format_id = format_id.into();
        if self.engine != EngineKind::YtDlp || !Self::is_valid_yt_dlp_format_selector(&format_id) {
            return Err(DownloadSourceError::InvalidYtDlpFormat);
        }
        self.yt_dlp_format = Some(format_id);
        Ok(self)
    }

    fn is_valid_yt_dlp_format_selector(selector: &str) -> bool {
        if selector.is_empty() || selector.len() > 128 {
            return false;
        }

        let mut alternatives = selector.split('/');
        let Some(video_format) = alternatives.next() else {
            return false;
        };
        if alternatives.clone().count() > 1 {
            return false;
        }

        let mut tracks = video_format.split('+');
        let Some(video_id) = tracks.next() else {
            return false;
        };
        if !Self::is_valid_yt_dlp_format_id(video_id) {
            return false;
        }
        match tracks.next() {
            None => alternatives.next().is_none(),
            Some("bestaudio") => alternatives.next() == Some(video_id) && tracks.next().is_none(),
            Some(_) => false,
        }
    }

    fn is_valid_yt_dlp_format_id(format_id: &str) -> bool {
        !format_id.is_empty()
            && format_id
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && format_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub const fn engine(&self) -> EngineKind {
        self.engine
    }

    #[must_use]
    pub fn yt_dlp_format(&self) -> Option<&str> {
        self.yt_dlp_format.as_deref()
    }

    #[must_use]
    pub fn is_magnet(&self) -> bool {
        self.url.starts_with("magnet:")
    }

    #[must_use]
    pub fn is_torrent_file(&self) -> bool {
        self.url.starts_with("torrent+file:")
    }

    #[must_use]
    pub fn torrent_file_path(&self) -> Option<PathBuf> {
        torrent_file_path(&self.url)
    }
}

fn torrent_file_path(source: &str) -> Option<PathBuf> {
    let file_url = source.strip_prefix("torrent+")?;
    let url = Url::parse(file_url).ok()?;
    (url.scheme() == "file")
        .then(|| url.to_file_path().ok())
        .flatten()
}

const MAX_MAGNET_URI_BYTES: usize = 16 * 1024;
const MAX_MAGNET_TRACKERS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MagnetInfo {
    pub info_hash: String,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
}

impl MagnetInfo {
    #[must_use]
    pub fn suggested_filename(&self) -> String {
        let name = self.display_name.as_deref().unwrap_or_default();
        let safe_name = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .chars()
            .filter(|character| {
                !character.is_control()
                    && !matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            })
            .collect::<String>()
            .trim_matches([' ', '.'])
            .chars()
            .take(180)
            .collect::<String>();
        if safe_name.is_empty() {
            format!("torrent-{}", self.info_hash)
        } else {
            safe_name
        }
    }
}

pub fn parse_magnet_uri(uri: &str) -> Result<MagnetInfo, DownloadSourceError> {
    if uri.len() > MAX_MAGNET_URI_BYTES {
        return Err(DownloadSourceError::InvalidMagnetUri);
    }
    let parsed = Url::parse(uri).map_err(|_| DownloadSourceError::InvalidMagnetUri)?;
    if parsed.scheme() != "magnet"
        || parsed.host_str().is_some()
        || !parsed.path().is_empty()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(DownloadSourceError::InvalidMagnetUri);
    }

    let mut info_hash = None;
    let mut display_name = None;
    let mut trackers = Vec::new();
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "xt" => {
                if !value.starts_with("urn:btih:") {
                    return Err(DownloadSourceError::InvalidMagnetUri);
                }
                if info_hash.is_some() {
                    return Err(DownloadSourceError::InvalidMagnetUri);
                }
                let hash = &value["urn:btih:".len()..];
                if !((hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
                    || (hash.len() == 32
                        && hash.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                && !matches!(byte, b'0' | b'1' | b'8' | b'9')
                        })))
                {
                    return Err(DownloadSourceError::InvalidMagnetUri);
                }
                info_hash = Some(hash.to_ascii_lowercase());
            }
            "dn" => {
                if display_name.is_some() || value.trim().is_empty() || value.len() > 1024 {
                    return Err(DownloadSourceError::InvalidMagnetUri);
                }
                display_name = Some(value.into_owned());
            }
            "tr" => {
                if trackers.len() >= MAX_MAGNET_TRACKERS {
                    return Err(DownloadSourceError::InvalidMagnetUri);
                }
                let tracker =
                    Url::parse(&value).map_err(|_| DownloadSourceError::InvalidMagnetUri)?;
                if !matches!(tracker.scheme(), "http" | "https" | "udp")
                    || tracker.host_str().is_none()
                    || !tracker.username().is_empty()
                    || tracker.password().is_some()
                    || tracker.fragment().is_some()
                {
                    return Err(DownloadSourceError::InvalidMagnetUri);
                }
                trackers.push(tracker.into());
            }
            _ => {}
        }
    }

    Ok(MagnetInfo {
        info_hash: info_hash.ok_or(DownloadSourceError::InvalidMagnetUri)?,
        display_name,
        trackers,
    })
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DownloadSourceError {
    #[error("download source URL must not be empty")]
    EmptyUrl,
    #[error("yt-dlp format must be a valid format ID for a yt-dlp source")]
    InvalidYtDlpFormat,
    #[error("magnet URI must contain one valid BitTorrent info hash and safe optional trackers")]
    InvalidMagnetUri,
    #[error("torrent file source must be a local .torrent file using aria2")]
    InvalidTorrentFile,
}

#[derive(Deserialize)]
struct DownloadSourceRepr {
    url: String,
    engine: EngineKind,
    #[serde(default)]
    yt_dlp_format: Option<String>,
}

impl TryFrom<DownloadSourceRepr> for DownloadSource {
    type Error = DownloadSourceError;

    fn try_from(value: DownloadSourceRepr) -> Result<Self, Self::Error> {
        let source = Self::new(value.url, value.engine)?;
        match value.yt_dlp_format {
            Some(format_id) => source.with_yt_dlp_format(format_id),
            None => Ok(source),
        }
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

    /// Changes queue priority without changing the lifecycle state.
    pub fn set_priority_at(
        &mut self,
        priority: crate::DownloadPriority,
        updated_at: Timestamp,
    ) -> Result<(), DownloadItemError> {
        self.validate_update_time(updated_at)?;
        self.priority = priority;
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

    use super::{
        DownloadItem, DownloadItemError, DownloadSource, DownloadSourceError, DownloadStatus,
        FileName, parse_magnet_uri,
    };
    use crate::{DownloadId, DownloadPriority, DownloadProgress, EngineKind, Timestamp};

    fn source() -> Result<DownloadSource, super::DownloadSourceError> {
        DownloadSource::new("https://example.com/file", EngineKind::DirectHttp)
    }

    #[test]
    fn parses_valid_magnet_metadata() -> Result<(), DownloadSourceError> {
        let magnet = parse_magnet_uri(
            "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=sample%20file&tr=https%3A%2F%2Ftracker.example%2Fannounce",
        )?;

        assert_eq!(magnet.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(magnet.display_name.as_deref(), Some("sample file"));
        assert_eq!(magnet.trackers, ["https://tracker.example/announce"]);
        assert!(
            DownloadSource::new(
                "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567",
                EngineKind::Aria2,
            )?
            .is_magnet()
        );
        Ok(())
    }

    #[test]
    fn rejects_malformed_or_unsafe_magnet_uris() {
        for magnet in [
            "magnet:?dn=missing-hash",
            "magnet:?xt=urn:btih:short",
            "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567",
            "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&tr=file%3A%2F%2F%2Fetc%2Fpasswd",
            "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&tr=https%3A%2F%2Fuser%3Apass%40tracker.example%2Fannounce",
        ] {
            assert!(parse_magnet_uri(magnet).is_err(), "accepted {magnet}");
        }
        assert!(matches!(
            DownloadSource::new(
                "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567",
                EngineKind::DirectHttp,
            ),
            Err(DownloadSourceError::InvalidMagnetUri)
        ));
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
    fn source_validates_and_round_trips_yt_dlp_format() -> Result<(), Box<dyn std::error::Error>> {
        let source = DownloadSource::new("https://example.com/video", EngineKind::YtDlp)?
            .with_yt_dlp_format("137")?;
        let json = serde_json::to_string(&source)?;
        let restored: DownloadSource = serde_json::from_str(&json)?;
        assert_eq!(restored, source);
        assert_eq!(restored.yt_dlp_format(), Some("137"));

        let adaptive_source = DownloadSource::new("https://example.com/video", EngineKind::YtDlp)?
            .with_yt_dlp_format("137+bestaudio/137")?;
        assert_eq!(adaptive_source.yt_dlp_format(), Some("137+bestaudio/137"));

        assert!(
            DownloadSource::new("https://example.com/video", EngineKind::DirectHttp)?
                .with_yt_dlp_format("137")
                .is_err()
        );
        assert!(
            DownloadSource::new("https://example.com/video", EngineKind::YtDlp)?
                .with_yt_dlp_format("--exec")
                .is_err()
        );
        for selector in [
            "137+bad-audio/137",
            "137+bestaudio/138",
            "137+bestaudio/137/138",
            "137;exec/137",
        ] {
            assert!(
                DownloadSource::new("https://example.com/video", EngineKind::YtDlp)?
                    .with_yt_dlp_format(selector)
                    .is_err(),
                "unexpectedly accepted selector: {selector}"
            );
        }
        Ok(())
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

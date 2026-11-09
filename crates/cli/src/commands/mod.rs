pub mod add;
pub mod config;
pub mod db;
pub mod diagnostics;
pub mod duplicate;
pub mod engines;
pub mod lifecycle;
pub mod list;
pub mod monitor;
pub mod plugins;
pub mod priority;
pub mod recovery;
pub mod remote;
pub mod schedule;
pub mod status;
pub mod torrent;
pub mod version;

use fetchx_models::{DownloadId, DownloadIdParseError};

/// Parse a download ID from a string (full UUID or unambiguous prefix).
pub fn parse_id(s: &str) -> Result<DownloadId, DownloadIdParseError> {
    DownloadId::parse(s)
}

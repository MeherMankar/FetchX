#![forbid(unsafe_code)]

mod download;
mod engine;
mod error;
mod ids;
mod priority;
mod progress;
mod timestamp;

pub use download::{
    DownloadItem, DownloadItemError, DownloadSource, DownloadSourceError, DownloadStatus, FileName,
    FileNameError,
};
pub use engine::EngineKind;
pub use error::{DownloadError, DownloadErrorCode, DownloadErrorMessageError};
pub use ids::{DownloadId, DownloadIdParseError};
pub use priority::DownloadPriority;
pub use progress::{DownloadProgress, DownloadProgressError};
pub use timestamp::{Timestamp, TimestampError};

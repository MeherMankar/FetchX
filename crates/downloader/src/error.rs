use std::io;

use mydownloader_core::StateMachineError;
use mydownloader_models::{DownloadError, DownloadErrorCode, DownloadId};
use mydownloader_storage::StorageError;
use thiserror::Error;

/// Failures reported by the direct HTTP/HTTPS engine.
#[derive(Debug, Error)]
pub enum DownloadEngineError {
    #[error("download {0} was not found")]
    NotFound(DownloadId),
    #[error("download URL is invalid")]
    InvalidUrl,
    #[error("only HTTP and HTTPS URLs are supported")]
    UnsupportedScheme,
    #[error("download is assigned to a different engine")]
    EngineMismatch,
    #[error("URLs containing embedded credentials are not supported")]
    EmbeddedCredentials,
    #[error("could not initialize HTTP client: {0}")]
    ClientConfiguration(#[source] reqwest::Error),
    #[error("HTTP engine configuration is invalid")]
    InvalidConfiguration,
    #[error("HTTP request failed")]
    Request(#[source] reqwest::Error),
    #[error("HTTP server returned status {0}")]
    HttpStatus(u16),
    #[error("HTTP response has an invalid Content-Length")]
    InvalidContentLength,
    #[error("HTTP response has an invalid Content-Range")]
    InvalidContentRange,
    #[error("HTTP response does not match the requested byte range")]
    RangeMismatch,
    #[error("server rejected a range request while the local partial file is incomplete")]
    RangeNotSatisfiable,
    #[error("server returned a partial response without a range request")]
    UnexpectedPartialResponse,
    #[error("partial download path is not a regular file")]
    InvalidPartialFile,
    #[error("partial download file changed while it was being opened")]
    PartialFileChanged,
    #[error("segment {index} (bytes {start}-{end}) failed validation or transfer")]
    SegmentFailed { index: usize, start: u64, end: u64 },
    #[error("a segmented download worker failed unexpectedly")]
    SegmentWorkerFailed,
    #[error("response exceeded Content-Length ({expected} bytes)")]
    ContentLengthExceeded { expected: u64 },
    #[error("response ended after {actual} bytes; expected {expected}")]
    IncompleteResponse { expected: u64, actual: u64 },
    #[error("destination already exists")]
    DestinationExists,
    #[error("temporary download file already exists")]
    TemporaryFileExists,
    #[error("download cancelled")]
    Cancelled,
    #[error("download byte count overflowed")]
    ByteCountOverflow,
    #[error("download progress could not be represented: {0}")]
    Progress(#[from] mydownloader_models::DownloadProgressError),
    #[error("download diagnostic could not be represented: {0}")]
    DomainError(#[from] mydownloader_models::DownloadErrorMessageError),
    #[error("download item could not be updated: {0}")]
    Domain(#[from] mydownloader_models::DownloadItemError),
    #[error("download timestamp could not be updated: {0}")]
    Timestamp(#[from] mydownloader_models::TimestampError),
    #[error("download state transition failed: {0}")]
    StateMachine(#[from] StateMachineError),
    #[error("download persistence failed: {0}")]
    Storage(#[from] StorageError),
    #[error("file operation failed: {0}")]
    Io(#[source] io::Error),
    #[error("download failed ({operation}) and failure state could not be persisted ({state})")]
    FailurePersistence {
        operation: Box<Self>,
        state: StateMachineError,
    },
}

impl DownloadEngineError {
    pub(crate) fn persisted_error(
        &self,
    ) -> Result<DownloadError, mydownloader_models::DownloadErrorMessageError> {
        let (code, message) = match self {
            Self::InvalidUrl
            | Self::UnsupportedScheme
            | Self::EngineMismatch
            | Self::EmbeddedCredentials => (
                DownloadErrorCode::Source,
                "The download source is invalid or unsupported.".to_owned(),
            ),
            Self::DestinationExists | Self::TemporaryFileExists | Self::Io(_) => (
                DownloadErrorCode::Destination,
                "The destination could not be written safely.".to_owned(),
            ),
            Self::Request(_) => (
                DownloadErrorCode::Engine,
                "The HTTP request failed.".to_owned(),
            ),
            Self::HttpStatus(status) => (
                DownloadErrorCode::Engine,
                format!("The HTTP server returned status {status}."),
            ),
            Self::InvalidContentLength
            | Self::InvalidContentRange
            | Self::RangeMismatch
            | Self::RangeNotSatisfiable
            | Self::UnexpectedPartialResponse
            | Self::InvalidPartialFile
            | Self::PartialFileChanged
            | Self::SegmentFailed { .. }
            | Self::SegmentWorkerFailed
            | Self::ContentLengthExceeded { .. }
            | Self::IncompleteResponse { .. }
            | Self::ByteCountOverflow => (
                DownloadErrorCode::Engine,
                "The HTTP response was incomplete or invalid.".to_owned(),
            ),
            Self::Cancelled => (
                DownloadErrorCode::Internal,
                "The download was cancelled.".to_owned(),
            ),
            Self::ClientConfiguration(_)
            | Self::InvalidConfiguration
            | Self::Progress(_)
            | Self::DomainError(_)
            | Self::Domain(_)
            | Self::Timestamp(_) => (
                DownloadErrorCode::Internal,
                "The download could not be processed.".to_owned(),
            ),
            Self::NotFound(_)
            | Self::StateMachine(_)
            | Self::Storage(_)
            | Self::FailurePersistence { .. } => (
                DownloadErrorCode::Internal,
                "The download could not be persisted.".to_owned(),
            ),
        };

        DownloadError::new(code, message)
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl From<io::Error> for DownloadEngineError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::AlreadyExists {
            Self::DestinationExists
        } else {
            Self::Io(error)
        }
    }
}

use std::io;

use fetchx_core::StateMachineError;
use fetchx_models::{DownloadError, DownloadErrorCode, DownloadId};
use fetchx_storage::StorageError;
use thiserror::Error;

use crate::aria2::Aria2ClientError;
use crate::supervisor::Aria2SupervisorError;
use crate::ytdlp::YtDlpError;

/// Failures reported by the direct HTTP/HTTPS engine.
#[derive(Debug, Error)]
pub enum DownloadEngineError {
    #[error("download {0} was not found")]
    NotFound(DownloadId),
    #[error("download URL is invalid")]
    InvalidUrl,
    #[error("only HTTP and HTTPS URLs are supported")]
    UnsupportedScheme,
    #[error("unsupported media format: {0}")]
    UnsupportedFormat(&'static str),
    #[error("download is assigned to a different engine")]
    EngineMismatch,
    #[error("the selected download engine is not implemented")]
    EngineNotAvailable,
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
    #[error("partial download metadata could not be encoded: {0}")]
    InvalidPartialMetadata(String),
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
    #[error("download paused")]
    Paused,
    #[error("queue resource manager is unavailable")]
    ResourceManagerUnavailable,
    #[error("download byte count overflowed")]
    ByteCountOverflow,
    #[error("download progress could not be represented: {0}")]
    Progress(#[from] fetchx_models::DownloadProgressError),
    #[error("download diagnostic could not be represented: {0}")]
    DomainError(#[from] fetchx_models::DownloadErrorMessageError),
    #[error("download item could not be updated: {0}")]
    Domain(#[from] fetchx_models::DownloadItemError),
    #[error("download timestamp could not be updated: {0}")]
    Timestamp(#[from] fetchx_models::TimestampError),
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
    #[error("aria2 RPC operation failed: {0}")]
    Aria2Client(#[from] Aria2ClientError),
    #[error("aria2 reported a failed transfer")]
    Aria2TransferFailed,
    #[error("aria2 job was removed outside the application")]
    Aria2InvalidGid,
    #[error("aria2 download requires reconciliation before it can be restarted")]
    Aria2ReconciliationRequired,
    #[error("aria2 reported an unexpected status")]
    Aria2UnexpectedStatus,
    #[error("aria2 output file is missing or has an unexpected size")]
    Aria2InvalidOutput,
    #[error("aria2 process supervisor failed: {0}")]
    Aria2Supervisor(#[from] Aria2SupervisorError),
    #[error("yt-dlp operation failed: {0}")]
    YtDlp(#[from] YtDlpError),
    #[error("native torrent engine error: {0}")]
    NativeTorrent(String),
    #[error("FTP error: {0}")]
    FtpError(String),
    #[error("FTP path contains directory traversal sequence")]
    FtpPathTraversal,
    #[error("FTP authentication requires credentials not stored in URL")]
    FtpCredentialRequired,
    #[error("SFTP error: {0}")]
    SftpError(String),
    #[error("SFTP host key is not recognised: {host}")]
    SftpUnknownHostKey { host: String },
    #[error("SFTP authentication rejected by server")]
    SftpAuthenticationFailed,
    #[error("Metalink error: {0}")]
    MetalinkError(String),
    #[error(
        "Metalink checksum mismatch: algorithm={algorithm} expected={expected} actual={actual}"
    )]
    MetalinkChecksumMismatch {
        algorithm: String,
        expected: String,
        actual: String,
    },
    #[error("SHA-256 mismatch: expected={expected} actual={actual}")]
    Sha256Mismatch { expected: String, actual: String },
}

impl DownloadEngineError {
    pub(crate) fn persisted_error(
        &self,
    ) -> Result<DownloadError, fetchx_models::DownloadErrorMessageError> {
        let (code, message) = match self {
            Self::InvalidUrl
            | Self::UnsupportedScheme
            | Self::EngineMismatch
            | Self::EmbeddedCredentials => (
                DownloadErrorCode::Source,
                "The download source is invalid or unsupported.".to_owned(),
            ),
            Self::UnsupportedFormat(_) => (
                DownloadErrorCode::Engine,
                "The selected media format is not supported.".to_owned(),
            ),
            Self::EngineNotAvailable => (
                DownloadErrorCode::Engine,
                "The selected download engine is not available.".to_owned(),
            ),
            Self::Aria2Client(crate::aria2::Aria2ClientError::InvalidUri) => (
                DownloadErrorCode::Source,
                "The aria2 source URI is invalid or unsupported.".to_owned(),
            ),
            Self::Aria2Client(crate::aria2::Aria2ClientError::InvalidPath) => (
                DownloadErrorCode::Destination,
                "The aria2 destination path is invalid.".to_owned(),
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
            | Self::InvalidPartialMetadata(_)
            | Self::SegmentFailed { .. }
            | Self::SegmentWorkerFailed
            | Self::ContentLengthExceeded { .. }
            | Self::IncompleteResponse { .. }
            | Self::ByteCountOverflow => (
                DownloadErrorCode::Engine,
                "The HTTP response was incomplete or invalid.".to_owned(),
            ),
            Self::Aria2Client(crate::aria2::Aria2ClientError::AmbiguousAddUri) => (
                DownloadErrorCode::Engine,
                crate::aria2::AMBIGUOUS_ADD_URI_DIAGNOSTIC.to_owned(),
            ),
            Self::Aria2Client(crate::aria2::Aria2ClientError::InvalidGid)
            | Self::Aria2InvalidGid
            | Self::Aria2ReconciliationRequired => (
                DownloadErrorCode::Engine,
                crate::aria2::RECONCILIATION_DIAGNOSTIC.to_owned(),
            ),
            Self::Aria2Client(_)
            | Self::Aria2TransferFailed
            | Self::Aria2UnexpectedStatus
            | Self::Aria2InvalidOutput
            | Self::Aria2Supervisor(_) => (
                DownloadErrorCode::Engine,
                "The aria2 download could not be completed safely.".to_owned(),
            ),
            Self::YtDlp(YtDlpError::InvalidUrl) => (
                DownloadErrorCode::Source,
                "The yt-dlp source URL is invalid or unsupported.".to_owned(),
            ),
            Self::YtDlp(YtDlpError::DestinationExists) => (
                DownloadErrorCode::Destination,
                "The destination file already exists.".to_owned(),
            ),
            Self::YtDlp(YtDlpError::OutputUnavailable) => (
                DownloadErrorCode::Destination,
                "The yt-dlp output could not be written safely.".to_owned(),
            ),
            Self::YtDlp(_) => (
                DownloadErrorCode::Engine,
                "The yt-dlp download could not be completed safely.".to_owned(),
            ),
            Self::NativeTorrent(_) => (
                DownloadErrorCode::Engine,
                "The native torrent download could not be completed safely.".to_owned(),
            ),
            Self::FtpError(_) | Self::FtpPathTraversal | Self::FtpCredentialRequired => (
                DownloadErrorCode::Engine,
                "The FTP download could not be completed.".to_owned(),
            ),
            Self::SftpError(_)
            | Self::SftpUnknownHostKey { .. }
            | Self::SftpAuthenticationFailed => (
                DownloadErrorCode::Engine,
                "The SFTP download could not be completed.".to_owned(),
            ),
            Self::MetalinkError(_) | Self::MetalinkChecksumMismatch { .. } => (
                DownloadErrorCode::Engine,
                "The Metalink download could not be completed.".to_owned(),
            ),
            Self::Sha256Mismatch { .. } => (
                DownloadErrorCode::Engine,
                "The downloaded file failed SHA-256 verification and was not published.".to_owned(),
            ),
            Self::Cancelled => (
                DownloadErrorCode::Internal,
                "The download was cancelled.".to_owned(),
            ),
            Self::Paused => (
                DownloadErrorCode::Internal,
                "The download was paused.".to_owned(),
            ),
            Self::ClientConfiguration(_)
            | Self::InvalidConfiguration
            | Self::ResourceManagerUnavailable
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

    pub(crate) fn is_paused(&self) -> bool {
        matches!(self, Self::Paused)
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

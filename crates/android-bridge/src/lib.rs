// SAFETY: This crate allows unsafe_code solely for the UniFFI scaffolding generated below.
// The workspace lint (`unsafe_code = "forbid"`) is not inherited here — see
// Cargo.toml [lints] for the crate-local override policy.
pub mod dto;
pub mod error;

// Re-export DTO types at the crate root so the UniFFI scaffolding can find them.
pub use dto::{
    BatchDownloadAudioFormatDto, BatchDownloadEngineDto, BatchDownloadErrorCodeDto,
    BatchDownloadOutcomeDto, BatchDownloadRequestDto, CoreConfig, DownloadEventDto,
    DownloadInfoDto, DownloadPriorityDto, DownloadProgressDto, DownloadRuntimeProgressDto,
    DownloadRuntimeStatusDto, DownloadStatusDto, DuplicateDownloadMatchDto,
    TorrentDownloadMetadataDto, TorrentFileDto, TorrentMetadataDto, TorrentTelemetryDto,
    YoutubePlayerClientDto, YoutubePoTokenContextDto, YoutubePoTokenDto, YtDlpFormatDto,
};
pub use error::MobileApiError;

// The `callback interface DownloadEventListener` in the UDL generates a
// `#[uniffi::export_for_udl(callback_interface)]` attribute that wraps this trait.
// The trait MUST be defined before `uniffi::include_scaffolding!` is called.
pub trait DownloadEventListener: Send + Sync {
    fn on_event(&self, event: DownloadEventDto);
}

pub trait YtDlpRuntime: Send + Sync {
    fn validate_version(&self) -> String;
    fn run(&self, arguments: Vec<String>, progress_path: String, cancel_path: String) -> String;
    fn discover_formats(&self, url: String) -> String;
    fn report_failure(&self, diagnostic_code: String);
}

// ── WVD parsing free function exposed via UniFFI ──────────────────────────────

/// DTO matching the `WvdDeviceDto` dictionary in the UDL.
pub struct WvdDeviceDto {
    pub device_type: u8,
    pub private_key_der: Vec<u8>,
    pub client_id_blob: Vec<u8>,
}

/// Error type matching the `[Error] enum WvdParseError` in the UDL.
#[derive(Debug, thiserror::Error)]
pub enum WvdParseError {
    #[error("WVD file is too short or truncated")]
    TooShort,
    #[error("WVD private key section overflows file")]
    KeyOverflow,
    #[error("WVD client-id section overflows file")]
    IdOverflow,
}

impl From<fetchx_widevine::WvdError> for WvdParseError {
    fn from(e: fetchx_widevine::WvdError) -> Self {
        match e {
            fetchx_widevine::WvdError::TooShort => Self::TooShort,
            fetchx_widevine::WvdError::KeyOverflow => Self::KeyOverflow,
            fetchx_widevine::WvdError::IdOverflow => Self::IdOverflow,
        }
    }
}

/// Parse a `.wvd` binary file into its component fields.
///
/// Exposed to Android Kotlin code via UniFFI as `parseWvdBytes(data: ByteArray): WvdDeviceDto`.
pub fn parse_wvd_bytes(data: Vec<u8>) -> Result<WvdDeviceDto, WvdParseError> {
    let device = fetchx_widevine::wvd::parse_wvd(&data)?;
    Ok(WvdDeviceDto {
        device_type: device.device_type,
        private_key_der: device.private_key_der,
        client_id_blob: device.client_id_blob,
    })
}

// The scaffolding must come after the trait definition above so that the
// generated glue can reference `DownloadEventListener` in this crate.
uniffi::include_scaffolding!("android_bridge");

// Service module defines `DownloadService` which the scaffolding's udl_derive(Object)
// references. The scaffolding's object glue delegates method calls to this impl.
pub mod service;
pub use service::{DownloadService, PluginSecurityService};

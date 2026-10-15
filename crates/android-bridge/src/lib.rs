// SAFETY: This crate allows unsafe_code solely for the UniFFI scaffolding generated below.
// The workspace lint (`unsafe_code = "forbid"`) is not inherited here — see
// Cargo.toml [lints] for the crate-local override policy.
pub mod dto;
pub mod error;

// Re-export DTO types at the crate root so the UniFFI scaffolding can find them.
pub use dto::{
    CoreConfig, DownloadEventDto, DownloadInfoDto, DownloadPriorityDto, DownloadProgressDto,
    DownloadStatusDto,
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
}

// The scaffolding must come after the trait definition above so that the
// generated glue can reference `DownloadEventListener` in this crate.
uniffi::include_scaffolding!("android_bridge");

// Service module defines `DownloadService` which the scaffolding's udl_derive(Object)
// references. The scaffolding's object glue delegates method calls to this impl.
pub mod service;
pub use service::DownloadService;

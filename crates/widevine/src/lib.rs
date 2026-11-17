//! `fetchx-widevine`: Widevine DRM support for FetchX.
//!
//! Provides:
//! - `.wvd` device file parsing ([`wvd`])
//! - Remote CDM HTTP client ([`remote_cdm`])
//! - PSSH box extraction from MPD manifests ([`pssh`])

#![forbid(unsafe_code)]

pub mod pssh;
pub mod remote_cdm;
pub mod wvd;

pub use pssh::{PsshBox, extract_pssh_from_mpd};
pub use remote_cdm::{RemoteCdm, RemoteCdmError};
pub use wvd::{WvdDevice, WvdError};

/// A decrypted Widevine content key.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ContentKey {
    /// Key ID bytes (typically 16 bytes, big-endian).
    pub key_id: Vec<u8>,
    /// Content key bytes (typically 16 bytes).
    pub key: Vec<u8>,
}

/// Trait implemented by any Widevine key provider.
#[async_trait::async_trait]
pub trait WidevineProvider: Send + Sync {
    /// Fetch decryption keys for the given PSSH data from the specified license server.
    async fn get_keys(
        &self,
        pssh: &[u8],
        license_url: &str,
    ) -> Result<Vec<ContentKey>, Box<dyn std::error::Error + Send + Sync>>;
}

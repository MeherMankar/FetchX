//! `fetchx-playready`: PlayReady DRM support for FetchX.
//!
//! Provides:
//! - `.prd` device file parsing ([`prd`])
//! - PlayReady PSSH detection and MPD extraction ([`pssh`])
//! - Remote CDM HTTP client ([`remote_cdm`])

#![forbid(unsafe_code)]

pub mod prd;
pub mod pssh;
pub mod remote_cdm;

pub use fetchx_widevine::ContentKey;
pub use prd::{PrdDevice, PrdError};
pub use pssh::{PLAYREADY_SYSTEM_ID, extract_pssh_from_mpd, is_playready_pssh};
pub use remote_cdm::RemotePlayReadyCdm;

/// Trait implemented by any PlayReady key provider.
#[async_trait::async_trait]
pub trait PlayReadyProvider: Send + Sync {
    /// Fetch decryption keys for the given PSSH data from the specified license server.
    async fn get_keys(
        &self,
        pssh: &[u8],
        license_url: &str,
    ) -> Result<Vec<ContentKey>, Box<dyn std::error::Error + Send + Sync>>;
}

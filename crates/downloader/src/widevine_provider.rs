//! Factory for Widevine CDM providers.
//!
//! Builds the best available [`WidevineProvider`] from a [`DrmConfig`].
//!
//! # Key cache shortcut
//!
//! When `config.keys` is non-empty the caller already has the decrypted
//! content keys (e.g. from a browser extension).  In that case the CDM
//! should not be invoked at all — use the cached keys directly instead of
//! calling `build_widevine_provider`.

use fetchx_models::DrmConfig;
use fetchx_widevine::{LocalCdm, RemoteCdm, WidevineProvider, WvdDevice};

/// Build the best available Widevine provider from [`DrmConfig`].
///
/// Priority:
/// 1. [`LocalCdm`] when `wvd_path` is set and the file can be loaded.
/// 2. [`RemoteCdm`] when `remote_cdm_url` is set.
/// 3. An error if neither is configured.
///
/// **Note**: if `config.keys` is non-empty the caller should use those keys
/// directly and must not invoke the CDM at all.
pub fn build_widevine_provider(
    config: &DrmConfig,
) -> Result<Box<dyn WidevineProvider + Send + Sync>, String> {
    if let Some(wvd_path) = &config.wvd_path {
        let device = WvdDevice::from_file(wvd_path)
            .map_err(|e| format!("Failed to load .wvd file at {:?}: {e}", wvd_path))?;
        return Ok(Box::new(LocalCdm::new(device)));
    }
    if let Some(url) = &config.remote_cdm_url {
        let device_name = config.remote_cdm_device.as_deref().unwrap_or("default");
        return Ok(Box::new(RemoteCdm::new(url, device_name, "")));
    }
    Err(
        "No Widevine CDM configured. Set wvd_path for local decryption, \
         or remote_cdm_url for a remote CDM server."
            .to_owned(),
    )
}

//! DRM key acquisition routing for HLS and DASH streams.
// NOTE: Direct DRM integration. For the plugin-based path, see HlsEngineConfig::drm_plugin.

#![allow(dead_code)] // acquire_keys and hex_to_bytes will be called once PSSH wiring is complete

use fetchx_models::DrmConfig;
use fetchx_playready::PlayReadyProvider;
use fetchx_widevine::ContentKey;

use crate::{
    DownloadEngineError,
    drm_router::{DrmSystem, detect_drm_system},
    widevine_provider::build_widevine_provider,
};

/// Attempts to obtain content keys for a DRM-protected manifest.
///
/// Returns `None` if DRM config is not set (download continues unprotected).
/// Returns `Err` if DRM config is set but key acquisition fails.
pub async fn acquire_keys(
    pssh: &[u8],
    license_url: &str,
    config: &DrmConfig,
) -> Result<Option<Vec<ContentKey>>, DownloadEngineError> {
    // If pre-cached keys exist in config, use them directly.
    if !config.keys.is_empty() {
        let keys: Vec<ContentKey> = config
            .keys
            .iter()
            .filter_map(|k| {
                Some(ContentKey {
                    key_id: hex_to_bytes(&k.kid)?,
                    key: hex_to_bytes(&k.k)?,
                })
            })
            .collect();
        return Ok(Some(keys));
    }

    match detect_drm_system(pssh) {
        DrmSystem::Widevine => {
            let provider = build_widevine_provider(config).map_err(|e| {
                DownloadEngineError::NativeTorrent(format!("Widevine CDM configuration error: {e}"))
            })?;
            let keys = provider.get_keys(pssh, license_url).await.map_err(|e| {
                DownloadEngineError::NativeTorrent(format!("Widevine key acquisition failed: {e}"))
            })?;
            Ok(Some(keys))
        }
        DrmSystem::PlayReady => {
            use fetchx_playready::RemotePlayReadyCdm;
            if let Some(url) = &config.playready_remote_cdm_url {
                let device = config
                    .playready_remote_cdm_device
                    .as_deref()
                    .unwrap_or("default");
                let cdm = RemotePlayReadyCdm::new(url, device, "");
                let keys = cdm.get_keys(pssh, license_url).await.map_err(|e| {
                    DownloadEngineError::NativeTorrent(format!(
                        "PlayReady key acquisition failed: {e}"
                    ))
                })?;
                Ok(Some(keys))
            } else {
                Err(DownloadEngineError::UnsupportedFormat(
                    "DRM-protected stream — configure a PlayReady device in Settings > DRM to decrypt",
                ))
            }
        }
        DrmSystem::Unknown => Err(DownloadEngineError::UnsupportedFormat(
            "DRM-protected stream with unknown DRM system",
        )),
    }
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

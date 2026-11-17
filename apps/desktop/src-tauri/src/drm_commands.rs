//! Tauri commands for Widevine DRM configuration.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::AppState;

/// Data-transfer object for DRM configuration, used in Tauri IPC.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DrmConfigDto {
    pub wvd_path: Option<String>,
    pub remote_cdm_url: Option<String>,
    pub remote_cdm_device: Option<String>,
}

/// Return the current DRM configuration.
#[tauri::command]
pub async fn get_drm_config(state: State<'_, AppState>) -> Result<DrmConfigDto, String> {
    let config = state.config_service.load().await.map_err(|e| e.to_string())?;
    Ok(DrmConfigDto {
        wvd_path: config.drm.wvd_path.map(|p| p.to_string_lossy().into_owned()),
        remote_cdm_url: config.drm.remote_cdm_url,
        remote_cdm_device: config.drm.remote_cdm_device,
    })
}

/// Persist updated DRM configuration.
#[tauri::command]
pub async fn set_drm_config(
    state: State<'_, AppState>,
    request: DrmConfigDto,
) -> Result<DrmConfigDto, String> {
    let mut config = state.config_service.load().await.map_err(|e| e.to_string())?;
    config.drm.wvd_path = request
        .wvd_path
        .as_deref()
        .map(std::path::Path::new)
        .map(|p| p.to_path_buf());
    config.drm.remote_cdm_url = request.remote_cdm_url.clone();
    config.drm.remote_cdm_device = request.remote_cdm_device.clone();
    state
        .config_service
        .save(&config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(request)
}

/// Open a native file dialog filtered to `.wvd` files and return the chosen path.
///
/// Requires `tauri-plugin-dialog` to be registered in the Tauri application.
#[tauri::command]
pub async fn pick_wvd_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .add_filter("Widevine Device", &["wvd"])
        .blocking_pick_file();
    Ok(path.map(|p| p.to_string_lossy().into_owned()))
}

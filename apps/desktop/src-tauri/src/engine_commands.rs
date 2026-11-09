//! Tauri commands for native engine configuration and diagnostics.

use std::{path::PathBuf, time::Duration};

use fetchx_core_services::{
    ConfigError, EngineAvailability, EngineConfig, YtDlpConfig as PersistedYtDlpConfig,
};
use fetchx_models::{ResourcePolicy, ResourceProfile};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::{fs, process::Command};

use crate::commands::AppState;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEngineConfigRequest {
    pub yt_dlp: SaveYtDlpConfigRequest,
    pub resource_policy: Option<ResourcePolicy>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveYtDlpConfigRequest {
    pub enabled: bool,
    pub executable_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineConfigDto {
    pub direct_http: DirectHttpConfigDto,
    pub yt_dlp: YtDlpConfigDto,
    pub resource_policy: ResourcePolicy,
}

#[derive(Clone, Debug, Serialize)]
pub struct DirectHttpConfigDto {
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct YtDlpConfigDto {
    pub enabled: bool,
    pub executable_path: Option<String>,
}

impl From<&EngineConfig> for EngineConfigDto {
    fn from(config: &EngineConfig) -> Self {
        Self {
            direct_http: DirectHttpConfigDto {
                enabled: config.direct_http.enabled,
            },
            yt_dlp: YtDlpConfigDto {
                enabled: config.yt_dlp.enabled,
                executable_path: config
                    .yt_dlp
                    .executable_path
                    .as_deref()
                    .map(|path| path.to_string_lossy().into_owned()),
            },
            resource_policy: config.resource_policy.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticResult {
    pub available: bool,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineIpcErrorCode {
    InvalidConfig,
    ExecutableNotFound,
    ExecutableInvalid,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineIpcError {
    pub code: EngineIpcErrorCode,
    pub message: String,
}

impl std::fmt::Display for EngineIpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EngineIpcError {}

impl From<ConfigError> for EngineIpcError {
    fn from(error: ConfigError) -> Self {
        Self {
            code: EngineIpcErrorCode::InvalidConfig,
            message: error.to_string(),
        }
    }
}

async fn validate_executable_path(path: &str) -> Result<PathBuf, EngineIpcError> {
    if path.trim().is_empty() {
        return Err(EngineIpcError {
            code: EngineIpcErrorCode::ExecutableNotFound,
            message: "Executable path must not be empty.".to_owned(),
        });
    }
    let path_buf = PathBuf::from(path);
    match fs::metadata(&path_buf).await {
        Ok(metadata) if metadata.is_file() => Ok(path_buf),
        Ok(_) => Err(EngineIpcError {
            code: EngineIpcErrorCode::ExecutableInvalid,
            message: format!("Path '{}' exists but is not a regular file.", path),
        }),
        Err(_) => Err(EngineIpcError {
            code: EngineIpcErrorCode::ExecutableNotFound,
            message: format!("Executable not found at '{}'.", path),
        }),
    }
}

async fn resolve_ytdlp_executable(
    config: &PersistedYtDlpConfig,
) -> Result<PathBuf, EngineIpcError> {
    if let Some(path) = &config.executable_path {
        return validate_executable_path(&path.to_string_lossy()).await;
    }
    Ok(PathBuf::from(if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }))
}

#[tauri::command]
pub async fn get_engine_config(
    state: State<'_, AppState>,
) -> Result<EngineConfigDto, EngineIpcError> {
    let config = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;
    Ok(EngineConfigDto::from(&config))
}

#[tauri::command]
pub async fn save_engine_config(
    state: State<'_, AppState>,
    request: SaveEngineConfigRequest,
) -> Result<EngineConfigDto, EngineIpcError> {
    let mut config = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;
    config.yt_dlp.enabled = request.yt_dlp.enabled;
    config.yt_dlp.executable_path = match &request.yt_dlp.executable_path {
        Some(path) if !path.trim().is_empty() => Some(validate_executable_path(path).await?),
        _ => None,
    };
    if let Some(policy) = request.resource_policy {
        config.resource_policy = match policy.profile {
            ResourceProfile::Custom => policy,
            profile => ResourcePolicy::for_profile(profile),
        };
    }
    state
        .config_service
        .save(&config)
        .await
        .map_err(EngineIpcError::from)?;
    Ok(EngineConfigDto::from(&config))
}

#[tauri::command]
pub async fn reset_engine_config(
    state: State<'_, AppState>,
) -> Result<EngineConfigDto, EngineIpcError> {
    let config = state
        .config_service
        .reset()
        .await
        .map_err(EngineIpcError::from)?;
    Ok(EngineConfigDto::from(&config))
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineInstanceStatusDto {
    pub enabled: bool,
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineStatusDto {
    pub direct_http: EngineInstanceStatusDto,
    pub yt_dlp: EngineInstanceStatusDto,
    pub native_torrent: EngineInstanceStatusDto,
}

pub(crate) fn engine_instance_dto(availability: &EngineAvailability) -> EngineInstanceStatusDto {
    match availability {
        EngineAvailability::Disabled => EngineInstanceStatusDto {
            enabled: false,
            available: false,
            version: None,
            error: None,
        },
        EngineAvailability::Available { version } => EngineInstanceStatusDto {
            enabled: true,
            available: true,
            version: Some(version.clone()),
            error: None,
        },
        EngineAvailability::Unavailable { reason } => EngineInstanceStatusDto {
            enabled: true,
            available: false,
            version: None,
            error: Some(reason.clone()),
        },
    }
}

#[tauri::command]
pub async fn get_engine_status(
    state: State<'_, AppState>,
) -> Result<EngineStatusDto, EngineIpcError> {
    let status = &state.engine_runtime.status;
    Ok(EngineStatusDto {
        direct_http: EngineInstanceStatusDto {
            enabled: true,
            available: true,
            version: None,
            error: None,
        },
        yt_dlp: engine_instance_dto(&status.yt_dlp),
        native_torrent: engine_instance_dto(&status.native_torrent),
    })
}

#[tauri::command]
pub async fn test_ytdlp_config(
    state: State<'_, AppState>,
) -> Result<DiagnosticResult, EngineIpcError> {
    let config = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;
    let executable = match resolve_ytdlp_executable(&config.yt_dlp).await {
        Ok(path) => path,
        Err(error) => {
            return Ok(DiagnosticResult {
                available: false,
                executable: None,
                version: None,
                message: error.message,
            });
        }
    };
    let mut command = Command::new(&executable);
    command.arg("--version");
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let result = tokio::time::timeout(Duration::from_secs(5), command.output()).await;
    match result {
        Ok(Ok(output)) if output.status.success() => Ok(DiagnosticResult {
            available: true,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: Some(String::from_utf8_lossy(&output.stdout).trim().to_owned()),
            message: "yt-dlp is available.".to_owned(),
        }),
        Ok(Ok(output)) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: format!(
                "yt-dlp exited with status {}.",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            ),
        }),
        Ok(Err(error)) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: format!("Could not start yt-dlp: {error}"),
        }),
        Err(_) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: "yt-dlp version check timed out.".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::SaveEngineConfigRequest;

    #[test]
    fn save_request_rejects_aria2_fields() {
        let json = r#"{
            "aria2": {"enabled": true, "rpc_port": 6800},
            "yt_dlp": {"enabled": false, "executable_path": null}
        }"#;
        assert!(serde_json::from_str::<SaveEngineConfigRequest>(json).is_err());
    }

    #[test]
    fn save_request_accepts_native_engine_settings() {
        let json = r#"{
            "yt_dlp": {"enabled": false, "executable_path": null}
        }"#;
        assert!(serde_json::from_str::<SaveEngineConfigRequest>(json).is_ok());
    }
}

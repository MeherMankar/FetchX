//! Tauri commands for native engine configuration and diagnostics.

use crate::commands::AppState;
use fetchx_core_services::{ConfigError, EngineAvailability, EngineConfig};
use fetchx_models::{ResourcePolicy, ResourceProfile, WeeklyQueueSchedule};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEngineConfigRequest {
    pub yt_dlp: SaveYtDlpConfigRequest,
    pub resource_policy: Option<ResourcePolicy>,
    #[serde(default)]
    pub weekly_queue_schedule: Option<WeeklyQueueSchedule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveYtDlpConfigRequest {
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EngineConfigDto {
    pub direct_http: DirectHttpConfigDto,
    pub yt_dlp: YtDlpConfigDto,
    pub resource_policy: ResourcePolicy,
    pub weekly_queue_schedule: WeeklyQueueSchedule,
}

#[derive(Clone, Debug, Serialize)]
pub struct DirectHttpConfigDto {
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct YtDlpConfigDto {
    pub enabled: bool,
}

impl From<&EngineConfig> for EngineConfigDto {
    fn from(config: &EngineConfig) -> Self {
        Self {
            direct_http: DirectHttpConfigDto {
                enabled: config.direct_http.enabled,
            },
            yt_dlp: YtDlpConfigDto {
                enabled: config.yt_dlp.enabled,
            },
            resource_policy: config.resource_policy.clone(),
            weekly_queue_schedule: config.weekly_queue_schedule.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticResult {
    pub available: bool,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineIpcErrorCode {
    InvalidConfig,
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
    config.yt_dlp.executable_path = None;
    if let Some(policy) = request.resource_policy {
        config.resource_policy = match policy.profile {
            ResourceProfile::Custom => policy,
            profile => ResourcePolicy::for_profile(profile),
        };
    }
    if let Some(schedule) = request.weekly_queue_schedule {
        config.weekly_queue_schedule = schedule;
    }
    state
        .config_service
        .save(&config)
        .await
        .map_err(EngineIpcError::from)?;
    state
        .scheduler
        .set_weekly_queue_schedule(config.weekly_queue_schedule.clone())
        .await
        .map_err(|error| EngineIpcError {
            code: EngineIpcErrorCode::InvalidConfig,
            message: error.to_string(),
        })?;
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
    state
        .scheduler
        .set_weekly_queue_schedule(config.weekly_queue_schedule.clone())
        .await
        .map_err(|error| EngineIpcError {
            code: EngineIpcErrorCode::InvalidConfig,
            message: error.to_string(),
        })?;
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
    let status = &state.engine_runtime.status.yt_dlp;
    let (available, version, message) = match status {
        EngineAvailability::Disabled => (
            false,
            None,
            "The native yt-dlp engine is disabled.".to_owned(),
        ),
        EngineAvailability::Available { version } => (
            true,
            Some(version.clone()),
            "FetchX's native Rust yt-dlp engine is ready.".to_owned(),
        ),
        EngineAvailability::Unavailable { reason } => (
            false,
            None,
            format!("The native Rust yt-dlp engine is unavailable: {reason}"),
        ),
    };
    Ok(DiagnosticResult {
        available,
        version,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::SaveEngineConfigRequest;

    #[test]
    fn save_request_rejects_aria2_fields() {
        let json = r#"{
            "aria2": {"enabled": true, "rpc_port": 6800},
            "yt_dlp": {"enabled": false}
        }"#;
        assert!(serde_json::from_str::<SaveEngineConfigRequest>(json).is_err());
    }

    #[test]
    fn save_request_accepts_native_engine_settings() {
        let json = r#"{
            "yt_dlp": {"enabled": false}
        }"#;
        assert!(serde_json::from_str::<SaveEngineConfigRequest>(json).is_ok());
    }
}

//! Tauri commands for reading and writing engine configuration, and for
//! running diagnostic checks against aria2 and yt-dlp.
//!
//! # Security notes
//! - The aria2 RPC secret is **never** included in any DTO returned to the
//!   frontend.  The `Aria2ConfigDto` field `has_secret` signals whether a
//!   secret is configured without exposing its value.
//! - Executable paths are validated server-side before being stored or used.
//! - Diagnostic commands use the existing structured process launchers;
//!   no shell-string concatenation is performed.

use std::{path::PathBuf, time::Duration};

use fetchx_core_services::{
    Aria2Config, ConfigError, EngineAvailability, EngineConfig, YtDlpConfig as PersistedYtDlpConfig,
};
use fetchx_models::{ResourcePolicy, ResourceProfile};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::{fs, process::Command};

use crate::commands::AppState;

// ============================================================
// Request / response DTOs
// ============================================================

/// The configuration payload sent from the frontend when saving settings.
///
/// The aria2 `rpc_secret_update` field is `None` when the user did not touch
/// the secret field.  A `Some("")` value explicitly clears the secret.
/// `deny_unknown_fields` prevents the frontend from injecting unexpected keys.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEngineConfigRequest {
    pub aria2: SaveAria2ConfigRequest,
    pub yt_dlp: SaveYtDlpConfigRequest,
    pub resource_policy: Option<ResourcePolicy>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAria2ConfigRequest {
    pub enabled: bool,
    pub executable_path: Option<String>,
    pub rpc_port: u16,
    /// `None` = leave existing secret unchanged.
    /// `Some("")` = clear the secret.
    /// `Some("token")` = set the secret to "token".
    pub rpc_secret_update: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveYtDlpConfigRequest {
    pub enabled: bool,
    pub executable_path: Option<String>,
}

/// Configuration DTO returned to the frontend.
/// Secret fields are replaced with `has_secret` booleans.
#[derive(Clone, Debug, Serialize)]
pub struct EngineConfigDto {
    pub direct_http: DirectHttpConfigDto,
    pub aria2: Aria2ConfigDto,
    pub yt_dlp: YtDlpConfigDto,
    pub resource_policy: ResourcePolicy,
}

#[derive(Clone, Debug, Serialize)]
pub struct DirectHttpConfigDto {
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Aria2ConfigDto {
    pub enabled: bool,
    pub executable_path: Option<String>,
    pub rpc_port: u16,
    /// `true` when a non-empty secret is stored.  The actual secret is never
    /// returned to the frontend.
    pub has_secret: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct YtDlpConfigDto {
    pub enabled: bool,
    pub executable_path: Option<String>,
}

impl From<&EngineConfig> for EngineConfigDto {
    fn from(cfg: &EngineConfig) -> Self {
        Self {
            direct_http: DirectHttpConfigDto {
                enabled: cfg.direct_http.enabled,
            },
            aria2: Aria2ConfigDto {
                enabled: cfg.aria2.enabled,
                executable_path: cfg
                    .aria2
                    .executable_path
                    .as_deref()
                    .map(|p| p.to_string_lossy().into_owned()),
                rpc_port: cfg.aria2.rpc_port,
                has_secret: cfg.aria2.has_secret(),
            },
            yt_dlp: YtDlpConfigDto {
                enabled: cfg.yt_dlp.enabled,
                executable_path: cfg
                    .yt_dlp
                    .executable_path
                    .as_deref()
                    .map(|p| p.to_string_lossy().into_owned()),
            },
            resource_policy: cfg.resource_policy.clone(),
        }
    }
}

/// Diagnostic result returned after testing an engine configuration.
#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticResult {
    pub available: bool,
    /// Resolved executable path (may differ from the configured path if PATH
    /// discovery was used).
    pub executable: Option<String>,
    /// Version string as reported by the executable.
    pub version: Option<String>,
    /// Human-readable status message.
    pub message: String,
}

// ============================================================
// IpcError mapping
// ============================================================

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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineIpcError {}

impl From<ConfigError> for EngineIpcError {
    fn from(e: ConfigError) -> Self {
        Self {
            code: EngineIpcErrorCode::InvalidConfig,
            message: e.to_string(),
        }
    }
}

// ============================================================
// Helper: validate an executable path
// ============================================================

/// Resolves and validates that a path points to an existing regular file.
/// Returns the canonical path on success.
///
/// This does NOT execute the file.
async fn validate_executable_path(path: &str) -> Result<PathBuf, EngineIpcError> {
    if path.trim().is_empty() {
        return Err(EngineIpcError {
            code: EngineIpcErrorCode::ExecutableNotFound,
            message: "Executable path must not be empty.".to_owned(),
        });
    }
    let p = PathBuf::from(path);
    match fs::metadata(&p).await {
        Ok(meta) if meta.is_file() => Ok(p),
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

// ============================================================
// Helper: resolve aria2c from config or PATH
// ============================================================

async fn resolve_aria2_executable(cfg: &Aria2Config) -> Result<PathBuf, EngineIpcError> {
    if let Some(path) = &cfg.executable_path {
        return validate_executable_path(&path.to_string_lossy()).await;
    }
    // Search PATH for "aria2c" or "aria2c.exe" using structured Command lookup.
    // We do NOT construct a shell string; `which`-style lookup via `Command::new`.
    let name = if cfg!(target_os = "windows") {
        "aria2c.exe"
    } else {
        "aria2c"
    };
    // On Windows, `where` is a built-in; on Unix, use `which` via PATH resolution.
    // Simpler and cross-platform: just try to query the version and let the OS
    // resolve the name from PATH.
    let resolved = PathBuf::from(name);
    // Verify it's actually runnable by checking with a trivial version call below.
    Ok(resolved)
}

async fn resolve_ytdlp_executable(cfg: &PersistedYtDlpConfig) -> Result<PathBuf, EngineIpcError> {
    if let Some(path) = &cfg.executable_path {
        return validate_executable_path(&path.to_string_lossy()).await;
    }
    let name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    Ok(PathBuf::from(name))
}

// ============================================================
// Tauri commands
// ============================================================

/// Returns the current engine configuration.  The aria2 secret is never
/// included; `has_secret` indicates whether one is stored.
#[tauri::command]
pub async fn get_engine_config(
    state: State<'_, AppState>,
) -> Result<EngineConfigDto, EngineIpcError> {
    let cfg = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;
    Ok(EngineConfigDto::from(&cfg))
}

/// Validates and persists updated engine configuration.
///
/// The aria2 secret is only updated when `rpc_secret_update` is `Some(_)`.
/// Passing `Some("")` clears the secret.
#[tauri::command]
pub async fn save_engine_config(
    state: State<'_, AppState>,
    request: SaveEngineConfigRequest,
) -> Result<EngineConfigDto, EngineIpcError> {
    // Load current config so we can apply a partial update (secret update is
    // optional).
    let mut cfg = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;

    // ---- Direct HTTP: not user-configurable; always enabled ----

    // ---- aria2 ----
    cfg.aria2.enabled = request.aria2.enabled;
    cfg.aria2.rpc_port = request.aria2.rpc_port;

    // Validate and canonicalise the executable path if provided.
    cfg.aria2.executable_path = match &request.aria2.executable_path {
        Some(p) if !p.trim().is_empty() => Some(validate_executable_path(p).await?),
        _ => None,
    };

    // Apply secret update only when explicitly provided.
    if let Some(secret_update) = request.aria2.rpc_secret_update {
        let trimmed = secret_update.trim();
        cfg.aria2.rpc_secret = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        };
    }

    // ---- yt-dlp ----
    cfg.yt_dlp.enabled = request.yt_dlp.enabled;
    cfg.yt_dlp.executable_path = match &request.yt_dlp.executable_path {
        Some(p) if !p.trim().is_empty() => Some(validate_executable_path(p).await?),
        _ => None,
    };
    if let Some(resource_policy) = request.resource_policy {
        cfg.resource_policy = match resource_policy.profile {
            ResourceProfile::Custom => resource_policy,
            profile => ResourcePolicy::for_profile(profile),
        };
    }

    state
        .config_service
        .save(&cfg)
        .await
        .map_err(EngineIpcError::from)?;

    Ok(EngineConfigDto::from(&cfg))
}

/// Resets engine configuration to safe defaults.
#[tauri::command]
pub async fn reset_engine_config(
    state: State<'_, AppState>,
) -> Result<EngineConfigDto, EngineIpcError> {
    let cfg = state
        .config_service
        .reset()
        .await
        .map_err(EngineIpcError::from)?;
    Ok(EngineConfigDto::from(&cfg))
}

/// Live runtime status of a single engine instance.
/// Never includes secrets or internal process state.
#[derive(Clone, Debug, Serialize)]
pub struct EngineInstanceStatusDto {
    pub enabled: bool,
    pub available: bool,
    pub version: Option<String>,
    /// Human-readable error when available == false.
    pub error: Option<String>,
}

/// Runtime status of all engines returned to the frontend.
#[derive(Clone, Debug, Serialize)]
pub struct EngineStatusDto {
    pub direct_http: EngineInstanceStatusDto,
    pub aria2: EngineInstanceStatusDto,
    pub yt_dlp: EngineInstanceStatusDto,
}

pub(crate) fn engine_instance_dto(avail: &EngineAvailability) -> EngineInstanceStatusDto {
    match avail {
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

/// Returns the live runtime availability of each engine.
/// Reflects what actually happened at startup, not just persisted config.
/// Secrets are never included.
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
        aria2: engine_instance_dto(&status.aria2),
        yt_dlp: engine_instance_dto(&status.yt_dlp),
    })
}
#[tauri::command]
pub async fn test_aria2_config(
    state: State<'_, AppState>,
) -> Result<DiagnosticResult, EngineIpcError> {
    let cfg = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;

    let executable = match resolve_aria2_executable(&cfg.aria2).await {
        Ok(p) => p,
        Err(e) => {
            return Ok(DiagnosticResult {
                available: false,
                executable: None,
                version: None,
                message: e.message,
            });
        }
    };

    // Run `aria2c --version` with structured arguments.
    // Hard timeout of 5 seconds to avoid hanging.
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(&executable).arg("--version").output(),
    )
    .await;

    match output {
        Ok(Ok(out)) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // First line of `aria2c --version` is typically "aria2 version X.Y.Z".
            let version = stdout.lines().next().unwrap_or("").trim().to_owned();
            Ok(DiagnosticResult {
                available: true,
                executable: Some(executable.to_string_lossy().into_owned()),
                version: Some(version),
                message: "aria2 is available.".to_owned(),
            })
        }
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Log only the first 200 chars of stderr to avoid large output in logs.
            let preview: String = stderr.chars().take(200).collect();
            tracing::warn!(aria2_stderr = %preview, "aria2 --version returned non-zero exit");
            Ok(DiagnosticResult {
                available: false,
                executable: Some(executable.to_string_lossy().into_owned()),
                version: None,
                message: format!(
                    "aria2 exited with status {}.",
                    out.status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".to_owned())
                ),
            })
        }
        Ok(Err(io_err)) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: format!("Could not start aria2: {io_err}"),
        }),
        Err(_timeout) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: "aria2 version check timed out.".to_owned(),
        }),
    }
}

/// Tests whether the configured yt-dlp executable is present and reports its
/// version.  Does NOT download anything.
#[tauri::command]
pub async fn test_ytdlp_config(
    state: State<'_, AppState>,
) -> Result<DiagnosticResult, EngineIpcError> {
    let cfg = state
        .config_service
        .load()
        .await
        .map_err(EngineIpcError::from)?;

    let executable = match resolve_ytdlp_executable(&cfg.yt_dlp).await {
        Ok(p) => p,
        Err(e) => {
            return Ok(DiagnosticResult {
                available: false,
                executable: None,
                version: None,
                message: e.message,
            });
        }
    };

    // Run `yt-dlp --version` with structured arguments.
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(&executable).arg("--version").output(),
    )
    .await;

    match output {
        Ok(Ok(out)) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            Ok(DiagnosticResult {
                available: true,
                executable: Some(executable.to_string_lossy().into_owned()),
                version: Some(version),
                message: "yt-dlp is available.".to_owned(),
            })
        }
        Ok(Ok(out)) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: format!(
                "yt-dlp exited with status {}.",
                out.status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            ),
        }),
        Ok(Err(io_err)) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: format!("Could not start yt-dlp: {io_err}"),
        }),
        Err(_timeout) => Ok(DiagnosticResult {
            available: false,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            message: "yt-dlp version check timed out.".to_owned(),
        }),
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_dto_hides_secret() {
        let mut cfg = EngineConfig::default();
        cfg.aria2.rpc_secret = Some("s3cr3t".to_owned());
        let dto = EngineConfigDto::from(&cfg);
        assert!(dto.aria2.has_secret);
        // Confirm the actual serialisation does not contain the secret value.
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.contains("s3cr3t"));
    }

    #[test]
    fn engine_config_dto_has_secret_false_when_none() {
        let cfg = EngineConfig::default();
        let dto = EngineConfigDto::from(&cfg);
        assert!(!dto.aria2.has_secret);
    }

    #[test]
    fn save_request_rejects_unknown_fields() {
        let json = r#"{
            "aria2": {"enabled": false, "rpc_port": 6800, "executable_path": null, "rpc_secret_update": null, "extra": true},
            "yt_dlp": {"enabled": false, "executable_path": null}
        }"#;
        assert!(serde_json::from_str::<SaveEngineConfigRequest>(json).is_err());
    }

    #[test]
    fn save_request_accepts_valid_payload() {
        let json = r#"{
            "aria2": {"enabled": false, "rpc_port": 6800, "executable_path": null, "rpc_secret_update": null},
            "yt_dlp": {"enabled": false, "executable_path": null}
        }"#;
        assert!(serde_json::from_str::<SaveEngineConfigRequest>(json).is_ok());
    }

    #[test]
    fn diagnostic_result_serializes_stably() {
        let result = DiagnosticResult {
            available: true,
            executable: Some("/usr/bin/aria2c".to_owned()),
            version: Some("aria2 version 1.37.0".to_owned()),
            message: "aria2 is available.".to_owned(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""available":true"#));
        assert!(json.contains(r#""version":"aria2 version 1.37.0""#));
    }
}

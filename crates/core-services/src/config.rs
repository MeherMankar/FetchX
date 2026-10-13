//! Engine configuration model — shared between desktop and CLI.
//!
//! Persisted as a versioned JSON blob in the `app_config` SQLite table.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use fetchx_storage::SqliteConfigRepository;

const CONFIG_KEY: &str = "engine_config";
const CURRENT_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: usize = 65_536;

// ── Domain model ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EngineConfig {
    pub version: u32,
    pub direct_http: DirectHttpConfig,
    pub aria2: Aria2Config,
    pub yt_dlp: YtDlpConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            direct_http: DirectHttpConfig::default(),
            aria2: Aria2Config::default(),
            yt_dlp: YtDlpConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DirectHttpConfig {
    pub enabled: bool,
}
impl Default for DirectHttpConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Aria2Config {
    pub enabled: bool,
    pub executable_path: Option<PathBuf>,
    pub rpc_port: u16,
    /// Never returned to any UI. Stored here for supervisor startup.
    pub rpc_secret: Option<String>,
}

impl Default for Aria2Config {
    fn default() -> Self {
        Self {
            enabled: false,
            executable_path: None,
            rpc_port: 6800,
            rpc_secret: None,
        }
    }
}

impl Aria2Config {
    pub fn has_secret(&self) -> bool {
        self.rpc_secret
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.rpc_port == 0 {
            return Err(ConfigError::InvalidPort);
        }
        if let Some(secret) = &self.rpc_secret {
            if secret.len() > 256 {
                return Err(ConfigError::SecretTooLong);
            }
            if secret.chars().any(char::is_whitespace) {
                return Err(ConfigError::SecretHasWhitespace);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct YtDlpConfig {
    pub enabled: bool,
    pub executable_path: Option<PathBuf>,
}

// ── Validation ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("unsupported configuration version {0}")]
    UnsupportedVersion(u32),
    #[error("aria2 RPC port must be between 1 and 65535")]
    InvalidPort,
    #[error("aria2 RPC secret must not exceed 256 characters")]
    SecretTooLong,
    #[error("aria2 RPC secret must not contain whitespace")]
    SecretHasWhitespace,
    #[error("configuration serialisation failed: {0}")]
    Serialization(String),
    #[error("configuration storage failed: {0}")]
    Storage(String),
}

impl EngineConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CURRENT_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.version));
        }
        self.aria2.validate()?;
        Ok(())
    }
}

// ── Persistence service ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct ConfigService {
    repo: SqliteConfigRepository,
}

impl ConfigService {
    pub fn new(repo: SqliteConfigRepository) -> Self {
        Self { repo }
    }

    pub async fn load(&self) -> Result<EngineConfig, ConfigError> {
        let raw = self
            .repo
            .get(CONFIG_KEY)
            .await
            .map_err(|e| ConfigError::Storage(e.to_string()))?;
        let Some(json) = raw else {
            return Ok(EngineConfig::default());
        };
        if json.len() > MAX_CONFIG_BYTES {
            tracing::warn!("stored engine config exceeds size limit; using defaults");
            return Ok(EngineConfig::default());
        }
        match serde_json::from_str::<EngineConfig>(&json) {
            Ok(cfg) => {
                if cfg.version > CURRENT_VERSION {
                    return Err(ConfigError::UnsupportedVersion(cfg.version));
                }
                Ok(cfg)
            }
            Err(err) => {
                tracing::warn!(%err, "engine config JSON is corrupt; using defaults");
                Ok(EngineConfig::default())
            }
        }
    }

    pub async fn save(&self, config: &EngineConfig) -> Result<(), ConfigError> {
        config.validate()?;
        let json =
            serde_json::to_string(config).map_err(|e| ConfigError::Serialization(e.to_string()))?;
        self.repo
            .set(CONFIG_KEY, &json)
            .await
            .map_err(|e| ConfigError::Storage(e.to_string()))
    }

    pub async fn reset(&self) -> Result<EngineConfig, ConfigError> {
        let defaults = EngineConfig::default();
        self.save(&defaults).await?;
        Ok(defaults)
    }
}

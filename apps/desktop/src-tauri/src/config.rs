//! Engine configuration model and persistence service.
//!
//! Configuration is stored as a versioned JSON blob in the `app_config`
//! SQLite table under the key `"engine_config"`.  The blob is small and
//! its schema is versioned so future additions can migrate safely.
//!
//! # Secret handling
//!
//! The aria2 RPC secret is stored in SQLite like every other config value.
//! SQLite does not encrypt data at rest on its own; the database file is
//! protected by OS file-system permissions (owner-read-only on Unix;
//! user-private on Windows through the `LOCALAPPDATA` directory ACL).
//!
//! The secret is **never** returned to the frontend in any DTO.  The
//! frontend receives only a boolean `has_secret`.  When the user saves a
//! new secret, the new value is validated and stored; the existing value
//! is replaced.
//!
//! If the project later gains a platform-native credential store (e.g.
//! through the Tauri `stronghold` plugin) the storage back-end in
//! `ConfigService::load` / `ConfigService::save` can be swapped without
//! changing the public API.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use fetchx_storage::SqliteConfigRepository;

const CONFIG_KEY: &str = "engine_config";
const CURRENT_VERSION: u32 = 1;
/// Maximum size of the stored JSON blob (64 KiB; well above any realistic value).
const MAX_CONFIG_BYTES: usize = 65_536;

// ============================================================
// Domain model
// ============================================================

/// Top-level configuration persisted as a single versioned JSON object.
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

/// Direct HTTP is always available; no user-editable options currently.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DirectHttpConfig {
    /// Informational only — this engine cannot be disabled.
    pub enabled: bool,
}

impl Default for DirectHttpConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Persistent configuration for the aria2 engine.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Aria2Config {
    pub enabled: bool,
    /// Absolute path to the aria2c executable, or `None` to use PATH discovery.
    pub executable_path: Option<PathBuf>,
    /// RPC listening port (1—65535, default 6800).
    pub rpc_port: u16,
    /// RPC secret token; `None` means no authentication.
    /// Never returned to the frontend — only exposed as `has_secret`.
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
    /// Returns true when the RPC secret is set to a non-empty value.
    pub fn has_secret(&self) -> bool {
        self.rpc_secret
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    }
}

/// Persistent configuration for the yt-dlp engine.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct YtDlpConfig {
    pub enabled: bool,
    /// Absolute path to the yt-dlp executable, or `None` to use PATH discovery.
    pub executable_path: Option<PathBuf>,
}

// ============================================================
// Validation
// ============================================================

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
    #[error("configuration could not be serialised: {0}")]
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

impl Aria2Config {
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

// ============================================================
// Persistence service
// ============================================================

/// Loads and persists `EngineConfig` via `SqliteConfigRepository`.
#[derive(Clone)]
pub struct ConfigService {
    repo: SqliteConfigRepository,
}

impl ConfigService {
    pub fn new(repo: SqliteConfigRepository) -> Self {
        Self { repo }
    }

    /// Loads the stored configuration, falling back to defaults if absent.
    ///
    /// Returns an error only for storage failures or unrecognised versions.
    /// Corrupt JSON is treated as "missing" and defaults are returned.
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
                // Unknown fields in older or same-version JSON are silently
                // ignored by serde, making forward compatibility safe.
                Ok(cfg)
            }
            Err(err) => {
                tracing::warn!(%err, "engine config JSON is corrupt; using defaults");
                Ok(EngineConfig::default())
            }
        }
    }

    /// Validates and persists `config`.
    pub async fn save(&self, config: &EngineConfig) -> Result<(), ConfigError> {
        config.validate()?;
        let json =
            serde_json::to_string(config).map_err(|e| ConfigError::Serialization(e.to_string()))?;
        self.repo
            .set(CONFIG_KEY, &json)
            .await
            .map_err(|e| ConfigError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Replaces the stored configuration with the default and returns it.
    pub async fn reset(&self) -> Result<EngineConfig, ConfigError> {
        let defaults = EngineConfig::default();
        self.save(&defaults).await?;
        Ok(defaults)
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use fetchx_storage::Database;

    use super::*;

    async fn service() -> ConfigService {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        ConfigService::new(db.config())
    }

    #[tokio::test]
    async fn fresh_install_returns_defaults() {
        let svc = service().await;
        let cfg = svc.load().await.unwrap();
        assert_eq!(cfg.version, 1);
        assert!(cfg.direct_http.enabled);
        assert!(!cfg.aria2.enabled);
        assert_eq!(cfg.aria2.rpc_port, 6800);
        assert!(cfg.aria2.rpc_secret.is_none());
        assert!(!cfg.yt_dlp.enabled);
    }

    #[tokio::test]
    async fn save_and_load_round_trips() {
        let svc = service().await;
        let mut cfg = svc.load().await.unwrap();
        cfg.aria2.enabled = true;
        cfg.aria2.rpc_port = 7800;
        cfg.aria2.rpc_secret = Some("supersecret".to_owned());
        cfg.yt_dlp.enabled = true;
        svc.save(&cfg).await.unwrap();

        let loaded = svc.load().await.unwrap();
        assert!(loaded.aria2.enabled);
        assert_eq!(loaded.aria2.rpc_port, 7800);
        assert_eq!(loaded.aria2.rpc_secret, Some("supersecret".to_owned()));
        assert!(loaded.yt_dlp.enabled);
    }

    #[tokio::test]
    async fn reset_restores_defaults() {
        let svc = service().await;
        let mut cfg = svc.load().await.unwrap();
        cfg.aria2.enabled = true;
        cfg.aria2.rpc_port = 7800;
        svc.save(&cfg).await.unwrap();

        let defaults = svc.reset().await.unwrap();
        assert!(!defaults.aria2.enabled);
        assert_eq!(defaults.aria2.rpc_port, 6800);

        let reloaded = svc.load().await.unwrap();
        assert!(!reloaded.aria2.enabled);
    }

    #[test]
    fn invalid_port_zero_is_rejected() {
        let mut cfg = EngineConfig::default();
        cfg.aria2.rpc_port = 0;
        assert!(matches!(cfg.validate(), Err(ConfigError::InvalidPort)));
    }

    #[test]
    fn secret_too_long_is_rejected() {
        let mut cfg = EngineConfig::default();
        cfg.aria2.rpc_secret = Some("a".repeat(257));
        assert!(matches!(cfg.validate(), Err(ConfigError::SecretTooLong)));
    }

    #[test]
    fn secret_with_whitespace_is_rejected() {
        let mut cfg = EngineConfig::default();
        cfg.aria2.rpc_secret = Some("bad secret".to_owned());
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::SecretHasWhitespace)
        ));
    }

    #[test]
    fn valid_config_passes_validation() {
        let mut cfg = EngineConfig::default();
        cfg.aria2.enabled = true;
        cfg.aria2.rpc_port = 6800;
        cfg.aria2.rpc_secret = Some("validtoken".to_owned());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn has_secret_returns_false_when_none() {
        let cfg = Aria2Config::default();
        assert!(!cfg.has_secret());
    }

    #[test]
    fn has_secret_returns_true_when_set() {
        let cfg = Aria2Config {
            rpc_secret: Some("token".to_owned()),
            ..Aria2Config::default()
        };
        assert!(cfg.has_secret());
    }

    #[tokio::test]
    async fn unsupported_version_returns_error() {
        let svc = service().await;
        // Manually store a config with a future version.
        let json = r#"{"version":99,"direct_http":{"enabled":true},"aria2":{"enabled":false,"executable_path":null,"rpc_port":6800,"rpc_secret":null},"yt_dlp":{"enabled":false,"executable_path":null}}"#;
        svc.repo.set(super::CONFIG_KEY, json).await.unwrap();
        let result = svc.load().await;
        assert!(matches!(result, Err(ConfigError::UnsupportedVersion(99))));
    }

    #[tokio::test]
    async fn corrupt_json_falls_back_to_defaults() {
        let svc = service().await;
        svc.repo
            .set(super::CONFIG_KEY, "not valid json {{{{")
            .await
            .unwrap();
        let cfg = svc.load().await.unwrap();
        assert_eq!(cfg.aria2.rpc_port, 6800);
    }
}

//! Engine configuration model — shared between desktop and CLI.
//!
//! Persisted as a versioned JSON blob in the `app_config` SQLite table.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use fetchx_models::ResourcePolicy;
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
    #[serde(default)]
    pub native_torrent: NativeTorrentConfig,
    #[serde(default)]
    pub ftp: FtpConfig,
    #[serde(default)]
    pub sftp: SftpConfig,
    #[serde(default)]
    pub metalink: MetalinkConfig,
    #[serde(default)]
    pub resource_policy: ResourcePolicy,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            direct_http: DirectHttpConfig::default(),
            aria2: Aria2Config::default(),
            yt_dlp: YtDlpConfig::default(),
            native_torrent: NativeTorrentConfig::default(),
            ftp: FtpConfig::default(),
            sftp: SftpConfig::default(),
            metalink: MetalinkConfig::default(),
            resource_policy: ResourcePolicy::default(),
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

impl Aria2Config {}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct YtDlpConfig {
    pub enabled: bool,
    pub executable_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NativeTorrentConfig {
    pub enabled: bool,
    /// TCP listen port for incoming BitTorrent connections. Default: 6881.
    pub listen_port: u16,
    /// Working directory for the librqbit session. Must be app-writable.
    /// On Android this must be set to the app's private files directory
    /// since /data/local/tmp is not writable by the app sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_dir: Option<std::path::PathBuf>,
}

impl Default for NativeTorrentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_port: 6881,
            session_dir: None,
        }
    }
}

// ── FTP / FTPS config ─────────────────────────────────────────────────────────

/// Configuration for the FTP / FTPS engine.
///
/// FTP and FTPS are always-available (pure Rust, no binary dependency).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FtpConfig {
    pub enabled: bool,
    /// Use PASV (passive) mode. Default: `true`.
    pub passive_mode: bool,
}

impl Default for FtpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            passive_mode: true,
        }
    }
}

// ── SFTP config ───────────────────────────────────────────────────────────────

/// Configuration for the SFTP engine.
///
/// SFTP is always-available (russh is pure Rust, no binary dependency).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SftpConfig {
    pub enabled: bool,
}

impl Default for SftpConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

// ── Metalink config ───────────────────────────────────────────────────────────

/// Configuration for the Metalink orchestration engine.
///
/// Metalink is always-available (pure Rust, no binary dependency).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MetalinkConfig {
    pub enabled: bool,
}

impl Default for MetalinkConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

// ── Validation ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("unsupported configuration version {0}")]
    UnsupportedVersion(u32),
    #[error("resource policy is invalid: {0}")]
    ResourcePolicy(String),
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
        self.resource_policy
            .validate()
            .map_err(|error| ConfigError::ResourcePolicy(error.to_string()))?;
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

    /// Persist startup defaults only when no engine configuration exists yet.
    /// Existing user choices are never overwritten by newly installed bundles.
    pub async fn initialize_if_missing(&self, defaults: &EngineConfig) -> Result<(), ConfigError> {
        if self
            .repo
            .get(CONFIG_KEY)
            .await
            .map_err(|error| ConfigError::Storage(error.to_string()))?
            .is_none()
        {
            self.save(defaults).await?;
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use fetchx_storage::Database;

    use super::{ConfigError, ConfigService};

    async fn service() -> ConfigService {
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database should initialize");
        ConfigService::new(database.config())
    }

    #[tokio::test]
    async fn defaults_persist_and_reset() {
        let service = service().await;
        let defaults = service.load().await.expect("defaults should load");
        assert_eq!(defaults.version, 1);
        assert!(!defaults.aria2.enabled);
        assert!(!defaults.yt_dlp.enabled);
        assert!(defaults.native_torrent.enabled);

        let mut changed = defaults;
        changed.aria2.enabled = true;
        service.save(&changed).await.expect("config should save");
        service.reset().await.expect("reset should save defaults");
        assert!(
            !service
                .load()
                .await
                .expect("reset config loads")
                .aria2
                .enabled
        );
        assert!(
            service
                .load()
                .await
                .expect("reset config loads")
                .native_torrent
                .enabled
        );
    }

    #[tokio::test]
    async fn resource_policy_survives_configuration_restart() {
        let service = service().await;
        let mut config = service.load().await.expect("defaults should load");
        config.resource_policy =
            fetchx_models::ResourcePolicy::for_profile(fetchx_models::ResourceProfile::Night);
        service
            .save(&config)
            .await
            .expect("resource profile should persist");
        assert_eq!(
            service
                .load()
                .await
                .expect("resource profile should load")
                .resource_policy,
            config.resource_policy
        );
    }

    #[tokio::test]
    async fn corrupt_configuration_falls_back_but_future_version_fails() {
        let service = service().await;
        service
            .repo
            .set(super::CONFIG_KEY, "not valid JSON")
            .await
            .expect("corrupt value should be storable");
        assert_eq!(
            service
                .load()
                .await
                .expect("corrupt config falls back")
                .version,
            1
        );

        let future = r#"{"version":99,"direct_http":{"enabled":true},"aria2":{"enabled":false,"executable_path":null,"rpc_port":6800,"rpc_secret":null},"yt_dlp":{"enabled":false,"executable_path":null}}"#;
        service
            .repo
            .set(super::CONFIG_KEY, future)
            .await
            .expect("future config should be storable");
        assert!(matches!(
            service.load().await,
            Err(ConfigError::UnsupportedVersion(99))
        ));
    }
}

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use thiserror::Error;

pub const DEFAULT_PORT: u16 = 39_800;
const MAX_CONNECTIONS: usize = 128;
const MAX_TIMEOUT_SECS: u64 = 300;
const MAX_EVENT_BACKLOG: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsConfig {
    pub certificate_path: PathBuf,
    pub private_key_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub bind_ip: IpAddr,
    pub port: u16,
    pub database_path: PathBuf,
    pub downloads_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub max_request_bytes: usize,
    pub max_connections: usize,
    pub request_timeout: Duration,
    /// Maximum queued notifications per WebSocket client before it is disconnected.
    pub event_backlog: usize,
    pub tls: Option<TlsConfig>,
    pub allowed_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let data_dir = default_data_dir();
        Self {
            bind_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            database_path: data_dir.join("fetchx.db"),
            downloads_dir: default_downloads_dir(),
            runtime_dir: data_dir.join("runtime"),
            max_request_bytes: fetchx_remote_protocol::MAX_MESSAGE_BYTES,
            max_connections: 32,
            request_timeout: Duration::from_secs(30),
            event_backlog: 256,
            tls: None,
            allowed_origins: vec!["http://127.0.0.1".to_owned(), "http://localhost".to_owned()],
        }
    }
}

impl ServerConfig {
    pub fn validate(&self) -> Result<(), ServerConfigError> {
        if self.max_request_bytes == 0
            || self.max_request_bytes > fetchx_remote_protocol::MAX_MESSAGE_BYTES
        {
            return Err(ServerConfigError::RequestSize);
        }
        if self.max_connections == 0 || self.max_connections > MAX_CONNECTIONS {
            return Err(ServerConfigError::ConnectionLimit);
        }
        if self.request_timeout.is_zero()
            || self.request_timeout > Duration::from_secs(MAX_TIMEOUT_SECS)
        {
            return Err(ServerConfigError::RequestTimeout);
        }
        if self.event_backlog == 0 || self.event_backlog > MAX_EVENT_BACKLOG {
            return Err(ServerConfigError::EventBacklog);
        }
        if let Some(tls) = &self.tls {
            if tls.certificate_path.as_os_str().is_empty()
                || tls.private_key_path.as_os_str().is_empty()
            {
                return Err(ServerConfigError::TlsConfiguration);
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_ip, self.port)
    }
}

fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("com.fetchx.desktop")
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Application Support")
            .join("com.fetchx.desktop")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join(".local")
                    .join("share")
            })
            .join("FetchX")
    }
}

fn default_downloads_dir() -> PathBuf {
    std::env::var_os(if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .map(PathBuf::from)
    .unwrap_or_else(std::env::temp_dir)
    .join("Downloads")
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ServerConfigError {
    #[error("request size limit must be between 1 byte and the protocol maximum")]
    RequestSize,
    #[error("connection limit must be between 1 and 128")]
    ConnectionLimit,
    #[error("request timeout must be between 1 and 300 seconds")]
    RequestTimeout,
    #[error("event backlog must be between 1 and 2048")]
    EventBacklog,
    #[error("TLS requires a certificate and private key path")]
    TlsConfiguration,
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use super::{DEFAULT_PORT, ServerConfig, ServerConfigError};

    #[test]
    fn defaults_bind_only_to_localhost() {
        let config = ServerConfig::default();
        assert_eq!(config.bind_ip, Ipv4Addr::LOCALHOST);
        assert_eq!(config.port, DEFAULT_PORT);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn unsafe_resource_limits_are_rejected() {
        let connection_limit = ServerConfig {
            max_connections: 129,
            ..ServerConfig::default()
        };
        assert_eq!(
            connection_limit.validate(),
            Err(ServerConfigError::ConnectionLimit)
        );

        let request_timeout = ServerConfig {
            request_timeout: Duration::from_secs(301),
            ..ServerConfig::default()
        };
        assert_eq!(
            request_timeout.validate(),
            Err(ServerConfigError::RequestTimeout)
        );
    }

    #[test]
    fn tls_requires_certificate_and_key_paths() {
        let config = ServerConfig {
            tls: Some(super::TlsConfig {
                certificate_path: Default::default(),
                private_key_path: Default::default(),
            }),
            ..ServerConfig::default()
        };
        assert_eq!(config.validate(), Err(ServerConfigError::TlsConfiguration));
    }

    #[test]
    fn valid_tls_paths_are_accepted() {
        let config = ServerConfig {
            tls: Some(super::TlsConfig {
                certificate_path: "certificate.pem".into(),
                private_key_path: "private-key.pem".into(),
            }),
            ..ServerConfig::default()
        };
        assert!(config.validate().is_ok());
    }
}

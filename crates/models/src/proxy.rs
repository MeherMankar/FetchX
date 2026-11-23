use serde::{Deserialize, Serialize};

/// Proxy protocol kind for desktop downloads.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    Http,
    Https,
    Socks5,
}

impl ProxyKind {
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
        }
    }
}

/// Desktop proxy configuration applied to Direct HTTP and HLS downloads.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct DesktopProxyConfig {
    pub kind: Option<ProxyKind>,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// Password is stored in the encrypted credential store, not serialized here.
    #[serde(skip)]
    pub password: Option<String>,
}

impl DesktopProxyConfig {
    pub fn is_enabled(&self) -> bool {
        self.kind.is_some() && !self.host.is_empty() && self.port > 0
    }

    pub fn proxy_url(&self) -> Option<String> {
        let kind = self.kind?;
        if self.host.is_empty() || self.port == 0 {
            return None;
        }
        let host = self.host.trim();
        let host = if host.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        Some(format!("{}://{host}:{}", kind.scheme(), self.port))
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let Some(kind) = self.kind else {
            return Ok(());
        };
        if self.host.trim().is_empty() || self.host.trim() != self.host || self.port == 0 {
            return Err("proxy host and port are required");
        }
        let url = url::Url::parse(&self.proxy_url().ok_or("proxy host and port are required")?)
            .map_err(|_| "proxy address is invalid")?;
        if url.scheme() != kind.scheme()
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !(url.path().is_empty() || url.path() == "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("proxy address is invalid");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopProxyConfig, ProxyKind};

    #[test]
    fn validates_proxy_addresses_and_formats_ipv6() {
        let proxy = DesktopProxyConfig {
            kind: Some(ProxyKind::Socks5),
            host: "::1".to_owned(),
            port: 1080,
            ..DesktopProxyConfig::default()
        };
        assert_eq!(proxy.proxy_url().as_deref(), Some("socks5://[::1]:1080"));
        assert!(proxy.validate().is_ok());
    }

    #[test]
    fn rejects_proxy_urls_with_credentials_or_extra_path() {
        for host in ["user@proxy.example", "proxy.example/path"] {
            let proxy = DesktopProxyConfig {
                kind: Some(ProxyKind::Http),
                host: host.to_owned(),
                port: 8080,
                ..DesktopProxyConfig::default()
            };
            assert!(proxy.validate().is_err(), "{host}");
        }
    }
}

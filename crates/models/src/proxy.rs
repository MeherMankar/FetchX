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

/// Desktop proxy configuration applied to all HTTP-based download engines.
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
        Some(format!("{}://{}:{}", kind.scheme(), self.host, self.port))
    }
}

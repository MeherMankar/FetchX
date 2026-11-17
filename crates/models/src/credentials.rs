
/// Key for storing HTTP credentials in the encrypted credential store.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CredentialKey {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl CredentialKey {
    pub fn new(scheme: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            scheme: scheme.into(),
            host: host.into(),
            port,
        }
    }
}

/// HTTP authentication credentials. The password is never serialized to logs or export files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpCredentials {
    pub username: String,
    pub password: String,
}

impl HttpCredentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

/// Marker type to ensure credentials are never accidentally serialized.
/// HttpCredentials does NOT implement Serialize/Deserialize by design.
impl std::fmt::Display for HttpCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HttpCredentials(username={}, password=***)",
            self.username
        )
    }
}

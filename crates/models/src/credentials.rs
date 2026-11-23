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
#[derive(Clone, Eq, PartialEq)]
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

impl std::fmt::Debug for HttpCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HttpCredentials(***)")
    }
}

impl std::fmt::Display for HttpCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpCredentials(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::HttpCredentials;

    #[test]
    fn formatting_never_exposes_credentials() {
        let credentials = HttpCredentials::new("private-user", "secret-password");
        assert_eq!(format!("{credentials:?}"), "HttpCredentials(***)");
        assert_eq!(format!("{credentials}"), "HttpCredentials(***)");
    }
}

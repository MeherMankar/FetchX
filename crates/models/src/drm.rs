use serde::{Deserialize, Serialize};

/// DRM key pair with hex-encoded key ID and key value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DrmKey {
    /// Hex-encoded key ID (kid), typically 32 hex chars.
    pub kid: String,
    /// Hex-encoded content key, typically 32 hex chars.
    pub k: String,
}

/// DRM configuration carrying extracted content keys and an optional license server URL.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DrmConfig {
    pub keys: Vec<DrmKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_server_url: Option<String>,
}

impl DrmConfig {
    /// Returns `true` when no keys have been stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Finds a key whose `kid` matches `kid_hex` (case-insensitive).
    #[must_use]
    pub fn find_key(&self, kid_hex: &str) -> Option<&DrmKey> {
        self.keys
            .iter()
            .find(|k| k.kid.eq_ignore_ascii_case(kid_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_key_is_case_insensitive() {
        let config = DrmConfig {
            keys: vec![DrmKey {
                kid: "AABBCCDD".to_owned(),
                k: "00112233445566778899aabbccddeeff".to_owned(),
            }],
            license_server_url: None,
        };
        assert!(config.find_key("aabbccdd").is_some());
        assert!(config.find_key("AABBCCDD").is_some());
        assert!(config.find_key("11223344").is_none());
    }

    #[test]
    fn is_empty_reflects_key_count() {
        let empty = DrmConfig::default();
        assert!(empty.is_empty());
        let nonempty = DrmConfig {
            keys: vec![DrmKey {
                kid: "aa".to_owned(),
                k: "bb".to_owned(),
            }],
            license_server_url: None,
        };
        assert!(!nonempty.is_empty());
    }

    #[test]
    fn drm_config_round_trips_serde() {
        let config = DrmConfig {
            keys: vec![DrmKey {
                kid: "deadbeef".to_owned(),
                k: "cafebabe".to_owned(),
            }],
            license_server_url: Some("https://license.example.com".to_owned()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: DrmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }

    #[test]
    fn license_server_url_is_omitted_when_none() {
        let config = DrmConfig {
            keys: vec![],
            license_server_url: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("license_server_url"));
    }
}

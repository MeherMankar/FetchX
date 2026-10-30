//! Strict, versioned plugin manifest types and validation.

use std::{collections::HashSet, fmt, path::PathBuf, str::FromStr};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

/// Plugin API version currently supported by the host.
pub const PLUGIN_API_VERSION: u32 = 1;
/// Manifest schema version currently supported by the host.
pub const PLUGIN_MANIFEST_VERSION: u32 = 1;
/// Maximum allowed size of a manifest file in bytes (16 KiB).
pub const MAX_MANIFEST_BYTES: usize = 16_384;

/// A parsed semantic version. Invalid or non-SemVer input cannot be represented.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PluginVersion(Version);

impl PluginVersion {
    pub fn parse(value: &str) -> Result<Self, semver::Error> {
        Version::parse(value).map(Self)
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for PluginVersion {
    type Err = semver::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for PluginVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PluginVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

/// Permissions are coarse user-consent categories; they never grant an
/// operation by themselves. A capability must also be explicitly approved.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermission {
    Network,
    FilesystemRead,
    FilesystemWrite,
    Downloads,
    Metadata,
    Process,
    Engine,
    Settings,
}

/// An individually authorizable plugin operation.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    NetworkHttpRequest,
    FilesystemRead,
    FilesystemWrite,
    DownloadCreate,
    DownloadList,
    DownloadPause,
    DownloadResume,
    DownloadCancel,
    DownloadRemove,
    MetadataRead,
    ProcessSpawn,
    EngineDirectHttp,
    EngineAria2,
    EngineYtDlp,
    EngineFfmpeg,
    SettingsRead,
    SettingsWrite,
}

impl PluginCapability {
    /// Returns the coarse permission that must also be declared and approved.
    pub fn required_permission(&self) -> PluginPermission {
        match self {
            Self::NetworkHttpRequest => PluginPermission::Network,
            Self::FilesystemRead => PluginPermission::FilesystemRead,
            Self::FilesystemWrite => PluginPermission::FilesystemWrite,
            Self::DownloadCreate
            | Self::DownloadList
            | Self::DownloadPause
            | Self::DownloadResume
            | Self::DownloadCancel
            | Self::DownloadRemove => PluginPermission::Downloads,
            Self::MetadataRead => PluginPermission::Metadata,
            Self::ProcessSpawn => PluginPermission::Process,
            Self::EngineDirectHttp | Self::EngineAria2 | Self::EngineYtDlp | Self::EngineFfmpeg => {
                PluginPermission::Engine
            }
            Self::SettingsRead | Self::SettingsWrite => PluginPermission::Settings,
        }
    }
}

/// Host platform supported by a plugin.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
    Android,
    #[serde(rename = "any")]
    Any,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCompatibility {
    pub minimum_api_version: u32,
    pub maximum_api_version: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSignatureAlgorithm {
    Ed25519,
}

pub const PLUGIN_SIGNATURE_VERSION: u32 = 1;

/// Ed25519 signature metadata for a signed plugin artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSignature {
    pub version: u32,
    pub algorithm: PluginSignatureAlgorithm,
    pub publisher_id: String,
    pub key_id: String,
    /// SHA-256 of the installed plugin artifact tree, excluding plugin.json.
    pub package_sha256: String,
    /// Standard base64-encoded signature bytes.
    pub signature: String,
}

/// A validated plugin manifest. Parse untrusted JSON with [`parse_manifest`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: PluginVersion,
    pub api_version: u32,
    pub compatibility: PluginCompatibility,
    pub publisher: String,
    pub description: String,
    pub executable: PathBuf,
    pub permissions: Vec<PluginPermission>,
    pub capabilities: Vec<PluginCapability>,
    pub platforms: Vec<Platform>,
    pub signature: Option<PluginSignature>,
}

impl PluginManifest {
    /// Revalidates values created outside the manifest parser.
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        let raw = PluginManifestRaw {
            manifest_version: self.manifest_version,
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.to_string(),
            api_version: self.api_version,
            compatibility: self.compatibility.clone(),
            publisher: self.publisher.clone(),
            description: self.description.clone(),
            executable: self.executable.clone(),
            permissions: self.permissions.clone(),
            capabilities: self.capabilities.clone(),
            platforms: self.platforms.clone(),
            signature: self.signature.clone(),
        };
        let validated = raw.validate()?;
        if validated != *self {
            return Err(ManifestValidationError::InvalidManifest);
        }
        Ok(())
    }
}

/// Raw deserialized manifest. It is not trusted until [`Self::validate`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestRaw {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub compatibility: PluginCompatibility,
    pub publisher: String,
    pub description: String,
    pub executable: PathBuf,
    pub permissions: Vec<PluginPermission>,
    pub capabilities: Vec<PluginCapability>,
    #[serde(default)]
    pub platforms: Vec<Platform>,
    #[serde(default)]
    pub signature: Option<PluginSignature>,
}

impl PluginManifestRaw {
    pub fn validate(self) -> Result<PluginManifest, ManifestValidationError> {
        if self.manifest_version != PLUGIN_MANIFEST_VERSION {
            return Err(ManifestValidationError::UnsupportedManifestVersion(
                self.manifest_version,
            ));
        }
        if self.id.len() < 2
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(ManifestValidationError::InvalidId(self.id));
        }
        for (field, value) in [
            ("name", self.name.as_str()),
            ("publisher", self.publisher.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ManifestValidationError::EmptyField(field));
            }
        }
        if self.description.len() > 256 {
            return Err(ManifestValidationError::DescriptionTooLong);
        }
        if self.api_version != PLUGIN_API_VERSION {
            return Err(ManifestValidationError::UnsupportedApiVersion(
                self.api_version,
            ));
        }
        if self.compatibility.minimum_api_version > self.compatibility.maximum_api_version
            || !(self.compatibility.minimum_api_version..=self.compatibility.maximum_api_version)
                .contains(&PLUGIN_API_VERSION)
        {
            return Err(ManifestValidationError::IncompatibleApiRange);
        }
        let version = PluginVersion::parse(&self.version)
            .map_err(|_| ManifestValidationError::InvalidVersion(self.version.clone()))?;
        if !self.executable.is_absolute() {
            return Err(ManifestValidationError::ExecutableNotAbsolute(
                self.executable.to_string_lossy().into_owned(),
            ));
        }
        use std::path::Component;
        if self
            .executable
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(ManifestValidationError::PathTraversal(
                self.executable.to_string_lossy().into_owned(),
            ));
        }
        if self.capabilities.is_empty() {
            return Err(ManifestValidationError::NoCapabilities);
        }
        ensure_unique(&self.permissions, "permissions")?;
        ensure_unique(&self.capabilities, "capabilities")?;
        ensure_unique(&self.platforms, "platforms")?;
        for capability in &self.capabilities {
            if !self.permissions.contains(&capability.required_permission()) {
                return Err(ManifestValidationError::PermissionNotDeclared(
                    capability.clone(),
                ));
            }
        }
        if let Some(signature) = &self.signature {
            validate_signature(signature)?;
        }

        Ok(PluginManifest {
            manifest_version: self.manifest_version,
            id: self.id,
            name: self.name,
            version,
            api_version: self.api_version,
            compatibility: self.compatibility,
            publisher: self.publisher,
            description: self.description,
            executable: self.executable,
            permissions: self.permissions,
            capabilities: self.capabilities,
            platforms: self.platforms,
            signature: self.signature,
        })
    }
}

fn ensure_unique<T: Eq + std::hash::Hash>(
    values: &[T],
    field: &'static str,
) -> Result<(), ManifestValidationError> {
    if values.iter().collect::<HashSet<_>>().len() != values.len() {
        return Err(ManifestValidationError::DuplicateValue(field));
    }
    Ok(())
}

fn validate_signature(signature: &PluginSignature) -> Result<(), ManifestValidationError> {
    if signature.version != PLUGIN_SIGNATURE_VERSION {
        return Err(ManifestValidationError::UnsupportedSignatureVersion(
            signature.version,
        ));
    }
    if signature.publisher_id.len() < 2
        || signature.publisher_id.len() > 64
        || !signature.publisher_id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(ManifestValidationError::InvalidPublisherId(
            signature.publisher_id.clone(),
        ));
    }
    if signature.key_id.trim().is_empty() {
        return Err(ManifestValidationError::EmptyField("signature.key_id"));
    }
    if !is_sha256_hex(&signature.key_id) || !is_sha256_hex(&signature.package_sha256) {
        return Err(ManifestValidationError::MalformedSignature);
    }
    let bytes = STANDARD
        .decode(&signature.signature)
        .map_err(|_| ManifestValidationError::MalformedSignature)?;
    if bytes.len() != 64 {
        return Err(ManifestValidationError::MalformedSignature);
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ManifestValidationError {
    #[error("plugin ID `{0}` is invalid (must match ^[a-z0-9_-]{{2,64}}$)")]
    InvalidId(String),
    #[error("required field `{0}` must not be empty")]
    EmptyField(&'static str),
    #[error("publisher ID `{0}` is invalid")]
    InvalidPublisherId(String),
    #[error("description exceeds the 256 character limit")]
    DescriptionTooLong,
    #[error("manifest version {0} is not supported (expected {PLUGIN_MANIFEST_VERSION})")]
    UnsupportedManifestVersion(u32),
    #[error("api_version {0} is not supported (expected {PLUGIN_API_VERSION})")]
    UnsupportedApiVersion(u32),
    #[error("host API compatibility range does not include the current API")]
    IncompatibleApiRange,
    #[error("plugin version `{0}` is not valid semantic versioning")]
    InvalidVersion(String),
    #[error("executable path `{0}` must be absolute")]
    ExecutableNotAbsolute(String),
    #[error("executable path `{0}` contains path traversal components")]
    PathTraversal(String),
    #[error("plugin signature version {0} is not supported")]
    UnsupportedSignatureVersion(u32),
    #[error("manifest must declare at least one capability")]
    NoCapabilities,
    #[error("capability {0:?} has no matching declared permission")]
    PermissionNotDeclared(PluginCapability),
    #[error("manifest field `{0}` contains duplicate values")]
    DuplicateValue(&'static str),
    #[error("signature metadata is malformed")]
    MalformedSignature,
    #[error("manifest values are invalid")]
    InvalidManifest,
    #[error("manifest JSON is invalid: {0}")]
    Json(String),
    #[error("manifest file is too large (max {MAX_MANIFEST_BYTES} bytes)")]
    FileTooLarge,
}

pub fn parse_manifest(bytes: &[u8]) -> Result<PluginManifest, ManifestValidationError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestValidationError::FileTooLarge);
    }
    let raw: PluginManifestRaw =
        serde_json::from_slice(bytes).map_err(|e| ManifestValidationError::Json(e.to_string()))?;
    raw.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_raw(id: &str) -> PluginManifestRaw {
        PluginManifestRaw {
            manifest_version: PLUGIN_MANIFEST_VERSION,
            id: id.to_owned(),
            name: "Test Plugin".to_owned(),
            version: "0.1.0".to_owned(),
            api_version: PLUGIN_API_VERSION,
            compatibility: PluginCompatibility {
                minimum_api_version: PLUGIN_API_VERSION,
                maximum_api_version: PLUGIN_API_VERSION,
            },
            publisher: "Test Publisher".to_owned(),
            description: "A test plugin".to_owned(),
            executable: if cfg!(windows) {
                PathBuf::from(r"C:\plugins\test\test-plugin.exe")
            } else {
                PathBuf::from("/opt/plugins/test/test-plugin")
            },
            permissions: vec![PluginPermission::Engine],
            capabilities: vec![PluginCapability::EngineDirectHttp],
            platforms: vec![],
            signature: None,
        }
    }

    #[test]
    fn valid_manifest_passes_and_semver_is_typed() {
        let manifest = minimal_raw("my-plugin").validate().unwrap();
        assert_eq!(manifest.version.to_string(), "0.1.0");
    }

    #[test]
    fn invalid_semver_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.version = "release".to_owned();
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::InvalidVersion(_))
        ));
    }

    #[test]
    fn wrong_manifest_version_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.manifest_version = 99;
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::UnsupportedManifestVersion(99))
        ));
    }

    #[test]
    fn unsupported_api_version_and_empty_capabilities_are_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.api_version = 99;
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::UnsupportedApiVersion(99))
        ));

        let mut raw = minimal_raw("my-plugin");
        raw.capabilities.clear();
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::NoCapabilities
        );
    }

    #[test]
    fn invalid_ids_are_rejected() {
        for id in ["", "a", "MyPlugin", "my plugin"] {
            assert!(matches!(
                minimal_raw(id).validate(),
                Err(ManifestValidationError::InvalidId(_))
            ));
        }
    }

    #[test]
    fn invalid_api_compatibility_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.compatibility.minimum_api_version = 2;
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::IncompatibleApiRange
        );
    }

    #[test]
    fn absent_permission_for_capability_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.permissions.clear();
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::PermissionNotDeclared(_))
        ));
    }

    #[test]
    fn duplicate_permissions_and_capabilities_are_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.permissions.push(PluginPermission::Engine);
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::DuplicateValue("permissions")
        );
        let mut raw = minimal_raw("my-plugin");
        raw.capabilities.push(PluginCapability::EngineDirectHttp);
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::DuplicateValue("capabilities")
        );
    }

    #[test]
    fn empty_name_and_publisher_are_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.name = " ".to_owned();
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::EmptyField("name")
        );
        let mut raw = minimal_raw("my-plugin");
        raw.publisher.clear();
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::EmptyField("publisher")
        );
    }

    #[test]
    fn description_and_absolute_path_rules_are_enforced() {
        let mut raw = minimal_raw("my-plugin");
        raw.description = "x".repeat(257);
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::DescriptionTooLong
        );
        let mut raw = minimal_raw("my-plugin");
        raw.executable = PathBuf::from("relative/path/plugin");
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::ExecutableNotAbsolute(_))
        ));
    }

    #[test]
    fn path_traversal_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.executable = if cfg!(windows) {
            PathBuf::from(r"C:\plugins\..\system32\cmd.exe")
        } else {
            PathBuf::from("/opt/plugins/../../../bin/sh")
        };
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::PathTraversal(_))
        ));
    }

    #[test]
    fn malformed_signature_metadata_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.signature = Some(PluginSignature {
            version: PLUGIN_SIGNATURE_VERSION,
            algorithm: PluginSignatureAlgorithm::Ed25519,
            publisher_id: "test.publisher".to_owned(),
            key_id: "00".repeat(32),
            package_sha256: "00".repeat(32),
            signature: "not-base64".to_owned(),
        });
        assert_eq!(
            raw.validate().unwrap_err(),
            ManifestValidationError::MalformedSignature
        );
    }

    #[test]
    fn valid_signature_shape_is_not_verified_by_manifest_parsing() {
        let mut raw = minimal_raw("my-plugin");
        raw.signature = Some(PluginSignature {
            version: PLUGIN_SIGNATURE_VERSION,
            algorithm: PluginSignatureAlgorithm::Ed25519,
            publisher_id: "test.publisher".to_owned(),
            key_id: "00".repeat(32),
            package_sha256: "00".repeat(32),
            signature: STANDARD.encode([0_u8; 64]),
        });
        assert!(raw.validate().is_ok());
    }

    #[test]
    fn unknown_fields_and_oversized_manifests_are_rejected() {
        let mut raw = serde_json::to_value(minimal_raw("my-plugin")).unwrap();
        raw.as_object_mut()
            .unwrap()
            .insert("unexpected_field".to_owned(), serde_json::Value::Bool(true));
        let json = serde_json::to_vec(&raw).unwrap();
        assert!(matches!(
            parse_manifest(&json),
            Err(ManifestValidationError::Json(_))
        ));
        assert_eq!(
            parse_manifest(&vec![b'x'; MAX_MANIFEST_BYTES + 1]).unwrap_err(),
            ManifestValidationError::FileTooLarge
        );
    }
}

//! Untrusted plugin catalog metadata and its local security assessment.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{
    PLUGIN_SIGNATURE_VERSION, Platform, PluginCapability, PluginManifest, PluginPermission,
    PluginPolicy, PluginPolicyDecision, PluginPublisherStore, PluginSignature, PluginSigningError,
    PluginTrust, PluginTrustContext, PluginVersion, PolicyEnvironment, manifest_fingerprint,
};

const MAX_REGISTRY_ENTRIES: usize = 10_000;
const MAX_REGISTRY_BYTES: usize = 4 * 1024 * 1024;
const MAX_RELEASES_PER_PLUGIN: usize = 500;
const MAX_DOWNLOADS_PER_RELEASE: usize = 64;
const MAX_COMPATIBILITY_TARGETS: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PluginRegistryId(String);

impl PluginRegistryId {
    pub fn parse(value: impl Into<String>) -> Result<Self, PluginRegistryError> {
        let value = value.into();
        if value.len() < 2
            || value.len() > 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
        {
            return Err(PluginRegistryError::InvalidId(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), PluginRegistryError> {
        Self::parse(self.0.clone()).map(|_| ())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryPublisher {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginArchitecture {
    Any,
    X86,
    X86_64,
    Armv7,
    Aarch64,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryTarget {
    pub platform: Platform,
    pub architecture: PluginArchitecture,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryCompatibility {
    pub minimum_fetchx_version: PluginVersion,
    pub maximum_fetchx_version: Option<PluginVersion>,
    pub targets: Vec<PluginRegistryTarget>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDownload {
    pub url: String,
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub target: PluginRegistryTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryRelease {
    pub version: PluginVersion,
    pub release_notes: String,
    pub downloads: Vec<PluginDownload>,
    /// An unsigned catalog may mirror the plugin manifest's signature metadata.
    /// It is never proof; the installed signed manifest is authoritative.
    pub signature: Option<PluginSignature>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryEntry {
    pub plugin_id: PluginRegistryId,
    pub name: String,
    pub description: String,
    pub publisher: PluginRegistryPublisher,
    pub permissions: Vec<PluginPermission>,
    pub capabilities: Vec<PluginCapability>,
    pub compatibility: PluginRegistryCompatibility,
    pub releases: Vec<PluginRegistryRelease>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryCatalog {
    pub entries: Vec<PluginRegistryEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum PluginRegistryError {
    #[error("registry entry ID `{0}` is invalid")]
    InvalidId(String),
    #[error("registry entry `{0}` is duplicated")]
    DuplicatePlugin(String),
    #[error("publisher `{0}` has conflicting registry identities")]
    ConflictingPublisher(String),
    #[error("plugin `{plugin_id}` has duplicate release version `{version}`")]
    DuplicateRelease { plugin_id: String, version: String },
    #[error("plugin `{plugin_id}` has invalid metadata: {reason}")]
    InvalidEntry { plugin_id: String, reason: String },
    #[error("registry contains too many entries")]
    TooManyEntries,
    #[error("registry response is unavailable: {0}")]
    Unavailable(String),
    #[error("registry response is invalid: {0}")]
    InvalidResponse(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrySignatureStatus {
    Unsigned,
    Invalid,
    ValidUntrusted,
    Verified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryPublisherStatus {
    Unknown,
    Untrusted,
    Trusted,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryRevocationStatus {
    NotRevoked,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryArtifactStatus {
    Unknown,
    Valid,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryManifestStatus {
    Match,
    Mismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryCompatibilityStatus {
    Compatible,
    Incompatible,
}

/// Computed security state; intentionally cannot be deserialized from a catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginRegistryAssessment {
    pub signature: RegistrySignatureStatus,
    pub publisher: RegistryPublisherStatus,
    pub revocation: RegistryRevocationStatus,
    pub artifact: RegistryArtifactStatus,
    pub manifest: RegistryManifestStatus,
    pub compatibility: RegistryCompatibilityStatus,
    pub manifest_sha256: String,
    pub policy_decision: Option<PluginPolicyDecision>,
    /// Artifact admission is not a plugin installation or runtime authorization.
    pub artifact_admission_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginRegistryEnvironment {
    pub fetchx_version: PluginVersion,
    pub platform: Platform,
    pub architecture: PluginArchitecture,
}

pub struct PluginRegistryAssessmentContext<'a> {
    pub publisher_store: &'a PluginPublisherStore,
    pub environment: &'a PluginRegistryEnvironment,
    pub approvals: Option<&'a PluginTrustContext>,
    pub policy_environment: &'a PolicyEnvironment,
}

impl PluginRegistryEnvironment {
    pub fn current(fetchx_version: PluginVersion) -> Self {
        #[cfg(target_os = "windows")]
        let platform = Platform::Windows;
        #[cfg(target_os = "android")]
        let platform = Platform::Android;
        #[cfg(target_os = "macos")]
        let platform = Platform::Macos;
        #[cfg(all(
            not(target_os = "windows"),
            not(target_os = "android"),
            not(target_os = "macos")
        ))]
        let platform = Platform::Linux;

        #[cfg(target_arch = "x86")]
        let architecture = PluginArchitecture::X86;
        #[cfg(target_arch = "x86_64")]
        let architecture = PluginArchitecture::X86_64;
        #[cfg(target_arch = "arm")]
        let architecture = PluginArchitecture::Armv7;
        #[cfg(target_arch = "aarch64")]
        let architecture = PluginArchitecture::Aarch64;
        #[cfg(not(any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64"
        )))]
        let architecture = PluginArchitecture::Any;

        Self {
            fetchx_version,
            platform,
            architecture,
        }
    }
}

impl PluginRegistryEntry {
    pub fn validate(&self) -> Result<(), PluginRegistryError> {
        self.plugin_id.validate()?;
        let invalid = |reason: &str| PluginRegistryError::InvalidEntry {
            plugin_id: self.plugin_id.as_str().to_owned(),
            reason: reason.to_owned(),
        };
        if self.name.trim().is_empty() || self.name.len() > 128 {
            return Err(invalid("name must be non-empty and at most 128 bytes"));
        }
        if self.description.trim().is_empty() || self.description.len() > 4096 {
            return Err(invalid(
                "description must be non-empty and at most 4096 bytes",
            ));
        }
        if !valid_publisher_id(&self.publisher.id)
            || self.publisher.display_name.trim().is_empty()
            || self.publisher.display_name.len() > 128
        {
            return Err(invalid("publisher identity or display name is invalid"));
        }
        if self.permissions.is_empty() || contains_duplicates(&self.permissions) {
            return Err(invalid("permissions must be non-empty and unique"));
        }
        if self.capabilities.is_empty() || contains_duplicates(&self.capabilities) {
            return Err(invalid("capabilities must be non-empty and unique"));
        }
        for capability in &self.capabilities {
            if !self.permissions.contains(&capability.required_permission()) {
                return Err(invalid("capability is missing its required permission"));
            }
        }
        let compatibility = &self.compatibility;
        if compatibility.targets.is_empty()
            || compatibility.targets.len() > MAX_COMPATIBILITY_TARGETS
            || contains_duplicates(&compatibility.targets)
            || compatibility
                .maximum_fetchx_version
                .as_ref()
                .is_some_and(|maximum| maximum < &compatibility.minimum_fetchx_version)
        {
            return Err(invalid("compatibility metadata is missing or invalid"));
        }
        if self.releases.is_empty() || self.releases.len() > MAX_RELEASES_PER_PLUGIN {
            return Err(invalid("release list is missing or too large"));
        }
        let mut versions = BTreeSet::new();
        for release in &self.releases {
            let version = release.version.to_string();
            if !versions.insert(version.clone()) {
                return Err(PluginRegistryError::DuplicateRelease {
                    plugin_id: self.plugin_id.as_str().to_owned(),
                    version,
                });
            }
            if release.release_notes.len() > 16_384
                || release.downloads.is_empty()
                || release.downloads.len() > MAX_DOWNLOADS_PER_RELEASE
            {
                return Err(invalid("release notes or downloads are invalid"));
            }
            let mut download_targets = Vec::new();
            for download in &release.downloads {
                validate_download(download).map_err(|reason| invalid(&reason))?;
                if download_targets.contains(&download.target) {
                    return Err(invalid("release contains duplicate download targets"));
                }
                download_targets.push(download.target.clone());
                if !compatibility.targets.contains(&download.target) {
                    return Err(invalid("download target is not declared compatible"));
                }
            }
            if release.signature.as_ref().is_some_and(|signature| {
                signature.version != PLUGIN_SIGNATURE_VERSION
                    || signature.publisher_id != self.publisher.id
            }) {
                return Err(invalid(
                    "release signature version or publisher does not match the entry",
                ));
            }
        }
        Ok(())
    }

    pub fn release(&self, version: &PluginVersion) -> Option<&PluginRegistryRelease> {
        self.releases
            .iter()
            .find(|release| &release.version == version)
    }

    pub fn assess_release(
        &self,
        release: &PluginRegistryRelease,
        download: &PluginDownload,
        manifest: &PluginManifest,
        actual_artifact_sha256: &str,
        context: &PluginRegistryAssessmentContext<'_>,
    ) -> Result<PluginRegistryAssessment, PluginRegistryError> {
        self.validate()?;
        manifest
            .validate()
            .map_err(|error| PluginRegistryError::InvalidResponse(error.to_string()))?;
        if !self.releases.contains(release) || !release.downloads.contains(download) {
            return Err(PluginRegistryError::InvalidResponse(
                "release or download does not belong to this catalog entry".to_owned(),
            ));
        }
        let compatibility = release_compatible(self, release, manifest, context.environment);
        let manifest_matches = manifest.id == self.plugin_id.as_str()
            && manifest.name == self.name
            && manifest.description == self.description
            && manifest.version == release.version
            && manifest.publisher == self.publisher.display_name
            && manifest.permissions == self.permissions
            && manifest.capabilities == self.capabilities
            && release
                .signature
                .as_ref()
                .is_none_or(|signature| manifest.signature.as_ref() == Some(signature))
            && manifest.signature.as_ref().is_some_and(|signature| {
                signature.publisher_id == self.publisher.id
                    && signature.package_sha256 == download.sha256
            });
        let manifest_sha256 = manifest_fingerprint(manifest)
            .map_err(|error| PluginRegistryError::InvalidResponse(error.to_string()))?;
        let artifact_valid = actual_artifact_sha256 == download.sha256
            && manifest
                .signature
                .as_ref()
                .is_some_and(|signature| signature.package_sha256 == actual_artifact_sha256);
        let publisher_status = context
            .publisher_store
            .publisher_trust(&self.publisher.id)
            .map(registry_publisher_status)
            .unwrap_or(RegistryPublisherStatus::Unknown);
        let revocation = if context
            .publisher_store
            .is_plugin_revoked(manifest.id.as_str())
        {
            RegistryRevocationStatus::Revoked
        } else {
            RegistryRevocationStatus::NotRevoked
        };

        let cryptographic_result = if manifest_matches && artifact_valid {
            context
                .publisher_store
                .verify_signature_cryptographically(manifest, actual_artifact_sha256)
        } else {
            Err(PluginSigningError::InvalidSignature)
        };
        let signature_status = match (manifest.signature.is_some(), cryptographic_result.is_ok()) {
            (false, _) => RegistrySignatureStatus::Unsigned,
            (true, false) => RegistrySignatureStatus::Invalid,
            (true, true) if publisher_status == RegistryPublisherStatus::Trusted => {
                RegistrySignatureStatus::Verified
            }
            (true, true) => RegistrySignatureStatus::ValidUntrusted,
        };
        let trusted_signature = signature_status == RegistrySignatureStatus::Verified
            && context
                .publisher_store
                .verify_manifest_signature(manifest, actual_artifact_sha256)
                .is_ok();
        let policy_decision = if trusted_signature {
            context
                .approvals
                .map(|approval| {
                    let verified_context = PluginTrustContext::verified_approval_for_manifest(
                        manifest,
                        actual_artifact_sha256,
                        context.publisher_store,
                        approval,
                    )
                    .map_err(|error| PluginRegistryError::InvalidResponse(error.to_string()))?;
                    PluginPolicy::evaluate(manifest, &verified_context, context.policy_environment)
                        .map(Some)
                        .map_err(|error| PluginRegistryError::InvalidResponse(error.to_string()))
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        let artifact_admission_allowed = manifest_matches
            && artifact_valid
            && trusted_signature
            && revocation == RegistryRevocationStatus::NotRevoked
            && compatibility == RegistryCompatibilityStatus::Compatible;

        Ok(PluginRegistryAssessment {
            signature: signature_status,
            publisher: publisher_status,
            revocation,
            artifact: if artifact_valid {
                RegistryArtifactStatus::Valid
            } else {
                RegistryArtifactStatus::Invalid
            },
            manifest: if manifest_matches {
                RegistryManifestStatus::Match
            } else {
                RegistryManifestStatus::Mismatch
            },
            compatibility,
            manifest_sha256,
            policy_decision,
            artifact_admission_allowed,
        })
    }
}

impl PluginRegistryCatalog {
    pub fn validate(&self) -> Result<(), PluginRegistryError> {
        if self.entries.len() > MAX_REGISTRY_ENTRIES {
            return Err(PluginRegistryError::TooManyEntries);
        }
        let mut plugin_ids = BTreeSet::new();
        let mut publishers = BTreeMap::new();
        for entry in &self.entries {
            entry.validate()?;
            if !plugin_ids.insert(entry.plugin_id.as_str()) {
                return Err(PluginRegistryError::DuplicatePlugin(
                    entry.plugin_id.as_str().to_owned(),
                ));
            }
            if publishers
                .insert(
                    entry.publisher.id.as_str(),
                    entry.publisher.display_name.as_str(),
                )
                .is_some_and(|display_name| display_name != entry.publisher.display_name)
            {
                return Err(PluginRegistryError::ConflictingPublisher(
                    entry.publisher.id.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// A provider supplies hostile catalog bytes; callers must parse and validate
/// the result. Provider availability never changes local plugin trust.
pub trait PluginRegistryProvider: Send + Sync {
    fn fetch_catalog(&self) -> Result<Vec<u8>, PluginRegistryError>;
}

#[derive(Clone, Debug)]
pub struct LocalPluginRegistryProvider {
    catalog: PluginRegistryCatalog,
}

impl LocalPluginRegistryProvider {
    pub fn new(catalog: PluginRegistryCatalog) -> Result<Self, PluginRegistryError> {
        catalog.validate()?;
        Ok(Self { catalog })
    }

    pub fn catalog(&self) -> &PluginRegistryCatalog {
        &self.catalog
    }
}

impl PluginRegistryProvider for LocalPluginRegistryProvider {
    fn fetch_catalog(&self) -> Result<Vec<u8>, PluginRegistryError> {
        serde_json::to_vec(&self.catalog)
            .map_err(|error| PluginRegistryError::InvalidResponse(error.to_string()))
    }
}

pub fn parse_registry_catalog(bytes: &[u8]) -> Result<PluginRegistryCatalog, PluginRegistryError> {
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(PluginRegistryError::InvalidResponse(
            "registry response exceeds the 4 MiB limit".to_owned(),
        ));
    }
    let catalog: PluginRegistryCatalog = serde_json::from_slice(bytes)
        .map_err(|error| PluginRegistryError::InvalidResponse(error.to_string()))?;
    catalog.validate()?;
    Ok(catalog)
}

fn release_compatible(
    entry: &PluginRegistryEntry,
    release: &PluginRegistryRelease,
    manifest: &PluginManifest,
    environment: &PluginRegistryEnvironment,
) -> RegistryCompatibilityStatus {
    let compatibility = &entry.compatibility;
    let version_supported = environment.fetchx_version >= compatibility.minimum_fetchx_version
        && compatibility
            .maximum_fetchx_version
            .as_ref()
            .is_none_or(|maximum| environment.fetchx_version <= *maximum);
    let target_supported = compatibility.targets.iter().any(|target| {
        (target.platform == Platform::Any || target.platform == environment.platform)
            && (target.architecture == PluginArchitecture::Any
                || target.architecture == environment.architecture)
    }) && release.downloads.iter().any(|download| {
        (download.target.platform == Platform::Any
            || download.target.platform == environment.platform)
            && (download.target.architecture == PluginArchitecture::Any
                || download.target.architecture == environment.architecture)
    }) && manifest
        .platforms
        .iter()
        .any(|platform| *platform == Platform::Any || *platform == environment.platform)
        && manifest.compatibility.minimum_api_version <= crate::PLUGIN_API_VERSION
        && manifest.compatibility.maximum_api_version >= crate::PLUGIN_API_VERSION;
    if version_supported && target_supported {
        RegistryCompatibilityStatus::Compatible
    } else {
        RegistryCompatibilityStatus::Incompatible
    }
}

fn validate_download(download: &PluginDownload) -> Result<(), String> {
    if download.filename.is_empty()
        || download.filename.len() > 255
        || download.filename == "."
        || download.filename == ".."
        || download
            .filename
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && !".-_".contains(character))
    {
        return Err("download filename is not a safe single path component".to_owned());
    }
    if !is_sha256_hex(&download.sha256) {
        return Err("download SHA-256 must be 64 lowercase hexadecimal characters".to_owned());
    }
    if download.size_bytes == 0 {
        return Err("download size must be greater than zero".to_owned());
    }
    let url = Url::parse(&download.url).map_err(|_| "download URL is malformed".to_owned())?;
    let authority = download
        .url
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .unwrap_or("");
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || authority.contains('@')
        || url.fragment().is_some()
    {
        return Err("download URL must be an HTTPS URL without user info or fragment".to_owned());
    }
    let raw_path = download.url.split_once("://").map_or("", |(_, rest)| {
        rest.split(['?', '#'])
            .next()
            .and_then(|authority_path| {
                authority_path
                    .find('/')
                    .map(|index| &authority_path[index..])
            })
            .unwrap_or("")
    });
    let lowered_path = raw_path.to_ascii_lowercase();
    if lowered_path.contains("%2e")
        || lowered_path.contains("%2f")
        || lowered_path.contains("%5c")
        || lowered_path
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
        || url.path().contains('\\')
    {
        return Err("download URL contains a traversal-like path".to_owned());
    }
    Ok(())
}

fn registry_publisher_status(trust: &PluginTrust) -> RegistryPublisherStatus {
    match trust {
        PluginTrust::Unknown => RegistryPublisherStatus::Unknown,
        PluginTrust::Untrusted => RegistryPublisherStatus::Untrusted,
        PluginTrust::Trusted => RegistryPublisherStatus::Trusted,
        PluginTrust::Revoked => RegistryPublisherStatus::Revoked,
    }
}

fn valid_publisher_id(value: &str) -> bool {
    (2..=64).contains(&value.len())
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
}

fn contains_duplicates<T: Eq + std::hash::Hash>(values: &[T]) -> bool {
    let mut seen = std::collections::HashSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer, SigningKey};

    use crate::{
        PLUGIN_API_VERSION, PLUGIN_MANIFEST_VERSION, PluginCapability, PluginPermission,
        PluginSignatureAlgorithm,
    };

    use super::*;

    const PUBLISHER_ID: &str = "publisher.test";

    struct Fixture {
        entry: PluginRegistryEntry,
        release: PluginRegistryRelease,
        download: PluginDownload,
        manifest: PluginManifest,
        publisher_store: PluginPublisherStore,
        artifact_hash: String,
    }

    fn fixture(trust: PluginTrust) -> Fixture {
        let signing_key = SigningKey::from_bytes(&[29_u8; 32]);
        let artifact_hash = crate::sha256_hex(b"registry fixture artifact");
        let target = PluginRegistryTarget {
            platform: Platform::Windows,
            architecture: PluginArchitecture::X86_64,
        };
        let download = PluginDownload {
            url: "https://registry.example/plugins/registry-plugin.zip".to_owned(),
            filename: "registry-plugin.zip".to_owned(),
            size_bytes: 32,
            sha256: artifact_hash.clone(),
            target: target.clone(),
        };
        let mut manifest = PluginManifest {
            manifest_version: PLUGIN_MANIFEST_VERSION,
            id: "registry-plugin".to_owned(),
            name: "Registry Plugin".to_owned(),
            version: PluginVersion::parse("1.2.0").unwrap(),
            api_version: PLUGIN_API_VERSION,
            compatibility: crate::PluginCompatibility {
                minimum_api_version: PLUGIN_API_VERSION,
                maximum_api_version: PLUGIN_API_VERSION,
            },
            publisher: "Publisher Test".to_owned(),
            description: "Deterministic registry test plugin".to_owned(),
            executable: if cfg!(windows) {
                std::path::PathBuf::from(r"C:\plugins\registry-plugin\plugin.exe")
            } else {
                std::path::PathBuf::from("/plugins/registry-plugin/plugin")
            },
            permissions: vec![PluginPermission::Downloads],
            capabilities: vec![PluginCapability::DownloadCreate],
            platforms: vec![Platform::Windows],
            signature: Some(PluginSignature {
                version: PLUGIN_SIGNATURE_VERSION,
                algorithm: PluginSignatureAlgorithm::Ed25519,
                publisher_id: PUBLISHER_ID.to_owned(),
                key_id: crate::sha256_hex(&signing_key.verifying_key().to_bytes()),
                package_sha256: artifact_hash.clone(),
                signature: STANDARD.encode([0_u8; 64]),
            }),
        };
        let signing_bytes = crate::canonical_manifest_signing_bytes(&manifest).unwrap();
        manifest.signature.as_mut().unwrap().signature =
            STANDARD.encode(signing_key.sign(&signing_bytes).to_bytes());
        let release = PluginRegistryRelease {
            version: manifest.version.clone(),
            release_notes: "Initial release".to_owned(),
            downloads: vec![download.clone()],
            signature: manifest.signature.clone(),
        };
        let entry = PluginRegistryEntry {
            plugin_id: PluginRegistryId::parse(manifest.id.clone()).unwrap(),
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            publisher: PluginRegistryPublisher {
                id: PUBLISHER_ID.to_owned(),
                display_name: manifest.publisher.clone(),
            },
            permissions: manifest.permissions.clone(),
            capabilities: manifest.capabilities.clone(),
            compatibility: PluginRegistryCompatibility {
                minimum_fetchx_version: PluginVersion::parse("1.0.0").unwrap(),
                maximum_fetchx_version: Some(PluginVersion::parse("2.0.0").unwrap()),
                targets: vec![target],
            },
            releases: vec![release.clone()],
        };
        let mut publisher_store = PluginPublisherStore::default();
        publisher_store
            .add_publisher_key(
                PUBLISHER_ID,
                &STANDARD.encode(signing_key.verifying_key().to_bytes()),
            )
            .unwrap();
        publisher_store
            .set_publisher_trust(PUBLISHER_ID, trust)
            .unwrap();
        Fixture {
            entry,
            release,
            download,
            manifest,
            publisher_store,
            artifact_hash,
        }
    }

    fn environment() -> (PluginRegistryEnvironment, PolicyEnvironment) {
        (
            PluginRegistryEnvironment {
                fetchx_version: PluginVersion::parse("1.5.0").unwrap(),
                platform: Platform::Windows,
                architecture: PluginArchitecture::X86_64,
            },
            PolicyEnvironment::new(
                Platform::Windows,
                PLUGIN_API_VERSION,
                vec![PluginCapability::DownloadCreate],
            ),
        )
    }

    fn assessment_context<'a>(
        fixture: &'a Fixture,
        environments: &'a (PluginRegistryEnvironment, PolicyEnvironment),
        approvals: Option<&'a PluginTrustContext>,
    ) -> PluginRegistryAssessmentContext<'a> {
        PluginRegistryAssessmentContext {
            publisher_store: &fixture.publisher_store,
            environment: &environments.0,
            approvals,
            policy_environment: &environments.1,
        }
    }

    fn assess(fixture: &Fixture, actual_hash: &str) -> PluginRegistryAssessment {
        let environments = environment();
        let context = assessment_context(fixture, &environments, None);
        fixture
            .entry
            .assess_release(
                &fixture.release,
                &fixture.download,
                &fixture.manifest,
                actual_hash,
                &context,
            )
            .unwrap()
    }

    #[test]
    fn local_provider_round_trips_valid_catalog() {
        let fixture = fixture(PluginTrust::Trusted);
        let catalog = PluginRegistryCatalog {
            entries: vec![fixture.entry],
        };
        let provider = LocalPluginRegistryProvider::new(catalog.clone()).unwrap();
        let bytes = provider.fetch_catalog().unwrap();
        assert_eq!(parse_registry_catalog(&bytes).unwrap(), catalog);
    }

    #[test]
    fn catalog_validation_rejects_duplicate_entries_and_releases() {
        let first = fixture(PluginTrust::Trusted);
        let catalog = PluginRegistryCatalog {
            entries: vec![first.entry.clone(), first.entry],
        };
        assert!(matches!(
            catalog.validate(),
            Err(PluginRegistryError::DuplicatePlugin(_))
        ));

        let mut entry = fixture(PluginTrust::Trusted).entry;
        entry.releases.push(entry.releases[0].clone());
        assert!(matches!(
            entry.validate(),
            Err(PluginRegistryError::DuplicateRelease { .. })
        ));
    }

    #[test]
    fn catalog_validation_rejects_conflicting_publisher_identity() {
        let first = fixture(PluginTrust::Trusted).entry;
        let mut second = first.clone();
        second.plugin_id = PluginRegistryId::parse("another-plugin").unwrap();
        second.name = "Another Plugin".to_owned();
        second.publisher.display_name = "Different publisher".to_owned();
        let catalog = PluginRegistryCatalog {
            entries: vec![first, second],
        };
        assert!(matches!(
            catalog.validate(),
            Err(PluginRegistryError::ConflictingPublisher(_))
        ));
    }

    #[test]
    fn catalog_validation_rejects_invalid_deserialized_ids() {
        let mut entry = fixture(PluginTrust::Trusted).entry;
        entry.plugin_id = PluginRegistryId("Invalid ID".to_owned());
        assert!(matches!(
            entry.validate(),
            Err(PluginRegistryError::InvalidId(_))
        ));
    }

    #[test]
    fn catalog_validation_rejects_hostile_download_urls() {
        for url in [
            "http://registry.example/plugin.zip",
            "https://user@registry.example/plugin.zip",
            "https://@registry.example/plugin.zip",
            "https://registry.example/a/../plugin.zip",
            "https://registry.example/a/%2e%2e/plugin.zip",
            "https://registry.example/a/%2f..%2fplugin.zip",
            "https://registry.example/plugin.zip#fragment",
        ] {
            let mut entry = fixture(PluginTrust::Trusted).entry;
            entry.releases[0].downloads[0].url = url.to_owned();
            assert!(
                entry.validate().is_err(),
                "expected URL to be rejected: {url}"
            );
        }
    }

    #[test]
    fn catalog_validation_rejects_permission_escalation_and_duplicate_targets() {
        let mut entry = fixture(PluginTrust::Trusted).entry;
        entry.permissions = vec![PluginPermission::FilesystemRead];
        assert!(entry.validate().is_err());

        let mut entry = fixture(PluginTrust::Trusted).entry;
        let duplicate_download = entry.releases[0].downloads[0].clone();
        entry.releases[0].downloads.push(duplicate_download);
        assert!(entry.validate().is_err());
    }

    #[test]
    fn assessment_requires_valid_signature_trust_and_compatible_artifact() {
        let trusted = fixture(PluginTrust::Trusted);
        let assessment = assess(&trusted, &trusted.artifact_hash);
        assert_eq!(assessment.signature, RegistrySignatureStatus::Verified);
        assert_eq!(assessment.publisher, RegistryPublisherStatus::Trusted);
        assert_eq!(assessment.artifact, RegistryArtifactStatus::Valid);
        assert_eq!(assessment.manifest, RegistryManifestStatus::Match);
        assert_eq!(
            assessment.compatibility,
            RegistryCompatibilityStatus::Compatible
        );
        assert!(assessment.artifact_admission_allowed);
        assert!(assessment.policy_decision.is_none());

        let untrusted = fixture(PluginTrust::Untrusted);
        let assessment = assess(&untrusted, &untrusted.artifact_hash);
        assert_eq!(
            assessment.signature,
            RegistrySignatureStatus::ValidUntrusted
        );
        assert!(!assessment.artifact_admission_allowed);

        let unknown = fixture(PluginTrust::Unknown);
        let assessment = assess(&unknown, &unknown.artifact_hash);
        assert_eq!(assessment.publisher, RegistryPublisherStatus::Unknown);
        assert_eq!(
            assessment.signature,
            RegistrySignatureStatus::ValidUntrusted
        );
        assert!(!assessment.artifact_admission_allowed);

        let wrong_artifact = assess(&trusted, &crate::sha256_hex(b"wrong artifact"));
        assert_eq!(wrong_artifact.signature, RegistrySignatureStatus::Invalid);
        assert_eq!(wrong_artifact.artifact, RegistryArtifactStatus::Invalid);
        assert!(!wrong_artifact.artifact_admission_allowed);
    }

    #[test]
    fn assessment_does_not_turn_registry_permissions_into_approvals() {
        let fixture = fixture(PluginTrust::Trusted);
        let approval = PluginTrustContext::for_manifest(
            &fixture.manifest,
            PluginTrust::Trusted,
            vec![PluginPermission::Downloads],
            vec![],
        )
        .unwrap();
        let environments = environment();
        let context = assessment_context(&fixture, &environments, Some(&approval));
        let assessment = fixture
            .entry
            .assess_release(
                &fixture.release,
                &fixture.download,
                &fixture.manifest,
                &fixture.artifact_hash,
                &context,
            )
            .unwrap();
        assert!(assessment.artifact_admission_allowed);
        let decision = assessment.policy_decision.unwrap();
        assert!(decision.granted_capabilities.is_empty());
        assert_eq!(
            decision.denied_capabilities[0].reason,
            crate::PolicyDenialReason::CapabilityNotApproved
        );

        let approved = PluginTrustContext::for_manifest(
            &fixture.manifest,
            PluginTrust::Trusted,
            vec![PluginPermission::Downloads],
            vec![PluginCapability::DownloadCreate],
        )
        .unwrap();
        let environments = environment();
        let context = assessment_context(&fixture, &environments, Some(&approved));
        let assessment = fixture
            .entry
            .assess_release(
                &fixture.release,
                &fixture.download,
                &fixture.manifest,
                &fixture.artifact_hash,
                &context,
            )
            .unwrap();
        assert!(
            assessment
                .policy_decision
                .unwrap()
                .allows(&PluginCapability::DownloadCreate)
        );

        let mut changed_manifest = fixture.manifest.clone();
        changed_manifest.description = "Different manifest".to_owned();
        let stale_approval = PluginTrustContext::for_manifest(
            &changed_manifest,
            PluginTrust::Trusted,
            vec![PluginPermission::Downloads],
            vec![PluginCapability::DownloadCreate],
        )
        .unwrap();
        let (registry_environment, policy_environment) = environment();
        let context = PluginRegistryAssessmentContext {
            publisher_store: &fixture.publisher_store,
            environment: &registry_environment,
            approvals: Some(&stale_approval),
            policy_environment: &policy_environment,
        };
        let assessment = fixture
            .entry
            .assess_release(
                &fixture.release,
                &fixture.download,
                &fixture.manifest,
                &fixture.artifact_hash,
                &context,
            )
            .unwrap();
        assert_eq!(
            assessment.policy_decision.unwrap().denied_capabilities[0].reason,
            crate::PolicyDenialReason::ManifestChanged
        );
    }

    #[test]
    fn assessment_rejects_catalog_permission_escalation_and_revocation() {
        let mut escalation_fixture = fixture(PluginTrust::Trusted);
        escalation_fixture
            .entry
            .permissions
            .push(PluginPermission::FilesystemWrite);
        let environments = environment();
        let context = assessment_context(&escalation_fixture, &environments, None);
        let assessment = escalation_fixture
            .entry
            .assess_release(
                &escalation_fixture.release,
                &escalation_fixture.download,
                &escalation_fixture.manifest,
                &escalation_fixture.artifact_hash,
                &context,
            )
            .unwrap();
        assert_eq!(assessment.manifest, RegistryManifestStatus::Mismatch);
        assert!(!assessment.artifact_admission_allowed);

        let mut revoked_fixture = fixture(PluginTrust::Trusted);
        revoked_fixture
            .publisher_store
            .revoke_plugin("registry-plugin");
        let assessment = assess(&revoked_fixture, &revoked_fixture.artifact_hash);
        assert_eq!(assessment.revocation, RegistryRevocationStatus::Revoked);
        assert!(!assessment.artifact_admission_allowed);
    }

    #[test]
    fn assessment_rejects_incompatible_targets() {
        let fixture = fixture(PluginTrust::Trusted);
        let (mut registry_environment, policy_environment) = environment();
        registry_environment.platform = Platform::Linux;
        let context = PluginRegistryAssessmentContext {
            publisher_store: &fixture.publisher_store,
            environment: &registry_environment,
            approvals: None,
            policy_environment: &policy_environment,
        };
        let assessment = fixture
            .entry
            .assess_release(
                &fixture.release,
                &fixture.download,
                &fixture.manifest,
                &fixture.artifact_hash,
                &context,
            )
            .unwrap();
        assert_eq!(
            assessment.compatibility,
            RegistryCompatibilityStatus::Incompatible
        );
        assert!(!assessment.artifact_admission_allowed);
    }

    #[test]
    fn parser_rejects_oversized_and_malformed_catalogs() {
        assert!(matches!(
            parse_registry_catalog(&vec![b' '; MAX_REGISTRY_BYTES + 1]),
            Err(PluginRegistryError::InvalidResponse(_))
        ));
        assert!(matches!(
            parse_registry_catalog(b"{"),
            Err(PluginRegistryError::InvalidResponse(_))
        ));
        assert!(matches!(
            parse_registry_catalog(br#"{"entries":[],"trusted":true}"#),
            Err(PluginRegistryError::InvalidResponse(_))
        ));
    }

    #[test]
    fn provider_errors_are_not_replaced_with_an_empty_catalog() {
        struct UnavailableProvider;

        impl PluginRegistryProvider for UnavailableProvider {
            fn fetch_catalog(&self) -> Result<Vec<u8>, PluginRegistryError> {
                Err(PluginRegistryError::Unavailable("offline".to_owned()))
            }
        }

        assert_eq!(
            UnavailableProvider.fetch_catalog(),
            Err(PluginRegistryError::Unavailable("offline".to_owned()))
        );
    }
}

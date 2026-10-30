//! Versioned plugin artifact signing and local publisher trust.

use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{PLUGIN_SIGNATURE_VERSION, PluginManifest, PluginSignatureAlgorithm, PluginTrust};

const SIGNING_DOMAIN: &[u8] = b"FetchX plugin signature v1\0";

#[derive(Serialize)]
struct CanonicalPluginManifest<'a> {
    signature_version: u32,
    algorithm: PluginSignatureAlgorithm,
    publisher_id: &'a str,
    key_id: &'a str,
    package_sha256: &'a str,
    manifest_version: u32,
    plugin_id: &'a str,
    name: &'a str,
    version: String,
    api_version: u32,
    compatibility: &'a crate::PluginCompatibility,
    publisher: &'a str,
    description: &'a str,
    executable: String,
    permissions: Vec<crate::PluginPermission>,
    capabilities: Vec<crate::PluginCapability>,
    platforms: Vec<crate::Platform>,
}

/// Errors encountered while validating signed metadata or publisher trust.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PluginSigningError {
    #[error("plugin manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("plugin has no signature")]
    Unsigned,
    #[error("plugin publisher `{0}` is unknown")]
    UnknownPublisher(String),
    #[error("plugin publisher `{0}` is not trusted")]
    UntrustedPublisher(String),
    #[error("plugin publisher `{0}` is revoked")]
    RevokedPublisher(String),
    #[error("plugin `{0}` is revoked")]
    RevokedPlugin(String),
    #[error("publisher `{publisher_id}` has no trusted key `{key_id}`")]
    UnknownPublisherKey {
        publisher_id: String,
        key_id: String,
    },
    #[error("plugin signature is invalid")]
    InvalidSignature,
    #[error("publisher public key is malformed")]
    InvalidPublicKey,
    #[error("plugin signature metadata is malformed: {0}")]
    MalformedSigningMetadata(String),
    #[error("signature algorithm is unsupported")]
    UnknownAlgorithm,
    #[error("signature version {0} is unsupported")]
    UnsupportedSignatureVersion(u32),
    #[error("plugin artifact hash mismatch")]
    PackageHashMismatch,
    #[error("canonical signing payload could not be generated: {0}")]
    Canonicalization(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PublisherRecord {
    trust: PluginTrust,
    keys: BTreeMap<String, [u8; 32]>,
}

impl Default for PublisherRecord {
    fn default() -> Self {
        Self {
            trust: PluginTrust::Unknown,
            keys: BTreeMap::new(),
        }
    }
}

/// Host-owned publisher keys, trust decisions, and local plugin revocations.
///
/// New publishers and keys are unknown until a host explicitly marks the
/// publisher trusted. Multiple fingerprints may be registered for explicit
/// key rotation; adding a key never changes publisher trust.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginPublisherStore {
    publishers: BTreeMap<String, PublisherRecord>,
    revoked_plugins: BTreeSet<String>,
}

impl PluginPublisherStore {
    /// Adds a public key to a publisher identity without changing its trust.
    /// The returned key ID is the lowercase SHA-256 fingerprint of the key.
    pub fn add_publisher_key(
        &mut self,
        publisher_id: impl Into<String>,
        public_key_base64: &str,
    ) -> Result<String, PluginSigningError> {
        let publisher_id = publisher_id.into();
        validate_publisher_id(&publisher_id)?;
        let decoded = STANDARD
            .decode(public_key_base64)
            .map_err(|_| PluginSigningError::InvalidPublicKey)?;
        let key_bytes: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| PluginSigningError::InvalidPublicKey)?;
        VerifyingKey::from_bytes(&key_bytes).map_err(|_| PluginSigningError::InvalidPublicKey)?;
        let key_id = sha256_hex(&key_bytes);
        self.publishers
            .entry(publisher_id)
            .or_default()
            .keys
            .insert(key_id.clone(), key_bytes);
        Ok(key_id)
    }

    /// Changes trust only for an already-known publisher identity.
    pub fn set_publisher_trust(
        &mut self,
        publisher_id: &str,
        trust: PluginTrust,
    ) -> Result<(), PluginSigningError> {
        let publisher = self
            .publishers
            .get_mut(publisher_id)
            .ok_or_else(|| PluginSigningError::UnknownPublisher(publisher_id.to_owned()))?;
        publisher.trust = trust;
        Ok(())
    }

    pub fn revoke_plugin(&mut self, plugin_id: impl Into<String>) {
        self.revoked_plugins.insert(plugin_id.into());
    }

    pub fn restore_plugin(&mut self, plugin_id: &str) {
        self.revoked_plugins.remove(plugin_id);
    }

    pub fn verify_manifest_signature(
        &self,
        manifest: &PluginManifest,
        actual_package_sha256: &str,
    ) -> Result<(), PluginSigningError> {
        let signature = manifest
            .signature
            .as_ref()
            .ok_or(PluginSigningError::Unsigned)?;
        if signature.version != PLUGIN_SIGNATURE_VERSION {
            return Err(PluginSigningError::UnsupportedSignatureVersion(
                signature.version,
            ));
        }
        if signature.algorithm != PluginSignatureAlgorithm::Ed25519 {
            return Err(PluginSigningError::UnknownAlgorithm);
        }
        manifest
            .validate()
            .map_err(|error| PluginSigningError::InvalidManifest(error.to_string()))?;
        if self.revoked_plugins.contains(&manifest.id) {
            return Err(PluginSigningError::RevokedPlugin(manifest.id.clone()));
        }
        if !is_sha256_hex(actual_package_sha256)
            || signature.package_sha256 != actual_package_sha256
        {
            return Err(PluginSigningError::PackageHashMismatch);
        }

        let publisher = self
            .publishers
            .get(&signature.publisher_id)
            .ok_or_else(|| PluginSigningError::UnknownPublisher(signature.publisher_id.clone()))?;
        match publisher.trust {
            PluginTrust::Unknown => {
                return Err(PluginSigningError::UnknownPublisher(
                    signature.publisher_id.clone(),
                ));
            }
            PluginTrust::Untrusted => {
                return Err(PluginSigningError::UntrustedPublisher(
                    signature.publisher_id.clone(),
                ));
            }
            PluginTrust::Revoked => {
                return Err(PluginSigningError::RevokedPublisher(
                    signature.publisher_id.clone(),
                ));
            }
            PluginTrust::Trusted => {}
        }

        let key_bytes = publisher.keys.get(&signature.key_id).ok_or_else(|| {
            PluginSigningError::UnknownPublisherKey {
                publisher_id: signature.publisher_id.clone(),
                key_id: signature.key_id.clone(),
            }
        })?;
        if sha256_hex(key_bytes) != signature.key_id {
            return Err(PluginSigningError::InvalidPublicKey);
        }
        let verifying_key = VerifyingKey::from_bytes(key_bytes)
            .map_err(|_| PluginSigningError::InvalidPublicKey)?;
        let signature_bytes = STANDARD
            .decode(&signature.signature)
            .map_err(|_| PluginSigningError::MalformedSigningMetadata("signature".to_owned()))?;
        let signature_array: [u8; 64] = signature_bytes.as_slice().try_into().map_err(|_| {
            PluginSigningError::MalformedSigningMetadata("signature length".to_owned())
        })?;
        let signature = Signature::from_bytes(&signature_array);
        let payload = canonical_manifest_signing_bytes(manifest)?;
        verifying_key
            .verify(&payload, &signature)
            .map_err(|_| PluginSigningError::InvalidSignature)
    }
}

/// Produces the exact bytes signed by plugin publishers.
///
/// The payload has a fixed domain prefix and fixed JSON field order. Set-like
/// permission, capability, and platform values are sorted by their serialized
/// value. Paths use forward slashes. The signature bytes themselves are omitted.
pub fn canonical_manifest_signing_bytes(
    manifest: &PluginManifest,
) -> Result<Vec<u8>, PluginSigningError> {
    let signature = manifest
        .signature
        .as_ref()
        .ok_or(PluginSigningError::Unsigned)?;
    if signature.version != PLUGIN_SIGNATURE_VERSION {
        return Err(PluginSigningError::UnsupportedSignatureVersion(
            signature.version,
        ));
    }
    manifest
        .validate()
        .map_err(|error| PluginSigningError::InvalidManifest(error.to_string()))?;
    let executable = manifest
        .executable
        .to_str()
        .ok_or_else(|| PluginSigningError::Canonicalization("non-Unicode executable path".into()))?
        .replace('\\', "/");
    let permissions = canonical_sort(&manifest.permissions)?;
    let capabilities = canonical_sort(&manifest.capabilities)?;
    let platforms = canonical_sort(&manifest.platforms)?;
    let canonical = CanonicalPluginManifest {
        signature_version: signature.version,
        algorithm: signature.algorithm.clone(),
        publisher_id: &signature.publisher_id,
        key_id: &signature.key_id,
        package_sha256: &signature.package_sha256,
        manifest_version: manifest.manifest_version,
        plugin_id: &manifest.id,
        name: &manifest.name,
        version: manifest.version.to_string(),
        api_version: manifest.api_version,
        compatibility: &manifest.compatibility,
        publisher: &manifest.publisher,
        description: &manifest.description,
        executable,
        permissions,
        capabilities,
        platforms,
    };
    let mut bytes = SIGNING_DOMAIN.to_vec();
    bytes.extend(
        serde_json::to_vec(&canonical)
            .map_err(|error| PluginSigningError::Canonicalization(error.to_string()))?,
    );
    Ok(bytes)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_sort<T: Clone + Serialize>(values: &[T]) -> Result<Vec<T>, PluginSigningError> {
    let mut encoded = values
        .iter()
        .map(|value| {
            serde_json::to_vec(&value)
                .map(|bytes| (bytes, value))
                .map_err(|error| PluginSigningError::Canonicalization(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    encoded.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(encoded
        .into_iter()
        .map(|(_, value)| value.clone())
        .collect())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_publisher_id(publisher_id: &str) -> Result<(), PluginSigningError> {
    if publisher_id.len() < 2
        || publisher_id.len() > 64
        || !publisher_id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(PluginSigningError::MalformedSigningMetadata(
            "publisher ID".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD;
    use ed25519_dalek::{Signer, SigningKey};

    use crate::{
        PLUGIN_API_VERSION, PLUGIN_MANIFEST_VERSION, Platform, PluginCapability,
        PluginCompatibility, PluginManifest, PluginPermission, PluginPolicy, PluginSignature,
        PluginSignatureAlgorithm, PluginTrustContext, PluginVersion, PolicyEnvironment,
    };

    use super::*;

    const PUBLISHER_ID: &str = "publisher.test";

    fn fixture() -> (PluginManifest, SigningKey, String) {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let package_hash = sha256_hex(b"deterministic plugin artifact");
        let key_id = sha256_hex(&signing_key.verifying_key().to_bytes());
        let mut manifest = PluginManifest {
            manifest_version: PLUGIN_MANIFEST_VERSION,
            id: "signed-plugin".to_owned(),
            name: "Signed Plugin".to_owned(),
            version: PluginVersion::parse("1.0.0").unwrap(),
            api_version: PLUGIN_API_VERSION,
            compatibility: PluginCompatibility {
                minimum_api_version: PLUGIN_API_VERSION,
                maximum_api_version: PLUGIN_API_VERSION,
            },
            publisher: "Publisher display name".to_owned(),
            description: "Test artifact".to_owned(),
            executable: if cfg!(windows) {
                std::path::PathBuf::from(r"C:\plugins\signed-plugin\plugin.exe")
            } else {
                std::path::PathBuf::from("/plugins/signed-plugin/plugin")
            },
            permissions: vec![PluginPermission::Downloads],
            capabilities: vec![PluginCapability::DownloadCreate],
            platforms: vec![Platform::Windows],
            signature: Some(PluginSignature {
                version: PLUGIN_SIGNATURE_VERSION,
                algorithm: PluginSignatureAlgorithm::Ed25519,
                publisher_id: PUBLISHER_ID.to_owned(),
                key_id,
                package_sha256: package_hash.clone(),
                signature: STANDARD.encode([0_u8; 64]),
            }),
        };
        let payload = canonical_manifest_signing_bytes(&manifest).unwrap();
        let signed = signing_key.sign(&payload);
        manifest.signature.as_mut().unwrap().signature = STANDARD.encode(signed.to_bytes());
        (manifest, signing_key, package_hash)
    }

    fn store_for(signing_key: &SigningKey) -> PluginPublisherStore {
        let mut store = PluginPublisherStore::default();
        store
            .add_publisher_key(
                PUBLISHER_ID,
                &STANDARD.encode(signing_key.verifying_key().to_bytes()),
            )
            .unwrap();
        store
            .set_publisher_trust(PUBLISHER_ID, PluginTrust::Trusted)
            .unwrap();
        store
    }

    #[test]
    fn trusted_valid_signature_requires_phase_23a_approvals() {
        let (manifest, signing_key, package_hash) = fixture();
        let store = store_for(&signing_key);
        store
            .verify_manifest_signature(&manifest, &package_hash)
            .unwrap();

        let unapproved = PluginTrustContext::verified_for_manifest(
            &manifest,
            &package_hash,
            &store,
            PluginTrust::Trusted,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let environment = PolicyEnvironment::new(
            Platform::Windows,
            PLUGIN_API_VERSION,
            vec![PluginCapability::DownloadCreate],
        );
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &unapproved, &environment)
                .unwrap()
                .denied_capabilities[0]
                .reason,
            crate::PolicyDenialReason::PermissionNotApproved
        );

        let approved = PluginTrustContext::verified_for_manifest(
            &manifest,
            &package_hash,
            &store,
            PluginTrust::Trusted,
            vec![PluginPermission::Downloads],
            vec![PluginCapability::DownloadCreate],
        )
        .unwrap();
        assert!(
            PluginPolicy::evaluate(&manifest, &approved, &environment)
                .unwrap()
                .fully_allowed()
        );
    }

    #[test]
    fn unknown_untrusted_and_revoked_publishers_are_denied() {
        let (manifest, signing_key, package_hash) = fixture();

        assert!(matches!(
            PluginPublisherStore::default().verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::UnknownPublisher(_))
        ));

        let mut store = store_for(&signing_key);
        store
            .set_publisher_trust(PUBLISHER_ID, PluginTrust::Untrusted)
            .unwrap();
        assert!(matches!(
            store.verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::UntrustedPublisher(_))
        ));

        store
            .set_publisher_trust(PUBLISHER_ID, PluginTrust::Revoked)
            .unwrap();
        assert!(matches!(
            store.verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::RevokedPublisher(_))
        ));
    }

    #[test]
    fn revoked_plugin_is_denied_even_with_valid_signature() {
        let (manifest, signing_key, package_hash) = fixture();
        let mut store = store_for(&signing_key);
        store.revoke_plugin(&manifest.id);
        assert_eq!(
            store.verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::RevokedPlugin(manifest.id.clone()))
        );
    }

    #[test]
    fn wrong_key_and_changed_publisher_identity_are_denied() {
        let (manifest, _, package_hash) = fixture();
        let wrong_key = SigningKey::from_bytes(&[9_u8; 32]);
        let mut wrong_store = PluginPublisherStore::default();
        wrong_store
            .add_publisher_key(
                PUBLISHER_ID,
                &STANDARD.encode(wrong_key.verifying_key().to_bytes()),
            )
            .unwrap();
        wrong_store
            .set_publisher_trust(PUBLISHER_ID, PluginTrust::Trusted)
            .unwrap();
        assert!(matches!(
            wrong_store.verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::UnknownPublisherKey { .. })
        ));

        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let mut trusted_other_identity = store_for(&signing_key);
        trusted_other_identity
            .add_publisher_key(
                "other.publisher",
                &STANDARD.encode(signing_key.verifying_key().to_bytes()),
            )
            .unwrap();
        trusted_other_identity
            .set_publisher_trust("other.publisher", PluginTrust::Trusted)
            .unwrap();
        let mut changed_identity = manifest;
        changed_identity.signature.as_mut().unwrap().publisher_id = "other.publisher".to_owned();
        assert!(matches!(
            trusted_other_identity.verify_manifest_signature(&changed_identity, &package_hash),
            Err(PluginSigningError::InvalidSignature)
        ));
    }

    #[test]
    fn explicitly_enrolled_rotation_key_uses_the_same_publisher_trust() {
        let (mut manifest, signing_key, package_hash) = fixture();
        let mut store = store_for(&signing_key);
        let rotated_key = SigningKey::from_bytes(&[13_u8; 32]);
        let rotated_key_id = store
            .add_publisher_key(
                PUBLISHER_ID,
                &STANDARD.encode(rotated_key.verifying_key().to_bytes()),
            )
            .unwrap();
        {
            let signature = manifest.signature.as_mut().unwrap();
            signature.key_id = rotated_key_id;
            signature.signature = STANDARD.encode([0_u8; 64]);
        }
        let payload = canonical_manifest_signing_bytes(&manifest).unwrap();
        manifest.signature.as_mut().unwrap().signature =
            STANDARD.encode(rotated_key.sign(&payload).to_bytes());

        assert!(
            store
                .verify_manifest_signature(&manifest, &package_hash)
                .is_ok()
        );
    }

    #[test]
    fn signed_manifest_fields_and_version_are_bound() {
        let (original, signing_key, package_hash) = fixture();
        let store = store_for(&signing_key);

        let mut changed_version = original.clone();
        changed_version.version = PluginVersion::parse("1.0.1").unwrap();
        assert_eq!(
            store.verify_manifest_signature(&changed_version, &package_hash),
            Err(PluginSigningError::InvalidSignature)
        );

        let mut changed_permission = original.clone();
        changed_permission
            .permissions
            .push(PluginPermission::Process);
        changed_permission
            .capabilities
            .push(PluginCapability::ProcessSpawn);
        assert_eq!(
            store.verify_manifest_signature(&changed_permission, &package_hash),
            Err(PluginSigningError::InvalidSignature)
        );

        let mut changed_capability = original.clone();
        changed_capability
            .capabilities
            .push(PluginCapability::DownloadList);
        assert_eq!(
            store.verify_manifest_signature(&changed_capability, &package_hash),
            Err(PluginSigningError::InvalidSignature)
        );
    }

    #[test]
    fn artifact_hash_and_signature_tampering_are_denied() {
        let (manifest, signing_key, package_hash) = fixture();
        let store = store_for(&signing_key);
        assert_eq!(
            store.verify_manifest_signature(&manifest, &sha256_hex(b"tampered artifact")),
            Err(PluginSigningError::PackageHashMismatch)
        );

        let mut bad_signature = manifest;
        bad_signature.signature.as_mut().unwrap().signature = STANDARD.encode([1_u8; 64]);
        assert_eq!(
            store.verify_manifest_signature(&bad_signature, &package_hash),
            Err(PluginSigningError::InvalidSignature)
        );
    }

    #[test]
    fn malformed_key_and_signature_version_are_rejected() {
        let mut store = PluginPublisherStore::default();
        assert_eq!(
            store.add_publisher_key(PUBLISHER_ID, "not base64"),
            Err(PluginSigningError::InvalidPublicKey)
        );

        let (mut manifest, signing_key, package_hash) = fixture();
        manifest.signature.as_mut().unwrap().version = PLUGIN_SIGNATURE_VERSION + 1;
        assert_eq!(
            store_for(&signing_key).verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::UnsupportedSignatureVersion(
                PLUGIN_SIGNATURE_VERSION + 1
            ))
        );
    }

    #[test]
    fn malformed_signature_bytes_are_rejected_without_panicking() {
        let (mut manifest, signing_key, package_hash) = fixture();
        manifest.signature.as_mut().unwrap().signature = "not-base64".to_owned();
        assert!(matches!(
            store_for(&signing_key).verify_manifest_signature(&manifest, &package_hash),
            Err(PluginSigningError::InvalidManifest(_))
        ));
    }

    #[test]
    fn canonical_payload_sorts_set_like_fields_and_rejects_forged_verified_state() {
        let (mut manifest, _, _) = fixture();
        manifest.capabilities.push(PluginCapability::DownloadList);
        let before = canonical_manifest_signing_bytes(&manifest).unwrap();
        manifest.capabilities.reverse();
        let after = canonical_manifest_signing_bytes(&manifest).unwrap();
        assert_eq!(before, after);

        let forged = serde_json::json!({
            "trust": "trusted",
            "signature_status": "verified",
            "approved_permissions": ["downloads"],
            "approved_capabilities": ["download_create"],
            "manifest_sha256": "00".repeat(32)
        });
        assert!(serde_json::from_value::<PluginTrustContext>(forged).is_err());
    }

    #[test]
    fn unsupported_algorithm_is_rejected_by_manifest_parser() {
        let (manifest, _, _) = fixture();
        let mut value = serde_json::to_value(manifest).unwrap();
        value["signature"]["algorithm"] = serde_json::Value::String("rsa".to_owned());
        assert!(matches!(
            crate::parse_manifest(&serde_json::to_vec(&value).unwrap()),
            Err(crate::ManifestValidationError::Json(_))
        ));
    }
}

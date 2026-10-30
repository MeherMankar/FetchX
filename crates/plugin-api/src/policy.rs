//! Shared, fail-closed plugin trust and capability policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::manifest::{
    ManifestValidationError, PLUGIN_API_VERSION, Platform, PluginCapability, PluginManifest,
    PluginPermission,
};
use crate::signing::{PluginPublisherStore, PluginSigningError};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTrust {
    Unknown,
    Trusted,
    Untrusted,
    Revoked,
}

/// Signature state recorded by the host trust infrastructure. A trusted plugin
/// must have a verified signature before authorization is granted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSignatureStatus {
    Unsigned,
    Unverified,
    Verified,
    Invalid,
}

impl<'de> Deserialize<'de> for PluginSignatureStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "unsigned" => Ok(Self::Unsigned),
            "unverified" => Ok(Self::Unverified),
            "invalid" => Ok(Self::Invalid),
            "verified" => Err(serde::de::Error::custom(
                "verified signature status is host-generated only",
            )),
            _ => Err(serde::de::Error::custom("unknown signature status")),
        }
    }
}

/// Host-owned approval record. `manifest_sha256` binds approvals to the exact
/// validated manifest; a manifest change invalidates prior approvals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTrustContext {
    pub trust: PluginTrust,
    signature_status: PluginSignatureStatus,
    pub approved_permissions: Vec<PluginPermission>,
    pub approved_capabilities: Vec<PluginCapability>,
    pub manifest_sha256: String,
}

impl PluginTrustContext {
    /// Creates an approval record bound to `manifest`. Calling this is a host
    /// decision and must only follow explicit user or administrator approval.
    pub fn for_manifest(
        manifest: &PluginManifest,
        trust: PluginTrust,
        approved_permissions: Vec<PluginPermission>,
        approved_capabilities: Vec<PluginCapability>,
    ) -> Result<Self, PolicyEvaluationError> {
        let signature_status = if manifest.signature.is_some() {
            PluginSignatureStatus::Unverified
        } else {
            PluginSignatureStatus::Unsigned
        };
        Ok(Self {
            trust,
            signature_status,
            approved_permissions,
            approved_capabilities,
            manifest_sha256: manifest_fingerprint(manifest)?,
        })
    }

    pub fn verified_for_manifest(
        manifest: &PluginManifest,
        actual_package_sha256: &str,
        publisher_store: &PluginPublisherStore,
        trust: PluginTrust,
        approved_permissions: Vec<PluginPermission>,
        approved_capabilities: Vec<PluginCapability>,
    ) -> Result<Self, PolicyEvaluationError> {
        publisher_store.verify_manifest_signature(manifest, actual_package_sha256)?;
        let mut context =
            Self::for_manifest(manifest, trust, approved_permissions, approved_capabilities)?;
        context.signature_status = PluginSignatureStatus::Verified;
        Ok(context)
    }

    /// Marks an existing approval as signature-verified without changing its
    /// manifest fingerprint; policy evaluation still rejects stale approvals.
    pub fn verified_approval_for_manifest(
        manifest: &PluginManifest,
        actual_package_sha256: &str,
        publisher_store: &PluginPublisherStore,
        approval: &Self,
    ) -> Result<Self, PolicyEvaluationError> {
        publisher_store.verify_manifest_signature(manifest, actual_package_sha256)?;
        let mut verified_approval = approval.clone();
        verified_approval.signature_status = PluginSignatureStatus::Verified;
        Ok(verified_approval)
    }

    pub fn unknown(manifest: &PluginManifest) -> Result<Self, PolicyEvaluationError> {
        Self::for_manifest(manifest, PluginTrust::Unknown, Vec::new(), Vec::new())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyEnvironment {
    pub platform: Platform,
    pub api_version: u32,
    pub supported_capabilities: Vec<PluginCapability>,
}

impl PolicyEnvironment {
    pub fn new(
        platform: Platform,
        api_version: u32,
        supported_capabilities: Vec<PluginCapability>,
    ) -> Self {
        Self {
            platform,
            api_version,
            supported_capabilities,
        }
    }

    /// Runtime platform used by the current process.
    pub fn current(supported_capabilities: Vec<PluginCapability>) -> Self {
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

        Self::new(platform, PLUGIN_API_VERSION, supported_capabilities)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDenialReason {
    TrustUnknown,
    PluginUntrusted,
    TrustRevoked,
    SignatureInvalid,
    SignatureStateMismatch,
    ManifestChanged,
    UnsupportedPlatform,
    UnsupportedApiVersion,
    PermissionNotDeclared,
    PermissionNotApproved,
    CapabilityNotApproved,
    CapabilityUnsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDenial {
    pub capability: PluginCapability,
    pub reason: PolicyDenialReason,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPolicyDecision {
    pub manifest_sha256: String,
    pub granted_capabilities: Vec<PluginCapability>,
    pub denied_capabilities: Vec<CapabilityDenial>,
}

impl PluginPolicyDecision {
    pub fn allows(&self, capability: &PluginCapability) -> bool {
        self.granted_capabilities.contains(capability)
    }

    pub fn fully_allowed(&self) -> bool {
        self.denied_capabilities.is_empty() && !self.granted_capabilities.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum PolicyEvaluationError {
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(#[from] ManifestValidationError),
    #[error("could not fingerprint plugin manifest: {0}")]
    Fingerprint(String),
    #[error("plugin signature verification failed: {0}")]
    Signing(#[from] PluginSigningError),
}

pub struct PluginPolicy;

impl PluginPolicy {
    pub fn evaluate(
        manifest: &PluginManifest,
        trust: &PluginTrustContext,
        environment: &PolicyEnvironment,
    ) -> Result<PluginPolicyDecision, PolicyEvaluationError> {
        manifest.validate()?;
        let current_fingerprint = manifest_fingerprint(manifest)?;
        let global_denial = if current_fingerprint != trust.manifest_sha256 {
            Some(PolicyDenialReason::ManifestChanged)
        } else {
            match trust.trust {
                PluginTrust::Unknown => Some(PolicyDenialReason::TrustUnknown),
                PluginTrust::Untrusted => Some(PolicyDenialReason::PluginUntrusted),
                PluginTrust::Revoked => Some(PolicyDenialReason::TrustRevoked),
                PluginTrust::Trusted => match (&trust.signature_status, &manifest.signature) {
                    (PluginSignatureStatus::Invalid, _) => {
                        Some(PolicyDenialReason::SignatureInvalid)
                    }
                    (PluginSignatureStatus::Verified, Some(_))
                    | (PluginSignatureStatus::Unsigned, None) => None,
                    _ => Some(PolicyDenialReason::SignatureStateMismatch),
                },
            }
        };

        let platform_supported = manifest.platforms.is_empty()
            || manifest.platforms.contains(&Platform::Any)
            || manifest.platforms.contains(&environment.platform);
        let api_supported = (manifest.compatibility.minimum_api_version
            ..=manifest.compatibility.maximum_api_version)
            .contains(&environment.api_version);

        let mut granted_capabilities = Vec::new();
        let mut denied_capabilities = Vec::new();
        for capability in &manifest.capabilities {
            let reason = global_denial.clone().or_else(|| {
                if !platform_supported {
                    Some(PolicyDenialReason::UnsupportedPlatform)
                } else if !api_supported {
                    Some(PolicyDenialReason::UnsupportedApiVersion)
                } else if !manifest
                    .permissions
                    .contains(&capability.required_permission())
                {
                    Some(PolicyDenialReason::PermissionNotDeclared)
                } else if !trust
                    .approved_permissions
                    .contains(&capability.required_permission())
                {
                    Some(PolicyDenialReason::PermissionNotApproved)
                } else if !trust.approved_capabilities.contains(capability) {
                    Some(PolicyDenialReason::CapabilityNotApproved)
                } else if !environment.supported_capabilities.contains(capability) {
                    Some(PolicyDenialReason::CapabilityUnsupported)
                } else {
                    None
                }
            });
            if let Some(reason) = reason {
                denied_capabilities.push(CapabilityDenial {
                    capability: capability.clone(),
                    reason,
                });
            } else {
                granted_capabilities.push(capability.clone());
            }
        }

        Ok(PluginPolicyDecision {
            manifest_sha256: current_fingerprint,
            granted_capabilities,
            denied_capabilities,
        })
    }
}

pub fn manifest_fingerprint(manifest: &PluginManifest) -> Result<String, PolicyEvaluationError> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| PolicyEvaluationError::Fingerprint(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        PLUGIN_MANIFEST_VERSION, PluginCapability, PluginCompatibility, PluginVersion,
    };

    fn manifest() -> PluginManifest {
        PluginManifest {
            manifest_version: PLUGIN_MANIFEST_VERSION,
            id: "my-plugin".to_owned(),
            name: "Test Plugin".to_owned(),
            version: PluginVersion::parse("1.2.3").unwrap(),
            api_version: PLUGIN_API_VERSION,
            compatibility: PluginCompatibility {
                minimum_api_version: PLUGIN_API_VERSION,
                maximum_api_version: PLUGIN_API_VERSION,
            },
            publisher: "Publisher".to_owned(),
            description: "Test".to_owned(),
            executable: if cfg!(windows) {
                std::path::PathBuf::from(r"C:\plugins\test\plugin.exe")
            } else {
                std::path::PathBuf::from("/plugins/test/plugin")
            },
            permissions: vec![PluginPermission::Downloads],
            capabilities: vec![PluginCapability::DownloadCreate],
            platforms: vec![],
            signature: None,
        }
    }

    fn approved_context(manifest: &PluginManifest) -> PluginTrustContext {
        PluginTrustContext::for_manifest(
            manifest,
            PluginTrust::Trusted,
            vec![PluginPermission::Downloads],
            vec![PluginCapability::DownloadCreate],
        )
        .unwrap()
    }

    fn environment() -> PolicyEnvironment {
        PolicyEnvironment::new(
            Platform::Windows,
            PLUGIN_API_VERSION,
            vec![PluginCapability::DownloadCreate],
        )
    }

    #[test]
    fn explicit_trust_permission_capability_and_runtime_support_are_required() {
        let manifest = manifest();
        let context = approved_context(&manifest);
        let decision = PluginPolicy::evaluate(&manifest, &context, &environment()).unwrap();
        assert!(decision.fully_allowed());
        assert!(decision.allows(&PluginCapability::DownloadCreate));
    }

    #[test]
    fn windows_and_android_use_the_same_fail_closed_policy() {
        let manifest = manifest();
        let context = PluginTrustContext::unknown(&manifest).unwrap();
        let windows = PluginPolicy::evaluate(
            &manifest,
            &context,
            &PolicyEnvironment::new(Platform::Windows, PLUGIN_API_VERSION, vec![]),
        )
        .unwrap();
        let android = PluginPolicy::evaluate(
            &manifest,
            &context,
            &PolicyEnvironment::new(Platform::Android, PLUGIN_API_VERSION, vec![]),
        )
        .unwrap();
        assert_eq!(windows, android);
        assert!(!windows.fully_allowed());
    }

    #[test]
    fn unknown_untrusted_and_revoked_plugins_are_denied() {
        let manifest = manifest();
        for (trust, reason) in [
            (PluginTrust::Unknown, PolicyDenialReason::TrustUnknown),
            (PluginTrust::Untrusted, PolicyDenialReason::PluginUntrusted),
            (PluginTrust::Revoked, PolicyDenialReason::TrustRevoked),
        ] {
            let context = PluginTrustContext::for_manifest(
                &manifest,
                trust,
                vec![PluginPermission::Downloads],
                vec![PluginCapability::DownloadCreate],
            )
            .unwrap();
            let decision = PluginPolicy::evaluate(&manifest, &context, &environment()).unwrap();
            assert!(!decision.fully_allowed());
            assert_eq!(decision.denied_capabilities[0].reason, reason);
        }
    }

    #[test]
    fn default_unknown_context_denies_everything() {
        let manifest = manifest();
        let context = PluginTrustContext::unknown(&manifest).unwrap();
        let decision = PluginPolicy::evaluate(&manifest, &context, &environment()).unwrap();
        assert!(decision.granted_capabilities.is_empty());
        assert_eq!(
            decision.denied_capabilities[0].reason,
            PolicyDenialReason::TrustUnknown
        );
    }

    #[test]
    fn permission_or_capability_approval_alone_does_not_grant() {
        let manifest = manifest();
        let permission_only = PluginTrustContext::for_manifest(
            &manifest,
            PluginTrust::Trusted,
            vec![PluginPermission::Downloads],
            vec![],
        )
        .unwrap();
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &permission_only, &environment())
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::CapabilityNotApproved
        );

        let capability_only = PluginTrustContext::for_manifest(
            &manifest,
            PluginTrust::Trusted,
            vec![],
            vec![PluginCapability::DownloadCreate],
        )
        .unwrap();
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &capability_only, &environment())
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::PermissionNotApproved
        );
    }

    #[test]
    fn unsupported_runtime_capability_is_denied() {
        let manifest = manifest();
        let context = approved_context(&manifest);
        let environment = PolicyEnvironment::new(Platform::Windows, PLUGIN_API_VERSION, vec![]);
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &context, &environment)
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::CapabilityUnsupported
        );
    }

    #[test]
    fn platform_and_api_mismatch_are_denied() {
        let mut manifest = manifest();
        manifest.platforms = vec![Platform::Android];
        let context = approved_context(&manifest);
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &context, &environment())
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::UnsupportedPlatform
        );
        manifest.platforms.clear();
        manifest.compatibility.minimum_api_version = 0;
        manifest.compatibility.maximum_api_version = 2;
        let context = approved_context(&manifest);
        assert_eq!(
            PluginPolicy::evaluate(
                &manifest,
                &context,
                &PolicyEnvironment::new(
                    Platform::Windows,
                    3,
                    vec![PluginCapability::DownloadCreate]
                )
            )
            .unwrap()
            .denied_capabilities[0]
                .reason,
            PolicyDenialReason::UnsupportedApiVersion
        );
    }

    #[test]
    fn manifest_change_invalidates_stored_approvals() {
        let manifest = manifest();
        let context = approved_context(&manifest);
        let mut changed = manifest.clone();
        changed.publisher = "Different publisher".to_owned();
        let decision = PluginPolicy::evaluate(&changed, &context, &environment()).unwrap();
        assert_eq!(
            decision.denied_capabilities[0].reason,
            PolicyDenialReason::ManifestChanged
        );
    }

    #[test]
    fn signature_presence_never_auto_trusts_and_invalid_status_denies() {
        let mut manifest = manifest();
        manifest.signature = Some(crate::manifest::PluginSignature {
            version: crate::manifest::PLUGIN_SIGNATURE_VERSION,
            algorithm: crate::manifest::PluginSignatureAlgorithm::Ed25519,
            publisher_id: "test.publisher".to_owned(),
            key_id: "00".repeat(32),
            package_sha256: "00".repeat(32),
            signature: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                [0_u8; 64],
            ),
        });
        let mut context = PluginTrustContext::unknown(&manifest).unwrap();
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &context, &environment())
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::TrustUnknown
        );
        context.trust = PluginTrust::Trusted;
        context.approved_permissions = vec![PluginPermission::Downloads];
        context.approved_capabilities = vec![PluginCapability::DownloadCreate];
        context.signature_status = PluginSignatureStatus::Invalid;
        assert_eq!(
            PluginPolicy::evaluate(&manifest, &context, &environment())
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::SignatureInvalid
        );
    }
}

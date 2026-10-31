//! Plugin Manager — Phase 19
//!
//! Discovers, validates, registers, and manages the lifecycle of external
//! FetchX plugins.
//!
//! # Architecture
//! ```text
//! PluginManager
//!   ┌── discover()  — scans plugin directory for plugin.json manifests
//!   ┌── register()  — validates and adds a plugin to the registry
//!   ┌── start()     — spawns plugin process, performs handshake
//!   ┌── stop()      — sends Shutdown, waits, kills if needed
//!   └─── registry()  — read-only view of registered plugins
//! ```
//!
//! # Security
//! - Plugin processes are spawned with `tokio::process::Command` using
//!   structured arguments only — no shell string construction.
//! - No secrets, credentials, cookies, or tokens are forwarded to plugin
//!   processes.
//! - Plugin stdin/stdout is strictly framed and bounded.
//! - A misbehaving plugin cannot panic the host.

#![forbid(unsafe_code)]

pub mod lifecycle;
pub mod registry;

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use fetchx_models::{
    DiagnosticComponent, DiagnosticCounter, DiagnosticEvent, DiagnosticLevel, global_diagnostics,
};
use fetchx_plugin_api::{
    HelloRequest, HostRequest, HostRequestBody, PROTOCOL_VERSION, PluginCapability, PluginManifest,
    PluginPermission, PluginPolicy, PluginPolicyDecision, PluginPublisherStore, PluginResponseBody,
    PluginSigningError, PluginTrust, PluginTrustContext, PolicyDenialReason, PolicyEnvironment,
    parse_manifest, parse_plugin_response,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    sync::Mutex,
    time::{Instant, timeout, timeout_at},
};
use tracing::{debug, info, warn};

pub use lifecycle::PluginLifecycleState;
pub use registry::{PluginRegistry, PluginRegistryError};

// ============================================================
// Timeouts / limits
// ============================================================

/// Maximum time to wait for a plugin process to start and complete handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum time to wait for a plugin to acknowledge a Shutdown request.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum allowed IPC payload for plugin messages. The current runtime remains
/// process-scoped and deny-closed; the bound is enforced at the host boundary.
pub const MAX_PLUGIN_IPC_PAYLOAD_BYTES: u32 = 65_536;
/// Number of crashes within a short window before the manager disables a plugin.
pub const MAX_RESTART_ATTEMPTS: u32 = 3;
pub const CRASH_LOOP_WINDOW: Duration = Duration::from_secs(60);
pub const RESTART_BACKOFF: Duration = Duration::from_secs(2);
const SUPPORTED_PLUGIN_CAPABILITIES: &[PluginCapability] = &[];

/// Host-enforced sandbox configuration for the plugin supervisor. These values are
/// intentionally conservative and reflect the runtime features that the current
/// host actually provides; they are not a claim of OS-level sandboxing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSandboxConfig {
    pub max_ipc_payload_bytes: u32,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_restart_attempts: u32,
    pub restart_backoff: Duration,
    pub crash_loop_window: Duration,
}

impl Default for PluginSandboxConfig {
    fn default() -> Self {
        Self {
            max_ipc_payload_bytes: MAX_PLUGIN_IPC_PAYLOAD_BYTES,
            startup_timeout: HANDSHAKE_TIMEOUT,
            shutdown_timeout: SHUTDOWN_TIMEOUT,
            max_restart_attempts: MAX_RESTART_ATTEMPTS,
            restart_backoff: RESTART_BACKOFF,
            crash_loop_window: CRASH_LOOP_WINDOW,
        }
    }
}

impl PluginSandboxConfig {
    pub fn validate_ipc_length(&self, payload_len: u32) -> Result<(), PluginManagerError> {
        if payload_len > self.max_ipc_payload_bytes {
            return Err(PluginManagerError::OversizedPayload {
                payload_len,
                max_payload_len: self.max_ipc_payload_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSandboxStatus {
    pub max_ipc_payload_bytes: u32,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub restart_count: u32,
    pub sandbox_disabled: bool,
    pub crash_loop_detected: bool,
}

/// Runtime record for a registered plugin.
#[derive(Debug)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginLifecycleState,
    manifest_path: Option<PathBuf>,
    artifact_root: PathBuf,
    /// Set while the plugin process is running.
    process: Option<Child>,
    /// Crash-loop tracking for the supervisor. This is a guardrail for the host,
    /// not a claim of OS-level sandbox enforcement.
    restart_count: u32,
    last_failure: Option<Instant>,
    sandbox_disabled: bool,
}

impl PluginEntry {
    fn new(manifest: PluginManifest, manifest_path: Option<PathBuf>) -> Self {
        let artifact_root = manifest_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .or_else(|| manifest.executable.parent().map(PathBuf::from))
            .unwrap_or_default();
        Self {
            manifest,
            state: PluginLifecycleState::Registered,
            manifest_path,
            artifact_root,
            process: None,
            restart_count: 0,
            last_failure: None,
            sandbox_disabled: false,
        }
    }
}

// ============================================================
// PluginManager
// ============================================================

/// Central service for plugin lifecycle management.
///
/// All public methods are `async` and safe to call concurrently.
/// Internal state is protected by a `Mutex`.
pub struct PluginManager {
    inner: Mutex<ManagerInner>,
}

struct ManagerInner {
    entries: HashMap<String, PluginEntry>,
    trust_contexts: HashMap<String, PluginTrustContext>,
    plugin_dir: PathBuf,
    sandbox_config: PluginSandboxConfig,
    publisher_store: PluginPublisherStore,
}

impl PluginManager {
    /// Returns the host-enforced sandbox limits currently applied by the manager.
    pub async fn sandbox_config(&self) -> PluginSandboxConfig {
        let inner = self.inner.lock().await;
        inner.sandbox_config.clone()
    }

    /// Records a runtime failure and returns `true` when the plugin has crossed the
    /// configured crash-loop threshold while the sandbox remains enabled.
    pub async fn record_runtime_failure(
        &self,
        id: &str,
        reason: &str,
    ) -> Result<bool, PluginManagerError> {
        let mut inner = self.inner.lock().await;
        let crash_loop_window = inner.sandbox_config.crash_loop_window;
        let max_restart_attempts = inner.sandbox_config.max_restart_attempts;
        let Some(entry) = inner.entries.get_mut(id) else {
            return Err(PluginManagerError::NotFound(id.to_owned()));
        };

        let now = Instant::now();
        match entry.last_failure {
            Some(last_failure) if now.duration_since(last_failure) <= crash_loop_window => {
                entry.restart_count += 1;
            }
            _ => {
                entry.restart_count = 1;
                entry.last_failure = Some(now);
            }
        }
        global_diagnostics().increment(DiagnosticCounter::PluginCrashes);
        global_diagnostics().record(
            DiagnosticEvent::new(
                DiagnosticComponent::Plugins,
                "plugin_runtime_failure",
                DiagnosticLevel::Error,
            )
            .with_plugin_id(id)
            .with_error_code("PLUGIN_RUNTIME_FAILURE"),
        );
        if entry.restart_count > 1 {
            global_diagnostics().increment(DiagnosticCounter::PluginRestarts);
        }

        if entry.restart_count >= max_restart_attempts {
            entry.sandbox_disabled = true;
            entry.state = PluginLifecycleState::Failed {
                reason: format!("restart limit reached after {reason}"),
            };
            global_diagnostics().increment(DiagnosticCounter::PluginsDisabled);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Plugins,
                    "plugin_disabled_after_crash_loop",
                    DiagnosticLevel::Error,
                )
                .with_plugin_id(id)
                .with_error_code("PLUGIN_CRASH_LOOP"),
            );
            return Ok(true);
        }

        entry.state = PluginLifecycleState::Failed {
            reason: reason.to_owned(),
        };
        Ok(false)
    }

    /// Returns the runtime monitoring state for a plugin. The current manager only
    /// enforces process isolation and bounded IPC; it does not pretend to provide
    /// OS-level sandboxing where no host runtime exists.
    pub async fn sandbox_status(
        &self,
        id: &str,
    ) -> Result<PluginSandboxStatus, PluginManagerError> {
        let inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        Ok(PluginSandboxStatus {
            max_ipc_payload_bytes: inner.sandbox_config.max_ipc_payload_bytes,
            startup_timeout: inner.sandbox_config.startup_timeout,
            shutdown_timeout: inner.sandbox_config.shutdown_timeout,
            restart_count: entry.restart_count,
            sandbox_disabled: entry.sandbox_disabled,
            crash_loop_detected: entry.restart_count >= inner.sandbox_config.max_restart_attempts,
        })
    }
    /// Creates a new manager that scans `plugin_dir` for plugins.
    pub fn new(plugin_dir: PathBuf) -> Arc<Self> {
        Self::new_with_publisher_store(plugin_dir, PluginPublisherStore::default())
    }

    /// Creates a manager with an explicitly configured local publisher store.
    pub fn new_with_publisher_store(
        plugin_dir: PathBuf,
        publisher_store: PluginPublisherStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ManagerInner {
                entries: HashMap::new(),
                trust_contexts: HashMap::new(),
                plugin_dir,
                sandbox_config: PluginSandboxConfig::default(),
                publisher_store,
            }),
        })
    }

    /// Registers a publisher key without trusting it. The key ID is its
    /// lowercase SHA-256 fingerprint.
    pub async fn add_publisher_key(
        &self,
        publisher_id: impl Into<String>,
        public_key_base64: &str,
    ) -> Result<String, PluginManagerError> {
        let mut inner = self.inner.lock().await;
        inner
            .publisher_store
            .add_publisher_key(publisher_id, public_key_base64)
            .map_err(PluginManagerError::Signing)
    }

    /// Changes an existing publisher's explicit local trust decision. A
    /// downgrade or revocation also stops any running plugins from that publisher.
    pub async fn set_publisher_trust(
        &self,
        publisher_id: &str,
        trust: PluginTrust,
    ) -> Result<(), PluginManagerError> {
        let running = {
            let mut inner = self.inner.lock().await;
            inner
                .publisher_store
                .set_publisher_trust(publisher_id, trust.clone())
                .map_err(PluginManagerError::Signing)?;
            if trust == PluginTrust::Trusted {
                Vec::new()
            } else {
                inner
                    .entries
                    .iter()
                    .filter(|(_, entry)| {
                        matches!(entry.state, PluginLifecycleState::Running)
                            && entry
                                .manifest
                                .signature
                                .as_ref()
                                .is_some_and(|signature| signature.publisher_id == publisher_id)
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            }
        };
        for id in running {
            self.stop(&id).await?;
        }
        Ok(())
    }

    /// Revokes one plugin locally and stops it if it is running.
    pub async fn revoke_plugin(&self, id: &str) -> Result<(), PluginManagerError> {
        let running = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            let running = matches!(entry.state, PluginLifecycleState::Running);
            inner.publisher_store.revoke_plugin(id);
            running
        };
        if running {
            self.stop(id).await?;
        }
        Ok(())
    }

    /// Removes a local plugin revocation. This does not grant publisher trust
    /// or restore manifest permissions and capabilities.
    pub async fn restore_plugin(&self, id: &str) -> Result<(), PluginManagerError> {
        let mut inner = self.inner.lock().await;
        if !inner.entries.contains_key(id) {
            return Err(PluginManagerError::NotFound(id.to_owned()));
        }
        inner.publisher_store.restore_plugin(id);
        Ok(())
    }

    /// Scans the plugin directory for `plugin.json` manifests, validates each
    /// one, and registers valid plugins.  Returns counts of found/valid/invalid.
    pub async fn discover(&self) -> DiscoverResult {
        let plugin_dir = {
            let inner = self.inner.lock().await;
            inner.plugin_dir.clone()
        };

        let mut result = DiscoverResult::default();

        let mut read_dir = match fs::read_dir(&plugin_dir).await {
            Ok(rd) => rd,
            Err(e) => {
                debug!(error = %e, dir = %plugin_dir.display(), "plugin directory not accessible");
                return result;
            }
        };

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.json");
            result.found += 1;

            let bytes = match fs::read(&manifest_path).await {
                Ok(b) => b,
                Err(e) => {
                    warn!(path = %manifest_path.display(), error = %e, "could not read plugin manifest");
                    result.invalid += 1;
                    continue;
                }
            };

            match parse_manifest(&bytes) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    match self
                        .register_discovered(manifest, manifest_path.clone())
                        .await
                    {
                        Ok(()) => {
                            info!(plugin_id = %id, "plugin registered");
                            result.registered += 1;
                        }
                        Err(e) => {
                            warn!(plugin_id = %id, error = %e, "plugin registration failed");
                            result.invalid += 1;
                        }
                    }
                }
                Err(e) => {
                    warn!(path = %manifest_path.display(), error = %e, "invalid plugin manifest");
                    result.invalid += 1;
                }
            }
        }

        result
    }

    /// Registers a validated [`PluginManifest`].
    ///
    /// Returns an error if a plugin with the same ID is already registered.
    pub async fn register(&self, manifest: PluginManifest) -> Result<(), PluginManagerError> {
        self.register_with_path(manifest, None).await
    }

    async fn register_discovered(
        &self,
        manifest: PluginManifest,
        manifest_path: PathBuf,
    ) -> Result<(), PluginManagerError> {
        self.register_with_path(manifest, Some(manifest_path)).await
    }

    async fn register_with_path(
        &self,
        manifest: PluginManifest,
        manifest_path: Option<PathBuf>,
    ) -> Result<(), PluginManagerError> {
        manifest
            .validate()
            .map_err(|error| PluginManagerError::InvalidManifest(error.to_string()))?;
        let trust_context = PluginTrustContext::unknown(&manifest)
            .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
        let mut inner = self.inner.lock().await;
        let id = manifest.id.clone();
        if inner.entries.contains_key(&id) {
            return Err(PluginManagerError::DuplicateId(id));
        }
        inner
            .entries
            .insert(id.clone(), PluginEntry::new(manifest, manifest_path));
        inner.trust_contexts.insert(id, trust_context);
        Ok(())
    }

    async fn reload_discovered_manifest(&self, id: &str) -> Result<(), PluginManagerError> {
        let manifest_path = {
            let inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            entry.manifest_path.clone()
        };
        let Some(manifest_path) = manifest_path else {
            return Ok(());
        };

        let bytes = match fs::read(&manifest_path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                self.revoke_approval(id).await?;
                return Err(PluginManagerError::ManifestReadFailed {
                    path: manifest_path.display().to_string(),
                    reason: error.to_string(),
                });
            }
        };
        let latest = match parse_manifest(&bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.revoke_approval(id).await?;
                return Err(PluginManagerError::InvalidManifest(error.to_string()));
            }
        };

        let mut inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get_mut(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        if entry.manifest != latest {
            let fresh_context = PluginTrustContext::unknown(&latest)
                .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
            entry.manifest = latest;
            let was_running = matches!(entry.state, PluginLifecycleState::Running);
            if !was_running {
                entry.state = PluginLifecycleState::Registered;
            }
            inner.trust_contexts.insert(id.to_owned(), fresh_context);
            drop(inner);
            if was_running {
                self.stop(id).await?;
            }
            return Err(PluginManagerError::ManifestChanged);
        }
        Ok(())
    }

    async fn revoke_approval(&self, id: &str) -> Result<(), PluginManagerError> {
        let stop_running = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            let context = PluginTrustContext::unknown(&entry.manifest)
                .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
            let running = matches!(entry.state, PluginLifecycleState::Running);
            inner.trust_contexts.insert(id.to_owned(), context);
            running
        };
        if stop_running {
            self.stop(id).await?;
        }
        Ok(())
    }

    /// Records explicit host approval for declared permissions and
    /// capabilities. Approvals are in-memory and reset when the manager exits.
    pub async fn set_trust_decision(
        &self,
        id: &str,
        trust: PluginTrust,
        approved_permissions: Vec<PluginPermission>,
        approved_capabilities: Vec<PluginCapability>,
    ) -> Result<(), PluginManagerError> {
        let should_stop = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            if approved_permissions
                .iter()
                .any(|permission| !entry.manifest.permissions.contains(permission))
                || approved_capabilities
                    .iter()
                    .any(|capability| !entry.manifest.capabilities.contains(capability))
            {
                return Err(PluginManagerError::InvalidTrustDecision);
            }
            let context = PluginTrustContext::for_manifest(
                &entry.manifest,
                trust,
                approved_permissions,
                approved_capabilities,
            )
            .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
            let decision = PluginPolicy::evaluate(
                &entry.manifest,
                &context,
                &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
            )
            .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
            inner.trust_contexts.insert(id.to_owned(), context);
            !decision.fully_allowed()
                && inner
                    .entries
                    .get(id)
                    .is_some_and(|entry| matches!(entry.state, PluginLifecycleState::Running))
        };
        if should_stop {
            self.stop(id).await?;
        }
        Ok(())
    }

    /// Authorizes one operation against the current host trust decision.
    /// Callers must use this immediately before every privileged operation.
    pub async fn authorize_capability(
        &self,
        id: &str,
        capability: &PluginCapability,
    ) -> Result<(), PluginManagerError> {
        self.reload_discovered_manifest(id).await?;
        let (initial_manifest, initial_context) = {
            let inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            let context =
                inner.trust_contexts.get(id).cloned().ok_or_else(|| {
                    PluginManagerError::Policy("missing trust decision".to_owned())
                })?;
            (entry.manifest.clone(), context)
        };
        let initial_decision = PluginPolicy::evaluate(
            &initial_manifest,
            &initial_context,
            &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
        )
        .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
        if !initial_decision.allows(capability)
            && !initial_decision.denied_capabilities.iter().any(|denial| {
                &denial.capability == capability
                    && denial.reason == PolicyDenialReason::SignatureStateMismatch
            })
        {
            let reason = initial_decision
                .denied_capabilities
                .iter()
                .find(|denial| &denial.capability == capability)
                .map(|denial| denial.reason.clone())
                .unwrap_or(PolicyDenialReason::CapabilityNotApproved);
            return Err(PluginManagerError::CapabilityDenied {
                id: id.to_owned(),
                capability: capability.clone(),
                reason,
            });
        }
        let (manifest, trust_context, publisher_store) = self.verified_context(id).await?;
        let inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        let current_context = inner
            .trust_contexts
            .get(id)
            .ok_or_else(|| PluginManagerError::Policy("missing trust decision".to_owned()))?;
        if entry.manifest != manifest
            || current_context.trust != trust_context.trust
            || current_context.approved_permissions != trust_context.approved_permissions
            || current_context.approved_capabilities != trust_context.approved_capabilities
            || inner.publisher_store != publisher_store
        {
            return Err(PluginManagerError::ManifestChanged);
        }
        let decision = PluginPolicy::evaluate(
            &entry.manifest,
            &trust_context,
            &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
        )
        .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
        if decision.allows(capability) {
            Ok(())
        } else {
            let reason = decision
                .denied_capabilities
                .iter()
                .find(|denial| &denial.capability == capability)
                .map(|denial| denial.reason.clone())
                .unwrap_or(PolicyDenialReason::CapabilityNotApproved);
            Err(PluginManagerError::CapabilityDenied {
                id: id.to_owned(),
                capability: capability.clone(),
                reason,
            })
        }
    }

    async fn verified_context(
        &self,
        id: &str,
    ) -> Result<(PluginManifest, PluginTrustContext, PluginPublisherStore), PluginManagerError>
    {
        let (manifest, context, artifact_root, publisher_store) = {
            let inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            let context =
                inner.trust_contexts.get(id).cloned().ok_or_else(|| {
                    PluginManagerError::Policy("missing trust decision".to_owned())
                })?;
            (
                entry.manifest.clone(),
                context,
                entry.artifact_root.clone(),
                inner.publisher_store.clone(),
            )
        };
        validate_executable_in_artifact(&manifest.executable, &artifact_root)?;
        let package_hash = hash_plugin_artifact_tree(artifact_root).await?;
        if let Err(error) = publisher_store.verify_manifest_signature(&manifest, &package_hash) {
            global_diagnostics().increment(DiagnosticCounter::PluginVerificationFailures);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Plugins,
                    "plugin_verification_failed",
                    DiagnosticLevel::Error,
                )
                .with_plugin_id(id)
                .with_error_code("PLUGIN_VERIFICATION_FAILED"),
            );
            return Err(PluginManagerError::Signing(error));
        }
        let verified_context = PluginTrustContext::verified_for_manifest(
            &manifest,
            &package_hash,
            &publisher_store,
            context.trust,
            context.approved_permissions,
            context.approved_capabilities,
        )
        .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
        Ok((manifest, verified_context, publisher_store))
    }

    pub async fn policy_decision(
        &self,
        id: &str,
    ) -> Result<PluginPolicyDecision, PluginManagerError> {
        let (initial_manifest, initial_context) = {
            let inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            let context =
                inner.trust_contexts.get(id).cloned().ok_or_else(|| {
                    PluginManagerError::Policy("missing trust decision".to_owned())
                })?;
            (entry.manifest.clone(), context)
        };
        let environment = PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec());
        let initial_decision =
            PluginPolicy::evaluate(&initial_manifest, &initial_context, &environment)
                .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
        if !initial_decision.fully_allowed()
            && initial_decision
                .denied_capabilities
                .iter()
                .any(|denial| denial.reason != PolicyDenialReason::SignatureStateMismatch)
        {
            return Ok(initial_decision);
        }

        let (manifest, verified_context, publisher_store) = self.verified_context(id).await?;
        let inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        let current_context = inner
            .trust_contexts
            .get(id)
            .ok_or_else(|| PluginManagerError::Policy("missing trust decision".to_owned()))?;
        if entry.manifest != manifest
            || current_context.trust != verified_context.trust
            || current_context.approved_permissions != verified_context.approved_permissions
            || current_context.approved_capabilities != verified_context.approved_capabilities
            || inner.publisher_store != publisher_store
        {
            return Err(PluginManagerError::TrustChanged);
        }
        PluginPolicy::evaluate(&entry.manifest, &verified_context, &environment)
            .map_err(|error| PluginManagerError::Policy(error.to_string()))
    }

    /// Unregisters a plugin.  The plugin must be stopped before unregistering.
    pub async fn unregister(&self, id: &str) -> Result<(), PluginManagerError> {
        let mut inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        if matches!(entry.state, PluginLifecycleState::Running) {
            return Err(PluginManagerError::MustStopFirst(id.to_owned()));
        }
        inner.entries.remove(id);
        inner.trust_contexts.remove(id);
        Ok(())
    }

    /// Starts the plugin process and performs the initial handshake.
    ///
    /// # Security
    /// - The executable path was validated when the manifest was parsed.
    /// - Arguments are structured; no shell string construction.
    /// - No secrets are forwarded to the plugin process.
    pub async fn start(&self, id: &str) -> Result<(), PluginManagerError> {
        self.reload_discovered_manifest(id).await?;
        let (snapshot_manifest, snapshot_context, artifact_root, publisher_store, sandbox_config) = {
            let inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            if matches!(entry.state, PluginLifecycleState::Running) {
                return Ok(()); // already running
            }
            if entry.sandbox_disabled {
                return Err(PluginManagerError::SandboxDisabled(id.to_owned()));
            }
            let trust_context =
                inner.trust_contexts.get(id).cloned().ok_or_else(|| {
                    PluginManagerError::Policy("missing trust decision".to_owned())
                })?;
            let initial_decision = PluginPolicy::evaluate(
                &entry.manifest,
                &trust_context,
                &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
            )
            .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
            if !initial_decision.fully_allowed()
                && initial_decision
                    .denied_capabilities
                    .iter()
                    .any(|denial| denial.reason != PolicyDenialReason::SignatureStateMismatch)
            {
                return Err(PluginManagerError::PolicyDenied(
                    initial_decision.denied_capabilities,
                ));
            }
            (
                entry.manifest.clone(),
                trust_context,
                entry.artifact_root.clone(),
                inner.publisher_store.clone(),
                inner.sandbox_config.clone(),
            )
        };

        validate_executable_in_artifact(&snapshot_manifest.executable, &artifact_root)?;
        let package_hash = hash_plugin_artifact_tree(artifact_root).await?;
        if let Err(error) =
            publisher_store.verify_manifest_signature(&snapshot_manifest, &package_hash)
        {
            global_diagnostics().increment(DiagnosticCounter::PluginVerificationFailures);
            global_diagnostics().record(
                DiagnosticEvent::new(
                    DiagnosticComponent::Plugins,
                    "plugin_verification_failed",
                    DiagnosticLevel::Error,
                )
                .with_plugin_id(id)
                .with_error_code("PLUGIN_VERIFICATION_FAILED"),
            );
            return Err(PluginManagerError::Signing(error));
        }
        let trust_context = PluginTrustContext::verified_for_manifest(
            &snapshot_manifest,
            &package_hash,
            &publisher_store,
            snapshot_context.trust.clone(),
            snapshot_context.approved_permissions.clone(),
            snapshot_context.approved_capabilities.clone(),
        )
        .map_err(|error| PluginManagerError::Policy(error.to_string()))?;

        let manifest = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            let current_context = inner
                .trust_contexts
                .get(id)
                .ok_or_else(|| PluginManagerError::Policy("missing trust decision".to_owned()))?;
            if entry.manifest != snapshot_manifest
                || current_context != &snapshot_context
                || inner.publisher_store != publisher_store
            {
                return Err(PluginManagerError::TrustChanged);
            }
            let decision = PluginPolicy::evaluate(
                &entry.manifest,
                &trust_context,
                &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
            )
            .map_err(|error| PluginManagerError::Policy(error.to_string()))?;
            if !decision.fully_allowed() {
                return Err(PluginManagerError::PolicyDenied(
                    decision.denied_capabilities,
                ));
            }
            let manifest = entry.manifest.clone();
            inner
                .trust_contexts
                .insert(id.to_owned(), trust_context.clone());
            inner
                .entries
                .get_mut(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?
                .state = PluginLifecycleState::Starting;
            manifest
        };

        // Spawn — no shell, structured args only, no secret env vars.
        let mut command = Command::new(&manifest.executable);
        command
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        for key in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let mut inner = self.inner.lock().await;
                if let Some(entry) = inner.entries.get_mut(id) {
                    entry.state = PluginLifecycleState::Failed {
                        reason: "process could not be started".to_owned(),
                    };
                }
                return Err(PluginManagerError::SpawnFailed(error.to_string()));
            }
        };

        // Perform handshake within the timeout.
        let handshake_result = timeout(
            sandbox_config.startup_timeout,
            do_handshake(
                &mut child,
                &manifest,
                &trust_context.approved_capabilities,
                &sandbox_config,
            ),
        )
        .await;

        match handshake_result {
            Ok(Ok(())) => {
                let mut inner = self.inner.lock().await;
                let current_context = inner.trust_contexts.get(id);
                let still_allowed = inner.publisher_store == publisher_store
                    && inner.entries.get(id).is_some_and(|entry| {
                        current_context.is_some_and(|context| {
                            PluginPolicy::evaluate(
                                &entry.manifest,
                                context,
                                &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
                            )
                            .is_ok_and(|decision| decision.fully_allowed())
                        })
                    });
                if !still_allowed {
                    if let Some(entry) = inner.entries.get_mut(id) {
                        entry.state = PluginLifecycleState::Stopped;
                        entry.restart_count = 0;
                        entry.last_failure = None;
                        entry.sandbox_disabled = false;
                    }
                    drop(inner);
                    let _ = child.kill().await;
                    return Err(PluginManagerError::PolicyDenied(Vec::new()));
                }
                if let Some(e) = inner.entries.get_mut(id) {
                    e.state = PluginLifecycleState::Running;
                    e.process = Some(child);
                    e.restart_count = 0;
                    e.last_failure = None;
                    e.sandbox_disabled = false;
                }
                global_diagnostics().increment(DiagnosticCounter::PluginStarts);
                global_diagnostics().record(
                    DiagnosticEvent::new(
                        DiagnosticComponent::Plugins,
                        "plugin_started",
                        DiagnosticLevel::Info,
                    )
                    .with_plugin_id(id),
                );
                info!(plugin_id = %id, "plugin started and handshake complete");
                Ok(())
            }
            Ok(Err(handshake_err)) => {
                // Kill the orphaned process.
                let _ = child.kill().await;
                let restart_disabled = self
                    .record_runtime_failure(id, &handshake_err.to_string())
                    .await?;
                if restart_disabled {
                    return Err(PluginManagerError::SandboxDisabled(id.to_owned()));
                }
                Err(handshake_err)
            }
            Err(_elapsed) => {
                let _ = child.kill().await;
                let restart_disabled = self
                    .record_runtime_failure(id, "handshake timed out")
                    .await?;
                if restart_disabled {
                    return Err(PluginManagerError::SandboxDisabled(id.to_owned()));
                }
                Err(PluginManagerError::HandshakeTimeout)
            }
        }
    }

    /// Sends a `Shutdown` message and waits for the process to exit.
    /// Kills the process if it does not exit within [`SHUTDOWN_TIMEOUT`].
    pub async fn stop(&self, id: &str) -> Result<(), PluginManagerError> {
        let mut child = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get_mut(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            if !matches!(entry.state, PluginLifecycleState::Running) {
                return Ok(());
            }
            entry.state = PluginLifecycleState::Stopping;
            entry.process.take()
        };

        if let Some(ref mut child) = child {
            // Send shutdown request.
            let shutdown_req = HostRequest {
                version: PROTOCOL_VERSION,
                body: HostRequestBody::Shutdown,
            };
            if let Some(stdin) = child.stdin.as_mut() {
                let json = serde_json::to_vec(&shutdown_req).unwrap_or_default();
                let length = (json.len() as u32).to_le_bytes();
                // Best-effort write — plugin may have already exited.
                let _ = stdin.write_all(&length).await;
                let _ = stdin.write_all(&json).await;
                let _ = stdin.flush().await;
            }
            // Wait with timeout.
            let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
            let _ = timeout_at(deadline, child.wait()).await;
            // Kill if still running.
            let _ = child.kill().await;
        }

        let mut inner = self.inner.lock().await;
        if let Some(e) = inner.entries.get_mut(id) {
            e.state = PluginLifecycleState::Stopped;
            e.process = None;
        }
        global_diagnostics().record(
            DiagnosticEvent::new(
                DiagnosticComponent::Plugins,
                "plugin_stopped",
                DiagnosticLevel::Info,
            )
            .with_plugin_id(id),
        );
        info!(plugin_id = %id, "plugin stopped");
        Ok(())
    }

    /// Stops all running plugins.  Errors are logged but not propagated.
    pub async fn stop_all(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.lock().await;
            inner
                .entries
                .iter()
                .filter(|(_, e)| matches!(e.state, PluginLifecycleState::Running))
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in ids {
            if let Err(e) = self.stop(&id).await {
                warn!(plugin_id = %id, error = %e, "error stopping plugin");
            }
        }
    }

    /// Returns a snapshot of all registered plugin metadata.
    pub async fn list(&self) -> Vec<PluginSummary> {
        let inner = self.inner.lock().await;
        inner
            .entries
            .values()
            .map(|e| PluginSummary {
                id: e.manifest.id.clone(),
                name: e.manifest.name.clone(),
                version: e.manifest.version.to_string(),
                state: e.state.clone(),
                capabilities: e
                    .manifest
                    .capabilities
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect(),
            })
            .collect()
    }

    /// Returns the summary for a single plugin, or `None` if not found.
    pub async fn get(&self, id: &str) -> Option<PluginSummary> {
        let inner = self.inner.lock().await;
        inner.entries.get(id).map(|e| PluginSummary {
            id: e.manifest.id.clone(),
            name: e.manifest.name.clone(),
            version: e.manifest.version.to_string(),
            state: e.state.clone(),
            capabilities: e
                .manifest
                .capabilities
                .iter()
                .map(|c| format!("{c:?}"))
                .collect(),
        })
    }
}

// ============================================================
// Handshake helper (no lock held)
// ============================================================

async fn do_handshake(
    child: &mut Child,
    manifest: &PluginManifest,
    approved_capabilities: &[PluginCapability],
    sandbox_config: &PluginSandboxConfig,
) -> Result<(), PluginManagerError> {
    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| PluginManagerError::HandshakeFailed("no stdin".to_owned()))?;
    let stdout = child
        .stdout
        .as_mut()
        .ok_or_else(|| PluginManagerError::HandshakeFailed("no stdout".to_owned()))?;

    // Send Hello.
    let hello = HostRequest {
        version: PROTOCOL_VERSION,
        body: HostRequestBody::Hello(HelloRequest {
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            expected_plugin_id: manifest.id.clone(),
        }),
    };
    let json = serde_json::to_vec(&hello)
        .map_err(|e| PluginManagerError::HandshakeFailed(format!("serialisation error: {e}")))?;
    let length = (json.len() as u32).to_le_bytes();
    stdin
        .write_all(&length)
        .await
        .map_err(|e| PluginManagerError::HandshakeFailed(e.to_string()))?;
    stdin
        .write_all(&json)
        .await
        .map_err(|e| PluginManagerError::HandshakeFailed(e.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|e| PluginManagerError::HandshakeFailed(e.to_string()))?;

    // Read HelloAck.
    let mut len_buf = [0u8; 4];
    stdout
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| PluginManagerError::HandshakeFailed(e.to_string()))?;
    let resp_len = u32::from_le_bytes(len_buf);
    sandbox_config
        .validate_ipc_length(resp_len)
        .map_err(|error| {
            PluginManagerError::HandshakeFailed(format!(
                "response too large ({resp_len} bytes): {error}"
            ))
        })?;
    let mut buf = vec![0u8; resp_len as usize];
    stdout
        .read_exact(&mut buf)
        .await
        .map_err(|e| PluginManagerError::HandshakeFailed(e.to_string()))?;

    let resp = parse_plugin_response(&buf)
        .map_err(|e| PluginManagerError::HandshakeFailed(e.to_string()))?;

    match resp.body {
        PluginResponseBody::HelloAck(ack) => {
            validate_hello_ack(manifest, approved_capabilities, &ack)?;
            debug!(plugin_id = %ack.plugin_id, version = %ack.plugin_version, "handshake complete");
            Ok(())
        }
        PluginResponseBody::Error(err) => Err(PluginManagerError::HandshakeFailed(format!(
            "plugin error: {} — {}",
            err.code, err.message
        ))),
        _ => Err(PluginManagerError::HandshakeFailed(
            "unexpected response to Hello".to_owned(),
        )),
    }
}

fn validate_hello_ack(
    manifest: &PluginManifest,
    approved_capabilities: &[PluginCapability],
    ack: &fetchx_plugin_api::protocol::HelloAckResponse,
) -> Result<(), PluginManagerError> {
    if ack.plugin_id != manifest.id {
        return Err(PluginManagerError::HandshakeFailed(format!(
            "plugin identified itself as `{}` but manifest says `{}`",
            ack.plugin_id, manifest.id
        )));
    }
    let plugin_version =
        fetchx_plugin_api::PluginVersion::parse(&ack.plugin_version).map_err(|error| {
            PluginManagerError::HandshakeFailed(format!(
                "plugin reported an invalid semantic version: {error}"
            ))
        })?;
    if plugin_version != manifest.version {
        return Err(PluginManagerError::HandshakeFailed(
            "plugin version does not match the validated manifest".to_owned(),
        ));
    }
    let mut reported = std::collections::HashSet::new();
    for capability in &ack.capabilities {
        if !reported.insert(capability)
            || !manifest.capabilities.contains(capability)
            || !approved_capabilities.contains(capability)
        {
            return Err(PluginManagerError::HandshakeFailed(
                "plugin reported an undeclared or unapproved capability".to_owned(),
            ));
        }
    }
    Ok(())
}

// ============================================================
// Public summary type
// ============================================================

/// A non-sensitive snapshot of a plugin's current state.
#[derive(Clone, Debug)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub state: PluginLifecycleState,
    pub capabilities: Vec<String>,
}

// ============================================================
// Discovery result
// ============================================================

#[derive(Clone, Debug, Default)]
pub struct DiscoverResult {
    /// Subdirectories with a `plugin.json`.
    pub found: usize,
    /// Successfully registered plugins.
    pub registered: usize,
    /// Manifests that failed validation.
    pub invalid: usize,
}

// ============================================================
// Error type
// ============================================================

#[derive(Debug, Error)]
pub enum PluginManagerError {
    #[error("plugin `{0}` is already registered")]
    DuplicateId(String),
    #[error("plugin `{0}` was not found")]
    NotFound(String),
    #[error("plugin `{0}` must be stopped before it can be unregistered")]
    MustStopFirst(String),
    #[error("plugin `{0}` was disabled by the supervisor after repeated runtime failures")]
    SandboxDisabled(String),
    #[error("plugin manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("IPC payload is too large ({payload_len} bytes, max {max_payload_len})")]
    OversizedPayload {
        payload_len: u32,
        max_payload_len: u32,
    },
    #[error("plugin manifest changed since registration; prior approvals were revoked")]
    ManifestChanged,
    #[error("could not reload plugin manifest `{path}`: {reason}")]
    ManifestReadFailed { path: String, reason: String },
    #[error("plugin trust decision contains undeclared permissions or capabilities")]
    InvalidTrustDecision,
    #[error("plugin policy evaluation failed: {0}")]
    Policy(String),
    #[error("plugin signature verification failed: {0}")]
    Signing(#[from] PluginSigningError),
    #[error("plugin trust or publisher state changed during verification")]
    TrustChanged,
    #[error("could not hash plugin artifact: {0}")]
    ArtifactHash(String),
    #[error("plugin policy denied requested capabilities: {0:?}")]
    PolicyDenied(Vec<fetchx_plugin_api::CapabilityDenial>),
    #[error("plugin `{id}` is not authorized for capability {capability:?}: {reason:?}")]
    CapabilityDenied {
        id: String,
        capability: PluginCapability,
        reason: PolicyDenialReason,
    },
    #[error("plugin executable not found: {0}")]
    ExecutableNotFound(String),
    #[error("could not spawn plugin process: {0}")]
    SpawnFailed(String),
    #[error("plugin handshake timed out")]
    HandshakeTimeout,
    #[error("plugin handshake failed: {0}")]
    HandshakeFailed(String),
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
}

fn validate_executable_in_artifact(
    executable: &std::path::Path,
    artifact_root: &std::path::Path,
) -> Result<(), PluginManagerError> {
    if !executable.is_file() {
        return Err(PluginManagerError::ExecutableNotFound(
            executable.to_string_lossy().into_owned(),
        ));
    }
    let root = std::fs::canonicalize(artifact_root)
        .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
    let executable_path = std::fs::canonicalize(executable)
        .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
    if !executable_path.starts_with(root) {
        return Err(PluginManagerError::ArtifactHash(
            "plugin executable is outside its artifact directory".to_owned(),
        ));
    }
    Ok(())
}

async fn hash_plugin_artifact_tree(root: PathBuf) -> Result<String, PluginManagerError> {
    tokio::task::spawn_blocking(move || hash_plugin_artifact_tree_sync(&root))
        .await
        .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?
}

fn hash_plugin_artifact_tree_sync(root: &std::path::Path) -> Result<String, PluginManagerError> {
    let mut files = Vec::new();
    collect_artifact_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    hasher.update(b"FetchX plugin artifact tree v1\0");
    for (relative_path, absolute_path) in files {
        let contents = std::fs::read(&absolute_path)
            .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
        let path_bytes = relative_path.as_bytes();
        let path_len = u64::try_from(path_bytes.len())
            .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
        let content_len = u64::try_from(contents.len())
            .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
        hasher.update(path_len.to_le_bytes());
        hasher.update(path_bytes);
        hasher.update(content_len.to_le_bytes());
        hasher.update(contents);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn collect_artifact_files(
    root: &std::path::Path,
    current: &std::path::Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), PluginManagerError> {
    let entries = std::fs::read_dir(current)
        .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
        if relative == std::path::Path::new("plugin.json") {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| PluginManagerError::ArtifactHash(error.to_string()))?;
        if file_type.is_symlink() {
            return Err(PluginManagerError::ArtifactHash(
                "symbolic links are not allowed in plugin artifacts".to_owned(),
            ));
        }
        if file_type.is_dir() {
            collect_artifact_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = relative.to_str().ok_or_else(|| {
                PluginManagerError::ArtifactHash(
                    "plugin artifact contains a non-Unicode path".to_owned(),
                )
            })?;
            files.push((relative.replace('\\', "/"), path));
        } else {
            return Err(PluginManagerError::ArtifactHash(
                "plugin artifact contains a non-regular file".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer, SigningKey};
    use fetchx_plugin_api::{
        PLUGIN_API_VERSION, PLUGIN_MANIFEST_VERSION, PLUGIN_SIGNATURE_VERSION, PluginCapability,
        PluginCompatibility, PluginManifest, PluginPermission, PluginPublisherStore,
        PluginSignature, PluginSignatureAlgorithm, PluginTrust, PluginVersion,
        canonical_manifest_signing_bytes, sha256_hex,
    };

    use super::*;

    fn manifest() -> PluginManifest {
        PluginManifest {
            manifest_version: PLUGIN_MANIFEST_VERSION,
            id: "test-plugin".to_owned(),
            name: "Test Plugin".to_owned(),
            version: PluginVersion::parse("1.0.0").unwrap(),
            api_version: PLUGIN_API_VERSION,
            compatibility: PluginCompatibility {
                minimum_api_version: PLUGIN_API_VERSION,
                maximum_api_version: PLUGIN_API_VERSION,
            },
            publisher: "Test Publisher".to_owned(),
            description: "Test plugin".to_owned(),
            executable: if cfg!(windows) {
                PathBuf::from(r"C:\nonexistent\fetchx-test-plugin.exe")
            } else {
                PathBuf::from("/nonexistent/fetchx-test-plugin")
            },
            permissions: vec![PluginPermission::Downloads],
            capabilities: vec![PluginCapability::DownloadCreate],
            platforms: vec![],
            signature: None,
        }
    }

    #[tokio::test]
    async fn unknown_plugin_cannot_start_or_receive_capabilities() {
        let manager = PluginManager::new(PathBuf::from("unused"));
        manager.register(manifest()).await.unwrap();

        assert!(matches!(
            manager
                .authorize_capability("test-plugin", &PluginCapability::DownloadCreate)
                .await,
            Err(PluginManagerError::CapabilityDenied {
                reason: PolicyDenialReason::TrustUnknown,
                ..
            })
        ));
        assert!(matches!(
            manager.start("test-plugin").await,
            Err(PluginManagerError::PolicyDenied(_))
        ));
        assert!(matches!(
            manager.get("test-plugin").await.unwrap().state,
            PluginLifecycleState::Registered
        ));
    }

    #[tokio::test]
    async fn approved_capability_is_still_denied_without_a_runtime_adapter() {
        let manager = PluginManager::new(PathBuf::from("unused"));
        manager.register(manifest()).await.unwrap();
        manager
            .set_trust_decision(
                "test-plugin",
                PluginTrust::Trusted,
                vec![PluginPermission::Downloads],
                vec![PluginCapability::DownloadCreate],
            )
            .await
            .unwrap();

        assert!(matches!(
            manager
                .authorize_capability("test-plugin", &PluginCapability::DownloadCreate)
                .await,
            Err(PluginManagerError::CapabilityDenied {
                reason: PolicyDenialReason::CapabilityUnsupported,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn plugin_runtime_failure_triggers_crash_loop_detection() {
        let manager = PluginManager::new(PathBuf::from("unused"));
        manager.register(manifest()).await.unwrap();

        let disabled = manager
            .record_runtime_failure("test-plugin", "failure 1")
            .await
            .unwrap();
        assert!(!disabled);

        let disabled = manager
            .record_runtime_failure("test-plugin", "failure 2")
            .await
            .unwrap();
        assert!(!disabled);

        let disabled = manager
            .record_runtime_failure("test-plugin", "failure 3")
            .await
            .unwrap();
        assert!(disabled);

        assert!(matches!(
            manager.start("test-plugin").await,
            Err(PluginManagerError::SandboxDisabled(_))
        ));
    }

    #[test]
    fn oversized_ipc_lengths_are_rejected() {
        let config = PluginSandboxConfig::default();
        let max = config.max_ipc_payload_bytes;

        assert!(config.validate_ipc_length(max).is_ok());
        assert!(matches!(
            config.validate_ipc_length(max + 1),
            Err(PluginManagerError::OversizedPayload { .. })
        ));
    }

    #[tokio::test]
    async fn trust_decision_cannot_grant_undeclared_capability() {
        let manager = PluginManager::new(PathBuf::from("unused"));
        manager.register(manifest()).await.unwrap();

        assert!(matches!(
            manager
                .set_trust_decision(
                    "test-plugin",
                    PluginTrust::Trusted,
                    vec![PluginPermission::Downloads],
                    vec![PluginCapability::DownloadRemove],
                )
                .await,
            Err(PluginManagerError::InvalidTrustDecision)
        ));
    }

    #[tokio::test]
    async fn approval_for_one_plugin_cannot_be_reused_by_another() {
        let manager = PluginManager::new(PathBuf::from("unused"));
        manager.register(manifest()).await.unwrap();
        let mut second = manifest();
        second.id = "other-plugin".to_owned();
        second.executable = if cfg!(windows) {
            PathBuf::from(r"C:\nonexistent\fetchx-other-plugin.exe")
        } else {
            PathBuf::from("/nonexistent/fetchx-other-plugin")
        };
        manager.register(second).await.unwrap();
        manager
            .set_trust_decision(
                "test-plugin",
                PluginTrust::Trusted,
                vec![PluginPermission::Downloads],
                vec![PluginCapability::DownloadCreate],
            )
            .await
            .unwrap();

        assert!(matches!(
            manager
                .authorize_capability("other-plugin", &PluginCapability::DownloadCreate)
                .await,
            Err(PluginManagerError::CapabilityDenied {
                reason: PolicyDenialReason::TrustUnknown,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn changed_discovered_manifest_revokes_prior_approval_before_start() {
        let directory = tempfile::tempdir().unwrap();
        let manifest_path = directory.path().join("plugin.json");
        let original = manifest();
        tokio::fs::write(&manifest_path, serde_json::to_vec(&original).unwrap())
            .await
            .unwrap();

        let manager = PluginManager::new(directory.path().to_path_buf());
        manager
            .register_discovered(original.clone(), manifest_path.clone())
            .await
            .unwrap();
        manager
            .set_trust_decision(
                "test-plugin",
                PluginTrust::Trusted,
                vec![PluginPermission::Downloads],
                vec![PluginCapability::DownloadCreate],
            )
            .await
            .unwrap();
        assert_eq!(
            manager
                .policy_decision("test-plugin")
                .await
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::CapabilityUnsupported
        );

        let mut modified = original;
        modified.publisher = "Changed Publisher".to_owned();
        tokio::fs::write(&manifest_path, serde_json::to_vec(&modified).unwrap())
            .await
            .unwrap();

        assert!(matches!(
            manager.start("test-plugin").await,
            Err(PluginManagerError::ManifestChanged)
        ));
        assert_eq!(
            manager
                .policy_decision("test-plugin")
                .await
                .unwrap()
                .denied_capabilities[0]
                .reason,
            PolicyDenialReason::TrustUnknown
        );
    }

    #[test]
    fn artifact_tree_hash_covers_payload_and_resources_but_not_manifest_json() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("resources")).unwrap();
        std::fs::write(root.join("plugin.exe"), b"payload-v1").unwrap();
        std::fs::write(root.join("resources").join("icon.bin"), b"resource-v1").unwrap();
        std::fs::write(root.join("plugin.json"), b"manifest-v1").unwrap();
        let first = hash_plugin_artifact_tree_sync(root).unwrap();

        std::fs::write(root.join("plugin.json"), b"manifest-v2").unwrap();
        assert_eq!(hash_plugin_artifact_tree_sync(root).unwrap(), first);

        std::fs::write(root.join("resources").join("icon.bin"), b"resource-v2").unwrap();
        assert_ne!(hash_plugin_artifact_tree_sync(root).unwrap(), first);
    }

    #[tokio::test]
    async fn manager_verifies_signed_artifact_before_start_and_rejects_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let plugin_dir = directory.path().join("signed-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let executable = plugin_dir.join(if cfg!(windows) {
            "plugin.exe"
        } else {
            "plugin"
        });
        std::fs::write(&executable, b"test executable bytes").unwrap();
        std::fs::create_dir(plugin_dir.join("resources")).unwrap();
        std::fs::write(
            plugin_dir.join("resources").join("asset.bin"),
            b"test resource bytes",
        )
        .unwrap();

        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
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
            publisher: "Publisher".to_owned(),
            description: "Test artifact".to_owned(),
            executable,
            permissions: vec![PluginPermission::Downloads],
            capabilities: vec![PluginCapability::DownloadCreate],
            platforms: vec![],
            signature: None,
        };
        let package_hash = hash_plugin_artifact_tree_sync(&plugin_dir).unwrap();
        manifest.signature = Some(PluginSignature {
            version: PLUGIN_SIGNATURE_VERSION,
            algorithm: PluginSignatureAlgorithm::Ed25519,
            publisher_id: "publisher.test".to_owned(),
            key_id: sha256_hex(&signing_key.verifying_key().to_bytes()),
            package_sha256: package_hash,
            signature: STANDARD.encode([0_u8; 64]),
        });
        let payload = canonical_manifest_signing_bytes(&manifest).unwrap();
        manifest.signature.as_mut().unwrap().signature =
            STANDARD.encode(signing_key.sign(&payload).to_bytes());
        let manifest_path = plugin_dir.join("plugin.json");
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let mut publisher_store = PluginPublisherStore::default();
        publisher_store
            .add_publisher_key(
                "publisher.test",
                &STANDARD.encode(signing_key.verifying_key().to_bytes()),
            )
            .unwrap();
        publisher_store
            .set_publisher_trust("publisher.test", PluginTrust::Trusted)
            .unwrap();
        let manager = PluginManager::new_with_publisher_store(
            directory.path().to_path_buf(),
            publisher_store,
        );
        manager
            .register_discovered(manifest.clone(), manifest_path)
            .await
            .unwrap();
        manager
            .set_trust_decision(
                "signed-plugin",
                PluginTrust::Trusted,
                vec![PluginPermission::Downloads],
                vec![PluginCapability::DownloadCreate],
            )
            .await
            .unwrap();

        let decision = manager.policy_decision("signed-plugin").await.unwrap();
        assert_eq!(
            decision.denied_capabilities[0].reason,
            PolicyDenialReason::CapabilityUnsupported
        );

        let valid_result = manager.start("signed-plugin").await;
        assert!(matches!(
            valid_result,
            Err(PluginManagerError::PolicyDenied(denials))
                if denials.iter().all(|denial| {
                    denial.reason == PolicyDenialReason::CapabilityUnsupported
                })
        ));

        std::fs::write(plugin_dir.join("resources").join("asset.bin"), b"tampered").unwrap();
        assert!(matches!(
            manager.start("signed-plugin").await,
            Err(PluginManagerError::Signing(
                PluginSigningError::PackageHashMismatch
            ))
        ));
    }

    #[tokio::test]
    async fn explicit_trust_does_not_skip_executable_validation() {
        let manager = PluginManager::new(PathBuf::from("unused"));
        manager.register(manifest()).await.unwrap();
        manager
            .set_trust_decision(
                "test-plugin",
                PluginTrust::Trusted,
                vec![PluginPermission::Downloads],
                vec![PluginCapability::DownloadCreate],
            )
            .await
            .unwrap();

        assert!(matches!(
            manager.start("test-plugin").await,
            Err(PluginManagerError::PolicyDenied(_))
        ));
    }

    #[test]
    fn handshake_requires_matching_identity_and_version() {
        let manifest = manifest();
        let approved = vec![PluginCapability::DownloadCreate];
        let mut ack = fetchx_plugin_api::protocol::HelloAckResponse {
            plugin_id: manifest.id.clone(),
            plugin_version: manifest.version.to_string(),
            capabilities: approved.clone(),
        };
        assert!(validate_hello_ack(&manifest, &approved, &ack).is_ok());

        ack.plugin_id = "other-plugin".to_owned();
        assert!(matches!(
            validate_hello_ack(&manifest, &approved, &ack),
            Err(PluginManagerError::HandshakeFailed(_))
        ));

        ack.plugin_id = manifest.id.clone();
        ack.plugin_version = "2.0.0".to_owned();
        assert!(matches!(
            validate_hello_ack(&manifest, &approved, &ack),
            Err(PluginManagerError::HandshakeFailed(_))
        ));
    }

    #[test]
    fn handshake_rejects_unapproved_duplicate_or_undeclared_capability() {
        let manifest = manifest();
        let mut ack = fetchx_plugin_api::protocol::HelloAckResponse {
            plugin_id: manifest.id.clone(),
            plugin_version: manifest.version.to_string(),
            capabilities: vec![PluginCapability::DownloadCreate],
        };
        assert!(validate_hello_ack(&manifest, &[], &ack).is_err());
        ack.capabilities = vec![
            PluginCapability::DownloadCreate,
            PluginCapability::DownloadCreate,
        ];
        assert!(validate_hello_ack(&manifest, &ack.capabilities, &ack).is_err());
        ack.capabilities = vec![PluginCapability::DownloadRemove];
        assert!(validate_hello_ack(&manifest, &ack.capabilities, &ack).is_err());
    }
}

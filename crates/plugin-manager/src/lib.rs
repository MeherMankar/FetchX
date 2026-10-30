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

use fetchx_plugin_api::{
    HelloRequest, HostRequest, HostRequestBody, PROTOCOL_VERSION, PluginCapability, PluginManifest,
    PluginPermission, PluginPolicy, PluginPolicyDecision, PluginResponseBody, PluginTrust,
    PluginTrustContext, PolicyDenialReason, PolicyEnvironment, parse_manifest,
    parse_plugin_response,
};
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
const SUPPORTED_PLUGIN_CAPABILITIES: &[PluginCapability] = &[];

// ============================================================
// Plugin entry (runtime state)
// ============================================================

/// Runtime record for a registered plugin.
#[derive(Debug)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginLifecycleState,
    manifest_path: Option<PathBuf>,
    /// Set while the plugin process is running.
    process: Option<Child>,
}

impl PluginEntry {
    fn new(manifest: PluginManifest, manifest_path: Option<PathBuf>) -> Self {
        Self {
            manifest,
            state: PluginLifecycleState::Registered,
            manifest_path,
            process: None,
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
}

impl PluginManager {
    /// Creates a new manager that scans `plugin_dir` for plugins.
    pub fn new(plugin_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ManagerInner {
                entries: HashMap::new(),
                trust_contexts: HashMap::new(),
                plugin_dir,
            }),
        })
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
        let inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        let context = inner
            .trust_contexts
            .get(id)
            .ok_or_else(|| PluginManagerError::Policy("missing trust decision".to_owned()))?;
        let decision = PluginPolicy::evaluate(
            &entry.manifest,
            context,
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

    pub async fn policy_decision(
        &self,
        id: &str,
    ) -> Result<PluginPolicyDecision, PluginManagerError> {
        let inner = self.inner.lock().await;
        let entry = inner
            .entries
            .get(id)
            .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
        let context = inner
            .trust_contexts
            .get(id)
            .ok_or_else(|| PluginManagerError::Policy("missing trust decision".to_owned()))?;
        PluginPolicy::evaluate(
            &entry.manifest,
            context,
            &PolicyEnvironment::current(SUPPORTED_PLUGIN_CAPABILITIES.to_vec()),
        )
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
        // Clone what we need before dropping the lock.
        let (manifest, trust_context) = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            if matches!(entry.state, PluginLifecycleState::Running) {
                return Ok(()); // already running
            }
            let trust_context =
                inner.trust_contexts.get(id).cloned().ok_or_else(|| {
                    PluginManagerError::Policy("missing trust decision".to_owned())
                })?;
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
                .entries
                .get_mut(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?
                .state = PluginLifecycleState::Starting;
            (manifest, trust_context)
        };

        // Validate the executable exists before spawning.
        if !manifest.executable.is_file() {
            let mut inner = self.inner.lock().await;
            if let Some(e) = inner.entries.get_mut(id) {
                e.state = PluginLifecycleState::Failed {
                    reason: "executable not found".to_owned(),
                };
            }
            return Err(PluginManagerError::ExecutableNotFound(
                manifest.executable.to_string_lossy().into_owned(),
            ));
        }

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
            HANDSHAKE_TIMEOUT,
            do_handshake(&mut child, &manifest, &trust_context.approved_capabilities),
        )
        .await;

        match handshake_result {
            Ok(Ok(())) => {
                let mut inner = self.inner.lock().await;
                let current_context = inner.trust_contexts.get(id);
                let still_allowed = inner.entries.get(id).is_some_and(|entry| {
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
                    }
                    drop(inner);
                    let _ = child.kill().await;
                    return Err(PluginManagerError::PolicyDenied(Vec::new()));
                }
                if let Some(e) = inner.entries.get_mut(id) {
                    e.state = PluginLifecycleState::Running;
                    e.process = Some(child);
                }
                info!(plugin_id = %id, "plugin started and handshake complete");
                Ok(())
            }
            Ok(Err(handshake_err)) => {
                // Kill the orphaned process.
                let _ = child.kill().await;
                let mut inner = self.inner.lock().await;
                if let Some(e) = inner.entries.get_mut(id) {
                    e.state = PluginLifecycleState::Failed {
                        reason: handshake_err.to_string(),
                    };
                }
                Err(handshake_err)
            }
            Err(_elapsed) => {
                let _ = child.kill().await;
                let reason = "handshake timed out".to_owned();
                let mut inner = self.inner.lock().await;
                if let Some(e) = inner.entries.get_mut(id) {
                    e.state = PluginLifecycleState::Failed {
                        reason: reason.clone(),
                    };
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
    if resp_len > fetchx_plugin_api::protocol::MAX_IPC_PAYLOAD_BYTES {
        return Err(PluginManagerError::HandshakeFailed(format!(
            "response too large ({resp_len} bytes)"
        )));
    }
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
    #[error("plugin manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("plugin manifest changed since registration; prior approvals were revoked")]
    ManifestChanged,
    #[error("could not reload plugin manifest `{path}`: {reason}")]
    ManifestReadFailed { path: String, reason: String },
    #[error("plugin trust decision contains undeclared permissions or capabilities")]
    InvalidTrustDecision,
    #[error("plugin policy evaluation failed: {0}")]
    Policy(String),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fetchx_plugin_api::{
        PLUGIN_API_VERSION, PLUGIN_MANIFEST_VERSION, PluginCapability, PluginCompatibility,
        PluginManifest, PluginPermission, PluginVersion,
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

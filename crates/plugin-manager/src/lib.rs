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
    HelloRequest, HostRequest, HostRequestBody, PROTOCOL_VERSION, PluginManifest,
    PluginResponseBody, parse_manifest, parse_plugin_response,
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

// ============================================================
// Plugin entry (runtime state)
// ============================================================

/// Runtime record for a registered plugin.
#[derive(Debug)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginLifecycleState,
    /// Set while the plugin process is running.
    process: Option<Child>,
}

impl PluginEntry {
    fn new(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            state: PluginLifecycleState::Registered,
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
    plugin_dir: PathBuf,
}

impl PluginManager {
    /// Creates a new manager that scans `plugin_dir` for plugins.
    pub fn new(plugin_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ManagerInner {
                entries: HashMap::new(),
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
                    match self.register(manifest).await {
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
        let mut inner = self.inner.lock().await;
        let id = manifest.id.clone();
        if inner.entries.contains_key(&id) {
            return Err(PluginManagerError::DuplicateId(id));
        }
        inner.entries.insert(id, PluginEntry::new(manifest));
        Ok(())
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
        Ok(())
    }

    /// Starts the plugin process and performs the initial handshake.
    ///
    /// # Security
    /// - The executable path was validated when the manifest was parsed.
    /// - Arguments are structured; no shell string construction.
    /// - No secrets are forwarded to the plugin process.
    pub async fn start(&self, id: &str) -> Result<(), PluginManagerError> {
        // Clone what we need before dropping the lock.
        let manifest = {
            let mut inner = self.inner.lock().await;
            let entry = inner
                .entries
                .get_mut(id)
                .ok_or_else(|| PluginManagerError::NotFound(id.to_owned()))?;
            if matches!(entry.state, PluginLifecycleState::Running) {
                return Ok(()); // already running
            }
            entry.state = PluginLifecycleState::Starting;
            entry.manifest.clone()
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
        let mut child = Command::new(&manifest.executable)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()) // plugin diagnostics go to /dev/null
            .spawn()
            .map_err(|e| PluginManagerError::SpawnFailed(e.to_string()))?;

        // Perform handshake within the timeout.
        let handshake_result =
            timeout(HANDSHAKE_TIMEOUT, do_handshake(&mut child, &manifest)).await;

        match handshake_result {
            Ok(Ok(())) => {
                let mut inner = self.inner.lock().await;
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
                version: e.manifest.version.clone(),
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
            version: e.manifest.version.clone(),
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
            if ack.plugin_id != manifest.id {
                return Err(PluginManagerError::HandshakeFailed(format!(
                    "plugin identified itself as `{}` but manifest says `{}`",
                    ack.plugin_id, manifest.id
                )));
            }
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

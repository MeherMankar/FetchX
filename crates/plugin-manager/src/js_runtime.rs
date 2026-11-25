//! JavaScript plugin runner via Node.js subprocess.
//!
//! When `node` is available on PATH, JS plugins are executed by spawning a
//! child `node` process, writing the event JSON to its stdin, and reading
//! stdout for log output and stderr for errors.  When `node` is not found the
//! runner degrades gracefully: `run_hook` returns `Ok(())` without spawning
//! anything.

use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Maximum wall-clock time allowed for a single JS hook invocation.
const JS_HOOK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum JsError {
    #[error("node process spawn failed: {0}")]
    SpawnFailed(String),
    #[error("could not write event to node stdin: {0}")]
    StdinWrite(String),
    #[error("JS plugin hook timed out after 5 seconds")]
    Timeout,
    #[error("node process exited with non-zero status: {0}")]
    NonZeroExit(i32),
    #[error("I/O error while reading node output: {0}")]
    Io(String),
}

/// Discovers and runs JavaScript plugins by spawning `node` as a subprocess.
#[derive(Debug, Clone)]
pub struct JsPluginRunner {
    /// Resolved path to the `node` executable, or `None` when not found.
    node_path: Option<PathBuf>,
}

impl JsPluginRunner {
    /// Searches PATH for a `node` / `node.exe` executable and returns a runner
    /// that wraps it.  When nothing is found, `node_path` is `None` and every
    /// `run_hook` call returns `Ok(())` immediately.
    pub fn detect() -> Self {
        let node_path = find_node_on_path();
        if let Some(ref path) = node_path {
            tracing::info!(path = %path.display(), "JS plugin runner: node detected");
        } else {
            tracing::debug!("JS plugin runner: node not found on PATH – JS plugins disabled");
        }
        Self { node_path }
    }

    /// Returns `true` when a `node` executable was found at construction time.
    pub fn is_available(&self) -> bool {
        self.node_path.is_some()
    }

    /// Spawns `node <script_path>` with `event_json` written to stdin.
    ///
    /// - stdout lines are logged at DEBUG level.
    /// - stderr lines are logged at WARN level.
    /// - Returns `Ok(())` immediately when `node` was not found (graceful
    ///   degradation).
    /// - Returns `Ok(())` when `node` exits with status 0.
    /// - Returns an error for spawn failures, write failures, timeouts, or
    ///   non-zero exit codes.
    pub async fn run_hook(&self, script_path: &Path, event_json: &str) -> Result<(), JsError> {
        let Some(ref node) = self.node_path else {
            // Node not available – degrade gracefully.
            return Ok(());
        };

        let fut = async {
            let mut child = Command::new(node)
                .arg(script_path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| JsError::SpawnFailed(e.to_string()))?;

            // Write event JSON to stdin and close it so the script sees EOF.
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(event_json.as_bytes())
                    .await
                    .map_err(|e| JsError::StdinWrite(e.to_string()))?;
                // Drop stdin to signal EOF.
            }

            let output = child
                .wait_with_output()
                .await
                .map_err(|e| JsError::Io(e.to_string()))?;

            // Log stdout at debug level.
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                tracing::debug!(plugin = %script_path.display(), "[js stdout] {}", line);
            }

            // Log stderr at warn level.
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines() {
                tracing::warn!(plugin = %script_path.display(), "[js stderr] {}", line);
            }

            if !output.status.success() {
                let code = output.status.code().unwrap_or(-1);
                return Err(JsError::NonZeroExit(code));
            }

            Ok(())
        };

        timeout(JS_HOOK_TIMEOUT, fut)
            .await
            .map_err(|_| JsError::Timeout)?
    }
}

/// Searches well-known locations and PATH for a `node` executable.
fn find_node_on_path() -> Option<PathBuf> {
    // Try the standard binary name first (relies on PATH).
    let candidates: &[&str] = if cfg!(windows) {
        &["node.exe", "node"]
    } else {
        &["node", "node.exe"]
    };

    for name in candidates {
        if let Ok(path) = which_node(name) {
            return Some(path);
        }
    }
    None
}

/// Minimal `which`-style lookup: iterate PATH entries and return the first
/// existing file match.
fn which_node(name: &str) -> Result<PathBuf, ()> {
    let path_var = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

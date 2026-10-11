//! Desktop IPC state file reading and shared types.

use std::path::PathBuf;

use mydownloader_browser_protocol::BrowserRequest;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// State file content written by the desktop on startup.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesktopState {
    /// Loopback TCP port the browser-integration server is listening on.
    pub port: u16,
    /// Per-session opaque token.  Must be sent with every request.
    /// Never logged; never included in responses to the browser.
    pub token: String,
    /// App version string for the handshake.
    pub app_version: String,
}

/// A request forwarded from the native host to the desktop IPC server.
/// The token authenticates the request as originating from the local native host.
#[derive(Debug, Serialize, Deserialize)]
pub struct DesktopIpcRequest {
    /// Session token.  Validated server-side; never echoed to the browser.
    pub token: String,
    /// The original browser request.
    pub request: BrowserRequest,
}

#[derive(Debug, Error)]
pub enum StateFileError {
    #[error("state file not found — is MyDownloader running?")]
    NotFound,
    #[error("state file could not be read: {0}")]
    Io(#[source] std::io::Error),
    #[error("state file contents are invalid")]
    Invalid,
}

/// Returns the expected path of the desktop browser-IPC state file.
///
/// The file lives in the platform's per-user app-data directory so it is
/// inaccessible to other OS users.
pub fn state_file_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // Tauri writes app data to %APPDATA% (Roaming) on Windows.
        std::env::var_os("APPDATA").map(|base| {
            PathBuf::from(base)
                .join("com.mydownloader.desktop")
                .join("browser-ipc.json")
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Linux / macOS: use XDG_RUNTIME_DIR if available, else ~/.local/share
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })?;
        Some(base.join("mydownloader").join("browser-ipc.json"))
    }
}

/// Reads and parses the desktop state file.
pub fn read_desktop_state() -> Result<DesktopState, StateFileError> {
    let path = state_file_path().ok_or(StateFileError::NotFound)?;
    let contents = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StateFileError::NotFound
        } else {
            StateFileError::Io(e)
        }
    })?;
    serde_json::from_str(&contents).map_err(|_| StateFileError::Invalid)
}

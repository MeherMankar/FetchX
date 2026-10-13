//! Platform-specific path resolution and service startup for the CLI.
//!
//! Mirrors the same paths the Tauri desktop application uses so the CLI
//! accesses the same database and downloads folder.

use std::path::PathBuf;

use fetchx_core_services::{CoreServices, CoreServicesConfig, CoreServicesError};

/// Platform-specific path for the application data directory.
fn app_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        // %APPDATA%\com.fetchx.desktop
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("com.fetchx.desktop")
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Library")
            .join("Application Support")
            .join("com.fetchx.desktop")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // XDG_DATA_HOME / ~/.local/share
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".local").join("share"))
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
            })
            .join("FetchX")
    }
}

/// Platform-specific downloads directory.
fn downloads_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Downloads"))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Downloads"))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

/// Start the shared core services using the same paths as the desktop app.
pub async fn start() -> Result<CoreServices, CoreServicesError> {
    let data_dir = app_data_dir();
    let cfg = CoreServicesConfig {
        database_path: data_dir.join("fetchx.db"),
        downloads_dir: downloads_dir(),
        runtime_dir: data_dir.join("runtime"),
    };
    tracing::debug!(
        database = %cfg.database_path.display(),
        downloads = %cfg.downloads_dir.display(),
        "starting core services"
    );
    CoreServices::start(cfg).await
}

/// Returns the plugin directory: `{data_dir}/plugins/`.
pub fn plugin_dir() -> Option<std::path::PathBuf> {
    Some(app_data_dir().join("plugins"))
}

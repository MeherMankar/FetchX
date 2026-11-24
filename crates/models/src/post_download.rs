use serde::{Deserialize, Serialize};

/// Action to perform automatically when a download completes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PostDownloadAction {
    /// Do nothing (default).
    #[default]
    Nothing,
    /// Open the downloaded file with the system default application.
    OpenFile,
    /// Open the folder that contains the downloaded file.
    OpenFolder,
    /// Run a shell command. The following environment variables are set:
    ///
    /// - `FETCHX_FILE` / `%FETCHX_FILE%` — absolute path to the downloaded file
    /// - `FETCHX_URL` / `%FETCHX_URL%` — the source URL of the download
    /// - `FETCHX_STATUS` / `%FETCHX_STATUS%` — `"completed"` or `"failed"`
    /// - `FETCHX_SIZE` / `%FETCHX_SIZE%` — file size in bytes as a decimal string
    /// - `FETCHX_FILENAME` / `%FETCHX_FILENAME%` — filename without directory path
    /// - `FETCHX_ID` / `%FETCHX_ID%` — the download ID (UUID string)
    /// - `FETCHX_ENGINE` / `%FETCHX_ENGINE%` — engine kind string (e.g. `"direct_http"`, `"yt_dlp"`)
    RunCommand { command: String },
    /// Extract the downloaded archive into a folder next to it.
    ExtractArchive {
        /// Delete the archive after successful extraction.
        delete_after: bool,
    },
}

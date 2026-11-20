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
    /// Run a shell command. `$FETCHX_FILE` / `%FETCHX_FILE%` is set to the
    /// downloaded file path.
    RunCommand { command: String },
}

use serde::{Deserialize, Serialize};

/// Action to perform when the download queue becomes idle.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PostQueueAction {
    /// Do nothing (default).
    #[default]
    Nothing,
    /// Run a user-specified command string using the system shell.
    RunCommand { command: String },
    /// Shut down the computer.
    Shutdown,
    /// Hibernate the computer.
    Hibernate,
    /// Put the computer to sleep.
    Sleep,
}

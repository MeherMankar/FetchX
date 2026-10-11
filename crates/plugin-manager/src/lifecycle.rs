//! Plugin lifecycle state machine.

/// The lifecycle state of a registered plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginLifecycleState {
    /// Manifest validated and registered; not yet started.
    Registered,
    /// Process is being launched; handshake in progress.
    Starting,
    /// Process is running and handshake was successful.
    Running,
    /// Shutdown requested; waiting for the process to exit.
    Stopping,
    /// Process has exited cleanly.
    Stopped,
    /// Process failed to start or crashed.
    Failed { reason: String },
}

impl PluginLifecycleState {
    /// Returns `true` when the plugin process is live.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// A stable string tag suitable for JSON serialisation / CLI display.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Registered => "registered",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed { .. } => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_is_detected() {
        assert!(PluginLifecycleState::Running.is_running());
        assert!(!PluginLifecycleState::Registered.is_running());
        assert!(!PluginLifecycleState::Stopped.is_running());
    }

    #[test]
    fn as_str_is_stable() {
        assert_eq!(PluginLifecycleState::Registered.as_str(), "registered");
        assert_eq!(PluginLifecycleState::Running.as_str(), "running");
        assert_eq!(
            PluginLifecycleState::Failed {
                reason: "oops".to_owned()
            }
            .as_str(),
            "failed"
        );
    }
}

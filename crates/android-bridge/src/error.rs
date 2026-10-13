use fetchx_core_services::CoreServicesError;
use fetchx_queue::QueueError;
use fetchx_scheduler::ScheduleError;

/// Error type that maps all internal errors to a flat mobile-friendly enum.
/// Variants must match the [Error] enum in android_bridge.udl exactly.
#[derive(Debug, thiserror::Error)]
pub enum MobileApiError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("engine unavailable: {0}")]
    UnavailableEngine(String),
    #[error("invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("runtime error: {0}")]
    Runtime(String),
}

impl From<QueueError> for MobileApiError {
    fn from(e: QueueError) -> Self {
        match e {
            QueueError::DownloadNotFound(id) => {
                MobileApiError::NotFound(format!("download {id} not found"))
            }
            QueueError::InvalidStatus { id, action, status } => {
                MobileApiError::InvalidStateTransition(format!(
                    "download {id} cannot perform {action} while in {status:?}"
                ))
            }
            QueueError::InvalidConfiguration => {
                MobileApiError::Validation("invalid queue configuration".to_string())
            }
            QueueError::Stopped => {
                MobileApiError::Runtime("queue manager has shut down".to_string())
            }
            QueueError::PauseSuperseded => {
                MobileApiError::Runtime("pause request was superseded by resume".to_string())
            }
            QueueError::UnsupportedEngineOperation { engine, operation } => {
                MobileApiError::UnavailableEngine(format!(
                    "{engine:?} engine does not support {operation}"
                ))
            }
            QueueError::Storage(e) => MobileApiError::Storage(e.to_string()),
            QueueError::ResourceLimitInUse => {
                MobileApiError::Runtime("resource limit is currently in use".to_string())
            }
            QueueError::ResourceInvariant => {
                MobileApiError::Runtime("queue resource invariant violated".to_string())
            }
            QueueError::StateMachine(e) => {
                // StateMachineError::NotFound should surface as NotFound, not InvalidStateTransition.
                use fetchx_core::StateMachineError;
                match e {
                    StateMachineError::NotFound(id) => {
                        MobileApiError::NotFound(format!("download {id} not found"))
                    }
                    other => MobileApiError::InvalidStateTransition(other.to_string()),
                }
            }
            QueueError::Engine(e) => MobileApiError::Runtime(e.to_string()),
            QueueError::Worker(e) => MobileApiError::Runtime(e.to_string()),
            QueueError::Actor(e) => MobileApiError::Runtime(e.to_string()),
            QueueError::ShutdownChannelClosed => {
                MobileApiError::Runtime("queue shutdown channel closed unexpectedly".to_string())
            }
        }
    }
}

impl From<ScheduleError> for MobileApiError {
    fn from(e: ScheduleError) -> Self {
        match e {
            ScheduleError::DownloadNotFound(id) => {
                MobileApiError::NotFound(format!("download {id} not found"))
            }
            ScheduleError::InvalidState { id, status } => MobileApiError::InvalidStateTransition(
                format!("download {id} cannot be scheduled while in {status:?}"),
            ),
            ScheduleError::Stopped => {
                MobileApiError::Runtime("scheduler has shut down".to_string())
            }
            ScheduleError::Storage(e) => MobileApiError::Storage(e.to_string()),
            ScheduleError::Queue(e) => MobileApiError::from(e),
            ScheduleError::Timestamp(e) => MobileApiError::Runtime(e.to_string()),
            ScheduleError::InvalidSchedule(e) => MobileApiError::Validation(e.to_string()),
            ScheduleError::ScheduleNotFound(id) => {
                MobileApiError::NotFound(format!("schedule for download {id} not found"))
            }
            ScheduleError::Actor(e) => MobileApiError::Runtime(e.to_string()),
        }
    }
}

impl From<CoreServicesError> for MobileApiError {
    fn from(e: CoreServicesError) -> Self {
        MobileApiError::Runtime(e.to_string())
    }
}

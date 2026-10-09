#![forbid(unsafe_code)]

mod state_machine;

pub use state_machine::{DownloadStateMachine, StateMachineError, validate_transition};

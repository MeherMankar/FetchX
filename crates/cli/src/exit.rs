//! Documented exit codes for the CLI.

/// Exit code type alias.
pub type ExitCode = u8;

pub const SUCCESS: ExitCode = 0;
pub const GENERAL_ERROR: ExitCode = 1;
pub const INVALID_ARGS: ExitCode = 2;
pub const NOT_FOUND: ExitCode = 3;
pub const INVALID_STATE: ExitCode = 4;
pub const ENGINE_UNAVAILABLE: ExitCode = 5;
#[allow(dead_code)]
pub const CONFIG_ERROR: ExitCode = 6;

//! Headless HTTP and WebSocket host for the shared FetchX remote protocol.

#![forbid(unsafe_code)]

mod auth;
mod config;
mod dispatch;
mod events;
mod transport;

pub use auth::{AuthError, AuthenticationService, initialize_auth, rotate_auth};
pub use config::{DEFAULT_PORT, ServerConfig, ServerConfigError};
pub use transport::{RemoteServer, ServerError};

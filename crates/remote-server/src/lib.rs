//! Headless HTTP and WebSocket host for the shared FetchX remote protocol.

#![forbid(unsafe_code)]

mod config;
mod dispatch;
mod events;
mod transport;

pub use config::{DEFAULT_PORT, ServerConfig, ServerConfigError};
pub use transport::{RemoteServer, ServerError};

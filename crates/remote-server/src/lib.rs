//! Headless HTTP and WebSocket host for the shared FetchX remote protocol.

#![forbid(unsafe_code)]

mod auth;
mod authorization;
mod config;
mod dispatch;
mod events;
mod tickets;
mod transport;

pub use auth::{
    AuthError, AuthenticationService, current_auth_role, initialize_auth, rotate_auth,
    set_auth_role,
};
pub use authorization::Role;
pub use config::{DEFAULT_PORT, ServerConfig, ServerConfigError, TlsConfig};
pub use transport::{RemoteServer, ServerError};

//! FetchX Plugin API — Phase 19
//!
//! Shared types used by both the FetchX host and external plugin processes.
//!
//! # Protocol overview
//!
//! Plugins run as child processes.  The host communicates with each plugin
//! via its stdin/stdout using a simple length-prefixed JSON framing:
//!
//! ```text
//! ┌──────────────────────┬─────────────────────────────┐
//! │  4 bytes (LE u32)    │  N bytes UTF-8 JSON         │
//! │  payload length = N  │  { "version": 1, ... }      │
//! └───────────────────────┴─────────────────────────────┘
//! ```
//!
//! Maximum payload size: [`MAX_PAYLOAD_BYTES`] (64 KiB).
//!
//! # Security
//! - All request types use [`#[serde(deny_unknown_fields)]`] where applicable.
//! - Plugin processes do NOT receive secrets, credentials, cookies, or tokens.
//! - The plugin directory path is validated by the host before any process is started.
//! - Plugin executables must be regular files at an absolute path.
//!
//! # Versioning
//! The current protocol version is [`PROTOCOL_VERSION`].
//! A plugin that reports a different version during handshake is rejected.

#![forbid(unsafe_code)]

pub mod framing;
pub mod manifest;
pub mod protocol;

// Re-export commonly used items.
pub use framing::{FramingError, read_frame, write_frame};
pub use manifest::{
    Capability, ManifestValidationError, PLUGIN_API_VERSION, Platform, PluginManifest,
    PluginManifestRaw, parse_manifest,
};
pub use protocol::{
    HelloRequest, HostRequest, HostRequestBody, PROTOCOL_VERSION, PluginResponse,
    PluginResponseBody, ProtocolError, parse_plugin_response,
};


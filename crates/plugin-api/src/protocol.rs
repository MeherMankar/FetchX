//! Host ↔ Plugin IPC protocol messages.
//!
//! The host sends [`HostRequest`] messages; the plugin responds with
//! [`PluginResponse`] messages.  All messages carry a `version` field;
//! version mismatches are rejected before any capability-specific processing.
//!
//! # Lifecycle
//! ```text
//! Host spawns plugin process
//!   → Host sends Hello
//!   ← Plugin sends HelloAck (with capabilities)
//!   → Host sends Ping (optional health check)
//!   ← Plugin sends Pong
//!   → Host sends Shutdown
//!   ← Plugin sends ShutdownAck
//!   → Plugin process exits
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current IPC protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum JSON payload for a single IPC message (64 KiB).
pub const MAX_IPC_PAYLOAD_BYTES: u32 = 65_536;

// ============================================================
// Host → Plugin requests
// ============================================================

/// A message sent from the host to a plugin process.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostRequest {
    pub version: u32,
    #[serde(flatten)]
    pub body: HostRequestBody,
}

/// Typed request body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostRequestBody {
    /// Initial handshake.  The plugin must respond with `HelloAck`.
    Hello(HelloRequest),
    /// Health check.  The plugin must respond with `Pong`.
    Ping,
    /// Ordered shutdown.  The plugin must respond with `ShutdownAck` then exit.
    Shutdown,
    /// Request plugin status information.
    Status,
}

/// Payload of the `Hello` request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelloRequest {
    /// Host application version.
    pub host_version: String,
    /// Plugin ID the host is expecting (for verification).
    pub expected_plugin_id: String,
}

// ============================================================
// Plugin → Host responses
// ============================================================

/// A message sent from a plugin process to the host.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginResponse {
    pub version: u32,
    #[serde(flatten)]
    pub body: PluginResponseBody,
}

/// Typed response body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginResponseBody {
    /// Successful hello handshake acknowledgement.
    HelloAck(HelloAckResponse),
    /// Response to a `Ping`.
    Pong(PongResponse),
    /// Acknowledgement of an ordered shutdown.
    ShutdownAck,
    /// Response to a `Status` request.
    StatusInfo(StatusInfoResponse),
    /// The plugin encountered an error and cannot continue.
    Error(PluginErrorResponse),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelloAckResponse {
    pub plugin_id: String,
    pub plugin_version: String,
    /// Capabilities the plugin declares (subset of what the manifest claimed).
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PongResponse {
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatusInfoResponse {
    pub plugin_id: String,
    pub uptime_seconds: u64,
    pub status: String,
}

/// Error response — never contains secrets, stack traces, or filesystem paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginErrorResponse {
    pub code: String,
    pub message: String,
}

// ============================================================
// Error type
// ============================================================

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IPC payload is too large ({0} bytes, max {MAX_IPC_PAYLOAD_BYTES})")]
    PayloadTooLarge(u32),
    #[error("JSON deserialisation failed: {0}")]
    Json(String),
    #[error("protocol version mismatch: host expects {PROTOCOL_VERSION}, plugin sent {0}")]
    VersionMismatch(u32),
    #[error("unexpected response type for this request")]
    UnexpectedResponseType,
}

/// Parse a raw byte slice into a [`PluginResponse`].
pub fn parse_plugin_response(bytes: &[u8]) -> Result<PluginResponse, ProtocolError> {
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    if len > MAX_IPC_PAYLOAD_BYTES {
        return Err(ProtocolError::PayloadTooLarge(len));
    }
    let resp: PluginResponse =
        serde_json::from_slice(bytes).map_err(|e| ProtocolError::Json(e.to_string()))?;
    if resp.version != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch(resp.version));
    }
    Ok(resp)
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn hello_request() -> HostRequest {
        HostRequest {
            version: PROTOCOL_VERSION,
            body: HostRequestBody::Hello(HelloRequest {
                host_version: "0.1.0".to_owned(),
                expected_plugin_id: "my-plugin".to_owned(),
            }),
        }
    }

    fn hello_ack() -> PluginResponse {
        PluginResponse {
            version: PROTOCOL_VERSION,
            body: PluginResponseBody::HelloAck(HelloAckResponse {
                plugin_id: "my-plugin".to_owned(),
                plugin_version: "0.1.0".to_owned(),
                capabilities: vec!["engine".to_owned()],
            }),
        }
    }

    #[test]
    fn hello_request_roundtrips() {
        let req = hello_request();
        let json = serde_json::to_vec(&req).unwrap();
        let parsed: HostRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(req, parsed);
    }

    #[test]
    fn hello_ack_roundtrips() {
        let resp = hello_ack();
        let json = serde_json::to_vec(&resp).unwrap();
        let parsed = parse_plugin_response(&json).unwrap();
        assert_eq!(resp, parsed);
    }

    #[test]
    fn version_mismatch_rejected() {
        let resp = PluginResponse {
            version: 99,
            body: PluginResponseBody::ShutdownAck,
        };
        let json = serde_json::to_vec(&resp).unwrap();
        assert!(matches!(
            parse_plugin_response(&json),
            Err(ProtocolError::VersionMismatch(99))
        ));
    }

    #[test]
    fn oversized_payload_rejected() {
        let large = vec![0u8; (MAX_IPC_PAYLOAD_BYTES + 1) as usize];
        assert!(matches!(
            parse_plugin_response(&large),
            Err(ProtocolError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn malformed_json_rejected() {
        assert!(parse_plugin_response(b"{not json}").is_err());
    }

    #[test]
    fn ping_pong_roundtrip() {
        let ping = HostRequest {
            version: PROTOCOL_VERSION,
            body: HostRequestBody::Ping,
        };
        let json = serde_json::to_vec(&ping).unwrap();
        let parsed: HostRequest = serde_json::from_slice(&json).unwrap();
        assert_eq!(ping, parsed);

        let pong = PluginResponse {
            version: PROTOCOL_VERSION,
            body: PluginResponseBody::Pong(PongResponse {
                status: "ok".to_owned(),
            }),
        };
        let json = serde_json::to_vec(&pong).unwrap();
        let parsed = parse_plugin_response(&json).unwrap();
        assert_eq!(pong, parsed);
    }

    #[test]
    fn shutdown_ack_roundtrip() {
        let resp = PluginResponse {
            version: PROTOCOL_VERSION,
            body: PluginResponseBody::ShutdownAck,
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let parsed = parse_plugin_response(&json).unwrap();
        assert_eq!(resp, parsed);
    }

    #[test]
    fn error_response_contains_no_sensitive_fields() {
        let err = PluginResponse {
            version: PROTOCOL_VERSION,
            body: PluginResponseBody::Error(PluginErrorResponse {
                code: "startup_failed".to_owned(),
                message: "Could not initialise plugin.".to_owned(),
            }),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("token"));
        assert!(!json.contains("password"));
    }
}

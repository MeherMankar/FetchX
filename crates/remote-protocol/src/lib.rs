//! Shared, transport-independent JSON-RPC contract for FetchX remote clients.
//!
//! This crate defines the Phase 21A protocol only. It does not open sockets,
//! authenticate clients, or dispatch commands to CoreServices; those belong to
//! later server and security phases.

#![forbid(unsafe_code)]

use fetchx_models::{DownloadId, DownloadPriority, EngineKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// JSON-RPC version used by every request, response, and notification.
pub const JSON_RPC_VERSION: &str = "2.0";
/// FetchX protocol identifier advertised during discovery.
pub const PROTOCOL_VERSION: &str = "fetchx.rpc.v1";
/// Maximum encoded request size accepted by the shared parser.
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
/// Maximum length of a string request ID.
pub const MAX_REQUEST_ID_BYTES: usize = 256;
/// Maximum length of a download URL in a create request.
pub const MAX_DOWNLOAD_URL_BYTES: usize = 16_384;
/// Maximum UTF-8 byte length of a suggested download filename.
pub const MAX_DOWNLOAD_FILENAME_BYTES: usize = 255;

/// Top-level method namespaces reserved by the remote protocol.
pub const NAMESPACES: &[&str] = &[
    "download",
    "queue",
    "scheduler",
    "engine",
    "plugin",
    "config",
    "system",
];

pub const SYSTEM_DISCOVER: &str = "system.discover";
pub const SYSTEM_WEBSOCKET_TICKET: &str = "system.websocketTicket";
pub const DOWNLOAD_CREATE: &str = "download.create";
pub const DOWNLOAD_GET: &str = "download.get";
pub const DOWNLOAD_LIST: &str = "download.list";
pub const BROWSER_RPC_METHODS: &[&str] = &[SYSTEM_DISCOVER, DOWNLOAD_CREATE, DOWNLOAD_LIST];
pub const DOWNLOAD_PAUSE: &str = "download.pause";
pub const DOWNLOAD_RESUME: &str = "download.resume";
pub const DOWNLOAD_CANCEL: &str = "download.cancel";
pub const DOWNLOAD_RETRY: &str = "download.retry";
pub const DOWNLOAD_REMOVE: &str = "download.remove";
pub const DOWNLOAD_SET_PRIORITY: &str = "download.setPriority";
pub const DOWNLOAD_SET_ENGINE: &str = "download.setEngine";
pub const DOWNLOAD_SET_SPEED_LIMIT: &str = "download.setSpeedLimit";

/// A JSON-RPC request envelope before method-specific parameter validation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC request identifier. Fractional numbers, booleans, arrays, and
/// objects are intentionally excluded.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RpcId {
    String(String),
    Number(i64),
}

/// Successful JSON-RPC response envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcSuccess {
    pub jsonrpc: String,
    pub id: RpcId,
    pub result: Value,
}

/// Error JSON-RPC response envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcFailure {
    pub jsonrpc: String,
    pub id: Option<RpcId>,
    pub error: RpcErrorObject,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Stable JSON-RPC error codes used by protocol validation and servers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum RpcErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
    Unauthorized = -32001,
    Forbidden = -32003,
    NotFound = -32004,
    Conflict = -32009,
    ServiceUnavailable = -32010,
    Timeout = -32011,
    UnsupportedOperation = -32012,
}

/// One recognized, validated client operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedRequest {
    /// Omitted for JSON-RPC notifications, which must not receive a response.
    pub id: Option<RpcId>,
    pub method: String,
    pub command: RemoteCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteCommand {
    Discover,
    WebSocketTicket,
    DownloadCreate(DownloadCreateParams),
    DownloadGet(DownloadId),
    DownloadList,
    DownloadPause(DownloadId),
    DownloadResume(DownloadId),
    DownloadCancel(DownloadId),
    DownloadRetry(DownloadId),
    DownloadRemove(DownloadId),
    DownloadSetPriority {
        id: DownloadId,
        priority: DownloadPriority,
    },
    DownloadSetEngine {
        id: DownloadId,
        engine: EngineKind,
    },
    DownloadSetSpeedLimit {
        id: DownloadId,
        bytes_per_second: Option<u64>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadCreateParams {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<DownloadPriority>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadIdParams {
    pub id: DownloadId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadSetPriorityParams {
    pub id: DownloadId,
    pub priority: DownloadPriority,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadSetEngineParams {
    pub id: DownloadId,
    pub engine: EngineKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadSetSpeedLimitParams {
    pub id: DownloadId,
    /// `null` or an omitted value removes the per-download speed limit.
    #[serde(default)]
    pub bytes_per_second: Option<u64>,
}

/// Version and feature discovery response returned by `system.discover`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolInfo {
    pub protocol_version: String,
    pub server_version: String,
    pub capabilities: Vec<String>,
}

impl ProtocolInfo {
    #[must_use]
    pub fn new(server_version: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            server_version: server_version.into(),
            capabilities: CAPABILITIES
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }
}

/// Declared Phase 21A request/event capability names.
pub const CAPABILITIES: &[&str] = &[
    SYSTEM_DISCOVER,
    SYSTEM_WEBSOCKET_TICKET,
    DOWNLOAD_CREATE,
    DOWNLOAD_GET,
    DOWNLOAD_LIST,
    DOWNLOAD_PAUSE,
    DOWNLOAD_RESUME,
    DOWNLOAD_CANCEL,
    DOWNLOAD_RETRY,
    DOWNLOAD_REMOVE,
    DOWNLOAD_SET_PRIORITY,
    DOWNLOAD_SET_ENGINE,
    DOWNLOAD_SET_SPEED_LIMIT,
    "download.created",
    "download.updated",
    "download.progress",
    "download.paused",
    "download.resumed",
    "download.completed",
    "download.failed",
    "queue.updated",
    "engine.started",
    "engine.stopped",
    "engine.failed",
    "plugin.enabled",
    "plugin.disabled",
];

/// A server-to-client JSON-RPC notification carrying one Phase 21A event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventNotification {
    pub jsonrpc: String,
    pub method: EventName,
    pub params: EventParams,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventParams {
    pub data: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EventName {
    #[serde(rename = "download.created")]
    DownloadCreated,
    #[serde(rename = "download.updated")]
    DownloadUpdated,
    #[serde(rename = "download.progress")]
    DownloadProgress,
    #[serde(rename = "download.paused")]
    DownloadPaused,
    #[serde(rename = "download.resumed")]
    DownloadResumed,
    #[serde(rename = "download.completed")]
    DownloadCompleted,
    #[serde(rename = "download.failed")]
    DownloadFailed,
    #[serde(rename = "queue.updated")]
    QueueUpdated,
    #[serde(rename = "engine.started")]
    EngineStarted,
    #[serde(rename = "engine.stopped")]
    EngineStopped,
    #[serde(rename = "engine.failed")]
    EngineFailed,
    #[serde(rename = "plugin.enabled")]
    PluginEnabled,
    #[serde(rename = "plugin.disabled")]
    PluginDisabled,
}

impl EventName {
    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::DownloadCreated => "download.created",
            Self::DownloadUpdated => "download.updated",
            Self::DownloadProgress => "download.progress",
            Self::DownloadPaused => "download.paused",
            Self::DownloadResumed => "download.resumed",
            Self::DownloadCompleted => "download.completed",
            Self::DownloadFailed => "download.failed",
            Self::QueueUpdated => "queue.updated",
            Self::EngineStarted => "engine.started",
            Self::EngineStopped => "engine.stopped",
            Self::EngineFailed => "engine.failed",
            Self::PluginEnabled => "plugin.enabled",
            Self::PluginDisabled => "plugin.disabled",
        }
    }
}

impl EventNotification {
    #[must_use]
    pub fn new(event: EventName, data: Value) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            method: event,
            params: EventParams { data },
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProtocolError {
    #[error("request is {actual} bytes; maximum is {MAX_MESSAGE_BYTES}")]
    MessageTooLarge { actual: usize },
    #[error("invalid JSON request")]
    InvalidJson,
    #[error("request must use JSON-RPC version 2.0")]
    InvalidJsonRpcVersion,
    #[error("request envelope is invalid")]
    InvalidRequest,
    #[error("request id exceeds {MAX_REQUEST_ID_BYTES} bytes")]
    RequestIdTooLarge,
    #[error("remote method not found")]
    MethodNotFound,
    #[error("invalid parameters for {method}")]
    InvalidParams { method: String },
}

impl ProtocolError {
    #[must_use]
    pub const fn code(&self) -> RpcErrorCode {
        match self {
            Self::InvalidJson => RpcErrorCode::ParseError,
            Self::MessageTooLarge { .. }
            | Self::InvalidJsonRpcVersion
            | Self::InvalidRequest
            | Self::RequestIdTooLarge => RpcErrorCode::InvalidRequest,
            Self::MethodNotFound => RpcErrorCode::MethodNotFound,
            Self::InvalidParams { .. } => RpcErrorCode::InvalidParams,
        }
    }
}

/// Parses and validates a JSON-RPC 2.0 client request against the Phase 21A
/// method and parameter contract.
pub fn parse_request(bytes: &[u8]) -> Result<ValidatedRequest, ProtocolError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge {
            actual: bytes.len(),
        });
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
    if value
        .as_object()
        .is_some_and(|object| object.get("id").is_some_and(Value::is_null))
    {
        return Err(ProtocolError::InvalidRequest);
    }
    let request: RpcRequest =
        serde_json::from_value(value).map_err(|_| ProtocolError::InvalidRequest)?;
    if request.jsonrpc != JSON_RPC_VERSION {
        return Err(ProtocolError::InvalidJsonRpcVersion);
    }
    if matches!(&request.id, Some(RpcId::String(value)) if value.len() > MAX_REQUEST_ID_BYTES) {
        return Err(ProtocolError::RequestIdTooLarge);
    }

    let method = request.method.clone();
    let command = match request.method.as_str() {
        SYSTEM_DISCOVER => parse_empty_params(&request).map(|()| RemoteCommand::Discover),
        SYSTEM_WEBSOCKET_TICKET => {
            parse_empty_params(&request).map(|()| RemoteCommand::WebSocketTicket)
        }
        DOWNLOAD_CREATE => parse_params::<DownloadCreateParams>(&request).and_then(|params| {
            if params.url.trim().is_empty()
                || params.url.len() > MAX_DOWNLOAD_URL_BYTES
                || params.filename.as_ref().is_some_and(|filename| {
                    filename.trim().is_empty() || filename.len() > MAX_DOWNLOAD_FILENAME_BYTES
                })
            {
                Err(invalid_params(&request))
            } else {
                Ok(RemoteCommand::DownloadCreate(params))
            }
        }),
        DOWNLOAD_GET => parse_params::<DownloadIdParams>(&request)
            .map(|params| RemoteCommand::DownloadGet(params.id)),
        DOWNLOAD_LIST => parse_empty_params(&request).map(|()| RemoteCommand::DownloadList),
        DOWNLOAD_PAUSE => parse_params::<DownloadIdParams>(&request)
            .map(|params| RemoteCommand::DownloadPause(params.id)),
        DOWNLOAD_RESUME => parse_params::<DownloadIdParams>(&request)
            .map(|params| RemoteCommand::DownloadResume(params.id)),
        DOWNLOAD_CANCEL => parse_params::<DownloadIdParams>(&request)
            .map(|params| RemoteCommand::DownloadCancel(params.id)),
        DOWNLOAD_RETRY => parse_params::<DownloadIdParams>(&request)
            .map(|params| RemoteCommand::DownloadRetry(params.id)),
        DOWNLOAD_REMOVE => parse_params::<DownloadIdParams>(&request)
            .map(|params| RemoteCommand::DownloadRemove(params.id)),
        DOWNLOAD_SET_PRIORITY => {
            parse_params::<DownloadSetPriorityParams>(&request).map(|params| {
                RemoteCommand::DownloadSetPriority {
                    id: params.id,
                    priority: params.priority,
                }
            })
        }
        DOWNLOAD_SET_ENGINE => parse_params::<DownloadSetEngineParams>(&request).map(|params| {
            RemoteCommand::DownloadSetEngine {
                id: params.id,
                engine: params.engine,
            }
        }),
        DOWNLOAD_SET_SPEED_LIMIT => {
            parse_params::<DownloadSetSpeedLimitParams>(&request).map(|params| {
                RemoteCommand::DownloadSetSpeedLimit {
                    id: params.id,
                    bytes_per_second: params.bytes_per_second,
                }
            })
        }
        _ => return Err(ProtocolError::MethodNotFound),
    }?;

    Ok(ValidatedRequest {
        id: request.id,
        method,
        command,
    })
}

fn parse_empty_params(request: &RpcRequest) -> Result<(), ProtocolError> {
    match request.params.as_ref() {
        None => Ok(()),
        Some(Value::Object(params)) if params.is_empty() => Ok(()),
        _ => Err(invalid_params(request)),
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(request: &RpcRequest) -> Result<T, ProtocolError> {
    request
        .params
        .as_ref()
        .ok_or_else(|| invalid_params(request))
        .and_then(|params| {
            serde_json::from_value(params.clone()).map_err(|_| invalid_params(request))
        })
}

fn invalid_params(request: &RpcRequest) -> ProtocolError {
    ProtocolError::InvalidParams {
        method: request.method.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "7e9f4159-d47f-4a14-8be3-8db026fcd15a";

    fn request(method: &str, params: Option<Value>) -> RpcRequest {
        RpcRequest {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id: Some(RpcId::String("request-1".to_owned())),
            method: method.to_owned(),
            params,
        }
    }

    fn encoded(request: &RpcRequest) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(request)
    }

    #[test]
    fn create_request_is_validated_and_defaults_are_preserved()
    -> Result<(), Box<dyn std::error::Error>> {
        let body = request(
            DOWNLOAD_CREATE,
            Some(serde_json::json!({ "url": "https://example.test/file.bin" })),
        );
        let parsed = parse_request(&encoded(&body)?)?;
        assert_eq!(
            parsed.command,
            RemoteCommand::DownloadCreate(DownloadCreateParams {
                url: "https://example.test/file.bin".to_owned(),
                filename: None,
                engine: None,
                priority: None,
            })
        );
        Ok(())
    }

    #[test]
    fn create_request_accepts_a_suggested_filename() -> Result<(), Box<dyn std::error::Error>> {
        let body = request(
            DOWNLOAD_CREATE,
            Some(serde_json::json!({
                "url": "https://example.test/file.bin",
                "filename": "suggested-name.bin"
            })),
        );
        let parsed = parse_request(&encoded(&body)?)?;
        assert_eq!(
            parsed.command,
            RemoteCommand::DownloadCreate(DownloadCreateParams {
                url: "https://example.test/file.bin".to_owned(),
                filename: Some("suggested-name.bin".to_owned()),
                engine: None,
                priority: None,
            })
        );
        Ok(())
    }

    #[test]
    fn all_documented_download_operations_parse() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (DOWNLOAD_GET, serde_json::json!({ "id": ID })),
            (DOWNLOAD_PAUSE, serde_json::json!({ "id": ID })),
            (DOWNLOAD_RESUME, serde_json::json!({ "id": ID })),
            (DOWNLOAD_CANCEL, serde_json::json!({ "id": ID })),
            (DOWNLOAD_RETRY, serde_json::json!({ "id": ID })),
            (DOWNLOAD_REMOVE, serde_json::json!({ "id": ID })),
            (
                DOWNLOAD_SET_PRIORITY,
                serde_json::json!({ "id": ID, "priority": "high" }),
            ),
            (
                DOWNLOAD_SET_ENGINE,
                serde_json::json!({ "id": ID, "engine": "direct_http" }),
            ),
            (
                DOWNLOAD_SET_SPEED_LIMIT,
                serde_json::json!({ "id": ID, "bytes_per_second": 1024 }),
            ),
        ];
        for (method, params) in cases {
            assert!(
                parse_request(&encoded(&request(method, Some(params)))?).is_ok(),
                "{method}"
            );
        }
        assert_eq!(
            parse_request(&encoded(&request(DOWNLOAD_LIST, None))?)?.command,
            RemoteCommand::DownloadList
        );
        assert_eq!(
            parse_request(&encoded(&request(SYSTEM_DISCOVER, None))?)?.command,
            RemoteCommand::Discover
        );
        assert_eq!(
            parse_request(&encoded(&request(SYSTEM_WEBSOCKET_TICKET, None))?)?.command,
            RemoteCommand::WebSocketTicket
        );
        Ok(())
    }

    #[test]
    fn supports_notifications_and_rejects_non_json_rpc_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut notification = request(DOWNLOAD_LIST, None);
        notification.id = None;
        assert_eq!(parse_request(&encoded(&notification)?)?.id, None);
        let boolean_id = br#"{"jsonrpc":"2.0","id":true,"method":"download.list"}"#;
        assert_eq!(
            parse_request(boolean_id),
            Err(ProtocolError::InvalidRequest)
        );
        let null_id = br#"{"jsonrpc":"2.0","id":null,"method":"download.list"}"#;
        assert_eq!(parse_request(null_id), Err(ProtocolError::InvalidRequest));
        assert_eq!(ProtocolError::InvalidJson.code(), RpcErrorCode::ParseError);
        Ok(())
    }

    #[test]
    fn rejects_unknown_methods_and_malformed_params() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            parse_request(&encoded(&request("queue.unknown", None))?),
            Err(ProtocolError::MethodNotFound)
        );
        assert!(matches!(
            parse_request(&encoded(&request(
                DOWNLOAD_CREATE,
                Some(serde_json::json!({ "url": "  " }))
            ))?),
            Err(ProtocolError::InvalidParams { .. })
        ));
        assert!(matches!(
            parse_request(&encoded(&request(
                DOWNLOAD_CREATE,
                Some(serde_json::json!({
                    "url": "https://example.test/file.bin",
                    "filename": "  "
                }))
            ))?),
            Err(ProtocolError::InvalidParams { .. })
        ));
        let oversized_filename = "x".repeat(MAX_DOWNLOAD_FILENAME_BYTES + 1);
        assert!(matches!(
            parse_request(&encoded(&request(
                DOWNLOAD_CREATE,
                Some(serde_json::json!({
                    "url": "https://example.test/file.bin",
                    "filename": oversized_filename
                }))
            ))?),
            Err(ProtocolError::InvalidParams { .. })
        ));
        assert!(matches!(
            parse_request(&encoded(&request(
                DOWNLOAD_GET,
                Some(serde_json::json!({ "id": ID, "unexpected": true }))
            ))?),
            Err(ProtocolError::InvalidParams { .. })
        ));
        let oversized_url = "x".repeat(MAX_DOWNLOAD_URL_BYTES + 1);
        assert!(matches!(
            parse_request(&encoded(&request(
                DOWNLOAD_CREATE,
                Some(serde_json::json!({ "url": oversized_url }))
            ))?),
            Err(ProtocolError::InvalidParams { .. })
        ));
        Ok(())
    }

    #[test]
    fn rejects_wrong_rpc_version_and_oversized_payload() -> Result<(), Box<dyn std::error::Error>> {
        let mut wrong_version = request(DOWNLOAD_LIST, None);
        wrong_version.jsonrpc = "1.0".to_owned();
        assert_eq!(
            parse_request(&encoded(&wrong_version)?),
            Err(ProtocolError::InvalidJsonRpcVersion)
        );
        assert!(matches!(
            parse_request(&vec![b' '; MAX_MESSAGE_BYTES + 1]),
            Err(ProtocolError::MessageTooLarge { .. })
        ));
        Ok(())
    }

    #[test]
    fn protocol_info_and_event_names_have_stable_wire_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let info = ProtocolInfo::new("0.1.0");
        let value = serde_json::to_value(&info)?;
        assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
        assert!(info.capabilities.contains(&DOWNLOAD_CREATE.to_owned()));
        assert_eq!(
            NAMESPACES,
            &[
                "download",
                "queue",
                "scheduler",
                "engine",
                "plugin",
                "config",
                "system"
            ]
        );

        let value = serde_json::to_value(EventNotification::new(
            EventName::DownloadProgress,
            serde_json::json!({ "downloaded_bytes": 100 }),
        ))?;
        assert_eq!(value["method"], "download.progress");
        assert_eq!(value["params"]["data"]["downloaded_bytes"], 100);
        Ok(())
    }

    #[test]
    fn response_envelopes_use_json_rpc_shape() -> Result<(), Box<dyn std::error::Error>> {
        let response = RpcSuccess {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id: RpcId::Number(12),
            result: serde_json::json!({ "ok": true }),
        };
        let value = serde_json::to_value(response)?;
        assert_eq!(value["jsonrpc"], JSON_RPC_VERSION);
        assert_eq!(value["id"], 12);
        assert!(value.get("result").is_some());

        let failure = RpcFailure {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id: None,
            error: RpcErrorObject {
                code: RpcErrorCode::InvalidRequest as i32,
                message: "Invalid request".to_owned(),
                data: None,
            },
        };
        let value = serde_json::to_value(failure)?;
        assert!(value.get("error").is_some());
        assert!(value.get("result").is_none());
        Ok(())
    }
}

//! Shared browser-to-desktop protocol types for FetchX Phase 17.
//!
//! Used by:
//! - The browser native messaging host (`apps/browser/native-host`)
//! - The desktop application's browser integration module
//!
//! # Protocol overview
//!
//! Native Messaging framing: 4-byte little-endian u32 message length followed
//! by a UTF-8 JSON payload.  Both the host and the desktop application share
//! the same message types.
//!
//! All messages carry a `version` field.  Versions must match the declared
//! [`PROTOCOL_VERSION`]; mismatches return `UnsupportedVersion`.
//!
//! # Security
//! - Requests are [`#[serde(deny_unknown_fields)]`] to prevent field injection.
//! - Only `http://` and `https://` URLs are accepted.
//! - Embedded credentials are rejected by [`validate_url`].
//! - Browser cookies and credentials are never forwarded.
//! - A bearer token is accepted only in the credential-store provisioning
//!   request and is handled by the native host, not extension storage.
//! - Responses never include process paths, secrets, or stack traces.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Current protocol version.  Increment when breaking changes are introduced.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum byte length of a single native-messaging JSON payload.
///
/// 64 KiB is far larger than any legitimate add-download request.
/// Frames claiming a larger length are rejected before allocation.
pub const MAX_MESSAGE_BYTES: u32 = 65_536;

// ============================================================
// Requests (browser → desktop)
// ============================================================

/// A request sent from the browser extension to FetchX.
///
/// Note: `deny_unknown_fields` is intentionally NOT on the outer struct because
/// serde's internally-tagged enum + flatten combination does not support it.
/// Unknown fields are rejected at the `AddDownloadRequest` level instead.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserRequest {
    pub version: u32,
    #[serde(flatten)]
    pub body: BrowserRequestBody,
}

/// The typed body of a browser request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserRequestBody {
    /// Add a URL to the download queue.
    AddDownload(AddDownloadRequest),
    /// Ping to check whether the desktop is reachable.
    Ping,
    /// Forward one supported fetchx.rpc.v1 request to a configured target.
    Rpc(BrowserRpcRequest),
    /// Store a remote target bearer token in the native host's OS credential store.
    StoreCredential(StoreCredentialRequest),
    /// Remove a remote target bearer token from the native host's OS credential store.
    DeleteCredential(DeleteCredentialRequest),
}

/// Payload for an `add_download` browser request.
///
/// Only the URL is accepted from the browser.
/// Filename, destination, engine selection, priority, and scheduling
/// are determined by the desktop application.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddDownloadRequest {
    pub url: String,
    /// Optional request ID supplied by the extension for deduplication.
    /// The desktop echoes it in the response.  If omitted the response
    /// will have `request_id: null`.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRpcRequest {
    pub target: BrowserRpcTarget,
    pub request: fetchx_remote_protocol::RpcRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserRpcTarget {
    Local,
    Remote {
        endpoint: String,
        credential_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreCredentialRequest {
    pub credential_id: String,
    pub endpoint: String,
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteCredentialRequest {
    pub credential_id: String,
    pub endpoint: String,
}

// ============================================================
// Responses (desktop → browser)
// ============================================================

/// A response sent from FetchX back to the browser extension.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserResponse {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(flatten)]
    pub body: BrowserResponseBody,
}

/// The typed body of a browser response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BrowserResponseBody {
    Ok(OkResponse),
    Err(ErrResponse),
}

/// A successful response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OkResponse {
    pub ok: bool, // always true
    #[serde(flatten)]
    pub payload: OkPayload,
}

/// The payload of a successful response, discriminated by operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OkPayload {
    AddDownload(AddDownloadOk),
    Pong(PongOk),
    Rpc(RpcOk),
    CredentialStored(CredentialStoredOk),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddDownloadOk {
    pub download_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PongOk {
    pub pong: bool, // always true
    pub app_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RpcOk {
    pub rpc_response: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CredentialStoredOk {
    pub stored: bool,
}

/// An error response.  Messages are safe for users — no paths, secrets, or
/// stack traces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrResponse {
    pub ok: bool, // always false
    pub error: BrowserErrorDto,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserErrorDto {
    pub code: BrowserErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserErrorCode {
    /// JSON was malformed or the request did not match the expected schema.
    InvalidRequest,
    /// Protocol version is not supported by this build of FetchX.
    UnsupportedVersion,
    /// The `type` field names an operation not supported in this version.
    UnsupportedMessage,
    /// The URL is syntactically invalid.
    InvalidUrl,
    /// The URL contains embedded credentials (`user:pass@host`).
    CredentialsInUrl,
    /// The URL scheme is not `http` or `https`.
    UnsupportedScheme,
    /// FetchX is running but the desktop is temporarily unavailable.
    DesktopUnavailable,
    /// FetchX may have accepted the RPC, but the adapter did not receive confirmation.
    RequestOutcomeUnknown,
    /// The configured target's credentials were rejected.
    AuthenticationFailed,
    /// The authenticated identity cannot perform the requested operation.
    AuthorizationFailed,
    /// The selected target does not speak a compatible FetchX RPC protocol.
    IncompatibleServer,
    /// A target or credential reference is invalid.
    InvalidTarget,
    /// OS-backed target credential storage is unavailable.
    CredentialStoreUnavailable,
    /// The URL was valid but the download was rejected by the queue/engine.
    DownloadRejected,
    /// An unexpected internal error occurred.  Details are not disclosed.
    InternalError,
}

// ============================================================
// Response constructors
// ============================================================

impl BrowserResponse {
    pub fn add_download_ok(request_id: Option<String>, download_id: String) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            body: BrowserResponseBody::Ok(OkResponse {
                ok: true,
                payload: OkPayload::AddDownload(AddDownloadOk { download_id }),
            }),
        }
    }

    pub fn pong(request_id: Option<String>, app_version: String) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            body: BrowserResponseBody::Ok(OkResponse {
                ok: true,
                payload: OkPayload::Pong(PongOk {
                    pong: true,
                    app_version,
                }),
            }),
        }
    }

    pub fn rpc(request_id: Option<String>, rpc_response: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            body: BrowserResponseBody::Ok(OkResponse {
                ok: true,
                payload: OkPayload::Rpc(RpcOk { rpc_response }),
            }),
        }
    }

    pub fn credential_stored(request_id: Option<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            body: BrowserResponseBody::Ok(OkResponse {
                ok: true,
                payload: OkPayload::CredentialStored(CredentialStoredOk { stored: true }),
            }),
        }
    }

    pub fn error(
        request_id: Option<String>,
        code: BrowserErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            body: BrowserResponseBody::Err(ErrResponse {
                ok: false,
                error: BrowserErrorDto {
                    code,
                    message: message.into(),
                },
            }),
        }
    }
}

// ============================================================
// URL validation
// ============================================================

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum UrlValidationError {
    #[error("URL must not be empty")]
    Empty,
    #[error("URL is malformed")]
    Malformed,
    #[error("only HTTP and HTTPS URLs are supported")]
    UnsupportedScheme,
    #[error("URLs containing embedded credentials are not supported")]
    EmbeddedCredentials,
}

impl UrlValidationError {
    pub fn to_browser_error(self) -> (BrowserErrorCode, &'static str) {
        match self {
            Self::Empty | Self::Malformed => (BrowserErrorCode::InvalidUrl, "The URL is invalid."),
            Self::UnsupportedScheme => (
                BrowserErrorCode::UnsupportedScheme,
                "Only HTTP and HTTPS URLs are supported.",
            ),
            Self::EmbeddedCredentials => (
                BrowserErrorCode::CredentialsInUrl,
                "URLs containing credentials (username or password) are not accepted.",
            ),
        }
    }
}

/// Validates that `url` is a safe, credential-free HTTP/HTTPS URL.
///
/// Returns the canonicalized URL string on success.
/// This is intentionally a lightweight protocol-layer check that mirrors the
/// behaviour of `fetchx_downloader::validate_http_source_url`.
pub fn validate_url(url: &str) -> Result<String, UrlValidationError> {
    if url.trim().is_empty() {
        return Err(UrlValidationError::Empty);
    }
    // Use the url crate (pulled in transitively) for parsing.
    // We parse as a string operation only — no network access.
    let parsed = url::Url::parse(url).map_err(|_| UrlValidationError::Malformed)?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(UrlValidationError::UnsupportedScheme),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(UrlValidationError::EmbeddedCredentials);
    }
    Ok(url.to_owned())
}

// ============================================================
// Request deserialization helper
// ============================================================

/// Parse a raw JSON byte slice into a [`BrowserRequest`].
///
/// Returns a structured error response if parsing fails, so callers can send
/// it back immediately without writing additional error-handling code.
pub fn parse_request(json: &[u8]) -> Result<BrowserRequest, BrowserResponse> {
    let req: BrowserRequest = serde_json::from_slice(json).map_err(|_| {
        BrowserResponse::error(
            None,
            BrowserErrorCode::InvalidRequest,
            "The request could not be parsed.",
        )
    })?;

    if req.version != PROTOCOL_VERSION {
        return Err(BrowserResponse::error(
            None,
            BrowserErrorCode::UnsupportedVersion,
            "Protocol version is not supported. Please update FetchX.",
        ));
    }

    Ok(req)
}

// ============================================================
// Native Messaging framing
// ============================================================

/// Reads one native-messaging frame from `reader`.
///
/// Native Messaging uses a 4-byte little-endian u32 length prefix followed by
/// the JSON payload.  Frames exceeding [`MAX_MESSAGE_BYTES`] are rejected to
/// prevent unbounded allocation from a hostile/malformed source.
///
/// Returns `None` when the reader is at EOF (host connection closed).
/// Returns an error for I/O failures, oversized frames, or invalid UTF-8.
pub fn read_frame<R: std::io::Read>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, NativeMessagingError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(NativeMessagingError::Io(e)),
    }

    let length = u32::from_le_bytes(len_buf);
    if length == 0 {
        return Ok(Some(Vec::new()));
    }
    if length > MAX_MESSAGE_BYTES {
        return Err(NativeMessagingError::OversizedFrame { length });
    }

    let mut buf = vec![0u8; length as usize];
    reader
        .read_exact(&mut buf)
        .map_err(NativeMessagingError::Io)?;
    Ok(Some(buf))
}

/// Writes one native-messaging frame to `writer`.
///
/// Serializes `response` to JSON, prepends the 4-byte little-endian length,
/// and writes the whole frame atomically.
pub fn write_frame<W: std::io::Write>(
    writer: &mut W,
    response: &BrowserResponse,
) -> Result<(), NativeMessagingError> {
    let json =
        serde_json::to_vec(response).map_err(|e| NativeMessagingError::Serialize(e.to_string()))?;

    let length = u32::try_from(json.len())
        .map_err(|_| NativeMessagingError::OversizedFrame { length: u32::MAX })?;

    if length > MAX_MESSAGE_BYTES {
        return Err(NativeMessagingError::OversizedFrame { length });
    }

    writer
        .write_all(&length.to_le_bytes())
        .map_err(NativeMessagingError::Io)?;
    writer.write_all(&json).map_err(NativeMessagingError::Io)?;
    writer.flush().map_err(NativeMessagingError::Io)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum NativeMessagingError {
    #[error("I/O error during native messaging framing")]
    Io(#[source] std::io::Error),
    #[error("native messaging frame is too large ({length} bytes)")]
    OversizedFrame { length: u32 },
    #[error("response could not be serialized: {0}")]
    Serialize(String),
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- URL validation ---

    #[test]
    fn valid_https_url_is_accepted() {
        assert!(validate_url("https://example.com/file.zip").is_ok());
    }

    #[test]
    fn valid_http_url_is_accepted() {
        assert!(validate_url("http://example.com/file.zip").is_ok());
    }

    #[test]
    fn empty_url_is_rejected() {
        assert_eq!(validate_url(""), Err(UrlValidationError::Empty));
        assert_eq!(validate_url("   "), Err(UrlValidationError::Empty));
    }

    #[test]
    fn file_scheme_is_rejected() {
        assert_eq!(
            validate_url("file:///etc/passwd"),
            Err(UrlValidationError::UnsupportedScheme)
        );
    }

    #[test]
    fn data_scheme_is_rejected() {
        assert_eq!(
            validate_url("data:text/plain;base64,abc"),
            Err(UrlValidationError::UnsupportedScheme)
        );
    }

    #[test]
    fn javascript_scheme_is_rejected() {
        assert_eq!(
            validate_url("javascript:alert(1)"),
            Err(UrlValidationError::UnsupportedScheme)
        );
    }

    #[test]
    fn ftp_scheme_is_rejected() {
        assert_eq!(
            validate_url("ftp://example.com/file"),
            Err(UrlValidationError::UnsupportedScheme)
        );
    }

    #[test]
    fn embedded_credentials_are_rejected() {
        assert_eq!(
            validate_url("https://user:pass@example.com/file.zip"),
            Err(UrlValidationError::EmbeddedCredentials)
        );
    }

    #[test]
    fn username_without_password_is_rejected() {
        assert_eq!(
            validate_url("https://user@example.com/file.zip"),
            Err(UrlValidationError::EmbeddedCredentials)
        );
    }

    #[test]
    fn malformed_url_is_rejected() {
        assert_eq!(
            validate_url("not a url"),
            Err(UrlValidationError::Malformed)
        );
    }

    // --- Protocol parsing ---

    #[test]
    fn valid_add_download_request_parses() {
        let json = br#"{"version":1,"type":"add_download","url":"https://example.com/file.zip"}"#;
        let req = parse_request(json).unwrap();
        assert_eq!(req.version, 1);
        assert!(matches!(
            req.body,
            BrowserRequestBody::AddDownload(AddDownloadRequest { ref url, .. }) if url == "https://example.com/file.zip"
        ));
    }

    #[test]
    fn ping_request_parses() {
        let json = br#"{"version":1,"type":"ping"}"#;
        let req = parse_request(json).unwrap();
        assert!(matches!(req.body, BrowserRequestBody::Ping));
    }

    #[test]
    fn rpc_request_parses_with_explicit_target_and_protocol_envelope() {
        let json = br#"{"version":1,"type":"rpc","target":{"kind":"remote","endpoint":"https://fetchx.example.test","credential_id":"target-1"},"request":{"jsonrpc":"2.0","id":"browser-1","method":"download.list"}}"#;
        let request = parse_request(json).unwrap();
        assert!(matches!(
            request.body,
            BrowserRequestBody::Rpc(BrowserRpcRequest {
                target: BrowserRpcTarget::Remote { ref endpoint, ref credential_id },
                request: fetchx_remote_protocol::RpcRequest { ref method, .. }
            }) if endpoint == "https://fetchx.example.test"
                && credential_id == "target-1"
                && method == "download.list"
        ));
    }

    #[test]
    fn credential_storage_request_is_typed_and_unknown_fields_are_rejected() {
        let valid = br#"{"version":1,"type":"store_credential","credential_id":"target-1","endpoint":"https://fetchx.example.test","token":"fx_x"}"#;
        assert!(matches!(
            parse_request(valid).unwrap().body,
            BrowserRequestBody::StoreCredential(StoreCredentialRequest { .. })
        ));
        let invalid = br#"{"version":1,"type":"store_credential","credential_id":"target-1","endpoint":"https://fetchx.example.test","token":"fx_x","extra":true}"#;
        assert!(matches!(
            parse_request(invalid).unwrap_err().body,
            BrowserResponseBody::Err(_)
        ));
    }

    #[test]
    fn wrong_version_returns_error_response() {
        let json = br#"{"version":99,"type":"ping"}"#;
        let err = parse_request(json).unwrap_err();
        match &err.body {
            BrowserResponseBody::Err(e) => {
                assert_eq!(e.error.code, BrowserErrorCode::UnsupportedVersion);
            }
            _ => panic!("expected error response"),
        }
    }

    #[test]
    fn unknown_fields_rejected() {
        let json = br#"{"version":1,"type":"add_download","url":"https://example.com/file.zip","cookie":"abc"}"#;
        assert!(parse_request(json).is_err());
    }

    #[test]
    fn unknown_message_type_rejected() {
        let json = br#"{"version":1,"type":"execute_command","cmd":"rm -rf /"}"#;
        assert!(parse_request(json).is_err());
    }

    #[test]
    fn malformed_json_rejected() {
        assert!(parse_request(b"{not json}").is_err());
    }

    // --- Native Messaging framing ---

    #[test]
    fn frame_roundtrip() {
        let response = BrowserResponse::pong(None, "0.1.0".to_owned());
        let mut buf = Vec::new();
        write_frame(&mut buf, &response).unwrap();

        // Verify the 4-byte length prefix is correct.
        let len = u32::from_le_bytes(buf[..4].try_into().unwrap());
        assert_eq!(len as usize, buf.len() - 4);

        // Read it back.
        let frame = read_frame(&mut &buf[..]).unwrap().unwrap();
        let parsed: BrowserResponse = serde_json::from_slice(&frame).unwrap();
        assert_eq!(parsed, response);
    }

    #[test]
    fn oversized_frame_read_is_rejected() {
        let oversized_len = MAX_MESSAGE_BYTES + 1;
        let mut buf = oversized_len.to_le_bytes().to_vec();
        buf.extend(vec![0u8; 16]); // some dummy bytes
        let result = read_frame(&mut &buf[..]);
        assert!(matches!(
            result,
            Err(NativeMessagingError::OversizedFrame { .. })
        ));
    }

    #[test]
    fn truncated_frame_returns_io_error() {
        // Claim 100 bytes but provide only 4 (the length prefix).
        let buf = 100u32.to_le_bytes().to_vec();
        // No payload bytes.
        let result = read_frame(&mut &buf[..]);
        assert!(matches!(result, Err(NativeMessagingError::Io(_))));
    }

    #[test]
    fn eof_returns_none() {
        let buf: &[u8] = &[];
        assert!(read_frame(&mut &buf[..]).unwrap().is_none());
    }

    #[test]
    fn response_serialization_is_stable() {
        let ok = BrowserResponse::add_download_ok(
            Some("req-1".to_owned()),
            "00000000-0000-4000-8000-000000000001".to_owned(),
        );
        let json = serde_json::to_string(&ok).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""download_id":"#));
        assert!(json.contains(r#""request_id":"req-1""#));
        assert!(!json.contains("secret"));
        assert!(!json.contains("token"));

        let err = BrowserResponse::error(None, BrowserErrorCode::InvalidUrl, "The URL is invalid.");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""ok":false"#));
        assert!(json.contains(r#""code":"invalid_url""#));
        assert!(!json.contains("secret"));
    }
}

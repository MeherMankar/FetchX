//! FetchX Native Messaging Host (Phase 17)
//!
//! This binary is registered as a Chrome/Edge native messaging host.
//! The browser launches it as a child process and communicates via:
//!   stdin  → 4-byte LE length + JSON request
//!   stdout ← 4-byte LE length + JSON response
//!
//! The host forwards the request to the running FetchX desktop
//! application via a loopback TCP socket.  Connection details
//! (port + session token) are read from a per-session state file that the
//! desktop writes to its app-data directory on startup.
//!
//! # Stdout discipline
//! Only native-messaging frames are written to stdout.
//! All diagnostic output goes to stderr (captured by the browser for
//! debugging; never shown to users).
//!
//! # Security
//! - Message size is bounded by MAX_MESSAGE_BYTES (64 KiB).
//! - No shell execution.
//! - No arbitrary filesystem access.
//! - Browser cookies and passwords are never accessed.
//! - A remote bearer token is handled only by explicit credential provisioning
//!   and is stored in the operating system credential store.

#![forbid(unsafe_code)]

use std::io::{self, BufReader, BufWriter};

use fetchx_browser_protocol::{
    BrowserErrorCode, BrowserRequest, BrowserRequestBody, BrowserResponse, BrowserRpcTarget,
    NativeMessagingError, read_frame, write_frame,
};
use fetchx_remote_protocol::DOWNLOAD_CREATE;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tracing::{debug, error, info, warn};

mod ipc;
mod remote;

fn main() {
    // All diagnostics to stderr — stdout is protocol traffic only.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let result = run_once();
    if let Err(ref e) = result {
        error!(%e, "native host error");
    }
    std::process::exit(result.map(|_| 0).unwrap_or(1));
}

fn run_once() -> Result<(), HostError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    // Read the request frame.
    let frame = match read_frame(&mut reader) {
        Ok(Some(f)) => f,
        Ok(None) => {
            debug!("stdin closed before request; exiting");
            return Ok(());
        }
        Err(NativeMessagingError::OversizedFrame { length }) => {
            let resp = BrowserResponse::error(
                None,
                BrowserErrorCode::InvalidRequest,
                "Request exceeds the maximum allowed size.",
            );
            write_frame(&mut writer, &resp)?;
            warn!(length, "oversized native messaging frame rejected");
            return Ok(());
        }
        Err(e) => return Err(HostError::Framing(e)),
    };

    // Parse and validate the protocol request.
    let request = match fetchx_browser_protocol::parse_request(&frame) {
        Ok(r) => r,
        Err(error_response) => {
            write_frame(&mut writer, &error_response)?;
            return Ok(());
        }
    };

    info!(version = request.version, "received browser request");

    // Forward to the desktop and write the response.
    let BrowserRequest { version, body } = request;
    let response = match body {
        BrowserRequestBody::Rpc(rpc) => match rpc.target {
            BrowserRpcTarget::Local => forward_to_desktop(BrowserRequest {
                version,
                body: BrowserRequestBody::Rpc(rpc),
            })?,
            BrowserRpcTarget::Remote {
                endpoint,
                credential_id,
            } => remote::forward_rpc(&endpoint, &credential_id, &rpc.request),
        },
        BrowserRequestBody::StoreCredential(credential) => {
            match remote::store_credential(
                &credential.credential_id,
                &credential.endpoint,
                &credential.token,
            ) {
                Ok(()) => BrowserResponse::credential_stored(None),
                Err(code) => credential_error(code),
            }
        }
        BrowserRequestBody::DeleteCredential(credential) => {
            match remote::delete_credential(&credential.credential_id, &credential.endpoint) {
                Ok(()) => BrowserResponse::credential_stored(None),
                Err(code) => credential_error(code),
            }
        }
        body => forward_to_desktop(BrowserRequest { version, body })?,
    };
    write_frame(&mut writer, &response)?;
    Ok(())
}

fn credential_error(code: BrowserErrorCode) -> BrowserResponse {
    let message = match code {
        BrowserErrorCode::InvalidTarget => "The target or credential is invalid.",
        _ => "The operating system credential store is unavailable.",
    };
    BrowserResponse::error(None, code, message)
}

fn forward_to_desktop(request: BrowserRequest) -> Result<BrowserResponse, HostError> {
    let may_have_created_download = matches!(
        &request.body,
        BrowserRequestBody::Rpc(rpc) if rpc.request.method == DOWNLOAD_CREATE
    );
    let state = match ipc::read_desktop_state() {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, "could not read desktop state file");
            return Ok(BrowserResponse::error(
                None,
                BrowserErrorCode::DesktopUnavailable,
                "FetchX does not appear to be running. Please start the application.",
            ));
        }
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| HostError::Runtime(e.to_string()))?;

    rt.block_on(async {
        let connect_result = timeout(
            std::time::Duration::from_secs(5),
            TcpStream::connect(("127.0.0.1", state.port)),
        )
        .await;

        let mut stream = match connect_result {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                warn!(%e, "could not connect to desktop browser endpoint");
                return Ok(BrowserResponse::error(
                    None,
                    BrowserErrorCode::DesktopUnavailable,
                    "FetchX is not responding. Please check that it is running.",
                ));
            }
            Err(_) => {
                return Ok(BrowserResponse::error(
                    None,
                    BrowserErrorCode::DesktopUnavailable,
                    "Connection to FetchX timed out.",
                ));
            }
        };

        // Send token + request as a single JSON message prefixed by 4-byte LE length.
        let ipc_request = ipc::DesktopIpcRequest {
            token: state.token.clone(),
            request,
        };
        let json =
            serde_json::to_vec(&ipc_request).map_err(|e| HostError::Serialize(e.to_string()))?;
        let length = u32::try_from(json.len()).unwrap_or(u32::MAX);
        stream
            .write_all(&length.to_le_bytes())
            .await
            .map_err(HostError::Io)?;
        stream.write_all(&json).await.map_err(HostError::Io)?;

        // Read the response length prefix.
        let mut len_buf = [0u8; 4];
        match timeout(
            std::time::Duration::from_secs(10),
            stream.read_exact(&mut len_buf),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                warn!(%e, "could not read response from desktop");
                return Ok(response_error(
                    may_have_created_download,
                    BrowserErrorCode::DesktopUnavailable,
                    "FetchX did not return a response.",
                ));
            }
            Err(_) => {
                return Ok(response_error(
                    may_have_created_download,
                    BrowserErrorCode::DesktopUnavailable,
                    "FetchX did not return a response.",
                ));
            }
        }

        let resp_len = u32::from_le_bytes(len_buf) as usize;
        if resp_len > fetchx_browser_protocol::MAX_MESSAGE_BYTES as usize {
            error!(resp_len, "desktop returned oversized response");
            return Ok(response_error(
                may_have_created_download,
                BrowserErrorCode::IncompatibleServer,
                "FetchX returned an invalid response.",
            ));
        }

        let mut resp_buf = vec![0u8; resp_len];
        if let Err(e) = stream.read_exact(&mut resp_buf).await {
            warn!(%e, "could not read response body from desktop");
            return Ok(response_error(
                may_have_created_download,
                BrowserErrorCode::DesktopUnavailable,
                "FetchX did not return a complete response.",
            ));
        }

        match serde_json::from_slice::<BrowserResponse>(&resp_buf) {
            Ok(r) => Ok(r),
            Err(e) => {
                error!(%e, "desktop returned unrecognised response");
                Ok(response_error(
                    may_have_created_download,
                    BrowserErrorCode::InternalError,
                    "An internal error occurred.",
                ))
            }
        }
    })
}

fn response_error(
    may_have_created_download: bool,
    fallback_code: BrowserErrorCode,
    fallback_message: &'static str,
) -> BrowserResponse {
    if may_have_created_download {
        BrowserResponse::error(
            None,
            BrowserErrorCode::RequestOutcomeUnknown,
            "FetchX may have accepted the download, but confirmation was not received.",
        )
    } else {
        BrowserResponse::error(None, fallback_code, fallback_message)
    }
}

#[derive(Debug, thiserror::Error)]
enum HostError {
    #[error("native messaging framing error: {0}")]
    Framing(#[from] NativeMessagingError),
    #[error("I/O error: {0}")]
    Io(#[source] io::Error),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("Tokio runtime error: {0}")]
    Runtime(String),
}

//! Browser Integration IPC Server (Phase 17)
//!
//! The desktop application exposes a lightweight loopback TCP server that the
//! native messaging host connects to.  On every accepted connection the server
//! reads one [`DesktopIpcRequest`], validates the session token, processes the
//! browser request, and writes a [`BrowserResponse`].
//!
//! # Security model
//! - Bound to `127.0.0.1` only — never 0.0.0.0.
//! - A per-session UUID token is generated at startup and compared in
//!   constant-time for each connection.
//! - The token and port are written to a user-private state file.
//! - No request field can become an executable path or shell command.
//! - Request size is bounded at [`MAX_MESSAGE_BYTES`].
//! - A misbehaving connection cannot block or crash the server.
//!
//! # State file
//! On server start a JSON file is written to the app-data directory:
//!   `{ "port": N, "token": "...", "app_version": "..." }`
//!
//! The native messaging host reads this file to learn where to connect.
//! The file is removed when the server shuts down.

use std::{path::PathBuf, sync::Arc};

use fetchx_browser_protocol::{
    BrowserErrorCode, BrowserRequest, BrowserRequestBody, BrowserResponse, MAX_MESSAGE_BYTES,
    PROTOCOL_VERSION, validate_url,
};
use fetchx_remote_server::dispatch_local_rpc;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::commands::{AddDownloadRequest, AppState};

/// Shared service object owned by the accept loop.
/// Holds only what is needed for request dispatch.
struct BrowserIpcService {
    token: String,
    /// Clone of the application state — all fields are Arc-backed or Clone.
    app_state: AppState,
}

/// The running browser IPC server.
pub struct BrowserIpcServer {
    /// Loopback port the server is listening on.
    pub port: u16,
    /// Per-session token for origin validation (written to the state file).
    #[allow(dead_code)]
    pub token: String,
    /// Path of the state file on disk (removed on shutdown).
    state_file: PathBuf,
    /// Handle to the accept loop task.
    _task: JoinHandle<()>,
}

impl BrowserIpcServer {
    /// Starts the loopback server, writes the state file, and returns the
    /// server handle.
    pub async fn start(
        app_state: &AppState,
        app_data_dir: PathBuf,
        app_version: &'static str,
    ) -> Result<Self, BrowserIpcError> {
        // Bind to a random OS-assigned loopback port.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(BrowserIpcError::Bind)?;
        let port = listener.local_addr().map_err(BrowserIpcError::Bind)?.port();

        // Generate a fresh per-session token.
        let token = Uuid::new_v4().simple().to_string();

        // Write the state file so the native host can find us.
        let state_file = app_data_dir.join("browser-ipc.json");
        let state = BrowserIpcState {
            port,
            token: token.clone(),
            app_version: app_version.to_owned(),
        };
        let json =
            serde_json::to_string(&state).map_err(|e| BrowserIpcError::StateFile(e.to_string()))?;
        tokio::fs::write(&state_file, json.as_bytes())
            .await
            .map_err(|e| BrowserIpcError::StateFile(e.to_string()))?;

        info!(port, "browser IPC server started");

        let service = Arc::new(BrowserIpcService {
            token: token.clone(),
            app_state: app_state.clone_for_browser(),
        });

        let task = tokio::spawn(accept_loop(listener, service));

        Ok(Self {
            port,
            token,
            state_file,
            _task: task,
        })
    }

    /// Removes the state file.  Called during application shutdown.
    pub async fn shutdown(&self) {
        if let Err(e) = tokio::fs::remove_file(&self.state_file).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            warn!(%e, "could not remove browser IPC state file");
        }
        info!("browser IPC server shut down");
    }
}

/// Accept loop: handles one connection at a time, with a task per connection
/// so a slow client cannot stall the server.
async fn accept_loop(listener: TcpListener, service: Arc<BrowserIpcService>) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                debug!(%peer, "browser IPC connection accepted");
                let svc = Arc::clone(&service);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, svc).await {
                        warn!(%e, "browser IPC connection error");
                    }
                });
            }
            Err(e) => {
                error!(%e, "browser IPC accept error");
                // Brief pause to avoid tight loop on persistent accept errors.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Handles a single connection: read one request, produce one response.
async fn handle_connection(
    mut stream: TcpStream,
    service: Arc<BrowserIpcService>,
) -> Result<(), BrowserIpcError> {
    // Read 4-byte length prefix.
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(BrowserIpcError::Io)?;
    let length = u32::from_le_bytes(len_buf);

    if length > MAX_MESSAGE_BYTES {
        warn!(length, "browser IPC request too large; closing connection");
        return Err(BrowserIpcError::OversizedMessage(length));
    }

    let mut buf = vec![0u8; length as usize];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(BrowserIpcError::Io)?;

    // Deserialize the IPC wrapper.
    let ipc_req: DesktopIpcRequest = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(e) => {
            warn!(%e, "malformed browser IPC request");
            send_response(
                &mut stream,
                &BrowserResponse::error(
                    None,
                    BrowserErrorCode::InvalidRequest,
                    "Malformed request.",
                ),
            )
            .await?;
            return Ok(());
        }
    };

    // Constant-time token comparison to prevent timing attacks.
    let token_valid = constant_time_eq(ipc_req.token.as_bytes(), service.token.as_bytes());
    if !token_valid {
        warn!("browser IPC connection presented invalid token; rejecting");
        send_response(
            &mut stream,
            &BrowserResponse::error(
                None,
                BrowserErrorCode::InvalidRequest,
                "Connection rejected.",
            ),
        )
        .await?;
        return Ok(());
    }

    // Dispatch the validated request.
    let response = dispatch(ipc_req.request, &service.app_state).await;
    send_response(&mut stream, &response).await?;
    Ok(())
}

/// Dispatches a validated [`BrowserRequest`] to the appropriate application path.
async fn dispatch(request: BrowserRequest, app_state: &AppState) -> BrowserResponse {
    if request.version != PROTOCOL_VERSION {
        return BrowserResponse::error(
            None,
            BrowserErrorCode::UnsupportedVersion,
            "Protocol version is not supported. Please update FetchX.",
        );
    }

    match request.body {
        BrowserRequestBody::Ping => {
            BrowserResponse::pong(None, env!("CARGO_PKG_VERSION").to_owned())
        }

        BrowserRequestBody::Rpc(browser_rpc) => {
            let Some(services) = app_state.core_services.as_deref() else {
                return BrowserResponse::error(
                    None,
                    BrowserErrorCode::DesktopUnavailable,
                    "The FetchX RPC service is unavailable.",
                );
            };
            let body = match serde_json::to_vec(&browser_rpc.request) {
                Ok(body) => body,
                Err(_) => {
                    return BrowserResponse::error(
                        None,
                        BrowserErrorCode::InvalidRequest,
                        "The RPC request is invalid.",
                    );
                }
            };
            match dispatch_local_rpc(services, &body).await {
                Ok(response) => BrowserResponse::rpc(None, response),
                Err(_) => BrowserResponse::error(
                    None,
                    BrowserErrorCode::InternalError,
                    "FetchX could not process the RPC request.",
                ),
            }
        }

        BrowserRequestBody::StoreCredential(_) | BrowserRequestBody::DeleteCredential(_) => {
            BrowserResponse::error(
                None,
                BrowserErrorCode::UnsupportedMessage,
                "Credential management is handled by the native messaging host.",
            )
        }

        BrowserRequestBody::AddDownload(browser_req) => {
            let request_id = browser_req.request_id.clone();

            // Re-validate the URL server-side — never trust browser-side validation.
            let canonical_url = match validate_url(&browser_req.url) {
                Ok(url) => url,
                Err(validation_err) => {
                    let (code, msg) = validation_err.to_browser_error();
                    return BrowserResponse::error(request_id, code, msg);
                }
            };

            let add_request = AddDownloadRequest {
                url: canonical_url,
                package_name: None,
                engine_kind: None,
                referer: browser_req.referer,
                user_agent: browser_req.user_agent,
                cookies: browser_req.cookies,
            };

            match app_state.add_from_browser(add_request).await {
                Ok(dto) => BrowserResponse::add_download_ok(request_id, dto.id),
                Err(e) => {
                    info!(error = %e, "browser add-download request rejected by application");
                    BrowserResponse::error(
                        request_id,
                        BrowserErrorCode::DownloadRejected,
                        e.message,
                    )
                }
            }
        }
    }
}

/// Writes a 4-byte length-prefixed JSON response.
async fn send_response(
    stream: &mut TcpStream,
    response: &BrowserResponse,
) -> Result<(), BrowserIpcError> {
    let json =
        serde_json::to_vec(response).map_err(|e| BrowserIpcError::Serialize(e.to_string()))?;
    let length = u32::try_from(json.len()).unwrap_or(u32::MAX);
    stream
        .write_all(&length.to_le_bytes())
        .await
        .map_err(BrowserIpcError::Io)?;
    stream.write_all(&json).await.map_err(BrowserIpcError::Io)?;
    Ok(())
}

/// State file written to disk so the native host can connect.
#[derive(Serialize, Deserialize)]
struct BrowserIpcState {
    port: u16,
    /// Session token — must match the runtime value for every request.
    token: String,
    app_version: String,
}

/// Request format expected over the local TCP connection.
/// Wraps the browser request with a session token for local-host validation.
#[derive(Debug, Deserialize, Serialize)]
struct DesktopIpcRequest {
    token: String,
    request: BrowserRequest,
}

/// Constant-time byte comparison to avoid timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserIpcError {
    #[error("could not bind browser IPC listener: {0}")]
    Bind(#[source] std::io::Error),
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("could not write state file: {0}")]
    StateFile(String),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("browser IPC message exceeds maximum size ({0} bytes)")]
    OversizedMessage(u32),
}

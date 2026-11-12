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

use std::{
    io::{self, BufReader, BufWriter},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use fetchx_browser_protocol::{
    BrowserErrorCode, BrowserRequest, BrowserRequestBody, BrowserResponse, BrowserRpcTarget,
    NativeMessagingError, read_frame, write_frame,
};
use fetchx_remote_protocol::DOWNLOAD_CREATE;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{sleep, timeout},
};
use tracing::{debug, error, info, warn};

mod ipc;
mod remote;

const DESKTOP_START_TIMEOUT: Duration = Duration::from_secs(20);
const DESKTOP_START_POLL_INTERVAL: Duration = Duration::from_millis(250);

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
    let may_launch_desktop = can_launch_desktop_for(&request);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| HostError::Runtime(e.to_string()))?;

    rt.block_on(async {
        let started_at = Instant::now();
        let mut launch_attempted = false;
        let (state, mut stream) = loop {
            match ipc::read_desktop_state() {
                Ok(state) => {
                    match timeout(
                        Duration::from_secs(1),
                        TcpStream::connect(("127.0.0.1", state.port)),
                    )
                    .await
                    {
                        Ok(Ok(stream)) => break (state, stream),
                        Ok(Err(error)) => {
                            warn!(%error, "could not connect to desktop browser endpoint");
                        }
                        Err(_) => {
                            warn!("connection to desktop browser endpoint timed out");
                        }
                    }
                }
                Err(error) => {
                    warn!(%error, "could not read desktop state file");
                }
            }

            if may_launch_desktop && !launch_attempted {
                launch_attempted = true;
                if let Err(error) = launch_desktop() {
                    warn!(%error, "could not start FetchX desktop application");
                    return Ok(BrowserResponse::error(
                        None,
                        BrowserErrorCode::DesktopUnavailable,
                        "Could not start FetchX automatically. Open the FetchX desktop app and try again.",
                    ));
                }
                info!("started FetchX desktop application for browser request");
            }

            if !may_launch_desktop {
                return Ok(BrowserResponse::error(
                    None,
                    BrowserErrorCode::DesktopUnavailable,
                    "FetchX is not running. Start the desktop app and try again.",
                ));
            }
            if started_at.elapsed() >= DESKTOP_START_TIMEOUT {
                return Ok(BrowserResponse::error(
                    None,
                    BrowserErrorCode::DesktopUnavailable,
                    "FetchX did not finish starting in time. Open the desktop app and try again.",
                ));
            }
            sleep(DESKTOP_START_POLL_INTERVAL).await;
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

fn can_launch_desktop_for(request: &BrowserRequest) -> bool {
    let BrowserRequestBody::Rpc(rpc) = &request.body else {
        return false;
    };
    if !matches!(rpc.target, BrowserRpcTarget::Local) {
        return false;
    }
    matches!(
        rpc.request.method.as_str(),
        DOWNLOAD_CREATE | "media.extractFormats" | "media.addFormat"
    )
}

fn desktop_executable_candidates(host_executable: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let executable_names = if cfg!(windows) {
        ["FetchX.exe", "fetchx-desktop.exe"]
    } else {
        ["fetchx-desktop", "FetchX"]
    };

    if let Some(host_directory) = host_executable.parent() {
        for name in executable_names {
            candidates.push(host_directory.join(name));
        }
        if host_directory
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("engines"))
            && let Some(resources_directory) = host_directory.parent()
            && resources_directory
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("resources"))
            && let Some(application_directory) = resources_directory.parent()
        {
            for name in executable_names {
                candidates.push(application_directory.join(name));
            }
        }
    }

    #[cfg(windows)]
    {
        let mut bases = vec![
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        ];
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            bases.push(Some(local_app_data.clone()));
            bases.push(Some(local_app_data.join("Programs")));
        }
        for base in bases.into_iter().flatten() {
            for name in executable_names {
                candidates.push(base.join("FetchX").join(name));
            }
        }
    }

    candidates
}

fn launch_desktop() -> Result<(), String> {
    let host_executable = std::env::current_exe()
        .map_err(|error| format!("could not locate native host: {error}"))?;
    let executable = desktop_executable_candidates(&host_executable)
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "could not locate FetchX desktop executable".to_owned())?;

    let mut command = Command::new(&executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = executable.parent() {
        command.current_dir(directory);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map_err(|error| format!("could not launch {}: {error}", executable.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use fetchx_browser_protocol::{BrowserRpcRequest, PROTOCOL_VERSION};
    use fetchx_remote_protocol::{RpcId, RpcRequest};

    use super::{can_launch_desktop_for, desktop_executable_candidates};

    fn local_rpc(method: &str) -> super::BrowserRequest {
        super::BrowserRequest {
            version: PROTOCOL_VERSION,
            body: super::BrowserRequestBody::Rpc(BrowserRpcRequest {
                target: super::BrowserRpcTarget::Local,
                request: RpcRequest {
                    jsonrpc: "2.0".to_owned(),
                    id: Some(RpcId::Number(1)),
                    method: method.to_owned(),
                    params: None,
                },
            }),
        }
    }

    #[test]
    fn starts_desktop_only_for_local_user_actions() {
        assert!(can_launch_desktop_for(&local_rpc("download.create")));
        assert!(can_launch_desktop_for(&local_rpc("media.extractFormats")));
        assert!(can_launch_desktop_for(&local_rpc("media.addFormat")));
        assert!(!can_launch_desktop_for(&local_rpc("system.discover")));
        assert!(!can_launch_desktop_for(&local_rpc("download.list")));
    }

    #[test]
    fn finds_install_locations_around_native_host() {
        let host_path =
            std::path::Path::new("C:\\FetchX\\resources\\engines\\fetchx-native-host.exe");
        let candidates = desktop_executable_candidates(host_path);
        assert!(
            candidates
                .iter()
                .any(|path| { path == std::path::Path::new("C:\\FetchX\\FetchX.exe") })
        );
        assert!(
            candidates
                .iter()
                .any(|path| { path == std::path::Path::new("C:\\FetchX\\fetchx-desktop.exe") })
        );
    }
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

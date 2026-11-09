//! Headless HTTP and WebSocket host for the shared FetchX remote protocol.

#![forbid(unsafe_code)]

mod auth;
mod authorization;
mod config;
mod dispatch;
mod events;
mod tickets;
mod transport;

use fetchx_core_services::CoreServices;
use fetchx_downloader::validate_http_source_url;
use fetchx_remote_protocol::{BROWSER_RPC_METHODS, RpcErrorCode, RpcFailure, parse_request};
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

pub use auth::{
    AuthError, AuthenticationService, current_auth_role, initialize_auth, rotate_auth,
    set_auth_role,
};
pub use authorization::Role;
pub use config::{DEFAULT_PORT, ServerConfig, ServerConfigError, TlsConfig};
pub use transport::{RemoteServer, ServerError};

/// Dispatches one validated browser RPC request for a trusted local adapter.
///
/// The caller must authenticate the local transport before invoking this
/// function. Authorization uses the remote protocol's operator role and method
/// permission mapping; browser adapters expose only the shared browser method
/// allowlist, and WebSocket ticket requests are not supported here.
pub async fn dispatch_local_rpc(
    services: &CoreServices,
    body: &[u8],
) -> Result<Value, serde_json::Error> {
    let request = match parse_request(body) {
        Ok(request) => request,
        Err(error) => {
            return serde_json::to_value(dispatch::RpcError::from_protocol(error).failure(None));
        }
    };
    if request.id.is_none() {
        return Ok(Value::Null);
    }
    let request_id = request.id.clone();
    let principal = authorization::AuthenticatedPrincipal::primary(Role::Operator);
    let browser_download_url_is_valid = match &request.command {
        fetchx_remote_protocol::RemoteCommand::DownloadCreate(params) => {
            validate_http_source_url(&params.url).is_ok()
        }
        _ => true,
    };

    let result = if matches!(
        request.command,
        fetchx_remote_protocol::RemoteCommand::WebSocketTicket
    ) {
        Err(dispatch::RpcError::new(
            RpcErrorCode::UnsupportedOperation,
            "WebSocket tickets are unavailable to local adapters",
        ))
    } else if !browser_download_url_is_valid {
        Err(dispatch::RpcError::new(
            RpcErrorCode::InvalidParams,
            "browser downloads require a valid HTTP(S) URL",
        ))
    } else if !principal.allows(authorization::required_permission(&request.command)) {
        Err(dispatch::RpcError::new(
            RpcErrorCode::Forbidden,
            "the authenticated role cannot perform this operation",
        ))
    } else if !BROWSER_RPC_METHODS.contains(&request.method.as_str()) {
        Err(dispatch::RpcError::new(
            RpcErrorCode::MethodNotFound,
            "method not found",
        ))
    } else {
        match timeout(
            Duration::from_secs(30),
            dispatch::dispatch(
                services,
                principal,
                request,
                &tickets::WebSocketTickets::default(),
                [0; 32],
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(dispatch::RpcError::new(
                RpcErrorCode::Timeout,
                "request timed out",
            )),
        }
    };

    match result {
        Ok(result) => match request_id {
            Some(id) => serde_json::to_value(dispatch::success(id, result)),
            None => Ok(Value::Null),
        },
        Err(error) => {
            if request_id.is_none() {
                return Ok(Value::Null);
            }
            let failure: RpcFailure = error.failure(request_id);
            serde_json::to_value(failure)
        }
    }
}

#[cfg(test)]
mod local_rpc_tests {
    use fetchx_core_services::{CoreServices, CoreServicesConfig};

    use super::dispatch_local_rpc;

    #[tokio::test]
    async fn local_dispatch_uses_fetchx_rpc_and_operator_authorization()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let services = CoreServices::start(CoreServicesConfig {
            database_path: directory.path().join("fetchx.db"),
            downloads_dir: directory.path().join("downloads"),
            runtime_dir: directory.path().join("runtime"),
            bundled_engines_dir: None,
            defer_queued_on_start: false,
        })
        .await?;

        let discovery = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","id":1,"method":"system.discover"}"#,
        )
        .await?;
        assert_eq!(discovery["result"]["protocol_version"], "fetchx.rpc.v1");

        let forbidden = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","id":2,"method":"download.setSpeedLimit","params":{"id":"00000000-0000-4000-8000-000000000000","bytes_per_second":1}}"#,
        )
        .await?;
        assert_eq!(forbidden["error"]["code"], -32003);

        let unsupported = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","id":3,"method":"download.pause","params":{"id":"00000000-0000-4000-8000-000000000000"}}"#,
        )
        .await?;
        assert_eq!(unsupported["error"]["code"], -32601);

        let browser_torrent_metadata = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","id":5,"method":"download.torrentMetadata","params":{"id":"00000000-0000-4000-8000-000000000000"}}"#,
        )
        .await?;
        assert_eq!(browser_torrent_metadata["error"]["code"], -32601);
        let browser_torrent_selection = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","id":6,"method":"download.selectTorrentFiles","params":{"id":"00000000-0000-4000-8000-000000000000","info_hash":"0123456789012345678901234567890123456789","selected_file_indexes":[0]}}"#,
        )
        .await?;
        assert_eq!(browser_torrent_selection["error"]["code"], -32601);

        let notification = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","method":"download.create","params":{"url":"https://example.test/file.zip"}}"#,
        )
        .await?;
        assert!(notification.is_null());

        let unsupported_browser_url = dispatch_local_rpc(
            &services,
            br#"{"jsonrpc":"2.0","id":4,"method":"download.create","params":{"url":"magnet:?xt=urn:btih:example"}}"#,
        )
        .await?;
        assert_eq!(unsupported_browser_url["error"]["code"], -32602);

        let malformed = dispatch_local_rpc(&services, b"{").await?;
        assert_eq!(malformed["error"]["code"], -32700);

        services.shutdown().await?;
        Ok(())
    }
}

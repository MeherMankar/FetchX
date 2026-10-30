use std::time::Duration;

use fetchx_browser_protocol::{BrowserErrorCode, BrowserResponse, validate_url};
use fetchx_remote_protocol::{
    BROWSER_RPC_METHODS, DOWNLOAD_CREATE, JSON_RPC_VERSION, MAX_MESSAGE_BYTES, PROTOCOL_VERSION,
    ProtocolInfo, RemoteCommand, RpcFailure, RpcId, RpcRequest, RpcSuccess, SYSTEM_DISCOVER,
    parse_request,
};
use futures_util::StreamExt;
use reqwest::{StatusCode, Url, redirect::Policy};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::timeout;

const CREDENTIAL_SERVICE: &str = "dev.fetchx.browser.targets";
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn store_credential(
    credential_id: &str,
    endpoint: &str,
    token: &str,
) -> Result<(), BrowserErrorCode> {
    let endpoint = validate_endpoint(endpoint)?;
    if !valid_credential_id(credential_id) || !valid_bearer_token(token) {
        return Err(BrowserErrorCode::InvalidTarget);
    }
    let account = credential_account(credential_id, endpoint.as_str());

    #[cfg(windows)]
    {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, &account)
            .map_err(|_| BrowserErrorCode::CredentialStoreUnavailable)?;
        entry
            .set_password(token)
            .map_err(|_| BrowserErrorCode::CredentialStoreUnavailable)
    }

    #[cfg(not(windows))]
    {
        let _ = account;
        Err(BrowserErrorCode::CredentialStoreUnavailable)
    }
}

pub fn delete_credential(credential_id: &str, endpoint: &str) -> Result<(), BrowserErrorCode> {
    let endpoint = validate_endpoint(endpoint)?;
    if !valid_credential_id(credential_id) {
        return Err(BrowserErrorCode::InvalidTarget);
    }
    let account = credential_account(credential_id, endpoint.as_str());

    #[cfg(windows)]
    {
        let entry = keyring::Entry::new(CREDENTIAL_SERVICE, &account)
            .map_err(|_| BrowserErrorCode::CredentialStoreUnavailable)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(BrowserErrorCode::CredentialStoreUnavailable),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = account;
        Err(BrowserErrorCode::CredentialStoreUnavailable)
    }
}

pub fn forward_rpc(endpoint: &str, credential_id: &str, request: &RpcRequest) -> BrowserResponse {
    let endpoint = match validate_endpoint(endpoint) {
        Ok(endpoint) => endpoint,
        Err(code) => return BrowserResponse::error(None, code, "The target endpoint is invalid."),
    };
    if !valid_credential_id(credential_id) {
        return BrowserResponse::error(
            None,
            BrowserErrorCode::InvalidTarget,
            "The selected target is invalid.",
        );
    }
    let request_id = request.id.clone();
    if request.jsonrpc != JSON_RPC_VERSION
        || request_id.is_none()
        || !BROWSER_RPC_METHODS.contains(&request.method.as_str())
    {
        return BrowserResponse::error(
            None,
            BrowserErrorCode::UnsupportedMessage,
            "This browser RPC operation is not supported.",
        );
    }
    let Ok(request_body) = serde_json::to_vec(request) else {
        return BrowserResponse::error(
            None,
            BrowserErrorCode::InternalError,
            "The RPC request could not be encoded.",
        );
    };
    let parsed_request = match parse_request(&request_body) {
        Ok(request) => request,
        Err(_) => {
            return BrowserResponse::error(
                None,
                BrowserErrorCode::InvalidRequest,
                "The RPC request is invalid.",
            );
        }
    };
    if matches!(
        &parsed_request.command,
        RemoteCommand::DownloadCreate(params) if validate_url(&params.url).is_err()
    ) {
        return BrowserResponse::error(
            None,
            BrowserErrorCode::InvalidRequest,
            "Browser downloads require a valid HTTP(S) URL.",
        );
    }
    let token = match get_credential(credential_id, endpoint.as_str()) {
        Ok(token) => token,
        Err(code) => {
            return BrowserResponse::error(
                None,
                code,
                "The selected target has no usable authentication credential.",
            );
        }
    };
    forward_rpc_with_token(endpoint, token, request)
}

fn forward_rpc_with_token(endpoint: Url, token: String, request: &RpcRequest) -> BrowserResponse {
    let request_id = request.id.clone();
    let may_have_created_download = request.method == DOWNLOAD_CREATE;
    let client = match reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(RPC_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return BrowserResponse::error(
                None,
                BrowserErrorCode::InternalError,
                "The RPC client could not be initialized.",
            );
        }
    };

    let request_id_for_response = request_id.clone();
    let rpc_request = request.clone();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return BrowserResponse::error(
                None,
                BrowserErrorCode::InternalError,
                "The RPC runtime could not be initialized.",
            );
        }
    };

    let response = runtime.block_on(async move {
        let rpc_url = endpoint
            .join("/rpc")
            .map_err(|_| BrowserErrorCode::InvalidTarget)?;
        let discovery = RpcRequest {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id: Some(RpcId::String("fetchx-browser-discovery".to_owned())),
            method: SYSTEM_DISCOVER.to_owned(),
            params: None,
        };
        let discovery_response = send_rpc(&client, &rpc_url, &token, &discovery).await?;
        let discovery_result = discovery_response
            .get("result")
            .cloned()
            .ok_or(BrowserErrorCode::IncompatibleServer)?;
        let protocol: ProtocolInfo = serde_json::from_value(discovery_result)
            .map_err(|_| BrowserErrorCode::IncompatibleServer)?;
        if protocol.protocol_version != PROTOCOL_VERSION
            || !protocol.capabilities.contains(&rpc_request.method)
        {
            return Err(BrowserErrorCode::IncompatibleServer);
        }
        let rpc_response = send_rpc(&client, &rpc_url, &token, &rpc_request)
            .await
            .map_err(|code| match (may_have_created_download, code) {
                (
                    true,
                    BrowserErrorCode::DesktopUnavailable | BrowserErrorCode::IncompatibleServer,
                ) => BrowserErrorCode::RequestOutcomeUnknown,
                (_, other) => other,
            })?;
        Ok(rpc_response)
    });

    match response {
        Ok(response) => BrowserResponse::rpc(request_id_for_response.map(rpc_id_string), response),
        Err(code) => {
            let message = match code {
                BrowserErrorCode::AuthenticationFailed => "The target rejected its credential.",
                BrowserErrorCode::AuthorizationFailed => {
                    "The target does not allow this browser operation."
                }
                BrowserErrorCode::IncompatibleServer => {
                    "The target does not support a compatible FetchX RPC capability."
                }
                BrowserErrorCode::RequestOutcomeUnknown => {
                    "The target may have accepted the download, but confirmation was not received."
                }
                _ => "The remote FetchX target is unavailable.",
            };
            BrowserResponse::error(None, code, message)
        }
    }
}

async fn send_rpc(
    client: &reqwest::Client,
    rpc_url: &Url,
    token: &str,
    request: &RpcRequest,
) -> Result<Value, BrowserErrorCode> {
    let response = timeout(
        RPC_TIMEOUT,
        client
            .post(rpc_url.clone())
            .bearer_auth(token)
            .json(request)
            .send(),
    )
    .await
    .map_err(|_| BrowserErrorCode::DesktopUnavailable)?
    .map_err(|_| BrowserErrorCode::DesktopUnavailable)?;

    match response.status() {
        StatusCode::UNAUTHORIZED => return Err(BrowserErrorCode::AuthenticationFailed),
        StatusCode::FORBIDDEN => return Err(BrowserErrorCode::AuthorizationFailed),
        status if !status.is_success() => return Err(BrowserErrorCode::DesktopUnavailable),
        _ => {}
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MESSAGE_BYTES as u64)
    {
        return Err(BrowserErrorCode::IncompatibleServer);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        let next = timeout(RPC_TIMEOUT, stream.next())
            .await
            .map_err(|_| BrowserErrorCode::DesktopUnavailable)?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|_| BrowserErrorCode::DesktopUnavailable)?;
        if body.len().saturating_add(chunk.len()) > MAX_MESSAGE_BYTES {
            return Err(BrowserErrorCode::IncompatibleServer);
        }
        body.extend_from_slice(&chunk);
    }
    let response: Value =
        serde_json::from_slice(&body).map_err(|_| BrowserErrorCode::IncompatibleServer)?;
    if response["jsonrpc"] != JSON_RPC_VERSION {
        return Err(BrowserErrorCode::IncompatibleServer);
    }
    if response.get("error").is_some() {
        let failure: RpcFailure = serde_json::from_value(response.clone())
            .map_err(|_| BrowserErrorCode::IncompatibleServer)?;
        if failure.id != request.id {
            return Err(BrowserErrorCode::IncompatibleServer);
        }
        match failure.error.code {
            -32001 => return Err(BrowserErrorCode::AuthenticationFailed),
            -32003 => return Err(BrowserErrorCode::AuthorizationFailed),
            _ => {}
        }
        return Ok(response);
    }
    if response.get("result").is_none() {
        return Err(BrowserErrorCode::IncompatibleServer);
    }
    let _: RpcSuccess = serde_json::from_value(response.clone())
        .map_err(|_| BrowserErrorCode::IncompatibleServer)?;
    let expected_id = request
        .id
        .as_ref()
        .ok_or(BrowserErrorCode::IncompatibleServer)?;
    let expected_id =
        serde_json::to_value(expected_id).map_err(|_| BrowserErrorCode::IncompatibleServer)?;
    if response.get("id") != Some(&expected_id) {
        return Err(BrowserErrorCode::IncompatibleServer);
    }
    Ok(response)
}

fn validate_endpoint(value: &str) -> Result<Url, BrowserErrorCode> {
    let endpoint = Url::parse(value).map_err(|_| BrowserErrorCode::InvalidTarget)?;
    let loopback = match endpoint.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    let secure = endpoint.scheme() == "https";
    if !secure && !(endpoint.scheme() == "http" && loopback) {
        return Err(BrowserErrorCode::InvalidTarget);
    }
    if endpoint.username() != ""
        || endpoint.password().is_some()
        || endpoint.path() != "/"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(BrowserErrorCode::InvalidTarget);
    }
    Ok(endpoint)
}

fn valid_credential_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_bearer_token(value: &str) -> bool {
    value.len() == 67
        && value.starts_with("fx_")
        && value[3..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn credential_account(credential_id: &str, endpoint: &str) -> String {
    let fingerprint = Sha256::digest(endpoint.as_bytes());
    let fingerprint = fingerprint
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{credential_id}-{fingerprint}")
}

fn rpc_id_string(id: RpcId) -> String {
    match id {
        RpcId::String(value) => value,
        RpcId::Number(value) => value.to_string(),
    }
}

#[cfg(windows)]
fn get_credential(credential_id: &str, endpoint: &str) -> Result<String, BrowserErrorCode> {
    let account = credential_account(credential_id, endpoint);
    let entry = keyring::Entry::new(CREDENTIAL_SERVICE, &account)
        .map_err(|_| BrowserErrorCode::CredentialStoreUnavailable)?;
    entry
        .get_password()
        .map_err(|_| BrowserErrorCode::CredentialStoreUnavailable)
}

#[cfg(not(windows))]
fn get_credential(_credential_id: &str, _endpoint: &str) -> Result<String, BrowserErrorCode> {
    Err(BrowserErrorCode::CredentialStoreUnavailable)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
        time::Duration,
    };

    use fetchx_browser_protocol::{BrowserErrorCode, BrowserResponseBody, OkPayload};
    use fetchx_remote_protocol::{JSON_RPC_VERSION, RpcId, RpcRequest};
    use serde_json::json;

    use super::{
        forward_rpc, forward_rpc_with_token, valid_bearer_token, valid_credential_id,
        validate_endpoint,
    };

    #[test]
    fn remote_target_requires_https_but_allows_loopback_http() {
        assert!(validate_endpoint("https://fetchx.example.test").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:39800").is_ok());
        assert!(validate_endpoint("http://[::1]:39800").is_ok());
        assert!(validate_endpoint("http://localhost:39800").is_ok());
        assert!(validate_endpoint("http://192.168.1.10:39800").is_err());
    }

    #[test]
    fn remote_target_rejects_non_origin_urls_and_credentials() {
        for endpoint in [
            "https://user:pass@fetchx.example.test",
            "https://fetchx.example.test/path",
            "https://fetchx.example.test?token=x",
            "file:///fetchx",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "accepted {endpoint}");
        }
    }

    #[test]
    fn credential_references_and_tokens_are_validated() {
        assert!(valid_credential_id("target-01"));
        assert!(!valid_credential_id("../target"));
        assert!(valid_bearer_token(
            "fx_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!valid_bearer_token("fx_short"));
    }

    #[test]
    fn remote_rpc_rejects_methods_outside_the_browser_allowlist() {
        let response = forward_rpc(
            "https://fetchx.example.test",
            "target-1",
            &RpcRequest {
                jsonrpc: JSON_RPC_VERSION.to_owned(),
                id: Some(RpcId::String("browser-pause".to_owned())),
                method: "download.pause".to_owned(),
                params: None,
            },
        );
        match response.body {
            BrowserResponseBody::Err(error) => {
                assert_eq!(
                    error.error.code,
                    fetchx_browser_protocol::BrowserErrorCode::UnsupportedMessage
                );
            }

            other => panic!("unexpected browser response: {other:?}"),
        }
    }

    #[test]
    fn remote_browser_download_rejects_non_http_urls_before_network_access() {
        let response = forward_rpc(
            "https://fetchx.example.test",
            "target-1",
            &RpcRequest {
                jsonrpc: JSON_RPC_VERSION.to_owned(),
                id: Some(RpcId::String("browser-download".to_owned())),
                method: "download.create".to_owned(),
                params: Some(json!({ "url": "magnet:?xt=urn:btih:example" })),
            },
        );
        match response.body {
            BrowserResponseBody::Err(error) => {
                assert_eq!(error.error.code, BrowserErrorCode::InvalidRequest);
            }
            other => panic!("unexpected browser response: {other:?}"),
        }
    }

    #[test]
    fn remote_rpc_reports_authentication_and_authorization_failures()
    -> Result<(), Box<dyn std::error::Error>> {
        for (status, expected_code) in [
            ("401 Unauthorized", BrowserErrorCode::AuthenticationFailed),
            ("403 Forbidden", BrowserErrorCode::AuthorizationFailed),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let address = listener.local_addr()?;
            let status = status.to_owned();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept discovery request");
                let _ = read_http_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .expect("write authentication failure");
            });
            let endpoint = validate_endpoint(&format!("http://{address}"))
                .map_err(|_| "invalid test endpoint")?;
            let response = forward_rpc_with_token(
                endpoint,
                format!("fx_{}", "a".repeat(64)),
                &RpcRequest {
                    jsonrpc: JSON_RPC_VERSION.to_owned(),
                    id: Some(RpcId::String("browser-list".to_owned())),
                    method: "download.list".to_owned(),
                    params: None,
                },
            );
            server.join().expect("mock RPC server");
            match response.body {
                BrowserResponseBody::Err(error) => {
                    assert_eq!(error.error.code, expected_code);
                }
                other => panic!("unexpected browser response: {other:?}"),
            }
        }
        Ok(())
    }

    #[test]
    fn remote_rpc_discovers_capability_and_sends_bearer_auth()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let token = format!("fx_{}", "a".repeat(64));
        let expected_token = token.clone();
        let server = thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept RPC request");
                let request = read_http_request(&mut stream);
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains(&format!("authorization: bearer {expected_token}"))
                );
                let body = request
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body)
                    .expect("HTTP body separator");
                let request: serde_json::Value =
                    serde_json::from_str(body).expect("JSON-RPC request body");
                let response = if index == 0 {
                    json!({
                        "jsonrpc": "2.0",
                        "id": "fetchx-browser-discovery",
                        "result": {
                            "protocol_version": "fetchx.rpc.v1",
                            "server_version": "0.1.0",
                            "capabilities": ["system.discover", "download.list"]
                        }
                    })
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": { "downloads": [] }
                    })
                };
                let body = serde_json::to_vec(&response).expect("serialize response");
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .expect("write response headers");
                stream.write_all(&body).expect("write response body");
            }
        });

        let endpoint =
            validate_endpoint(&format!("http://{address}")).map_err(|_| "invalid test endpoint")?;
        let response = forward_rpc_with_token(
            endpoint,
            token,
            &RpcRequest {
                jsonrpc: JSON_RPC_VERSION.to_owned(),
                id: Some(RpcId::String("browser-list".to_owned())),
                method: "download.list".to_owned(),
                params: None,
            },
        );
        server.join().expect("mock RPC server");
        match response.body {
            BrowserResponseBody::Ok(ok) => match ok.payload {
                OkPayload::Rpc(rpc) => {
                    assert_eq!(rpc.rpc_response["result"]["downloads"], json!([]));
                }
                other => panic!("unexpected browser response payload: {other:?}"),
            },
            other => panic!("unexpected browser response: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn remote_rpc_rejects_oversized_chunked_responses() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || {
            let (mut discovery_stream, _) = listener.accept().expect("accept discovery request");
            let _ = read_http_request(&mut discovery_stream);
            let discovery = json!({
                "jsonrpc": "2.0",
                "id": "fetchx-browser-discovery",
                "result": {
                    "protocol_version": "fetchx.rpc.v1",
                    "server_version": "0.1.0",
                    "capabilities": ["system.discover", "download.list"]
                }
            });
            let discovery = serde_json::to_vec(&discovery).expect("serialize discovery");
            write!(
                discovery_stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                discovery.len()
            )
            .expect("write discovery headers");
            discovery_stream
                .write_all(&discovery)
                .expect("write discovery response");

            let (mut rpc_stream, _) = listener.accept().expect("accept RPC request");
            let _ = read_http_request(&mut rpc_stream);
            write!(
                rpc_stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n",
                fetchx_remote_protocol::MAX_MESSAGE_BYTES + 1
            )
            .expect("write oversized response headers");
            rpc_stream
                .write_all(&vec![b'x'; fetchx_remote_protocol::MAX_MESSAGE_BYTES + 1])
                .expect("write oversized response body");
            rpc_stream
                .write_all(b"\r\n0\r\n\r\n")
                .expect("finish chunked response");
        });

        let endpoint =
            validate_endpoint(&format!("http://{address}")).map_err(|_| "invalid test endpoint")?;
        let response = forward_rpc_with_token(
            endpoint,
            format!("fx_{}", "a".repeat(64)),
            &RpcRequest {
                jsonrpc: JSON_RPC_VERSION.to_owned(),
                id: Some(RpcId::String("browser-list".to_owned())),
                method: "download.list".to_owned(),
                params: None,
            },
        );
        server.join().expect("mock RPC server");
        match response.body {
            BrowserResponseBody::Err(error) => assert_eq!(
                error.error.code,
                fetchx_browser_protocol::BrowserErrorCode::IncompatibleServer
            ),
            other => panic!("unexpected browser response: {other:?}"),
        }
        Ok(())
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set mock socket timeout");
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let count = stream.read(&mut chunk).expect("read HTTP request");
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|length| length.trim().parse::<usize>().ok())
                    })
                    .expect("content length");
                if bytes.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8(bytes).expect("HTTP request UTF-8")
    }
}

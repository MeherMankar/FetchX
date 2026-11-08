use std::{env, sync::atomic::AtomicU64, time::Duration};

use fetchx_remote_protocol::{
    DOWNLOAD_CANCEL, DOWNLOAD_CREATE, DOWNLOAD_FIND_DUPLICATE, DOWNLOAD_LIST, DOWNLOAD_PAUSE,
    DOWNLOAD_RESUME, JSON_RPC_VERSION, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, ProtocolInfo,
    QUEUE_MOVE_DOWN, QUEUE_MOVE_UP, QUEUE_PAUSE_DISPATCH, QUEUE_RESUME_DISPATCH, RpcFailure,
    RpcId, RpcRequest, RpcSuccess, SYSTEM_DISCOVER,
};
use reqwest::{StatusCode, Url, redirect::Policy};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    args::{RemoteCommand, RemoteOpts},
    exit::{self, ExitCode},
    output::{self, Table},
};

const TOKEN_ENV: &str = "FETCHX_REMOTE_TOKEN";
const URL_ENV: &str = "FETCHX_REMOTE_URL";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub async fn run(options: RemoteOpts, json_output: bool) -> ExitCode {
    let explicit_url = match &options.command {
        RemoteCommand::Connect { url } => url.as_deref(),
        _ => None,
    };
    let server = explicit_url
        .or(options.server.as_deref())
        .map(str::to_owned)
        .or_else(|| env::var(URL_ENV).ok());
    let Some(server) = server else {
        output::print_error(
            json_output,
            "remote server URL is required; use --server or FETCHX_REMOTE_URL",
        );
        return exit::INVALID_ARGS;
    };

    let token = match env::var(TOKEN_ENV) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            output::print_error(
                json_output,
                "remote authentication token is required in FETCHX_REMOTE_TOKEN",
            );
            return exit::CONFIG_ERROR;
        }
    };

    let client = match RemoteClient::new(&server, token) {
        Ok(client) => client,
        Err(error) => {
            output::print_error(json_output, &error);
            return exit::INVALID_ARGS;
        }
    };

    let result = match options.command {
        RemoteCommand::Connect { .. } | RemoteCommand::Status => {
            client.status().await.map(|status| {
                if json_output {
                    output::print_json_ok(&json!({
                        "ok": true,
                        "server": status.health,
                        "protocol": status.protocol,
                    }));
                } else {
                    println!(
                        "Connected to FetchX {} ({})",
                        status.protocol.server_version, status.protocol.protocol_version
                    );
                    println!(
                        "Health: {}",
                        status.health["status"].as_str().unwrap_or("unknown")
                    );
                }
            })
        }
        RemoteCommand::Downloads => client.downloads().await.map(|downloads| {
            if json_output {
                output::print_json_ok(&json!({
                    "ok": true,
                    "count": downloads.len(),
                    "downloads": downloads,
                }));
            } else if downloads.is_empty() {
                println!("No remote downloads.");
            } else {
                let mut table = Table::new(vec!["ID", "STATUS", "DOWNLOADED", "FILE", "ENGINE"]);
                for download in &downloads {
                    table.add_row(vec![
                        short_id(download["id"].as_str().unwrap_or("")),
                        text_field(download, "status"),
                        output::fmt_bytes(download["downloaded_bytes"].as_u64().unwrap_or(0)),
                        output::truncate(download["filename"].as_str().unwrap_or(""), 32),
                        text_field(download, "engine"),
                    ]);
                }
                table.print();
                println!("\n{} remote download(s)", downloads.len());
            }
        }),
        RemoteCommand::FindDuplicate { url } => {
            client.find_duplicate(&url).await.map(|duplicate| {
                if json_output {
                    output::print_json_ok(&json!({
                        "duplicate": duplicate.is_some(),
                        "download": duplicate,
                    }));
                } else if let Some(download) = duplicate {
                    println!(
                        "Duplicate download found: {} ({})",
                        download["filename"].as_str().unwrap_or("unknown file"),
                        download["id"].as_str().unwrap_or("unknown ID")
                    );
                } else {
                    println!("No matching download found.");
                }
            })
        }
        RemoteCommand::Add { url } => client.add(&url).await.map(|id| {
            if json_output {
                output::print_json_ok(&json!({ "ok": true, "id": id }));
            } else {
                println!("Remote download created: {id}");
            }
        }),
        RemoteCommand::Pause { id } => {
            run_lifecycle(&client, DOWNLOAD_PAUSE, id, json_output).await
        }
        RemoteCommand::Resume { id } => {
            run_lifecycle(&client, DOWNLOAD_RESUME, id, json_output).await
        }
        RemoteCommand::Cancel { id } => {
            run_lifecycle(&client, DOWNLOAD_CANCEL, id, json_output).await
        }
        RemoteCommand::QueueUp { id } => {
            run_lifecycle(&client, QUEUE_MOVE_UP, id, json_output).await
        }
        RemoteCommand::QueueDown { id } => {
            run_lifecycle(&client, QUEUE_MOVE_DOWN, id, json_output).await
        }
        RemoteCommand::QueueStop => {
            run_queue_dispatch(&client, QUEUE_PAUSE_DISPATCH, true, json_output).await
        }
        RemoteCommand::QueueStart => {
            run_queue_dispatch(&client, QUEUE_RESUME_DISPATCH, false, json_output).await
        }
    };

    match result {
        Ok(()) => exit::SUCCESS,
        Err(error) => {
            output::print_error(json_output, &error);
            exit::GENERAL_ERROR
        }
    }
}

async fn run_queue_dispatch(
    client: &RemoteClient,
    method: &'static str,
    paused: bool,
    json_output: bool,
) -> Result<(), String> {
    let result = client.call(method, None).await?;
    let dispatch_paused = result["dispatch_paused"]
        .as_bool()
        .ok_or_else(|| "remote server returned an invalid queue state".to_owned())?;
    if dispatch_paused != paused {
        return Err("remote server did not apply the requested queue state".to_owned());
    }
    if json_output {
        output::print_json_ok(&json!({
            "ok": true,
            "dispatch_paused": dispatch_paused,
        }));
    } else {
        println!(
            "Remote queue dispatch {}.",
            if paused { "held" } else { "resumed" }
        );
    }
    Ok(())
}

async fn run_lifecycle(
    client: &RemoteClient,
    method: &'static str,
    id: String,
    json_output: bool,
) -> Result<(), String> {
    let result = client.call(method, Some(json!({ "id": id }))).await?;
    let returned_id = result["id"].as_str().unwrap_or(&id);
    if json_output {
        output::print_json_ok(&json!({ "ok": true, "id": returned_id }));
    } else {
        let action = method.strip_prefix("download.").unwrap_or("updated");
        println!("Remote download {returned_id}: {action} requested.");
    }
    Ok(())
}

struct RemoteStatus {
    health: Value,
    protocol: ProtocolInfo,
}

struct RemoteClient {
    base_url: Url,
    http: reqwest::Client,
    token: String,
}

impl RemoteClient {
    fn new(server: &str, token: String) -> Result<Self, String> {
        let base_url = validate_server_url(server)?;
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| "could not initialize remote HTTP client".to_owned())?;
        Ok(Self {
            base_url,
            http,
            token,
        })
    }

    async fn status(&self) -> Result<RemoteStatus, String> {
        let health_url = self
            .base_url
            .join("/health")
            .map_err(|_| "remote server URL could not be used for health checks".to_owned())?;
        let response = self
            .http
            .get(health_url)
            .send()
            .await
            .map_err(|_| "could not reach the remote server".to_owned())?;
        let health = read_json(response).await?;
        if health["status"] != "ok" {
            return Err("remote server health check failed".to_owned());
        }

        let protocol = self.call(SYSTEM_DISCOVER, None).await?;
        let protocol: ProtocolInfo = serde_json::from_value(protocol)
            .map_err(|_| "remote server returned invalid protocol discovery data".to_owned())?;
        if protocol.protocol_version != PROTOCOL_VERSION {
            return Err("remote server uses an incompatible FetchX protocol".to_owned());
        }
        Ok(RemoteStatus { health, protocol })
    }

    async fn downloads(&self) -> Result<Vec<Value>, String> {
        let result = self.call(DOWNLOAD_LIST, None).await?;
        result["downloads"]
            .as_array()
            .cloned()
            .ok_or_else(|| "remote server returned an invalid downloads response".to_owned())
    }

    async fn find_duplicate(&self, url: &str) -> Result<Option<Value>, String> {
        self.call(DOWNLOAD_FIND_DUPLICATE, Some(json!({ "url": url })))
            .await
            .and_then(|result| {
                serde_json::from_value(result)
                    .map_err(|_| "remote server returned an invalid duplicate response".to_owned())
            })
    }

    async fn add(&self, url: &str) -> Result<String, String> {
        let result = self
            .call(DOWNLOAD_CREATE, Some(json!({ "url": url })))
            .await?;
        result["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "remote server returned an invalid download ID".to_owned())
    }

    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let request_id = RpcId::String(format!(
            "fetchx-cli-{}",
            NEXT_REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let request = RpcRequest {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id: Some(request_id.clone()),
            method: method.to_owned(),
            params,
        };
        let body = serde_json::to_vec(&request)
            .map_err(|_| "could not encode remote protocol request".to_owned())?;
        let rpc_url = self
            .base_url
            .join("/rpc")
            .map_err(|_| "remote server URL could not be used for RPC".to_owned())?;
        let response = self
            .http
            .post(rpc_url)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| "could not reach the remote server".to_owned())?;
        let status = response.status();
        let response = read_response_bytes(response).await?;
        parse_rpc_response(status, &response, &request_id)
    }
}

fn validate_server_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "invalid remote server URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "remote server URL must be an HTTP(S) origin without credentials or path".to_owned(),
        );
    }

    let is_loopback = match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host)
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    };
    if url.scheme() != "https" && !is_loopback {
        return Err("HTTPS is required for non-loopback remote servers".to_owned());
    }
    Ok(url)
}

async fn read_json(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let body = read_response_bytes(response).await?;
    if !status.is_success() {
        return Err(format!("remote server returned HTTP {}", status.as_u16()));
    }
    serde_json::from_slice(&body).map_err(|_| "remote server returned invalid JSON".to_owned())
}

async fn read_response_bytes(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MESSAGE_BYTES as u64)
    {
        return Err("remote server response exceeds the size limit".to_owned());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "could not read remote server response".to_owned())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_MESSAGE_BYTES {
            return Err("remote server response exceeds the size limit".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_rpc_response(
    status: StatusCode,
    body: &[u8],
    expected_id: &RpcId,
) -> Result<Value, String> {
    let response: RpcResponse = serde_json::from_slice(body)
        .map_err(|_| "remote server returned an invalid JSON-RPC response".to_owned())?;
    match response {
        RpcResponse::Success(success) => {
            validate_success(&success, status, expected_id)?;
            Ok(success.result)
        }
        RpcResponse::Failure(failure) => {
            validate_failure(&failure, status, expected_id)?;
            Err(format_rpc_error(&failure, status))
        }
    }
}

fn validate_success(
    response: &RpcSuccess,
    status: StatusCode,
    expected_id: &RpcId,
) -> Result<(), String> {
    if !status.is_success() || response.jsonrpc != JSON_RPC_VERSION || &response.id != expected_id {
        return Err("remote server returned a mismatched JSON-RPC response".to_owned());
    }
    Ok(())
}

fn validate_failure(
    response: &RpcFailure,
    status: StatusCode,
    expected_id: &RpcId,
) -> Result<(), String> {
    if response.jsonrpc != JSON_RPC_VERSION
        || response.id.as_ref().is_some_and(|id| id != expected_id)
    {
        return Err("remote server returned a mismatched JSON-RPC response".to_owned());
    }
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::UNAUTHORIZED && response.error.code != -32001
        || status == StatusCode::FORBIDDEN && response.error.code != -32003
    {
        return Err(format!("remote server returned HTTP {}", status.as_u16()));
    }
    Ok(())
}

fn format_rpc_error(response: &RpcFailure, status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED => "remote authentication failed (HTTP 401)".to_owned(),
        StatusCode::FORBIDDEN => "remote authorization denied (HTTP 403)".to_owned(),
        _ => format!(
            "remote RPC error {}: {}",
            response.error.code, response.error.message
        ),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RpcResponse {
    Success(RpcSuccess),
    Failure(RpcFailure),
}

fn short_id(id: &str) -> String {
    id.get(..8).unwrap_or(id).to_owned()
}

fn text_field(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or("—").replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_origins_and_loopback_http() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            validate_server_url("http://127.0.0.1:39800")?.as_str(),
            "http://127.0.0.1:39800/"
        );
        assert_eq!(
            validate_server_url("http://[::1]:39800")?.as_str(),
            "http://[::1]:39800/"
        );
        assert!(validate_server_url("https://fetchx.example.test").is_ok());
        Ok(())
    }

    #[test]
    fn rejects_insecure_or_ambiguous_remote_urls() {
        for url in [
            "http://fetchx.example.test",
            "https://user:password@fetchx.example.test",
            "https://fetchx.example.test/path",
            "https://fetchx.example.test/?token=secret",
            "ftp://fetchx.example.test",
        ] {
            assert!(validate_server_url(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn rpc_response_requires_matching_id_and_protocol() -> Result<(), Box<dyn std::error::Error>> {
        let id = RpcId::String("request-1".to_owned());
        let success = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "result": { "status": "ok" }
        });
        let encoded = serde_json::to_vec(&success)?;
        assert_eq!(
            parse_rpc_response(StatusCode::OK, &encoded, &id)?["status"],
            "ok"
        );

        let mismatched = json!({
            "jsonrpc": "2.0",
            "id": "another-request",
            "result": {}
        });
        let encoded = serde_json::to_vec(&mismatched)?;
        assert!(parse_rpc_response(StatusCode::OK, &encoded, &id).is_err());
        Ok(())
    }

    #[test]
    fn authentication_errors_are_sanitized() -> Result<(), Box<dyn std::error::Error>> {
        let id = RpcId::String("request-1".to_owned());
        let failure = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "error": { "code": -32001, "message": "Unauthorized" }
        });
        let encoded = serde_json::to_vec(&failure)?;
        let error = parse_rpc_response(StatusCode::UNAUTHORIZED, &encoded, &id)
            .expect_err("unauthorized response must fail");
        assert_eq!(error, "remote authentication failed (HTTP 401)");
        assert!(!error.contains("token"));
        Ok(())
    }

    #[tokio::test]
    async fn remote_rpc_does_not_follow_redirects() -> Result<(), Box<dyn std::error::Error>> {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::timeout,
        };

        let rpc_listener = TcpListener::bind("127.0.0.1:0").await?;
        let rpc_address = rpc_listener.local_addr()?;
        let redirect_listener = TcpListener::bind("127.0.0.1:0").await?;
        let redirect_address = redirect_listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = rpc_listener.accept().await?;
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await?;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{redirect_address}/rpc\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).await?;
            stream.shutdown().await
        });

        let client = RemoteClient::new(&format!("http://{rpc_address}"), "test-token".to_owned())?;
        assert!(client.call(DOWNLOAD_LIST, None).await.is_err());
        server.await??;
        assert!(
            timeout(Duration::from_millis(150), redirect_listener.accept())
                .await
                .is_err(),
            "remote client followed an RPC redirect"
        );
        Ok(())
    }
}

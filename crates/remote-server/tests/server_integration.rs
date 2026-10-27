use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::{sleep, timeout},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

use fetchx_remote_server::{
    RemoteServer, Role, ServerConfig, current_auth_role, initialize_auth, rotate_auth,
    set_auth_role,
};

#[tokio::test]
async fn remote_http_and_event_stream_use_core_download_lifecycle()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let config = test_config(temp.path().to_path_buf());
    let restart_config = config.clone();
    let token = initialize_auth(&config.database_path).await?;
    let downloads_dir = config.downloads_dir.clone();
    let server = RemoteServer::start(config).await?;
    let address = server.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(server.run_until(async {
        let _ = shutdown_receiver.await;
    }));

    let public_client = reqwest::Client::new();
    let response = public_client
        .get(format!("http://{address}/health"))
        .send()
        .await?;
    let health: Value = serde_json::from_slice(&response.bytes().await?)?;
    assert_eq!(health["status"], "ok");
    let client = authenticated_client(&token)?;

    let unauthenticated_request = json!({
        "jsonrpc": "2.0",
        "id": "missing-token",
        "method": "download.list"
    });
    let missing_response = send_rpc(&public_client, address, &unauthenticated_request).await?;
    assert_eq!(missing_response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let missing_body: Value = serde_json::from_slice(&missing_response.bytes().await?)?;
    assert_eq!(missing_body["error"]["code"], -32001);
    assert!(!missing_body.to_string().contains(&token));

    let invalid_client = reqwest::Client::builder()
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_static("Bearer fx_invalid"),
            );
            headers
        })
        .build()?;
    let invalid_response = send_rpc(&invalid_client, address, &unauthenticated_request).await?;
    assert_eq!(invalid_response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let invalid_body: Value = serde_json::from_slice(&invalid_response.bytes().await?)?;
    assert_eq!(invalid_body["error"]["code"], missing_body["error"]["code"]);
    assert_eq!(
        invalid_body["error"]["message"],
        missing_body["error"]["message"]
    );

    let unauthenticated_socket = connect_async(format!("ws://{address}/events")).await;
    let Err(tokio_tungstenite::tungstenite::Error::Http(handshake_error)) = unauthenticated_socket
    else {
        return Err(std::io::Error::other("unauthenticated WebSocket was not rejected").into());
    };
    assert_eq!(handshake_error.status(), reqwest::StatusCode::UNAUTHORIZED);

    let oversized = vec![b' '; fetchx_remote_protocol::MAX_MESSAGE_BYTES + 1];
    let oversized_response = client
        .post(format!("http://{address}/rpc"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(oversized)
        .send()
        .await?;
    let oversized_error: Value = serde_json::from_slice(&oversized_response.bytes().await?)?;
    assert_eq!(oversized_error["error"]["code"], -32600);

    let (websocket, _) = connect_authenticated_websocket(address, &token).await?;
    let (_websocket_sink, mut websocket_stream) = websocket.split();
    let (second_websocket, _) = connect_authenticated_websocket(address, &token).await?;
    let (_second_websocket_sink, mut second_websocket_stream) = second_websocket.split();
    sleep(Duration::from_millis(50)).await;

    let expected_payload = [0x46, 0x58, 0x21, 0x42].repeat(32_768);
    let fixture_hash = Sha256::digest(&expected_payload);
    let fixture_address = start_http_fixture(expected_payload.clone()).await?;
    let unauthenticated_create = send_rpc(
        &public_client,
        address,
        &json!({
            "jsonrpc": "2.0",
            "id": "unauthorized-create",
            "method": "download.create",
            "params": { "url": format!("http://{fixture_address}/payload.bin") }
        }),
    )
    .await?;
    assert_eq!(
        unauthenticated_create.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let unauthorized_create_body: Value =
        serde_json::from_slice(&unauthenticated_create.bytes().await?)?;
    assert_eq!(unauthorized_create_body["error"]["code"], -32001);
    let create = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "create-1",
            "method": "download.create",
            "params": { "url": format!("http://{fixture_address}/payload.bin") }
        }),
    )
    .await?;
    assert!(create.get("error").is_none(), "{create}");
    let id = create["result"]["id"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("create response omitted download ID"))?;

    let completed_event = timeout(
        Duration::from_secs(15),
        receive_completed(&mut websocket_stream),
    )
    .await;
    let completed_event = match completed_event {
        Ok(Ok(event)) => event,
        Ok(Err(_)) | Err(_) => {
            let current = post_rpc(
                &client,
                address,
                json!({
                    "jsonrpc": "2.0",
                    "id": "timeout-status",
                    "method": "download.get",
                    "params": { "id": id }
                }),
            )
            .await?;
            panic!("download did not complete; current state: {current}");
        }
    };
    assert_eq!(
        completed_event["params"]["data"]["download"]["status"],
        "completed"
    );
    assert_eq!(
        completed_event["params"]["data"]["download"]["progress_basis_points"],
        10_000
    );
    let second_completed_event = timeout(
        Duration::from_secs(15),
        receive_completed(&mut second_websocket_stream),
    )
    .await??;
    assert_eq!(
        second_completed_event["params"]["data"]["download"]["id"],
        id
    );
    let remaining_event = timeout(Duration::from_secs(3), websocket_stream.next())
        .await?
        .ok_or_else(|| std::io::Error::other("event stream ended before queue update"))??;
    let Message::Text(remaining_event) = remaining_event else {
        return Err(std::io::Error::other("event stream ended before queue update").into());
    };
    let remaining_event: Value = serde_json::from_str(remaining_event.as_ref())?;
    assert_eq!(remaining_event["method"], "queue.updated");

    let get = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "download.get",
            "params": { "id": id }
        }),
    )
    .await?;
    assert_eq!(get["result"]["status"], "completed");
    assert!(get["result"].get("destination").is_none());
    assert!(get["result"].get("url").is_none());
    assert_eq!(
        get["result"]["downloaded_bytes"],
        expected_payload.len() as u64
    );

    let file_name = get["result"]["filename"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("download response omitted filename"))?;
    let completed_file = downloads_dir.join(format!("{id}-{file_name}"));
    let actual_payload = tokio::fs::read(completed_file).await?;
    assert_eq!(actual_payload.len(), expected_payload.len());
    assert_eq!(Sha256::digest(&actual_payload), fixture_hash);

    let discovery = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "discover",
            "method": "system.discover"
        }),
    )
    .await?;
    let capabilities = discovery["result"]["capabilities"]
        .as_array()
        .ok_or_else(|| std::io::Error::other("discovery omitted capabilities"))?;
    assert!(!capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            Some("download.retry" | "download.setEngine" | "download.setSpeedLimit")
        )
    }));
    let unsupported = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "unsupported",
            "method": "download.setSpeedLimit",
            "params": { "id": id, "bytes_per_second": 1024 }
        }),
    )
    .await?;
    assert_eq!(unsupported["error"]["code"], -32012);

    let rotated_token = rotate_auth(&restart_config.database_path).await?;
    let old_token_response = send_rpc(
        &client,
        address,
        &json!({
            "jsonrpc": "2.0",
            "id": "old-token",
            "method": "download.list"
        }),
    )
    .await?;
    assert_eq!(
        old_token_response.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let rotated_client = authenticated_client(&rotated_token)?;
    let new_token_response = send_rpc(
        &rotated_client,
        address,
        &json!({
            "jsonrpc": "2.0",
            "id": "rotated-token",
            "method": "download.list"
        }),
    )
    .await?;
    assert!(new_token_response.status().is_success());

    let revocation_probe = post_rpc(
        &rotated_client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "revocation-probe",
            "method": "download.create",
            "params": { "url": "http://127.0.0.1:9/auth-revocation-probe.bin" }
        }),
    )
    .await?;
    assert!(revocation_probe.get("result").is_some());
    timeout(Duration::from_secs(3), async {
        loop {
            match websocket_stream.next().await {
                Some(Ok(Message::Text(_))) => {
                    return Err(std::io::Error::other(
                        "revoked WebSocket received a protected event",
                    ));
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(()),
                Some(Ok(_)) => {}
            }
        }
    })
    .await??;

    let _ = shutdown_sender.send(());
    server_task.await??;

    let restarted = RemoteServer::start(restart_config).await?;
    let restarted_address = restarted.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(restarted.run_until(async {
        let _ = shutdown_receiver.await;
    }));
    let restored = post_rpc(
        &rotated_client,
        restarted_address,
        json!({
            "jsonrpc": "2.0",
            "id": "restored",
            "method": "download.get",
            "params": { "id": id }
        }),
    )
    .await?;
    assert_eq!(restored["result"]["status"], "completed");
    let restored_name = restored["result"]["filename"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("restored download omitted filename"))?;
    let restored_payload =
        tokio::fs::read(downloads_dir.join(format!("{id}-{restored_name}"))).await?;
    assert_eq!(Sha256::digest(&restored_payload), fixture_hash);
    let old_after_restart = send_rpc(
        &client,
        restarted_address,
        &json!({
            "jsonrpc": "2.0",
            "id": "old-after-restart",
            "method": "download.list"
        }),
    )
    .await?;
    assert_eq!(
        old_after_restart.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let _ = shutdown_sender.send(());
    server_task.await??;
    Ok(())
}

#[tokio::test]
async fn authorization_roles_gate_rpc_events_and_survive_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let config = test_config(temp.path().to_path_buf());
    let restart_config = config.clone();
    let token = initialize_auth(&config.database_path).await?;

    // Databases created by Phase 21C have no role record and retain admin access.
    assert_eq!(current_auth_role(&config.database_path).await?, Role::Admin);
    let server = RemoteServer::start(config).await?;
    let address = server.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(server.run_until(async {
        let _ = shutdown_receiver.await;
    }));
    let client = authenticated_client(&token)?;

    let admin_remove = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "admin-remove-check",
            "method": "download.remove",
            "params": { "id": "00000000-0000-0000-0000-000000000001" }
        }),
    )
    .await?;
    assert_eq!(admin_remove["error"]["code"], -32004);

    set_auth_role(&restart_config.database_path, Role::ReadOnly).await?;
    for (id, method, params) in [
        (
            "create",
            "download.create",
            json!({ "url": "http://127.0.0.1:9/forbidden.bin" }),
        ),
        (
            "pause",
            "download.pause",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
        ),
        (
            "resume",
            "download.resume",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
        ),
        (
            "cancel",
            "download.cancel",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
        ),
        (
            "retry",
            "download.retry",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
        ),
        (
            "remove",
            "download.remove",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
        ),
        (
            "priority",
            "download.setPriority",
            json!({
                "id": "00000000-0000-0000-0000-000000000001",
                "priority": "low"
            }),
        ),
        (
            "engine",
            "download.setEngine",
            json!({
                "id": "00000000-0000-0000-0000-000000000001",
                "engine": "direct_http"
            }),
        ),
        (
            "speed-limit",
            "download.setSpeedLimit",
            json!({
                "id": "00000000-0000-0000-0000-000000000001",
                "bytes_per_second": 1024
            }),
        ),
    ] {
        let response = send_rpc(
            &client,
            address,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }),
        )
        .await?;
        assert_eq!(
            response.status(),
            reqwest::StatusCode::FORBIDDEN,
            "{method}"
        );
        let body: Value = serde_json::from_slice(&response.bytes().await?)?;
        assert_eq!(body["error"]["code"], -32003, "{method}");
        assert_eq!(body["error"]["message"], "permission denied", "{method}");
    }

    for request in [
        json!({ "jsonrpc": "2.0", "id": "read-list", "method": "download.list" }),
        json!({ "jsonrpc": "2.0", "id": "read-discover", "method": "system.discover" }),
    ] {
        let response = send_rpc(&client, address, &request).await?;
        assert!(response.status().is_success());
        let body: Value = serde_json::from_slice(&response.bytes().await?)?;
        assert!(body.get("result").is_some());
    }
    let read_get = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "read-get",
            "method": "download.get",
            "params": { "id": "00000000-0000-0000-0000-000000000001" }
        }),
    )
    .await?;
    assert_eq!(read_get["error"]["code"], -32004);

    let unauthorized_socket = connect_authenticated_websocket(address, &token).await;
    let Err(err) = unauthorized_socket else {
        return Err(std::io::Error::other("read-only WebSocket was not rejected").into());
    };
    let Some(tokio_tungstenite::tungstenite::Error::Http(handshake_error)) =
        err.downcast_ref::<tokio_tungstenite::tungstenite::Error>()
    else {
        return Err(
            std::io::Error::other("read-only WebSocket returned an unexpected error").into(),
        );
    };
    assert_eq!(handshake_error.status(), reqwest::StatusCode::FORBIDDEN);

    set_auth_role(&restart_config.database_path, Role::Operator).await?;
    let (mut operator_socket, _) = connect_authenticated_websocket(address, &token).await?;
    let expected_payload = [0x21_u8, 0x46, 0x58, 0x44].repeat(16_384);
    let fixture_hash = Sha256::digest(&expected_payload);
    let fixture_address = start_http_fixture(expected_payload.clone()).await?;
    let create = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "operator-create",
            "method": "download.create",
            "params": { "url": format!("http://{fixture_address}/operator.bin") }
        }),
    )
    .await?;
    let download_id = create["result"]["id"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("operator download was rejected"))?;
    let completed = timeout(
        Duration::from_secs(15),
        receive_completed(&mut operator_socket),
    )
    .await??;
    assert_eq!(completed["params"]["data"]["download"]["id"], download_id);
    let completed_path = restart_config
        .downloads_dir
        .join(format!("{download_id}-operator.bin"));
    let completed_payload = tokio::fs::read(completed_path).await?;
    assert_eq!(Sha256::digest(&completed_payload), fixture_hash);

    set_auth_role(&restart_config.database_path, Role::ReadOnly).await?;
    operator_socket.close(None).await?;
    let operator_remove = send_rpc(
        &client,
        address,
        &json!({
            "jsonrpc": "2.0",
            "id": "operator-remove",
            "method": "download.remove",
            "params": { "id": download_id }
        }),
    )
    .await?;
    assert_eq!(operator_remove.status(), reqwest::StatusCode::FORBIDDEN);

    let _ = shutdown_sender.send(());
    server_task.await??;

    let restarted = RemoteServer::start(restart_config.clone()).await?;
    let restarted_address = restarted.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(restarted.run_until(async {
        let _ = shutdown_receiver.await;
    }));
    assert_eq!(
        current_auth_role(&restart_config.database_path).await?,
        Role::ReadOnly
    );
    let persisted_read = send_rpc(
        &client,
        restarted_address,
        &json!({ "jsonrpc": "2.0", "id": "persisted-read", "method": "download.list" }),
    )
    .await?;
    assert!(persisted_read.status().is_success());
    let persisted_write = send_rpc(
        &client,
        restarted_address,
        &json!({
            "jsonrpc": "2.0",
            "id": "persisted-write",
            "method": "download.create",
            "params": { "url": "http://127.0.0.1:9/forbidden-after-restart.bin" }
        }),
    )
    .await?;
    assert_eq!(persisted_write.status(), reqwest::StatusCode::FORBIDDEN);
    let _ = shutdown_sender.send(());
    server_task.await??;
    Ok(())
}

async fn receive_completed<S, E>(stream: &mut S) -> Result<Value, Box<dyn std::error::Error>>
where
    S: Stream<Item = Result<Message, E>> + Unpin,
    E: std::error::Error + 'static,
{
    loop {
        let Some(message) = stream.next().await else {
            return Err(std::io::Error::other("event stream closed").into());
        };
        if let Message::Text(message) = message? {
            if let Ok(notification) = serde_json::from_str::<Value>(message.as_ref()) {
                if notification["method"] == "download.completed" {
                    return Ok(notification);
                }
            }
        }
    }
}

async fn post_rpc(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    request: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = send_rpc(client, address, &request).await?;
    Ok(serde_json::from_slice(&response.bytes().await?)?)
}

async fn send_rpc(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    request: &Value,
) -> Result<reqwest::Response, Box<dyn std::error::Error>> {
    client
        .post(format!("http://{address}/rpc"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(request)?)
        .send()
        .await
        .map_err(Into::into)
}

fn authenticated_client(token: &str) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
    let mut headers = reqwest::header::HeaderMap::new();
    let authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(Box::<dyn std::error::Error>::from)?;
    headers.insert(reqwest::header::AUTHORIZATION, authorization);
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(Into::into)
}

async fn connect_authenticated_websocket(
    address: std::net::SocketAddr,
    token: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    Box<dyn std::error::Error>,
> {
    let mut request = format!("ws://{address}/events").into_client_request()?;
    request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    Ok(connect_async(request).await?)
}

async fn start_http_fixture(
    payload: Vec<u8>,
) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            match send_fixture(&mut stream, &payload).await {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });
    Ok(address)
}

async fn send_fixture(stream: &mut TcpStream, payload: &[u8]) -> Result<bool, std::io::Error> {
    let mut request = vec![0; 4096];
    let count = stream.read(&mut request).await?;
    let is_head = String::from_utf8_lossy(&request[..count]).starts_with("HEAD ");
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/octet-stream\r\n\r\n",
        payload.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    if !is_head {
        stream.write_all(payload).await?;
    }
    stream.shutdown().await?;
    Ok(!is_head)
}

fn test_config(root: PathBuf) -> ServerConfig {
    ServerConfig {
        bind_ip: Ipv4Addr::LOCALHOST.into(),
        port: 0,
        database_path: root.join("fetchx.db"),
        downloads_dir: root.join("downloads"),
        runtime_dir: root.join("runtime"),
        max_request_bytes: fetchx_remote_protocol::MAX_MESSAGE_BYTES,
        max_connections: 16,
        request_timeout: Duration::from_secs(10),
        event_backlog: 64,
    }
}

use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue},
};

use fetchx_remote_server::{
    RemoteServer, Role, ServerConfig, current_auth_role, initialize_auth, rotate_auth,
    set_auth_role,
};

#[tokio::test]
async fn remote_http_auth_and_security_limits_survive_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let config = test_config(temp.path().to_path_buf());
    let restart_config = config.clone();
    let token = initialize_auth(&config.database_path).await?;
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
    let (mut websocket_sink, _websocket_stream) = websocket.split();

    let local_target = "http://127.0.0.1:9/payload.bin";
    let unauthenticated_create = send_rpc(
        &public_client,
        address,
        &json!({
            "jsonrpc": "2.0",
            "id": "unauthorized-create",
            "method": "download.create",
            "params": { "url": local_target }
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
    let rejected_create = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "create-1",
            "method": "download.create",
            "params": { "url": local_target }
        }),
    )
    .await?;
    assert_eq!(rejected_create["error"]["code"], -32602);

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
            "params": { "id": "00000000-0000-0000-0000-000000000001", "bytes_per_second": 1024 }
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

    websocket_sink.close().await?;

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
            "method": "download.list"
        }),
    )
    .await?;
    assert_eq!(
        restored["result"]["downloads"].as_array().map(Vec::len),
        Some(0)
    );
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
    let operator_remove = post_rpc(
        &client,
        address,
        json!({
            "jsonrpc": "2.0",
            "id": "operator-cancel",
            "method": "download.cancel",
            "params": { "id": "00000000-0000-0000-0000-000000000001" }
        }),
    )
    .await?;
    assert_eq!(operator_remove["error"]["code"], -32010);

    set_auth_role(&restart_config.database_path, Role::ReadOnly).await?;
    operator_socket.close(None).await?;
    let operator_remove = send_rpc(
        &client,
        address,
        &json!({
            "jsonrpc": "2.0",
            "id": "readonly-cancel",
            "method": "download.cancel",
            "params": { "id": "00000000-0000-0000-0000-000000000001" }
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

#[tokio::test]
async fn browser_rpc_cors_and_single_use_websocket_ticket_work()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let mut config = test_config(temp.path().to_path_buf());
    config
        .allowed_origins
        .push("http://localhost:5173".to_owned());
    let token = initialize_auth(&config.database_path).await?;
    let server = RemoteServer::start(config.clone()).await?;
    let address = server.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(server.run_until(async {
        let _ = shutdown_receiver.await;
    }));
    let client = authenticated_client(&token)?;

    let preflight = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{address}/rpc"))
        .header(reqwest::header::ORIGIN, "http://localhost:5173")
        .header(reqwest::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(
            reqwest::header::ACCESS_CONTROL_REQUEST_HEADERS,
            "authorization,content-type",
        )
        .send()
        .await?;
    assert_eq!(preflight.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        preflight
            .headers()
            .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("http://localhost:5173")
    );

    let denied_preflight = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{address}/rpc"))
        .header(reqwest::header::ORIGIN, "https://untrusted.example")
        .header(reqwest::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(
            reqwest::header::ACCESS_CONTROL_REQUEST_HEADERS,
            "authorization,content-type",
        )
        .send()
        .await?;
    assert_eq!(denied_preflight.status(), reqwest::StatusCode::FORBIDDEN);

    let ticket_response = client
        .post(format!("http://{address}/rpc"))
        .header(reqwest::header::ORIGIN, "http://localhost:5173")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "browser-ticket",
            "method": "system.websocketTicket"
        }))?)
        .send()
        .await?;
    assert!(ticket_response.status().is_success());
    assert_eq!(
        ticket_response
            .headers()
            .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("http://localhost:5173")
    );
    let ticket_body: Value = serde_json::from_slice(&ticket_response.bytes().await?)?;
    let ticket = ticket_body["result"]["ticket"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("WebSocket ticket is missing"))?;
    assert_eq!(ticket.len(), 69);
    assert_eq!(ticket_body["result"]["expires_in_seconds"], 30);

    let mut websocket_request =
        format!("ws://{address}/events?ticket={ticket}").into_client_request()?;
    websocket_request.headers_mut().insert(
        reqwest::header::ORIGIN,
        HeaderValue::from_static("http://localhost:5173"),
    );
    let (mut websocket, _) = connect_async(websocket_request).await?;
    websocket.close(None).await?;

    let mut replay_request =
        format!("ws://{address}/events?ticket={ticket}").into_client_request()?;
    replay_request.headers_mut().insert(
        reqwest::header::ORIGIN,
        HeaderValue::from_static("http://localhost:5173"),
    );
    let Err(tokio_tungstenite::tungstenite::Error::Http(replay_response)) =
        connect_async(replay_request).await
    else {
        return Err(std::io::Error::other("WebSocket ticket was replayable").into());
    };
    assert_eq!(replay_response.status(), reqwest::StatusCode::UNAUTHORIZED);

    let role_ticket_response = client
        .post(format!("http://{address}/rpc"))
        .header(reqwest::header::ORIGIN, "http://localhost:5173")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "browser-role-ticket",
            "method": "system.websocketTicket"
        }))?)
        .send()
        .await?;
    let role_ticket_body: Value = serde_json::from_slice(&role_ticket_response.bytes().await?)?;
    let role_ticket = role_ticket_body["result"]["ticket"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("role WebSocket ticket is missing"))?;
    set_auth_role(&config.database_path, Role::ReadOnly).await?;
    let mut role_request =
        format!("ws://{address}/events?ticket={role_ticket}").into_client_request()?;
    role_request.headers_mut().insert(
        reqwest::header::ORIGIN,
        HeaderValue::from_static("http://localhost:5173"),
    );
    let Err(tokio_tungstenite::tungstenite::Error::Http(role_response)) =
        connect_async(role_request).await
    else {
        return Err(std::io::Error::other("read-only role received event access").into());
    };
    assert_eq!(role_response.status(), reqwest::StatusCode::FORBIDDEN);

    let _ = shutdown_sender.send(());
    server_task.await??;
    Ok(())
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
        tls: None,
        allowed_origins: vec!["http://127.0.0.1".to_owned(), "http://localhost".to_owned()],
    }
}

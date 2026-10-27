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
use tokio_tungstenite::{connect_async, tungstenite::Message};

use fetchx_remote_server::{RemoteServer, ServerConfig};

#[tokio::test]
async fn remote_http_and_event_stream_use_core_download_lifecycle()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let config = test_config(temp.path().to_path_buf());
    let restart_config = config.clone();
    let downloads_dir = config.downloads_dir.clone();
    let server = RemoteServer::start(config).await?;
    let address = server.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(server.run_until(async {
        let _ = shutdown_receiver.await;
    }));

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/health"))
        .send()
        .await?;
    let health: Value = serde_json::from_slice(&response.bytes().await?)?;
    assert_eq!(health["status"], "ok");

    let oversized = vec![b' '; fetchx_remote_protocol::MAX_MESSAGE_BYTES + 1];
    let oversized_response = client
        .post(format!("http://{address}/rpc"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(oversized)
        .send()
        .await?;
    let oversized_error: Value = serde_json::from_slice(&oversized_response.bytes().await?)?;
    assert_eq!(oversized_error["error"]["code"], -32600);

    let (websocket, _) = connect_async(format!("ws://{address}/events")).await?;
    let (_websocket_sink, mut websocket_stream) = websocket.split();
    let (second_websocket, _) = connect_async(format!("ws://{address}/events")).await?;
    let (_second_websocket_sink, mut second_websocket_stream) = second_websocket.split();
    sleep(Duration::from_millis(50)).await;

    let expected_payload = [0x46, 0x58, 0x21, 0x42].repeat(32_768);
    let fixture_hash = Sha256::digest(&expected_payload);
    let fixture_address = start_http_fixture(expected_payload.clone()).await?;
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

    let _ = shutdown_sender.send(());
    server_task.await??;

    let restarted = RemoteServer::start(restart_config).await?;
    let restarted_address = restarted.local_addr();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(restarted.run_until(async {
        let _ = shutdown_receiver.await;
    }));
    let restored = post_rpc(
        &client,
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
    let response = client
        .post(format!("http://{address}/rpc"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&request)?)
        .send()
        .await?;
    Ok(serde_json::from_slice(&response.bytes().await?)?)
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

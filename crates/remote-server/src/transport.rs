use std::{future::Future, io, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_queue::DownloadEvent;
use fetchx_remote_protocol::{EventNotification, RpcErrorCode, RpcFailure, parse_request};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast, mpsc, oneshot, watch},
    time::{Instant, interval, timeout},
};
use tracing::{debug, warn};

use crate::{
    config::ServerConfig,
    dispatch::{RpcError, dispatch, success},
    events,
};

#[derive(Clone)]
struct AppState {
    services: Arc<CoreServices>,
    config: ServerConfig,
    connections: Arc<Semaphore>,
    shutdown: watch::Receiver<bool>,
}

pub struct RemoteServer {
    listener: TcpListener,
    address: std::net::SocketAddr,
    app: Router,
    services: Arc<CoreServices>,
    shutdown_sender: watch::Sender<bool>,
}

impl RemoteServer {
    pub async fn start(config: ServerConfig) -> Result<Self, ServerError> {
        config.validate()?;
        let listener = TcpListener::bind(config.socket_addr()).await?;
        let address = listener.local_addr()?;
        let services = Arc::new(
            CoreServices::start(CoreServicesConfig {
                database_path: config.database_path.clone(),
                downloads_dir: config.downloads_dir.clone(),
                runtime_dir: config.runtime_dir.clone(),
                bundled_engines_dir: None,
            })
            .await
            .map_err(|error| ServerError::Core(error.to_string()))?,
        );
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let state = AppState {
            services: Arc::clone(&services),
            connections: Arc::new(Semaphore::new(config.max_connections)),
            config,
            shutdown: shutdown_receiver,
        };
        let app = Router::new()
            .route("/health", get(health))
            .route("/rpc", post(rpc))
            .route("/events", get(websocket))
            .with_state(state);
        Ok(Self {
            listener,
            address,
            app,
            services,
            shutdown_sender,
        })
    }

    #[must_use]
    pub const fn local_addr(&self) -> std::net::SocketAddr {
        self.address
    }

    pub async fn run_until<F>(self, shutdown: F) -> Result<(), ServerError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let Self {
            listener,
            app,
            services,
            shutdown_sender,
            ..
        } = self;
        let shutdown_signal = async move {
            shutdown.await;
            let _ = shutdown_sender.send(true);
        };
        let server_result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await;
        let shutdown_result = services
            .shutdown()
            .await
            .map_err(|error| ServerError::Shutdown(error.to_string()));
        server_result?;
        shutdown_result
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    Config(#[from] crate::ServerConfigError),
    #[error("could not bind remote server: {0}")]
    Io(#[from] io::Error),
    #[error("could not start FetchX services: {0}")]
    Core(String),
    #[error("could not gracefully stop FetchX services: {0}")]
    Shutdown(String),
}

async fn health(State(state): State<AppState>) -> Response {
    let _permit = match Arc::clone(&state.connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    axum::Json(json!({
        "status": "ok",
        "protocol_version": fetchx_remote_protocol::PROTOCOL_VERSION,
    }))
    .into_response()
}

async fn rpc(State(state): State<AppState>, request: Request<Body>) -> Response {
    let deadline = Instant::now() + state.config.request_timeout;
    let permit = match Arc::clone(&state.connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return rpc_response(
                RpcError::new(RpcErrorCode::ServiceUnavailable, "server is at capacity")
                    .failure(None),
            );
        }
    };
    let body = match timeout(
        state.config.request_timeout,
        to_bytes(request.into_body(), state.config.max_request_bytes),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(error)) => {
            warn!(%error, "rejected remote JSON-RPC request body");
            drop(permit);
            return rpc_response(
                RpcError::new(
                    RpcErrorCode::InvalidRequest,
                    "invalid or oversized request body",
                )
                .failure(None),
            );
        }
        Err(_) => {
            drop(permit);
            return rpc_response(
                RpcError::new(RpcErrorCode::Timeout, "request body timed out").failure(None),
            );
        }
    };
    let request = match parse_request(&body) {
        Ok(request) => request,
        Err(error) => {
            debug!(%error, "rejected invalid remote JSON-RPC request");
            drop(permit);
            return rpc_response(RpcError::from_protocol(error).failure(None));
        }
    };
    let Some(id) = request.id.clone() else {
        let _permit = permit;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let result = timeout(remaining, dispatch(&state.services, request)).await;
        if result.is_err() {
            warn!("remote JSON-RPC notification timed out");
        }
        return StatusCode::NO_CONTENT.into_response();
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let result = timeout(remaining, dispatch(&state.services, request)).await;
    drop(permit);
    match result {
        Ok(Ok(result)) => {
            let success = success(id.clone(), result);
            match serde_json::to_value(success) {
                Ok(response) => axum::Json(response).into_response(),
                Err(_) => rpc_response(
                    RpcError::new(RpcErrorCode::InternalError, "internal error").failure(Some(id)),
                ),
            }
        }
        Ok(Err(error)) => rpc_response(error.failure(Some(id))),
        Err(_) => rpc_response(
            RpcError::new(RpcErrorCode::Timeout, "request timed out").failure(Some(id)),
        ),
    }
}

fn rpc_response(response: RpcFailure) -> Response {
    axum::Json(response).into_response()
}

async fn websocket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let permit = match Arc::clone(&state.connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let state_for_socket = state.clone();
    upgrade
        .max_message_size(state.config.max_request_bytes)
        .max_frame_size(state.config.max_request_bytes)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            serve_websocket(socket, state_for_socket).await;
        })
        .into_response()
}

async fn serve_websocket(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (event_sender, mut event_receiver) =
        mpsc::channel::<EventNotification>(state.config.event_backlog);
    let (overflow_sender, mut overflow_receiver) = oneshot::channel();
    let mut broadcast_receiver = state.services.event_bus.subscribe();
    let mut shutdown = state.shutdown.clone();
    if *shutdown.borrow() {
        return;
    }
    let event_task = tokio::spawn(async move {
        pump_events(&mut broadcast_receiver, event_sender, overflow_sender).await;
    });
    let mut heartbeat = interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            biased;
            overflow = &mut overflow_receiver => {
                if overflow.is_ok() {
                    debug!("closing remote WebSocket after event buffer overflow");
                }
                break;
            }
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    debug!("closing remote WebSocket during graceful shutdown");
                }
                break;
            }
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if !matches!(
                            timeout(state.config.request_timeout, sink.send(Message::Pong(payload))).await,
                            Ok(Ok(()))
                        ) {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {}
                }
            }
            event = event_receiver.recv() => {
                let Some(event) = event else { break };
                let value = match events::serialized(&event) {
                    Ok(value) => value,
                    Err(error) => {
                        warn!(%error, "could not serialize remote event notification");
                        break;
                    }
                };
                let message = Message::Text(value.to_string().into());
                if !matches!(
                    timeout(state.config.request_timeout, sink.send(message)).await,
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if !matches!(
                    timeout(state.config.request_timeout, sink.send(Message::Ping(Vec::new().into()))).await,
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
        }
    }
    event_task.abort();
}

async fn pump_events(
    receiver: &mut broadcast::Receiver<DownloadEvent>,
    sender: mpsc::Sender<EventNotification>,
    overflow: oneshot::Sender<()>,
) {
    let mut overflow = Some(overflow);
    loop {
        match receiver.recv().await {
            Ok(event) => {
                for notification in events::notifications(event) {
                    match sender.try_send(notification) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            if let Some(overflow) = overflow.take() {
                                let _ = overflow.send(());
                            }
                            return;
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => return,
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                debug!(skipped, "closing remote WebSocket after broadcast lag");
                if let Some(overflow) = overflow.take() {
                    let _ = overflow.send(());
                }
                return;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

    use super::{RemoteServer, ServerConfig};

    #[tokio::test]
    async fn server_binds_and_serves_health_on_loopback() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path().to_path_buf());
        let server = RemoteServer::start(config).await?;
        let address = server.local_addr();
        assert_eq!(address.ip(), Ipv4Addr::LOCALHOST);

        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let server_task = tokio::spawn(server.run_until(async {
            let _ = shutdown_receiver.await;
        }));
        let response = reqwest::get(format!("http://{address}/health")).await?;
        assert!(response.status().is_success());
        let body = response.bytes().await?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body)?["status"],
            "ok"
        );
        let _ = shutdown_sender.send(());
        server_task.await??;
        Ok(())
    }

    pub(super) fn test_config(root: PathBuf) -> ServerConfig {
        ServerConfig {
            bind_ip: Ipv4Addr::LOCALHOST.into(),
            port: 0,
            database_path: root.join("fetchx.db"),
            downloads_dir: root.join("downloads"),
            runtime_dir: root.join("runtime"),
            max_request_bytes: fetchx_remote_protocol::MAX_MESSAGE_BYTES,
            max_connections: 16,
            request_timeout: Duration::from_secs(5),
            event_backlog: 64,
        }
    }

    use tokio::sync::oneshot;
}

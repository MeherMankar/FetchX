use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderMap, Request, StatusCode,
        header::{AUTHORIZATION, ORIGIN},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fetchx_core_services::{CoreServices, CoreServicesConfig};
use fetchx_queue::DownloadEvent;
use fetchx_remote_protocol::{EventNotification, RpcErrorCode, RpcFailure, parse_request};
use futures_util::{SinkExt, StreamExt};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as ConnectionBuilder,
    service::TowerToHyperService,
};
use serde_json::json;
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Semaphore, broadcast, mpsc, oneshot, watch},
    time::{interval, timeout},
};
use tokio_rustls::{TlsAcceptor, rustls};
use tracing::{debug, warn};

use crate::{
    auth::{AuthError, AuthenticationService},
    authorization::{AuthenticatedPrincipal, Permission},
    config::ServerConfig,
    dispatch::{RpcError, dispatch, success},
    events,
};

#[derive(Clone)]
struct AppState {
    services: Arc<CoreServices>,
    config: ServerConfig,
    connections: Arc<Semaphore>,
    authentication: Arc<AuthenticationService>,
    shutdown: watch::Receiver<bool>,
    rate_limits: Arc<RateLimiter>,
}

pub struct RemoteServer {
    listener: TcpListener,
    address: std::net::SocketAddr,
    app: Router,
    services: Arc<CoreServices>,
    shutdown_sender: watch::Sender<bool>,
    tls: Option<Arc<rustls::ServerConfig>>,
    max_connections: usize,
}

impl RemoteServer {
    pub async fn start(config: ServerConfig) -> Result<Self, ServerError> {
        config.validate()?;
        let tls = config
            .tls
            .as_ref()
            .map(load_tls_config)
            .transpose()?
            .map(Arc::new);
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
        let authentication =
            match AuthenticationService::from_config(services.database.config()).await {
                Ok(authentication) => Arc::new(authentication),
                Err(error) => {
                    if let Err(shutdown_error) = services.shutdown().await {
                        warn!(%shutdown_error, "service shutdown failed after auth startup error");
                    }
                    return Err(ServerError::Authentication(error.to_string()));
                }
            };
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let state = AppState {
            services: Arc::clone(&services),
            connections: Arc::new(Semaphore::new(config.max_connections)),
            authentication,
            config: config.clone(),
            shutdown: shutdown_receiver,
            rate_limits: Arc::new(RateLimiter::new()),
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
            tls,
            max_connections: config.max_connections,
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
            tls,
            max_connections,
            ..
        } = self;
        let shutdown_signal = async move {
            shutdown.await;
            let _ = shutdown_sender.send(true);
        };
        let server_result = if let Some(tls) = tls {
            serve_tls(listener, app, tls, max_connections, shutdown_signal).await
        } else {
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal)
                .await
                .map_err(ServerError::Io)
        };
        let shutdown_result = services
            .shutdown()
            .await
            .map_err(|error| ServerError::Shutdown(error.to_string()));
        server_result?;
        shutdown_result
    }
}

fn load_tls_config(config: &crate::TlsConfig) -> Result<rustls::ServerConfig, ServerError> {
    use rustls::pki_types::{CertificateDer, pem::PemObject};

    let certificates = CertificateDer::pem_file_iter(&config.certificate_path)
        .map_err(|error| {
            ServerError::Tls(format!(
                "could not read TLS certificate file {}: {error}",
                config.certificate_path.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            ServerError::Tls(format!(
                "could not parse TLS certificate file {}: {error}",
                config.certificate_path.display()
            ))
        })?;
    if certificates.is_empty() {
        return Err(ServerError::Tls(format!(
            "TLS certificate file {} contains no certificates",
            config.certificate_path.display()
        )));
    }

    let private_key = rustls::pki_types::PrivateKeyDer::from_pem_file(&config.private_key_path)
        .map_err(|error| {
            ServerError::Tls(format!(
                "could not read TLS private key file {}: {error}",
                config.private_key_path.display()
            ))
        })?;
    rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| ServerError::Tls(format!("invalid TLS protocol configuration: {error}")))?
    .with_no_client_auth()
    .with_single_cert(certificates, private_key)
    .map_err(|error| ServerError::Tls(format!("invalid TLS certificate/key pair: {error}")))
}

async fn serve_tls<F>(
    listener: TcpListener,
    app: Router,
    tls: Arc<rustls::ServerConfig>,
    max_connections: usize,
    shutdown: F,
) -> Result<(), ServerError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let acceptor = TlsAcceptor::from(tls);
    let handshake_slots = Arc::new(Semaphore::new(max_connections));
    let mut connections = tokio::task::JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(slot) = Arc::clone(&handshake_slots).try_acquire_owned() else {
                    debug!("dropping TLS connection after reaching the connection limit");
                    drop(stream);
                    continue;
                };
                let acceptor = acceptor.clone();
                let app = app.clone();
                connections.spawn(async move {
                    let _slot = slot;
                    serve_tls_connection(stream, acceptor, app).await;
                });
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result {
                    debug!(%error, "TLS connection task failed");
                }
            }
        }
    }
    if tokio::time::timeout(Duration::from_secs(5), async {
        while connections.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        connections.abort_all();
    }
    Ok(())
}

async fn serve_tls_connection(stream: TcpStream, acceptor: TlsAcceptor, app: Router) {
    let stream = match acceptor.accept(stream).await {
        Ok(stream) => stream,
        Err(error) => {
            debug!(%error, "TLS handshake failed");
            return;
        }
    };
    let builder = ConnectionBuilder::new(TokioExecutor::new());
    if let Err(error) = builder
        .serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(app))
        .await
    {
        debug!(%error, "TLS HTTP connection failed");
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
    #[error("{0}")]
    Authentication(String),
    #[error("{0}")]
    Tls(String),
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
    if state.rate_limits.is_rate_limited("rpc") {
        return rate_limit_response();
    }
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
    let authorization = request.headers().get(AUTHORIZATION).cloned();
    let remaining = deadline.saturating_duration_since(Instant::now());
    let principal = match verify_authentication(&state, authorization.as_ref(), remaining).await {
        Ok(principal) => principal,
        Err(error) => {
            drop(permit);
            return authentication_error_response(error);
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let body = match timeout(
        remaining,
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
        let result = timeout(remaining, dispatch(&state.services, principal, request)).await;
        if result.is_err() {
            warn!("remote JSON-RPC notification timed out");
        }
        return StatusCode::NO_CONTENT.into_response();
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let result = timeout(remaining, dispatch(&state.services, principal, request)).await;
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
        Ok(Err(error)) if error.code() == RpcErrorCode::Forbidden => {
            rpc_response_with_status(error.failure(Some(id)), StatusCode::FORBIDDEN)
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

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if state.rate_limits.is_rate_limited("ws") {
        return rate_limit_response();
    }
    let permit = match Arc::clone(&state.connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    if let Some(origin) = origin {
        if !origin_allowed(&state.config, origin) {
            drop(permit);
            return forbidden_origin_response();
        }
    }
    let Some(authorization) = headers.get(AUTHORIZATION).cloned() else {
        drop(permit);
        return authentication_failure();
    };
    let principal =
        match verify_authentication(&state, Some(&authorization), state.config.request_timeout)
            .await
        {
            Ok(principal) => principal,
            Err(error) => {
                drop(permit);
                return authentication_error_response(error);
            }
        };
    if !principal.allows(Permission::EventsRead) {
        drop(permit);
        return authorization_failure();
    }
    let state_for_socket = state.clone();
    upgrade
        .max_message_size(state.config.max_request_bytes)
        .max_frame_size(state.config.max_request_bytes)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            serve_websocket(socket, state_for_socket, authorization).await;
        })
        .into_response()
}

async fn verify_authentication(
    state: &AppState,
    authorization: Option<&axum::http::HeaderValue>,
    timeout_budget: Duration,
) -> Result<AuthenticatedPrincipal, AuthenticationFailure> {
    match timeout(
        timeout_budget,
        state.authentication.authenticate_bearer(authorization),
    )
    .await
    {
        Ok(Ok(principal)) => Ok(principal),
        Ok(Err(AuthError::Unauthorized | AuthError::NotInitialized)) => {
            Err(AuthenticationFailure::Unauthorized)
        }
        Ok(Err(error)) => {
            warn!(%error, "authentication verifier lookup failed");
            Err(AuthenticationFailure::Unavailable)
        }
        Err(_) => Err(AuthenticationFailure::TimedOut),
    }
}

enum AuthenticationFailure {
    Unauthorized,
    Unavailable,
    TimedOut,
}

fn authentication_error_response(error: AuthenticationFailure) -> Response {
    match error {
        AuthenticationFailure::Unauthorized => authentication_failure(),
        AuthenticationFailure::Unavailable => rpc_response_with_status(
            RpcError::new(
                RpcErrorCode::ServiceUnavailable,
                "authentication unavailable",
            )
            .failure(None),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        AuthenticationFailure::TimedOut => rpc_response_with_status(
            RpcError::new(RpcErrorCode::Timeout, "authentication timed out").failure(None),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    }
}

fn authentication_failure() -> Response {
    rpc_response_with_status(
        RpcError::new(RpcErrorCode::Unauthorized, "authentication required").failure(None),
        StatusCode::UNAUTHORIZED,
    )
}

fn authorization_failure() -> Response {
    rpc_response_with_status(
        RpcError::new(RpcErrorCode::Forbidden, "permission denied").failure(None),
        StatusCode::FORBIDDEN,
    )
}

fn forbidden_origin_response() -> Response {
    rpc_response_with_status(
        RpcError::new(RpcErrorCode::Forbidden, "origin not allowed").failure(None),
        StatusCode::FORBIDDEN,
    )
}

fn rate_limit_response() -> Response {
    rpc_response_with_status(
        RpcError::new(RpcErrorCode::ServiceUnavailable, "rate limit exceeded").failure(None),
        StatusCode::TOO_MANY_REQUESTS,
    )
}

fn rpc_response_with_status(response: RpcFailure, status: StatusCode) -> Response {
    let mut response = rpc_response(response);
    *response.status_mut() = status;
    response
}

fn origin_allowed(config: &ServerConfig, origin: &str) -> bool {
    if config.allowed_origins.is_empty() {
        return false;
    }
    if let Ok(parsed) = url::Url::parse(origin) {
        let origin = parsed.origin().unicode_serialization();
        return config.allowed_origins.iter().any(|candidate| {
            let Ok(candidate_url) = url::Url::parse(candidate) else {
                return false;
            };
            candidate_url.origin().unicode_serialization() == origin
        });
    }
    false
}

#[derive(Clone, Default)]
struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self::default()
    }

    fn is_rate_limited(&self, bucket: &str) -> bool {
        let now = std::time::Instant::now();
        let mut buckets = self.buckets.lock().unwrap();
        let queue = buckets.entry(bucket.to_owned()).or_default();
        while queue
            .front()
            .is_some_and(|ts| now.duration_since(*ts) > Duration::from_secs(60))
        {
            queue.pop_front();
        }
        queue.push_back(now);
        if queue.len() > 120 {
            return true;
        }
        false
    }
}

async fn serve_websocket(
    socket: WebSocket,
    state: AppState,
    authorization: axum::http::HeaderValue,
) {
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
                let principal = match timeout(
                    state.config.request_timeout,
                    state.authentication.authenticate_bearer(Some(&authorization)),
                )
                .await
                {
                    Ok(Ok(principal)) => principal,
                    _ => {
                        debug!("closing remote WebSocket after authentication was revoked");
                        break;
                    }
                };
                if !principal.allows(Permission::EventsRead) {
                    debug!("closing remote WebSocket after event permission was revoked");
                    break;
                }
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

    use super::{RemoteServer, ServerConfig, ServerError, origin_allowed};
    use crate::initialize_auth;

    #[tokio::test]
    async fn server_refuses_startup_until_authentication_is_initialized()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path().to_path_buf());
        let error = match RemoteServer::start(config).await {
            Ok(_) => {
                return Err(std::io::Error::other("server started without authentication").into());
            }
            Err(error) => error,
        };
        assert!(matches!(error, ServerError::Authentication(_)));
        Ok(())
    }

    #[tokio::test]
    async fn server_binds_and_serves_health_on_loopback() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path().to_path_buf());
        initialize_auth(&config.database_path).await?;
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

    #[tokio::test]
    async fn tls_startup_rejects_missing_certificate_and_key_without_plaintext_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let missing_certificate = ServerConfig {
            tls: Some(crate::TlsConfig {
                certificate_path: temp.path().join("missing-cert.pem"),
                private_key_path: temp.path().join("missing-key.pem"),
            }),
            ..test_config(temp.path().to_path_buf())
        };
        let error = match RemoteServer::start(missing_certificate).await {
            Ok(_) => return Err(std::io::Error::other("server started without TLS files").into()),
            Err(error) => error,
        };
        assert!(matches!(error, ServerError::Tls(_)));

        let certificate_path = temp.path().join("invalid-cert.pem");
        let private_key_path = temp.path().join("invalid-key.pem");
        tokio::fs::write(&certificate_path, b"not a PEM certificate").await?;
        tokio::fs::write(&private_key_path, b"not a PEM private key").await?;
        let invalid_pair = ServerConfig {
            tls: Some(crate::TlsConfig {
                certificate_path,
                private_key_path,
            }),
            ..test_config(temp.path().to_path_buf())
        };
        let error = match RemoteServer::start(invalid_pair).await {
            Ok(_) => return Err(std::io::Error::other("server accepted invalid TLS files").into()),
            Err(error) => error,
        };
        assert!(matches!(error, ServerError::Tls(_)));
        Ok(())
    }

    #[test]
    fn origin_validation_accepts_loopback_clients_only() {
        let config = ServerConfig {
            allowed_origins: vec!["http://127.0.0.1".to_owned(), "http://localhost".to_owned()],
            ..ServerConfig::default()
        };
        assert!(origin_allowed(&config, "http://127.0.0.1"));
        assert!(origin_allowed(&config, "http://localhost"));
        assert!(!origin_allowed(&config, "https://example.com"));
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
            tls: None,
            allowed_origins: vec!["http://127.0.0.1".to_owned(), "http://localhost".to_owned()],
        }
    }

    use tokio::sync::oneshot;
}

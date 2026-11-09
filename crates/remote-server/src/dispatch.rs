use std::path::PathBuf;

use fetchx_core_services::{
    CoreServices, CoreServicesError, EngineRuntimeState as CoreEngineRuntimeState,
};
use fetchx_downloader::{save_initial_context_headers, validate_http_source_url};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadRuntimeStatus, DownloadSource, EngineKind,
    FileName, Timestamp,
};
use fetchx_queue::QueueError;
use fetchx_remote_protocol::{
    CAPABILITIES, DOWNLOAD_CANCEL, DOWNLOAD_CREATE, DOWNLOAD_FIND_DUPLICATE, DOWNLOAD_GET,
    DOWNLOAD_LIST, DOWNLOAD_PAUSE, DOWNLOAD_REMOVE, DOWNLOAD_RESUME, DOWNLOAD_RUNTIME_STATUS,
    DOWNLOAD_SET_ENGINE, DOWNLOAD_SET_PRIORITY, DOWNLOAD_SET_SPEED_LIMIT, DownloadCreateParams,
    ENGINE_RUNTIME_STATUS, EngineRuntimeState, EngineRuntimeStatusEntry, EngineRuntimeStatusResult,
    JSON_RPC_VERSION, PROTOCOL_VERSION, ProtocolError, ProtocolInfo, QUEUE_MOVE_DOWN,
    QUEUE_MOVE_UP, QUEUE_PAUSE_DISPATCH, QUEUE_RESUME_DISPATCH, RemoteCommand, RpcErrorCode,
    RpcFailure, RpcId, RpcSuccess, SYSTEM_WEBSOCKET_TICKET, ValidatedRequest,
};
use fetchx_storage::{DownloadRepository, StorageError};
use serde_json::{Value, json};
use tracing::warn;

use crate::authorization::{AuthenticatedPrincipal, required_permission};
use crate::tickets::WebSocketTickets;

#[derive(Clone, Copy)]
pub(crate) struct RpcError {
    code: RpcErrorCode,
    message: &'static str,
}

impl RpcError {
    pub(crate) const fn new(code: RpcErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    pub(crate) const fn code(self) -> RpcErrorCode {
        self.code
    }

    pub(crate) fn from_protocol(error: ProtocolError) -> Self {
        match error.code() {
            RpcErrorCode::ParseError => Self::new(RpcErrorCode::ParseError, "parse error"),
            RpcErrorCode::InvalidRequest => {
                Self::new(RpcErrorCode::InvalidRequest, "invalid request")
            }
            RpcErrorCode::MethodNotFound => {
                Self::new(RpcErrorCode::MethodNotFound, "method not found")
            }
            RpcErrorCode::InvalidParams => Self::new(RpcErrorCode::InvalidParams, "invalid params"),
            _ => Self::new(RpcErrorCode::InternalError, "internal error"),
        }
    }

    pub(crate) fn failure(self, id: Option<RpcId>) -> RpcFailure {
        RpcFailure {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            id,
            error: fetchx_remote_protocol::RpcErrorObject {
                code: self.code as i32,
                message: self.message.to_owned(),
                data: None,
            },
        }
    }
}

pub(crate) async fn dispatch(
    services: &CoreServices,
    principal: AuthenticatedPrincipal,
    request: ValidatedRequest,
    websocket_tickets: &WebSocketTickets,
    credential_verifier: [u8; 32],
) -> Result<Value, RpcError> {
    let permission = required_permission(&request.command);
    if !principal.allows(permission) {
        tracing::debug!(
            principal = principal.identity(),
            role = ?principal.role(),
            "remote RPC authorization denied"
        );
        return Err(RpcError::new(RpcErrorCode::Forbidden, "permission denied"));
    }
    if matches!(&request.command, RemoteCommand::WebSocketTicket) && request.id.is_none() {
        return Err(RpcError::new(
            RpcErrorCode::InvalidRequest,
            "WebSocket ticket requires a request id",
        ));
    }
    let result = match request.command {
        RemoteCommand::Discover => serde_json::to_value(discovery_info())
            .map_err(|_| RpcError::new(RpcErrorCode::InternalError, "internal error"))?,
        RemoteCommand::WebSocketTicket => {
            let ticket = websocket_tickets.issue(credential_verifier).map_err(|_| {
                RpcError::new(
                    RpcErrorCode::ServiceUnavailable,
                    "WebSocket ticket service unavailable",
                )
            })?;
            json!({ "ticket": ticket, "expires_in_seconds": 30 })
        }
        RemoteCommand::EngineRuntimeStatus => {
            let engines = services
                .engine_runtime_statuses()
                .into_iter()
                .map(|entry| EngineRuntimeStatusEntry {
                    engine: entry.engine,
                    state: match entry.state {
                        CoreEngineRuntimeState::Available => EngineRuntimeState::Available,
                        CoreEngineRuntimeState::Disabled => EngineRuntimeState::Disabled,
                        CoreEngineRuntimeState::Unavailable => EngineRuntimeState::Unavailable,
                    },
                    version: entry.version,
                })
                .collect();
            serde_json::to_value(EngineRuntimeStatusResult { engines })
                .map_err(|_| RpcError::new(RpcErrorCode::InternalError, "internal error"))?
        }
        RemoteCommand::DownloadCreate(params) => {
            let priority = params.priority.unwrap_or_default();
            let id = if params.url.starts_with("magnet:") {
                let selected_engine = match params.engine {
                    Some(engine @ (EngineKind::Aria2 | EngineKind::NativeTorrent))
                        if services.engine_runtime.is_available(engine) =>
                    {
                        Some(engine)
                    }
                    Some(_) => None,
                    None if services.engine_runtime.is_available(EngineKind::Aria2) => {
                        Some(EngineKind::Aria2)
                    }
                    None if services
                        .engine_runtime
                        .is_available(EngineKind::NativeTorrent) =>
                    {
                        Some(EngineKind::NativeTorrent)
                    }
                    None => None,
                };
                let Some(selected_engine) = selected_engine else {
                    return Err(RpcError::new(
                        RpcErrorCode::UnsupportedOperation,
                        "requested torrent engine is not available",
                    ));
                };
                services
                    .enqueue_magnet_with_engine(&params.url, selected_engine, priority, None)
                    .await
                    .map_err(map_core_error)?
            } else {
                create_http_download(services, &params, priority).await?
            };
            json!({ "id": id })
        }
        RemoteCommand::DownloadGet(id) => {
            let item = services
                .repository
                .get(id)
                .await
                .map_err(map_storage_error)?
                .ok_or_else(not_found)?;
            safe_download(&item)
        }
        RemoteCommand::DownloadList => {
            let items = services
                .repository
                .list()
                .await
                .map_err(map_storage_error)?;
            let items = items.iter().map(safe_download).collect::<Vec<_>>();
            json!({ "downloads": items })
        }
        RemoteCommand::DownloadRuntimeStatus => {
            let statuses = services
                .download_runtime_statuses()
                .await
                .map_err(map_storage_error)?;
            let statuses = statuses.iter().map(safe_runtime_status).collect::<Vec<_>>();
            json!({ "statuses": statuses })
        }
        RemoteCommand::DownloadFindDuplicate(params) => {
            let duplicate = services
                .find_duplicate_download(&params.url)
                .await
                .map_err(map_storage_error)?;
            serde_json::to_value(duplicate)
                .map_err(|_| RpcError::new(RpcErrorCode::InternalError, "internal error"))?
        }
        RemoteCommand::DownloadPause(id) => {
            queue_command(services.queue().pause(id).await)?;
            json!({ "id": id })
        }
        RemoteCommand::DownloadResume(id) => {
            queue_command(services.queue().resume(id).await)?;
            json!({ "id": id })
        }
        RemoteCommand::DownloadCancel(id) => {
            queue_command(services.queue().cancel(id).await)?;
            json!({ "id": id })
        }
        RemoteCommand::DownloadRemove(id) => {
            queue_command(services.queue().remove_download(id).await)?;
            json!({ "id": id, "removed": true })
        }
        RemoteCommand::DownloadSetPriority { id, priority } => {
            queue_command(services.queue().set_priority(id, priority).await)?;
            json!({ "id": id, "priority": priority })
        }
        RemoteCommand::QueueMoveUp(id) => {
            queue_command(services.queue().move_up(id).await)?;
            json!({ "id": id })
        }
        RemoteCommand::QueueMoveDown(id) => {
            queue_command(services.queue().move_down(id).await)?;
            json!({ "id": id })
        }
        RemoteCommand::QueuePauseDispatch => {
            services
                .queue()
                .pause_dispatch()
                .await
                .map_err(map_queue_error)?;
            json!({ "dispatch_paused": true })
        }
        RemoteCommand::QueueResumeDispatch => {
            services
                .queue()
                .resume_dispatch()
                .await
                .map_err(map_queue_error)?;
            json!({ "dispatch_paused": false })
        }
        RemoteCommand::DownloadRetry(_) => {
            return Err(unsupported("download.retry"));
        }
        RemoteCommand::DownloadSetEngine { .. } => {
            return Err(unsupported(DOWNLOAD_SET_ENGINE));
        }
        RemoteCommand::DownloadSetSpeedLimit { .. } => {
            return Err(unsupported(DOWNLOAD_SET_SPEED_LIMIT));
        }
    };
    Ok(result)
}

pub(crate) fn success(id: RpcId, result: Value) -> RpcSuccess {
    RpcSuccess {
        jsonrpc: JSON_RPC_VERSION.to_owned(),
        id,
        result,
    }
}

pub(crate) fn discovery_info() -> ProtocolInfo {
    // Keep this list aligned with dispatch and events actually exposed by the server.
    let supported = [
        "system.discover",
        SYSTEM_WEBSOCKET_TICKET,
        ENGINE_RUNTIME_STATUS,
        DOWNLOAD_CREATE,
        DOWNLOAD_GET,
        DOWNLOAD_LIST,
        DOWNLOAD_RUNTIME_STATUS,
        DOWNLOAD_FIND_DUPLICATE,
        DOWNLOAD_PAUSE,
        DOWNLOAD_RESUME,
        DOWNLOAD_CANCEL,
        DOWNLOAD_REMOVE,
        DOWNLOAD_SET_PRIORITY,
        QUEUE_MOVE_UP,
        QUEUE_MOVE_DOWN,
        QUEUE_PAUSE_DISPATCH,
        QUEUE_RESUME_DISPATCH,
        "download.created",
        "download.updated",
        "download.progress",
        "download.paused",
        "download.completed",
        "download.failed",
        "queue.updated",
    ];
    let capabilities = CAPABILITIES
        .iter()
        .filter(|capability| supported.contains(capability))
        .map(|capability| (*capability).to_owned())
        .collect();
    ProtocolInfo {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        server_version: env!("CARGO_PKG_VERSION").to_owned(),
        capabilities,
    }
}

fn safe_download(item: &DownloadItem) -> Value {
    let progress = item.progress();
    json!({
        "id": item.id(),
        "filename": item.filename().as_str(),
        "status": item.status(),
        "priority": item.priority(),
        "engine": item.source().engine(),
        "downloaded_bytes": progress.downloaded_bytes(),
        "total_bytes": progress.total_bytes(),
        "speed_bytes_per_second": progress.speed_bytes_per_second(),
        "estimated_remaining_seconds": progress.estimated_remaining_seconds(),
        "progress_basis_points": progress.percentage_basis_points(),
        "error_code": item.error().map(|error| error.code()),
        "created_at": item.created_at(),
        "updated_at": item.updated_at(),
    })
}

fn safe_runtime_status(status: &DownloadRuntimeStatus) -> Value {
    let progress = status.progress();
    json!({
        "id": status.id(),
        "status": status.status(),
        "progress": {
            "downloaded_bytes": progress.downloaded_bytes(),
            "total_bytes": progress.total_bytes(),
            "speed_bytes_per_second": progress.speed_bytes_per_second(),
            "estimated_remaining_seconds": progress.estimated_remaining_seconds(),
            "percentage_basis_points": progress.percentage_basis_points(),
        },
        "updated_at": status.updated_at().as_unix_millis(),
    })
}

pub(crate) fn safe_download_event(item: &DownloadItem, revision: u64) -> Value {
    json!({ "revision": revision, "download": safe_download(item) })
}

async fn create_http_download(
    services: &CoreServices,
    params: &DownloadCreateParams,
    priority: DownloadPriority,
) -> Result<DownloadId, RpcError> {
    if params
        .engine
        .is_some_and(|engine| engine != EngineKind::DirectHttp)
    {
        return Err(RpcError::new(
            RpcErrorCode::UnsupportedOperation,
            "only direct_http is supported for remote HTTP downloads",
        ));
    }
    let validated_url = validate_http_source_url(&params.url)
        .map_err(|_| RpcError::new(RpcErrorCode::InvalidParams, "invalid HTTP(S) URL"))?;
    let url = url::Url::parse(&validated_url)
        .map_err(|_| RpcError::new(RpcErrorCode::InvalidParams, "invalid HTTP(S) URL"))?;
    let filename = safe_filename(
        params
            .filename
            .as_deref()
            .filter(|filename| !filename.trim().is_empty())
            .or_else(|| url.path_segments().and_then(Iterator::last)),
    );
    let id = DownloadId::new();
    let source = DownloadSource::new(validated_url.clone(), EngineKind::DirectHttp)
        .map_err(|_| RpcError::new(RpcErrorCode::InvalidParams, "invalid HTTP(S) URL"))?;
    let filename = FileName::new(filename)
        .map_err(|_| RpcError::new(RpcErrorCode::InvalidParams, "invalid filename"))?;
    let destination: PathBuf = services
        .downloads_dir()
        .join(format!("{id}-{}", filename.as_str()));
    let mut context_headers = Vec::new();
    if let Some(ref r) = params.referer {
        context_headers.push(("Referer".to_string(), r.clone()));
    }
    if let Some(ref ua) = params.user_agent {
        context_headers.push(("User-Agent".to_string(), ua.clone()));
    }
    if let Some(ref c) = params.cookies {
        context_headers.push(("Cookie".to_string(), c.clone()));
    }
    if !context_headers.is_empty() {
        let _ = save_initial_context_headers(&destination, &validated_url, context_headers).await;
    }
    let created_at = Timestamp::now()
        .map_err(|_| RpcError::new(RpcErrorCode::InternalError, "internal error"))?;
    let mut item = DownloadItem::new(id, source, filename, destination, created_at);
    item.set_priority_at(priority, created_at)
        .map_err(|_| RpcError::new(RpcErrorCode::InternalError, "internal error"))?;
    services
        .queue()
        .enqueue(item)
        .await
        .map_err(map_queue_error)?;
    Ok(id)
}

fn safe_filename(candidate: Option<&str>) -> String {
    let candidate = candidate.unwrap_or("download");
    let candidate = candidate.rsplit(['/', '\\']).next().unwrap_or(candidate);
    let mut name = candidate
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect::<String>();
    while name.starts_with('.') {
        name.remove(0);
    }
    if name.is_empty() || name == ".." {
        "download".to_owned()
    } else {
        name
    }
}

fn queue_command(result: Result<(), QueueError>) -> Result<(), RpcError> {
    result.map_err(map_queue_error)
}

fn map_queue_error(error: QueueError) -> RpcError {
    match error {
        QueueError::DownloadNotFound(_) => not_found(),
        QueueError::InvalidStatus { .. } => RpcError::new(
            RpcErrorCode::Conflict,
            "operation conflicts with download state",
        ),
        QueueError::UnsupportedEngineOperation { .. } => RpcError::new(
            RpcErrorCode::UnsupportedOperation,
            "operation is unsupported",
        ),
        other => {
            warn!(%other, "remote queue operation failed");
            RpcError::new(RpcErrorCode::ServiceUnavailable, "queue operation failed")
        }
    }
}

fn map_storage_error(error: StorageError) -> RpcError {
    warn!(%error, "remote download query failed");
    RpcError::new(RpcErrorCode::InternalError, "download query failed")
}

fn map_core_error(error: CoreServicesError) -> RpcError {
    match error {
        CoreServicesError::UnavailableEngine(_) => RpcError::new(
            RpcErrorCode::ServiceUnavailable,
            "torrent engine unavailable",
        ),
        CoreServicesError::InvalidMagnet(_) | CoreServicesError::InvalidTorrent(_) => {
            RpcError::new(RpcErrorCode::InvalidParams, "invalid torrent request")
        }
        _ => {
            warn!(%error, "remote download creation failed");
            RpcError::new(
                RpcErrorCode::ServiceUnavailable,
                "download could not be created",
            )
        }
    }
}

fn not_found() -> RpcError {
    RpcError::new(RpcErrorCode::NotFound, "download not found")
}

fn unsupported(method: &'static str) -> RpcError {
    tracing::debug!(method, "remote operation is not supported by CoreServices");
    RpcError::new(
        RpcErrorCode::UnsupportedOperation,
        "operation is not supported by this server",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fetchx_models::{DownloadProgress, DownloadStatus};

    #[test]
    fn runtime_status_rpc_shape_includes_shared_progress_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        let item = DownloadItem::from_parts(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file.bin", EngineKind::DirectHttp)?,
            FileName::new("file.bin")?,
            PathBuf::from("file.bin"),
            DownloadStatus::Downloading,
            DownloadPriority::default(),
            DownloadProgress::new(50, Some(100), Some(25), Some(2))?,
            Timestamp::from_unix_millis(10),
            Timestamp::from_unix_millis(20),
            None,
        )?;

        let payload = safe_runtime_status(&DownloadRuntimeStatus::from(&item));

        assert_eq!(payload["status"], "downloading");
        assert_eq!(payload["progress"]["downloaded_bytes"], 50);
        assert_eq!(payload["progress"]["total_bytes"], 100);
        assert_eq!(payload["progress"]["speed_bytes_per_second"], 25);
        assert_eq!(payload["progress"]["estimated_remaining_seconds"], 2);
        assert_eq!(payload["progress"]["percentage_basis_points"], 5_000);
        assert_eq!(payload["updated_at"], 20);
        Ok(())
    }

    #[test]
    fn browser_filename_is_reduced_to_a_safe_basename() {
        assert_eq!(
            safe_filename(Some(r"..\private\report final?.zip")),
            "report_final_.zip"
        );
        assert_eq!(safe_filename(Some("../../secret.txt")), "secret.txt");
        assert_eq!(safe_filename(Some(r"..\..\secret.txt")), "secret.txt");
        assert_eq!(safe_filename(Some(r"C:\Windows\file.txt")), "file.txt");
        assert_eq!(safe_filename(Some("report?.txt")), "report_.txt");
        assert_eq!(safe_filename(Some("../../")), "download");
    }
}

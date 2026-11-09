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
    ARIA2_RECOVERY_ASSESSMENT_VERSION, Aria2RecoveryAssessmentResult,
    Aria2RecoveryState as ProtocolAria2RecoveryState, CAPABILITIES, DOWNLOAD_ARIA2_RECOVERY,
    DOWNLOAD_CANCEL, DOWNLOAD_CREATE, DOWNLOAD_FIND_DUPLICATE, DOWNLOAD_GET, DOWNLOAD_LIST,
    DOWNLOAD_PAUSE, DOWNLOAD_REMOVE, DOWNLOAD_RESUME, DOWNLOAD_RUNTIME_STATUS,
    DOWNLOAD_SELECT_TORRENT_FILES, DOWNLOAD_SET_ENGINE, DOWNLOAD_SET_PRIORITY,
    DOWNLOAD_SET_SPEED_LIMIT, DOWNLOAD_TORRENT_METADATA, DownloadCreateParams,
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

#[derive(Clone, Copy, Debug)]
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
        RemoteCommand::DownloadAria2Recovery(id) => {
            let assessment = services
                .assess_aria2_recovery(id)
                .await
                .map_err(|_| {
                    RpcError::new(
                        RpcErrorCode::ServiceUnavailable,
                        "aria2 recovery assessment unavailable",
                    )
                })?
                .ok_or_else(not_found)?;
            let result = Aria2RecoveryAssessmentResult {
                assessment_version: ARIA2_RECOVERY_ASSESSMENT_VERSION,
                download_id: assessment.download_id,
                state: protocol_recovery_state(assessment.state),
                explanation: assessment.explanation.to_owned(),
                next_step: assessment.next_step.to_owned(),
            };
            serde_json::to_value(result)
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
        RemoteCommand::DownloadTorrentMetadata(id) => {
            let metadata = services
                .native_magnet_metadata(id)
                .await
                .map_err(map_torrent_metadata_error)?
                .ok_or_else(not_found)?;
            json!({
                "id": id,
                "metadata_state": metadata.metadata_state,
                "info_hash": metadata.info_hash,
                "name": metadata.name,
                "files": metadata.files.iter().map(|file| json!({
                    "index": file.index,
                    "path": file.path,
                    "size": file.size,
                })).collect::<Vec<_>>(),
                "selected_file_indexes": metadata.selected_file_indexes,
                "selected_total_bytes": metadata.selected_total_bytes,
                "selection_confirmed": metadata.selection_confirmed,
            })
        }
        RemoteCommand::DownloadSelectTorrentFiles {
            id,
            info_hash,
            selected_file_indexes,
        } => {
            services
                .select_native_magnet_files(id, &info_hash, selected_file_indexes)
                .await
                .map_err(map_torrent_metadata_error)?;
            json!({ "id": id, "selection_confirmed": true })
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
        DOWNLOAD_TORRENT_METADATA,
        DOWNLOAD_SELECT_TORRENT_FILES,
        DOWNLOAD_ARIA2_RECOVERY,
        DOWNLOAD_RUNTIME_STATUS,
        DOWNLOAD_FIND_DUPLICATE,
        DOWNLOAD_ARIA2_RECOVERY,
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

fn protocol_recovery_state(
    state: fetchx_core_services::Aria2RecoveryState,
) -> ProtocolAria2RecoveryState {
    match state {
        fetchx_core_services::Aria2RecoveryState::NotAria2 => ProtocolAria2RecoveryState::NotAria2,
        fetchx_core_services::Aria2RecoveryState::NotRequired => {
            ProtocolAria2RecoveryState::NotRequired
        }
        fetchx_core_services::Aria2RecoveryState::RuntimeUnavailable => {
            ProtocolAria2RecoveryState::RuntimeUnavailable
        }
        fetchx_core_services::Aria2RecoveryState::ReadyToStart => {
            ProtocolAria2RecoveryState::ReadyToStart
        }
        fetchx_core_services::Aria2RecoveryState::JobAvailable => {
            ProtocolAria2RecoveryState::JobAvailable
        }
        fetchx_core_services::Aria2RecoveryState::MissingTorrentMetadata => {
            ProtocolAria2RecoveryState::MissingTorrentMetadata
        }
        fetchx_core_services::Aria2RecoveryState::MissingGid => {
            ProtocolAria2RecoveryState::MissingGid
        }
        fetchx_core_services::Aria2RecoveryState::StaleGid => ProtocolAria2RecoveryState::StaleGid,
        fetchx_core_services::Aria2RecoveryState::MissingControlFile => {
            ProtocolAria2RecoveryState::MissingControlFile
        }
        fetchx_core_services::Aria2RecoveryState::ControlFilePresent => {
            ProtocolAria2RecoveryState::ControlFilePresent
        }
        fetchx_core_services::Aria2RecoveryState::Unknown => ProtocolAria2RecoveryState::Unknown,
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

fn map_torrent_metadata_error(error: CoreServicesError) -> RpcError {
    match error {
        CoreServicesError::TorrentDownloadNotFound => not_found(),
        CoreServicesError::StaleTorrentMetadata | CoreServicesError::TorrentSelectionConflict => {
            RpcError::new(
                RpcErrorCode::Conflict,
                "torrent metadata or selection state changed; refresh and try again",
            )
        }
        CoreServicesError::InvalidTorrent(_) => RpcError::new(
            RpcErrorCode::InvalidParams,
            "invalid torrent metadata request",
        ),
        other => {
            warn!(%other, "remote torrent metadata operation failed");
            RpcError::new(
                RpcErrorCode::ServiceUnavailable,
                "torrent metadata operation failed",
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
    use crate::authorization::{AuthenticatedPrincipal, Role};
    use fetchx_core_services::{CoreServices, CoreServicesConfig, EngineConfig};
    use fetchx_models::{
        DownloadProgress, DownloadSource, DownloadStatus, FileName, TorrentFile, TorrentMetadata,
    };
    use fetchx_remote_protocol::{DOWNLOAD_SELECT_TORRENT_FILES, DOWNLOAD_TORRENT_METADATA};
    use fetchx_storage::{
        DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow, TorrentSeedState,
        TorrentSourceType,
    };
    use std::path::PathBuf;

    #[tokio::test]
    async fn native_magnet_rpc_is_authorized_sanitized_and_persists_selection()
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
        let id = DownloadId::new();
        let now = Timestamp::now()?;
        let info_hash = "0123456789abcdef0123456789abcdef01234567";
        let magnet = format!("magnet:?xt=urn:btih:{info_hash}&dn=bundle");
        let item = DownloadItem::from_parts(
            id,
            DownloadSource::new(&magnet, EngineKind::NativeTorrent)?,
            FileName::new("bundle")?,
            directory.path().join("bundle"),
            DownloadStatus::MetadataAcquiring,
            DownloadPriority::default(),
            DownloadProgress::new(0, None, None, None)?,
            now,
            now,
            None,
        )?;
        services.repository.create(&item).await?;
        let metadata = TorrentMetadata {
            info_hash: info_hash.to_owned(),
            name: "bundle".to_owned(),
            total_size: 10,
            files: vec![TorrentFile {
                path: vec!["bundle".to_owned(), "selected.bin".to_owned()],
                size: 10,
            }],
            trackers: vec!["https://secret.test/announce?token=hidden".to_owned()],
            piece_length: 16_384,
            piece_count: 1,
        };
        services
            .database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::Magnet,
                magnet_uri: Some(magnet),
                local_torrent_path: Some(directory.path().join("bundle.torrent")),
                info_hash: info_hash.to_owned(),
                torrent_name: metadata.name.clone(),
                torrent_metadata: Some(metadata),
                destination: item.destination().to_path_buf(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: Vec::new(),
                selected_total_bytes: 0,
                selection_confirmed: false,
                aria2_gid: None,
                seed_state: TorrentSeedState::Downloading,
                uploaded_bytes: 0,
                upload_speed_bps: 0,
                peer_count: 0,
                seed_count: 0,
                created_at: now,
                updated_at: now,
            })
            .await?;

        let discovery = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::Operator),
            fetchx_remote_protocol::parse_request(
                br#"{"jsonrpc":"2.0","id":1,"method":"system.discover"}"#,
            )?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect("discovery should succeed");
        assert!(
            discovery["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|cap| cap == DOWNLOAD_TORRENT_METADATA))
        );
        assert!(
            discovery["capabilities"].as_array().is_some_and(|caps| {
                caps.iter().any(|cap| cap == DOWNLOAD_SELECT_TORRENT_FILES)
            })
        );

        let metadata_request = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"{DOWNLOAD_TORRENT_METADATA}","params":{{"id":"{id}"}}}}"#
        );
        let result = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::ReadOnly),
            fetchx_remote_protocol::parse_request(metadata_request.as_bytes())?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect("metadata read should succeed");
        assert_eq!(result["metadata_state"], "ready");
        assert_eq!(result["files"][0]["index"], 0);
        assert!(result["files"][0]["path"][1].as_str().is_some());
        assert!(!result.to_string().contains("secret.test"));
        assert!(!result.to_string().contains("token=hidden"));

        let select_request = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"{DOWNLOAD_SELECT_TORRENT_FILES}","params":{{"id":"{id}","info_hash":"{info_hash}","selected_file_indexes":[0]}}}}"#
        );
        let denied = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::ReadOnly),
            fetchx_remote_protocol::parse_request(select_request.as_bytes())?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect_err("read-only role must not confirm file selections");
        assert_eq!(denied.code(), RpcErrorCode::Forbidden);

        let empty_request = format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"{DOWNLOAD_SELECT_TORRENT_FILES}","params":{{"id":"{id}","info_hash":"{info_hash}","selected_file_indexes":[]}}}}"#
        );
        let invalid = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::Operator),
            fetchx_remote_protocol::parse_request(empty_request.as_bytes())?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect_err("empty selection must be rejected");
        assert_eq!(invalid.code(), RpcErrorCode::InvalidParams);

        let stale_request = format!(
            r#"{{"jsonrpc":"2.0","id":5,"method":"{DOWNLOAD_SELECT_TORRENT_FILES}","params":{{"id":"{id}","info_hash":"abcdefabcdefabcdefabcdefabcdefabcdefabcd","selected_file_indexes":[0]}}}}"#
        );
        let stale = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::Operator),
            fetchx_remote_protocol::parse_request(stale_request.as_bytes())?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect_err("stale metadata must require refresh");
        assert_eq!(stale.code(), RpcErrorCode::Conflict);

        let confirmed = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::Operator),
            fetchx_remote_protocol::parse_request(select_request.as_bytes())?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect("confirmed selection should succeed");
        assert_eq!(confirmed["selection_confirmed"], true);
        let persisted = services
            .torrent_download_metadata(id)
            .await?
            .ok_or("selection row is missing")?;
        assert_eq!(persisted.selected_file_indexes, [0]);
        assert!(persisted.selection_confirmed);

        services.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn aria2_recovery_rpc_is_read_only_sanitized_and_available_to_read_only_role()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = CoreServicesConfig {
            database_path: directory.path().join("fetchx.db"),
            downloads_dir: directory.path().join("downloads"),
            runtime_dir: directory.path().join("runtime"),
            bundled_engines_dir: None,
            defer_queued_on_start: true,
        };
        let mut engine_config = EngineConfig::default();
        engine_config.aria2.enabled = false;
        let services = CoreServices::start_with_engine_config(paths, Some(engine_config)).await?;

        let id = DownloadId::new();
        let now = Timestamp::now()?;
        let destination = directory.path().join("partial.bin");
        let partial_bytes = b"existing aria2 partial";
        tokio::fs::write(&destination, partial_bytes).await?;
        let item = DownloadItem::from_parts(
            id,
            DownloadSource::new("https://example.test/partial.bin", EngineKind::Aria2)?,
            FileName::new("partial.bin")?,
            destination.clone(),
            DownloadStatus::Paused,
            DownloadPriority::default(),
            DownloadProgress::new(4, Some(100), None, None)?,
            now,
            now,
            None,
        )?;
        services.repository.create(&item).await?;
        let before = services
            .repository
            .get(id)
            .await?
            .ok_or("missing persisted aria2 download")?;

        let discovery = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::ReadOnly),
            fetchx_remote_protocol::parse_request(
                br#"{"jsonrpc":"2.0","id":1,"method":"system.discover"}"#,
            )?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect("discovery should succeed");
        assert!(
            discovery["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|cap| cap == DOWNLOAD_ARIA2_RECOVERY))
        );

        let request = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"{DOWNLOAD_ARIA2_RECOVERY}","params":{{"id":"{id}"}}}}"#
        );
        let result = dispatch(
            &services,
            AuthenticatedPrincipal::primary(Role::ReadOnly),
            fetchx_remote_protocol::parse_request(request.as_bytes())?,
            &WebSocketTickets::default(),
            [0; 32],
        )
        .await
        .expect("read-only assessment should succeed");

        assert_eq!(
            result["assessment_version"],
            ARIA2_RECOVERY_ASSESSMENT_VERSION
        );
        assert_eq!(result["download_id"], id.to_string());
        assert_eq!(result["state"], "runtime_unavailable");
        assert!(result["explanation"].as_str().is_some());
        assert!(result["next_step"].as_str().is_some());
        let response = result.to_string();
        assert!(!response.contains(&destination.to_string_lossy().to_string()));
        assert!(!response.contains("aria2_gid"));
        assert!(!response.contains("rpc_secret"));

        let after = services
            .repository
            .get(id)
            .await?
            .ok_or("aria2 download disappeared after assessment")?;
        assert_eq!(before, after);
        assert_eq!(tokio::fs::read(destination).await?, partial_bytes);
        services.shutdown().await?;
        Ok(())
    }

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

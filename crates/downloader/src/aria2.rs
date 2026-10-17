use std::{fmt, path::Path, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadSource, DownloadStatus, EngineKind,
    Timestamp, parse_torrent_metainfo,
};
use fetchx_storage::{
    DownloadRepository, SqliteTorrentMetadataRepository, TorrentMetadataRepository,
};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    sync::{Mutex, watch},
    time::{Instant, sleep_until, timeout},
};
use tracing::{info, warn};

use crate::{
    Aria2Supervisor, Aria2SupervisorError, DownloadEngine, DownloadEngineError,
    ManagedDownloadControl, ManagedDownloadEngine, ResourceDemand,
};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(750);
const MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD: usize = 1;
const MAX_PROCESS_RESTARTS_PER_DOWNLOAD: usize = 3;
const MAX_RPC_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_URI_BYTES: usize = 16 * 1024;
pub(crate) const AMBIGUOUS_ADD_URI_DIAGNOSTIC: &str =
    "aria2 submission outcome is ambiguous; automatic retry is disabled pending reconciliation.";
pub(crate) const RECONCILIATION_DIAGNOSTIC: &str =
    "aria2 job could not be reconciled; automatic retry is disabled pending reconciliation.";

#[derive(Clone)]
pub struct Aria2ClientConfig {
    endpoint: String,
    token: Option<String>,
    request_timeout: Duration,
}

impl Aria2ClientConfig {
    pub fn new(
        endpoint: impl Into<String>,
        token: Option<String>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            token,
            request_timeout,
        }
    }
}

impl fmt::Debug for Aria2ClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Aria2ClientConfig")
            .field("endpoint", &self.endpoint)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum Aria2ClientError {
    #[error("aria2 RPC endpoint must use HTTP and resolve to loopback")]
    InvalidEndpoint,
    #[error("aria2 RPC timeout must be greater than zero")]
    InvalidTimeout,
    #[error("aria2 HTTP client could not be configured")]
    ClientConfiguration,
    #[error("aria2 RPC request could not be encoded")]
    RequestEncoding,
    #[error("aria2 RPC transport failed")]
    Transport,
    #[error("aria2 RPC request timed out")]
    Timeout,
    #[error("aria2 RPC endpoint returned HTTP status {0}")]
    HttpStatus(StatusCode),
    #[error("aria2 RPC response was not valid JSON")]
    InvalidJson,
    #[error("aria2 RPC response exceeded its size limit")]
    ResponseTooLarge,
    #[error("aria2 RPC response was malformed or did not match the request")]
    InvalidResponse,
    #[error("aria2 RPC returned error code {code}")]
    Rpc { code: i64 },
    #[error("aria2 rejected the requested download GID")]
    InvalidGid,
    #[error("aria2 addUri outcome is ambiguous; it was not retried")]
    AmbiguousAddUri,
    #[error("aria2 input path cannot be represented safely")]
    InvalidPath,
    #[error("aria2 URI is empty or contains embedded credentials")]
    InvalidUri,
    #[error("aria2 torrent file is missing, invalid, or exceeds the supported size")]
    InvalidTorrentFile,
    #[error("aria2 could not read the torrent file")]
    TorrentFileRead,
    #[error("aria2 download rate limit must be greater than zero")]
    InvalidRateLimit,
}

#[derive(Clone)]
pub struct Aria2Client {
    endpoint: Url,
    token: Option<Arc<str>>,
    client: Client,
    next_request_id: Arc<Mutex<u64>>,
    request_timeout: Duration,
}

impl fmt::Debug for Aria2Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Aria2Client")
            .field("endpoint", &self.endpoint)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl Aria2Client {
    pub fn new(config: Aria2ClientConfig) -> Result<Self, Aria2ClientError> {
        if config.request_timeout.is_zero() {
            return Err(Aria2ClientError::InvalidTimeout);
        }
        let endpoint =
            Url::parse(&config.endpoint).map_err(|_| Aria2ClientError::InvalidEndpoint)?;
        let loopback_host = endpoint
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if endpoint.scheme() != "http"
            || !loopback_host
            || endpoint.username() != ""
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
            || endpoint.query().is_some()
        {
            return Err(Aria2ClientError::InvalidEndpoint);
        }

        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(config.request_timeout)
            .build()
            .map_err(|_| Aria2ClientError::ClientConfiguration)?;
        Ok(Self {
            endpoint,
            token: config.token.map(Arc::from),
            client,
            next_request_id: Arc::new(Mutex::new(1)),
            request_timeout: config.request_timeout,
        })
    }

    pub async fn add_uri(
        &self,
        uri: &str,
        destination: &Path,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        self.add_uri_with_rate_limit(uri, destination, None).await
    }

    pub async fn add_uri_with_rate_limit(
        &self,
        uri: &str,
        destination: &Path,
        max_download_rate_bytes_per_second: Option<u64>,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        self.add_uri_with_options(uri, destination, max_download_rate_bytes_per_second, None)
            .await
    }

    pub async fn add_uri_with_options(
        &self,
        uri: &str,
        destination: &Path,
        max_download_rate_bytes_per_second: Option<u64>,
        select_file: Option<&[u32]>,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        self.add_uri_with_full_options(
            uri,
            destination,
            max_download_rate_bytes_per_second,
            None,
            None,
            select_file,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_uri_with_full_options(
        &self,
        uri: &str,
        destination: &Path,
        max_download_rate_bytes_per_second: Option<u64>,
        max_upload_rate_bytes_per_second: Option<u64>,
        bt_max_peers: Option<usize>,
        select_file: Option<&[u32]>,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        if uri.len() > MAX_SOURCE_URI_BYTES {
            return Err(Aria2ClientError::InvalidUri);
        }
        let mut options = if uri.starts_with("magnet:") {
            DownloadSource::new(uri, EngineKind::Aria2)
                .map_err(|_| Aria2ClientError::InvalidUri)?;
            let directory = destination.to_str().ok_or(Aria2ClientError::InvalidPath)?;
            let peers = bt_max_peers.unwrap_or(MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD);
            json!({
                "dir": directory,
                "bt-max-peers": peers.to_string(),
                "allow-overwrite": "false",
                "auto-file-renaming": "false",
            })
        } else {
            let parsed_uri = Url::parse(uri).map_err(|_| Aria2ClientError::InvalidUri)?;
            if !matches!(parsed_uri.scheme(), "http" | "https")
                || parsed_uri.host_str().is_none()
                || !parsed_uri.username().is_empty()
                || parsed_uri.password().is_some()
                || parsed_uri.fragment().is_some()
            {
                return Err(Aria2ClientError::InvalidUri);
            }
            let (directory, filename) = aria2_destination(destination)?;
            json!({
                "dir": directory,
                "out": filename,
                "max-connection-per-server": MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD.to_string(),
                "split": "1",
                "allow-overwrite": "false",
                "auto-file-renaming": "false",
            })
        };
        if let Some(rate) = max_download_rate_bytes_per_second {
            if rate == 0 {
                return Err(Aria2ClientError::InvalidRateLimit);
            }
            options["max-download-limit"] = json!(rate.to_string());
        }
        if let Some(rate) = max_upload_rate_bytes_per_second {
            if rate == 0 {
                return Err(Aria2ClientError::InvalidRateLimit);
            }
            // Only applies to BitTorrent tasks.
            if uri.starts_with("magnet:") {
                options["max-upload-limit"] = json!(rate.to_string());
            }
        }
        if let Some(indexes) = select_file {
            if !indexes.is_empty() {
                let select_str = indexes
                    .iter()
                    .map(|i| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                options["select-file"] = json!(select_str);
            }
        }
        let result: String = match self.call("aria2.addUri", vec![json!([uri]), options]).await {
            Ok(result) => result,
            Err(Aria2ClientError::Rpc { code }) => {
                return Err(Aria2ClientError::Rpc { code });
            }
            Err(_) => return Err(Aria2ClientError::AmbiguousAddUri),
        };
        Aria2Gid::new(result).map_err(|_| Aria2ClientError::AmbiguousAddUri)
    }

    pub async fn add_torrent_file_with_rate_limit(
        &self,
        torrent_path: &Path,
        destination: &Path,
        max_download_rate_bytes_per_second: Option<u64>,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        self.add_torrent_file_with_options(
            torrent_path,
            destination,
            max_download_rate_bytes_per_second,
            None,
        )
        .await
    }

    pub async fn add_torrent_file_with_options(
        &self,
        torrent_path: &Path,
        destination: &Path,
        max_download_rate_bytes_per_second: Option<u64>,
        select_file: Option<&[u32]>,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        self.add_torrent_file_with_full_options(
            torrent_path,
            destination,
            max_download_rate_bytes_per_second,
            None,
            None,
            select_file,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_torrent_file_with_full_options(
        &self,
        torrent_path: &Path,
        destination: &Path,
        max_download_rate_bytes_per_second: Option<u64>,
        max_upload_rate_bytes_per_second: Option<u64>,
        bt_max_peers: Option<usize>,
        select_file: Option<&[u32]>,
    ) -> Result<Aria2Gid, Aria2ClientError> {
        let metadata = tokio::fs::symlink_metadata(torrent_path)
            .await
            .map_err(|_| Aria2ClientError::TorrentFileRead)?;
        if !metadata.file_type().is_file()
            || !torrent_path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
            || metadata.len() > 8 * 1024 * 1024
        {
            return Err(Aria2ClientError::InvalidTorrentFile);
        }
        let torrent_file = tokio::fs::File::open(torrent_path)
            .await
            .map_err(|_| Aria2ClientError::TorrentFileRead)?;
        let mut bytes = Vec::new();
        torrent_file
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Aria2ClientError::TorrentFileRead)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(Aria2ClientError::InvalidTorrentFile);
        }
        parse_torrent_metainfo(&bytes).map_err(|_| Aria2ClientError::InvalidTorrentFile)?;
        let encoded = BASE64.encode(bytes);
        let directory = destination.to_str().ok_or(Aria2ClientError::InvalidPath)?;
        let peers = bt_max_peers.unwrap_or(MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD);
        let mut options = json!({
            "dir": directory,
            "bt-max-peers": peers.to_string(),
            "allow-overwrite": "false",
            "auto-file-renaming": "false",
        });
        if let Some(rate) = max_download_rate_bytes_per_second {
            if rate == 0 {
                return Err(Aria2ClientError::InvalidRateLimit);
            }
            options["max-download-limit"] = json!(rate.to_string());
        }
        if let Some(rate) = max_upload_rate_bytes_per_second {
            if rate == 0 {
                return Err(Aria2ClientError::InvalidRateLimit);
            }
            options["max-upload-limit"] = json!(rate.to_string());
        }
        if let Some(indexes) = select_file {
            if !indexes.is_empty() {
                let select_str = indexes
                    .iter()
                    .map(|i| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                options["select-file"] = json!(select_str);
            }
        }
        let result: String = match self
            .call("aria2.addTorrent", vec![json!(encoded), json!([]), options])
            .await
        {
            Ok(result) => result,
            Err(Aria2ClientError::Rpc { code }) => return Err(Aria2ClientError::Rpc { code }),
            Err(_) => return Err(Aria2ClientError::AmbiguousAddUri),
        };
        Aria2Gid::new(result).map_err(|_| Aria2ClientError::AmbiguousAddUri)
    }

    pub async fn tell_status(&self, gid: &Aria2Gid) -> Result<Aria2Status, Aria2ClientError> {
        let fields = [
            "gid",
            "status",
            "totalLength",
            "completedLength",
            "downloadSpeed",
            "uploadSpeed",
            "uploadLength",
            "connections",
            "numSeeders",
            "seeder",
            "bittorrent",
            "infoHash",
            "errorCode",
        ];
        let status: RawStatus = self
            .call("aria2.tellStatus", vec![json!(gid.as_str()), json!(fields)])
            .await
            .map_err(map_gid_error)?;
        let status: Aria2Status = status.try_into()?;
        if status.gid != *gid {
            return Err(Aria2ClientError::InvalidResponse);
        }
        Ok(status)
    }

    pub async fn pause(&self, gid: &Aria2Gid) -> Result<Aria2Gid, Aria2ClientError> {
        let result: String = self
            .call("aria2.pause", vec![json!(gid.as_str())])
            .await
            .map_err(map_gid_error)?;
        ensure_gid_matches(Aria2Gid::new(result)?, gid)
    }

    pub async fn unpause(&self, gid: &Aria2Gid) -> Result<Aria2Gid, Aria2ClientError> {
        let result: String = self
            .call("aria2.unpause", vec![json!(gid.as_str())])
            .await
            .map_err(map_gid_error)?;
        ensure_gid_matches(Aria2Gid::new(result)?, gid)
    }

    pub async fn remove(&self, gid: &Aria2Gid) -> Result<Aria2Gid, Aria2ClientError> {
        let result: String = self
            .call("aria2.remove", vec![json!(gid.as_str())])
            .await
            .map_err(map_gid_error)?;
        ensure_gid_matches(Aria2Gid::new(result)?, gid)
    }

    pub async fn force_remove(&self, gid: &Aria2Gid) -> Result<Aria2Gid, Aria2ClientError> {
        let result: String = self
            .call("aria2.forceRemove", vec![json!(gid.as_str())])
            .await
            .map_err(map_gid_error)?;
        ensure_gid_matches(Aria2Gid::new(result)?, gid)
    }

    pub async fn get_version(&self) -> Result<Aria2Version, Aria2ClientError> {
        self.call("aria2.getVersion", Vec::new()).await
    }

    pub async fn shutdown(&self) -> Result<(), Aria2ClientError> {
        let _: String = self.call("aria2.shutdown", Vec::new()).await?;
        Ok(())
    }

    async fn call<T>(&self, method: &str, mut params: Vec<Value>) -> Result<T, Aria2ClientError>
    where
        T: DeserializeOwned,
    {
        if let Some(token) = &self.token {
            params.insert(0, Value::String(format!("token:{token}")));
        }
        let request_id = {
            let mut next_id = self.next_request_id.lock().await;
            let current = *next_id;
            *next_id = current
                .checked_add(1)
                .ok_or(Aria2ClientError::InvalidResponse)?;
            current.to_string()
        };
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: &request_id,
            method,
            params,
        };
        let body = serde_json::to_vec(&request).map_err(|_| Aria2ClientError::RequestEncoding)?;
        let response = timeout(
            self.request_timeout,
            self.client
                .post(self.endpoint.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send(),
        )
        .await
        .map_err(|_| Aria2ClientError::Timeout)?
        .map_err(|error| {
            if error.is_timeout() {
                Aria2ClientError::Timeout
            } else {
                Aria2ClientError::Transport
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(Aria2ClientError::HttpStatus(status));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RPC_RESPONSE_BYTES as u64)
        {
            return Err(Aria2ClientError::ResponseTooLarge);
        }
        let deadline = Instant::now() + self.request_timeout;
        let mut response = response;
        let mut body = Vec::new();
        loop {
            let chunk = timeout(
                deadline.saturating_duration_since(Instant::now()),
                response.chunk(),
            )
            .await
            .map_err(|_| Aria2ClientError::Timeout)?
            .map_err(|error| {
                if error.is_timeout() {
                    Aria2ClientError::Timeout
                } else {
                    Aria2ClientError::Transport
                }
            })?;
            let Some(chunk) = chunk else {
                break;
            };
            if body.len().saturating_add(chunk.len()) > MAX_RPC_RESPONSE_BYTES {
                return Err(Aria2ClientError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let response: RpcResponse =
            serde_json::from_slice(&body).map_err(|_| Aria2ClientError::InvalidJson)?;
        if response.jsonrpc != "2.0"
            || response.id != Value::String(request_id)
            || response.result.is_some() == response.error.is_some()
        {
            return Err(Aria2ClientError::InvalidResponse);
        }
        if let Some(error) = response.error {
            return Err(Aria2ClientError::Rpc { code: error.code });
        }
        serde_json::from_value(response.result.ok_or(Aria2ClientError::InvalidResponse)?)
            .map_err(|_| Aria2ClientError::InvalidResponse)
    }
}

fn aria2_destination(destination: &Path) -> Result<(String, String), Aria2ClientError> {
    let filename = destination
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or(Aria2ClientError::InvalidPath)?;
    let directory = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_str()
        .ok_or(Aria2ClientError::InvalidPath)?;
    Ok((directory.to_owned(), filename.to_owned()))
}

async fn validate_torrent_directory(
    destination: &Path,
    status: &Aria2Status,
) -> Result<(), DownloadEngineError> {
    let metadata = tokio::fs::symlink_metadata(destination)
        .await
        .map_err(DownloadEngineError::Io)?;
    if !metadata.file_type().is_dir() {
        return Err(DownloadEngineError::Aria2InvalidOutput);
    }

    let mut pending = vec![destination.to_path_buf()];
    let mut total_size = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(directory)
            .await
            .map_err(DownloadEngineError::Io)?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(DownloadEngineError::Io)?
        {
            let path = entry.path();
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(DownloadEngineError::Io)?;
            if metadata.file_type().is_symlink() {
                return Err(DownloadEngineError::Aria2InvalidOutput);
            }
            if metadata.file_type().is_dir() {
                pending.push(path);
            } else if metadata.file_type().is_file() {
                total_size = total_size
                    .checked_add(metadata.len())
                    .ok_or(DownloadEngineError::Aria2InvalidOutput)?;
            } else {
                return Err(DownloadEngineError::Aria2InvalidOutput);
            }
        }
    }

    if total_size != status.completed_bytes
        || status.total_bytes.is_some_and(|total| total_size != total)
    {
        return Err(DownloadEngineError::Aria2InvalidOutput);
    }
    Ok(())
}

fn map_gid_error(error: Aria2ClientError) -> Aria2ClientError {
    match error {
        Aria2ClientError::Rpc { code: 1 } => Aria2ClientError::InvalidGid,
        error => error,
    }
}

fn ensure_gid_matches(result: Aria2Gid, expected: &Aria2Gid) -> Result<Aria2Gid, Aria2ClientError> {
    if result == *expected {
        Ok(result)
    } else {
        Err(Aria2ClientError::InvalidResponse)
    }
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: &'a str,
    method: &'a str,
    params: Vec<Value>,
}

#[derive(Deserialize)]
struct RpcResponse {
    jsonrpc: String,
    id: Value,
    result: Option<Value>,
    error: Option<RpcErrorObject>,
}

#[derive(Deserialize)]
struct RpcErrorObject {
    code: i64,
    #[allow(dead_code)]
    message: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Aria2Gid(String);

impl Aria2Gid {
    fn new(gid: String) -> Result<Self, Aria2ClientError> {
        if gid.is_empty() || !gid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Aria2ClientError::InvalidResponse);
        }
        Ok(Self(gid))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Aria2DownloadStatus {
    Active,
    Waiting,
    Paused,
    Complete,
    Error,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Aria2Status {
    pub gid: Aria2Gid,
    pub status: Aria2DownloadStatus,
    pub completed_bytes: u64,
    pub total_bytes: Option<u64>,
    pub download_speed: u64,
    pub upload_speed: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub connections: usize,
    pub num_seeders: Option<u64>,
    /// `true` when aria2 reports this peer is acting as a seeder.
    pub seeder: Option<bool>,
    pub info_hash: Option<String>,
    pub error_code: Option<String>,
    pub tracker_uris: Option<Vec<String>>,
}

impl TryFrom<RawStatus> for Aria2Status {
    type Error = Aria2ClientError;

    fn try_from(raw: RawStatus) -> Result<Self, Self::Error> {
        let status = match raw.status.as_str() {
            "active" => Aria2DownloadStatus::Active,
            "waiting" => Aria2DownloadStatus::Waiting,
            "paused" => Aria2DownloadStatus::Paused,
            "complete" => Aria2DownloadStatus::Complete,
            "error" => Aria2DownloadStatus::Error,
            "removed" => Aria2DownloadStatus::Removed,
            _ => return Err(Aria2ClientError::InvalidResponse),
        };
        let completed_bytes = parse_counter(&raw.completed_length)?;
        let total = parse_counter(&raw.total_length)?;
        let total_bytes = (total != 0).then_some(total);
        if total_bytes.is_some_and(|total| completed_bytes > total) {
            return Err(Aria2ClientError::InvalidResponse);
        }
        let download_speed = parse_counter(&raw.download_speed)?;
        let upload_speed = raw.upload_speed.as_deref().map(parse_counter).transpose()?;
        let uploaded_bytes = raw
            .upload_length
            .as_deref()
            .map(parse_counter)
            .transpose()?;
        let connections = raw
            .connections
            .parse::<usize>()
            .map_err(|_| Aria2ClientError::InvalidResponse)?;
        let num_seeders = raw.num_seeders.as_deref().map(parse_counter).transpose()?;
        let seeder = raw
            .seeder
            .as_deref()
            .map(|value| match value {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(Aria2ClientError::InvalidResponse),
            })
            .transpose()?;
        let tracker_uris = raw
            .bittorrent
            .and_then(|bittorrent| bittorrent.announce_list)
            .map(|tiers| tiers.into_iter().flatten().collect());
        Ok(Self {
            gid: Aria2Gid::new(raw.gid)?,
            status,
            completed_bytes,
            total_bytes,
            download_speed,
            upload_speed,
            uploaded_bytes,
            connections,
            num_seeders,
            seeder,
            info_hash: raw.info_hash,
            error_code: raw.error_code,
            tracker_uris,
        })
    }
}

fn parse_counter(value: &str) -> Result<u64, Aria2ClientError> {
    value
        .parse::<u64>()
        .map_err(|_| Aria2ClientError::InvalidResponse)
}

impl Aria2Status {
    /// Extracts torrent-specific telemetry from this status snapshot.
    pub fn torrent_telemetry(&self) -> fetchx_models::TorrentTelemetry {
        let share_ratio = self
            .uploaded_bytes
            .zip((self.completed_bytes > 0).then_some(self.completed_bytes))
            .map(|(uploaded, downloaded)| uploaded as f64 / downloaded as f64);
        fetchx_models::TorrentTelemetry {
            download_speed_bps: Some(self.download_speed),
            upload_speed_bps: self.upload_speed,
            downloaded_bytes: Some(self.completed_bytes),
            uploaded_bytes: self.uploaded_bytes,
            peer_count: Some(self.connections as u64),
            seed_count: self.num_seeders,
            share_ratio,
            seeding: self.seeder,
            tracker_uris: self.tracker_uris.clone(),
        }
    }
}

#[derive(Deserialize)]
struct RawStatus {
    gid: String,
    status: String,
    #[serde(rename = "completedLength")]
    completed_length: String,
    #[serde(rename = "totalLength")]
    total_length: String,
    #[serde(rename = "downloadSpeed")]
    download_speed: String,
    #[serde(rename = "uploadSpeed")]
    upload_speed: Option<String>,
    #[serde(rename = "uploadLength")]
    upload_length: Option<String>,
    connections: String,
    #[serde(rename = "numSeeders")]
    num_seeders: Option<String>,
    seeder: Option<String>,
    #[serde(rename = "infoHash")]
    info_hash: Option<String>,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
    bittorrent: Option<RawBittorrent>,
}

#[derive(Deserialize)]
struct RawBittorrent {
    #[serde(rename = "announceList")]
    announce_list: Option<Vec<Vec<String>>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Aria2Version {
    version: String,
    #[serde(rename = "enabledFeatures")]
    enabled_features: Vec<String>,
}

impl Aria2Version {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn enabled_features(&self) -> &[String] {
        &self.enabled_features
    }
}

pub struct Aria2Engine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    client: Option<Aria2Client>,
    supervisor: Option<Arc<Aria2Supervisor>>,
    torrent_metadata: Option<SqliteTorrentMetadataRepository>,
    poll_interval: Duration,
    gids: Mutex<std::collections::HashMap<DownloadId, RuntimeGid>>,
}

#[derive(Clone)]
struct RuntimeGid {
    generation: u64,
    gid: Aria2Gid,
}

impl<R> Aria2Engine<R>
where
    R: DownloadRepository + Clone,
{
    pub fn new(repository: R, client: Aria2Client) -> Self {
        Self::with_poll_interval(repository, client, DEFAULT_POLL_INTERVAL)
    }

    pub fn with_poll_interval(repository: R, client: Aria2Client, poll_interval: Duration) -> Self {
        Self {
            state_machine: DownloadStateMachine::new(repository.clone()),
            repository,
            client: Some(client),
            supervisor: None,
            torrent_metadata: None,
            poll_interval: poll_interval.max(Duration::from_millis(10)),
            gids: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Creates an engine whose RPC client and process generation are owned by a supervisor.
    pub fn with_supervisor(repository: R, supervisor: Arc<Aria2Supervisor>) -> Self {
        Self {
            state_machine: DownloadStateMachine::new(repository.clone()),
            repository,
            client: None,
            supervisor: Some(supervisor),
            torrent_metadata: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
            gids: Mutex::new(std::collections::HashMap::new()),
        }
    }

    #[must_use]
    pub fn with_torrent_metadata_repository(
        mut self,
        repository: SqliteTorrentMetadataRepository,
    ) -> Self {
        self.torrent_metadata = Some(repository);
        self
    }

    pub async fn torrent_telemetry(
        &self,
        id: DownloadId,
    ) -> Result<Option<fetchx_models::TorrentTelemetry>, DownloadEngineError> {
        let Some(repository) = &self.torrent_metadata else {
            return Ok(None);
        };
        let Some(metadata) = repository.get_torrent_metadata(id).await? else {
            return Ok(None);
        };
        let Some(gid_text) = metadata.aria2_gid.as_deref() else {
            return Ok(None);
        };
        let gid = Aria2Gid::new(gid_text.to_owned())
            .map_err(|_| DownloadEngineError::Aria2ReconciliationRequired)?;
        let (client, _) = self.current_client().await?;
        let status = client.tell_status(&gid).await?;
        if status
            .info_hash
            .as_deref()
            .is_some_and(|hash| !hash.eq_ignore_ascii_case(&metadata.info_hash))
        {
            return Err(DownloadEngineError::Aria2ReconciliationRequired);
        }
        Ok(Some(status.torrent_telemetry()))
    }

    async fn current_client(&self) -> Result<(Aria2Client, u64), DownloadEngineError> {
        match (&self.supervisor, &self.client) {
            (Some(supervisor), _) => Ok(supervisor.client_runtime().await?),
            (None, Some(client)) => Ok((client.clone(), 0)),
            (None, None) => Err(DownloadEngineError::InvalidConfiguration),
        }
    }

    async fn gid_for_generation(&self, id: DownloadId, generation: u64) -> Option<Aria2Gid> {
        let mut gids = self.gids.lock().await;
        match gids.get(&id) {
            Some(entry) if entry.generation == generation => Some(entry.gid.clone()),
            Some(_) => {
                gids.remove(&id);
                None
            }
            None => None,
        }
    }

    async fn restore_persisted_gid(
        &self,
        id: DownloadId,
        item: &DownloadItem,
        client: &Aria2Client,
        generation: u64,
    ) -> Result<Option<Aria2Gid>, DownloadEngineError> {
        let Some(metadata_repository) = &self.torrent_metadata else {
            return Ok(None);
        };
        if !item.source().is_magnet() && !item.source().is_torrent_file() {
            return Ok(None);
        }
        let Some(mut metadata) = metadata_repository.get_torrent_metadata(id).await? else {
            return Err(DownloadEngineError::Aria2ReconciliationRequired);
        };
        let Some(stored_gid) = metadata.aria2_gid.as_deref() else {
            return Ok(None);
        };
        let gid = Aria2Gid::new(stored_gid.to_owned())
            .map_err(|_| DownloadEngineError::Aria2ReconciliationRequired)?;
        let status = match client.tell_status(&gid).await {
            Ok(status) => status,
            Err(Aria2ClientError::InvalidGid) => {
                metadata.aria2_gid = None;
                metadata.updated_at = Timestamp::now()?.max(metadata.updated_at);
                metadata_repository
                    .update_torrent_metadata(&metadata)
                    .await?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        if status
            .info_hash
            .as_deref()
            .is_some_and(|hash| !hash.eq_ignore_ascii_case(&metadata.info_hash))
        {
            return Err(DownloadEngineError::Aria2ReconciliationRequired);
        }
        if matches!(
            status.status,
            Aria2DownloadStatus::Error | Aria2DownloadStatus::Removed
        ) {
            metadata.aria2_gid = None;
            metadata.updated_at = Timestamp::now()?.max(metadata.updated_at);
            metadata_repository
                .update_torrent_metadata(&metadata)
                .await?;
            return Ok(None);
        }
        self.gids.lock().await.insert(
            id,
            RuntimeGid {
                generation,
                gid: gid.clone(),
            },
        );
        Ok(Some(gid))
    }

    async fn persist_torrent_gid(
        &self,
        id: DownloadId,
        gid: Option<&Aria2Gid>,
    ) -> Result<(), DownloadEngineError> {
        let Some(metadata_repository) = &self.torrent_metadata else {
            return Ok(());
        };
        let Some(mut metadata) = metadata_repository.get_torrent_metadata(id).await? else {
            return if gid.is_some() {
                Err(DownloadEngineError::Aria2ReconciliationRequired)
            } else {
                Ok(())
            };
        };
        metadata.aria2_gid = gid.map(|gid| gid.as_str().to_owned());
        metadata.updated_at = Timestamp::now()?.max(metadata.updated_at);
        metadata_repository
            .update_torrent_metadata(&metadata)
            .await?;
        Ok(())
    }

    async fn download_controlled(
        &self,
        id: DownloadId,
        mut control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if current.source().engine() != EngineKind::Aria2 {
            return Err(DownloadEngineError::EngineMismatch);
        }
        if current.error().is_some_and(|error| {
            matches!(
                error.message(),
                AMBIGUOUS_ADD_URI_DIAGNOSTIC | RECONCILIATION_DIAGNOSTIC
            )
        }) {
            return Err(DownloadEngineError::Aria2ReconciliationRequired);
        }

        let resuming_paused = current.status() == DownloadStatus::Paused;
        let mut item = match current.status() {
            DownloadStatus::Queued | DownloadStatus::Failed | DownloadStatus::Cancelled => {
                self.state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?
            }
            DownloadStatus::Paused => {
                self.state_machine
                    .transition(id, DownloadStatus::Downloading)
                    .await?
            }
            status => {
                return Err(DownloadEngineError::StateMachine(
                    StateMachineError::InvalidTransition {
                        from: status,
                        to: DownloadStatus::Preparing,
                    },
                ));
            }
        };
        let result = self
            .run_job(id, &mut item, resuming_paused, &mut control)
            .await;
        match result {
            Ok(item) => Ok(item),
            Err(error) if matches!(error, DownloadEngineError::Paused) => {
                self.transition_to_paused(id).await?;
                info!(download_id = %id, "aria2 download paused");
                Err(error)
            }
            Err(error) if matches!(error, DownloadEngineError::Cancelled) => {
                self.transition_to_cancelled(id).await?;
                self.gids.lock().await.remove(&id);
                self.persist_torrent_gid(id, None).await?;
                info!(download_id = %id, "aria2 download cancelled");
                Err(error)
            }
            Err(error) => {
                self.persist_failure(id, &error).await?;
                warn!(download_id = %id, "aria2 download failed");
                Err(error)
            }
        }
    }

    async fn run_job(
        &self,
        id: DownloadId,
        item: &mut DownloadItem,
        resuming_paused: bool,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if *control.pause.borrow_and_update() {
            return Err(DownloadEngineError::Paused);
        }
        let (mut client, mut generation) = self.current_client().await?;
        let runtime_gid = self.gid_for_generation(id, generation).await;
        let persisted_gid = if runtime_gid.is_none() {
            self.restore_persisted_gid(id, item, &client, generation)
                .await?
        } else {
            None
        };
        let mut gid = match runtime_gid.or(persisted_gid) {
            Some(gid) => {
                if resuming_paused || item.status() == DownloadStatus::Preparing {
                    client.unpause(&gid).await.map_err(|error| {
                        if matches!(&error, Aria2ClientError::InvalidGid) {
                            DownloadEngineError::Aria2ReconciliationRequired
                        } else {
                            error.into()
                        }
                    })?;
                }
                Some(gid)
            }
            None if item.progress().downloaded_bytes() == 0 || self.supervisor.is_some() => None,
            None => return Err(DownloadEngineError::Aria2ReconciliationRequired),
        };
        if gid.is_none() {
            gid = Some(
                self.submit_job(
                    id,
                    item,
                    &client,
                    generation,
                    control.bandwidth.external_download_rate_bytes_per_second,
                    None,
                )
                .await?,
            );
        }
        let mut gid = gid.ok_or(DownloadEngineError::Aria2ReconciliationRequired)?;
        if item.status() == DownloadStatus::Preparing {
            *item = self
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
        }
        report_aria2_demand(control, id, MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD, 0).await?;
        let mut expected_updated_at = item.updated_at();
        let mut pause_sent = false;
        let mut restart_attempts = 0;

        loop {
            if let Some(supervisor) = &self.supervisor {
                let (updated_client, updated_generation) = supervisor.client_runtime().await?;
                if updated_generation != generation {
                    if *control.cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                    if *control.pause.borrow_and_update() {
                        return Err(DownloadEngineError::Paused);
                    }
                    if restart_attempts >= MAX_PROCESS_RESTARTS_PER_DOWNLOAD {
                        return Err(DownloadEngineError::Aria2Supervisor(
                            Aria2SupervisorError::RestartFailed,
                        ));
                    }
                    restart_attempts += 1;
                    client = updated_client;
                    generation = updated_generation;
                    gid = self
                        .submit_job(
                            id,
                            item,
                            &client,
                            generation,
                            control.bandwidth.external_download_rate_bytes_per_second,
                            None,
                        )
                        .await?;
                    pause_sent = false;
                }
            }
            if *control.cancellation.borrow_and_update() {
                client.remove(&gid).await?;
                return Err(DownloadEngineError::Cancelled);
            }
            if *control.pause.borrow_and_update() && !pause_sent {
                client.pause(&gid).await?;
                pause_sent = true;
            }

            let poll_at = Instant::now() + self.poll_interval;
            tokio::select! {
                biased;
                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        client.remove(&gid).await?;
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                changed = control.pause.changed(), if !pause_sent => {
                    if changed.is_err() || *control.pause.borrow_and_update() {
                        client.pause(&gid).await?;
                        pause_sent = true;
                    }
                }
                _ = sleep_until(poll_at) => {
                    let status = match client.tell_status(&gid).await {
                        Ok(status) => status,
                        Err(error) if self.supervisor.is_some() && Self::is_process_transport_error(&error) => {
                            if restart_attempts >= MAX_PROCESS_RESTARTS_PER_DOWNLOAD {
                                return Err(error.into());
                            }
                            restart_attempts += 1;
                            let supervisor = self.supervisor.as_ref().ok_or(
                                DownloadEngineError::Aria2Supervisor(Aria2SupervisorError::RestartFailed)
                            )?;
                            supervisor.restart_after_failure(generation).await?;
                            let (updated_client, updated_generation) = supervisor.client_runtime().await?;
                            if *control.cancellation.borrow_and_update() {
                                return Err(DownloadEngineError::Cancelled);
                            }
                            if *control.pause.borrow_and_update() {
                                return Err(DownloadEngineError::Paused);
                            }
                            client = updated_client;
                            generation = updated_generation;
                            gid = self
                                .submit_job(
                                    id,
                                    item,
                                    &client,
                                    generation,
                                    control.bandwidth.external_download_rate_bytes_per_second,
                                    None,
                                )
                                .await?;
                            pause_sent = false;
                            continue;
                        }
                        Err(error) => return Err(error.into()),
                    };
                    report_aria2_demand(
                        control,
                        id,
                        MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD,
                        status.connections,
                    ).await?;
                    match status.status {
                        Aria2DownloadStatus::Active | Aria2DownloadStatus::Waiting => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            if status.seeder == Some(true)
                                && (item.source().is_magnet() || item.source().is_torrent_file())
                                && item.status() == DownloadStatus::Downloading
                            {
                                *item = self
                                    .state_machine
                                    .transition(id, DownloadStatus::Seeding)
                                    .await?;
                                expected_updated_at = item.updated_at();
                            }
                        }
                        Aria2DownloadStatus::Paused if pause_sent => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            return Err(DownloadEngineError::Paused);
                        }
                        Aria2DownloadStatus::Paused => {
                            return Err(DownloadEngineError::Aria2UnexpectedStatus);
                        }
                        Aria2DownloadStatus::Complete => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            // If aria2 is still seeding, transition to Seeding rather than
                            // Completed. Download complete != torrent task stopped.
                            if status.seeder == Some(true)
                                && (item.source().is_magnet() || item.source().is_torrent_file())
                            {
                                *item = self.state_machine.transition(id, DownloadStatus::Seeding).await?;
                                // Keep polling to track upload stats while seeding.
                                expected_updated_at = item.updated_at();
                            } else {
                                self.validate_completed_output(item, &status).await?;
                                *item = self.state_machine.transition(id, DownloadStatus::Completed).await?;
                                self.gids.lock().await.remove(&id);
                                self.persist_torrent_gid(id, None).await?;
                                return Ok(item.clone());
                            }
                        }
                        Aria2DownloadStatus::Error => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            self.gids.lock().await.remove(&id);
                            self.persist_torrent_gid(id, None).await?;
                            return Err(DownloadEngineError::Aria2TransferFailed);
                        }
                        Aria2DownloadStatus::Removed => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            self.gids.lock().await.remove(&id);
                            self.persist_torrent_gid(id, None).await?;
                            return Err(DownloadEngineError::Aria2InvalidGid);
                        }
                    }
                }
            }
        }
    }

    async fn submit_job(
        &self,
        id: DownloadId,
        item: &DownloadItem,
        client: &Aria2Client,
        generation: u64,
        max_download_rate_bytes_per_second: Option<u64>,
        select_file: Option<&[u32]>,
    ) -> Result<Aria2Gid, DownloadEngineError> {
        let selected_indexes = if select_file.is_none()
            && (item.source().is_magnet() || item.source().is_torrent_file())
        {
            match &self.torrent_metadata {
                Some(repository) => Some(
                    repository
                        .get_torrent_metadata(id)
                        .await?
                        .ok_or(DownloadEngineError::Aria2ReconciliationRequired)?
                        .selected_file_indexes,
                ),
                None => None,
            }
        } else {
            None
        };
        let select_file = select_file.or(selected_indexes.as_deref());
        if self.supervisor.is_some() && item.progress().downloaded_bytes() > 0 {
            let mut control_file = item.destination().as_os_str().to_owned();
            control_file.push(".aria2");
            match tokio::fs::symlink_metadata(control_file).await {
                Ok(metadata) if metadata.file_type().is_file() => {}
                Ok(_) => return Err(DownloadEngineError::Aria2ReconciliationRequired),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(DownloadEngineError::Aria2ReconciliationRequired);
                }
                Err(error) => return Err(DownloadEngineError::Io(error)),
            }
        }
        let gid = if let Some(torrent_path) = item.source().torrent_file_path() {
            client
                .add_torrent_file_with_options(
                    &torrent_path,
                    item.destination(),
                    max_download_rate_bytes_per_second,
                    select_file,
                )
                .await?
        } else {
            client
                .add_uri_with_options(
                    item.source().url(),
                    item.destination(),
                    max_download_rate_bytes_per_second,
                    select_file,
                )
                .await?
        };
        if item.source().is_magnet() || item.source().is_torrent_file() {
            if let Err(error) = self.persist_torrent_gid(id, Some(&gid)).await {
                if let Err(remove_error) = client.remove(&gid).await {
                    warn!(download_id = %id, gid = %gid.as_str(), %remove_error, "could not remove aria2 task after GID persistence failure");
                }
                return Err(error);
            }
        }
        self.gids.lock().await.insert(
            id,
            RuntimeGid {
                generation,
                gid: gid.clone(),
            },
        );
        Ok(gid)
    }

    async fn persist_progress(
        &self,
        item: &mut DownloadItem,
        expected_updated_at: &mut Timestamp,
        status: &Aria2Status,
    ) -> Result<(), DownloadEngineError> {
        if status.completed_bytes < item.progress().downloaded_bytes() {
            return Err(DownloadEngineError::Aria2ReconciliationRequired);
        }
        let eta = status
            .total_bytes
            .zip((status.download_speed > 0).then_some(status.download_speed))
            .and_then(|(total, speed)| {
                total
                    .checked_sub(status.completed_bytes)?
                    .checked_div(speed)
            });
        item.set_progress_at(
            DownloadProgress::new(
                status.completed_bytes,
                status.total_bytes,
                Some(status.download_speed),
                eta,
            )?,
            next_timestamp(item.updated_at())?,
        )?;
        if !self
            .repository
            .update_if_status_and_timestamp_current(
                item,
                DownloadStatus::Downloading,
                *expected_updated_at,
            )
            .await?
        {
            // Also accept the Seeding state for progress updates during seeding.
            let saved = self
                .repository
                .update_if_status_and_timestamp_current(
                    item,
                    DownloadStatus::Seeding,
                    *expected_updated_at,
                )
                .await?;
            if !saved {
                return Err(DownloadEngineError::StateMachine(
                    self.current_state_error(item.id()).await?,
                ));
            }
        }
        *expected_updated_at = item.updated_at();
        Ok(())
    }

    fn is_process_transport_error(error: &Aria2ClientError) -> bool {
        matches!(
            error,
            Aria2ClientError::Transport
                | Aria2ClientError::Timeout
                | Aria2ClientError::HttpStatus(_)
                | Aria2ClientError::InvalidJson
                | Aria2ClientError::InvalidResponse
        )
    }

    async fn validate_completed_output(
        &self,
        item: &DownloadItem,
        status: &Aria2Status,
    ) -> Result<(), DownloadEngineError> {
        if item.source().is_magnet() || item.source().is_torrent_file() {
            return validate_torrent_directory(item.destination(), status).await;
        }
        let metadata = tokio::fs::symlink_metadata(item.destination())
            .await
            .map_err(DownloadEngineError::Io)?;
        if !metadata.file_type().is_file()
            || metadata.len() != status.completed_bytes
            || status
                .total_bytes
                .is_some_and(|total| metadata.len() != total)
        {
            return Err(DownloadEngineError::Aria2InvalidOutput);
        }
        Ok(())
    }

    async fn current_state_error(
        &self,
        id: DownloadId,
    ) -> Result<StateMachineError, DownloadEngineError> {
        match self.repository.get(id).await? {
            None => Ok(StateMachineError::NotFound(id)),
            Some(item) => Ok(StateMachineError::ConcurrentTransition {
                from: DownloadStatus::Downloading,
                to: item.status(),
                actual: item.status(),
            }),
        }
    }

    async fn transition_to_cancelled(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        match self
            .state_machine
            .transition(id, DownloadStatus::Cancelled)
            .await
        {
            Ok(_) => Ok(()),
            Err(StateMachineError::InvalidTransition {
                from: DownloadStatus::Cancelled,
                to: DownloadStatus::Cancelled,
            }) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn transition_to_paused(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if current.status() == DownloadStatus::Paused {
            return Ok(());
        }
        self.state_machine
            .transition(id, DownloadStatus::Paused)
            .await?;
        Ok(())
    }

    async fn persist_failure(
        &self,
        id: DownloadId,
        operation: &DownloadEngineError,
    ) -> Result<(), DownloadEngineError> {
        let current = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if !matches!(
            current.status(),
            DownloadStatus::Preparing | DownloadStatus::Downloading
        ) {
            return Ok(());
        }
        let error = operation.persisted_error()?;
        self.state_machine
            .transition_with_error(id, DownloadStatus::Failed, error)
            .await?;
        Ok(())
    }
}

async fn report_aria2_demand(
    control: &ManagedDownloadControl,
    download_id: DownloadId,
    desired_concurrency: usize,
    active_concurrency: usize,
) -> Result<(), DownloadEngineError> {
    let Some(sender) = &control.resource_demand else {
        return Ok(());
    };
    sender
        .send(ResourceDemand {
            download_id,
            desired_concurrency,
            active_concurrency,
        })
        .await
        .map_err(|_| DownloadEngineError::ResourceManagerUnavailable)
}

fn next_timestamp(previous: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    let next = previous
        .as_unix_millis()
        .checked_add(1)
        .ok_or(fetchx_models::TimestampError::OutOfRange)?;
    Ok(Timestamp::now()?.max(Timestamp::from_unix_millis(next)))
}

impl<R> DownloadEngine for Aria2Engine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let (pause_sender, pause) = watch::channel(false);
        let (limit_sender, connection_limit) = watch::channel(1);
        let result = self
            .download_controlled(
                id,
                ManagedDownloadControl {
                    cancellation,
                    pause,
                    connection_limit,
                    resource_demand: None,
                    bandwidth: crate::BandwidthControl::default(),
                },
            )
            .await;
        drop((pause_sender, limit_sender));
        result
    }
}

impl<R> ManagedDownloadEngine for Aria2Engine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        self.download_controlled(id, control).await
    }

    #[allow(clippy::manual_async_fn)]
    fn cancel_inactive(
        &self,
        id: DownloadId,
    ) -> impl std::future::Future<Output = Result<(), DownloadEngineError>> + Send {
        async move {
            let Some(item) = self.repository.get(id).await? else {
                return Err(DownloadEngineError::NotFound(id));
            };
            if item.status() != DownloadStatus::Paused {
                return Ok(());
            }
            let Some(entry) = self.gids.lock().await.remove(&id) else {
                return if self.supervisor.is_some() {
                    Ok(())
                } else {
                    Err(DownloadEngineError::Aria2ReconciliationRequired)
                };
            };
            let (client, generation) = self.current_client().await?;
            if entry.generation == generation {
                client.remove(&entry.gid).await?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, error::Error, path::PathBuf, time::Duration};

    use fetchx_core::DownloadStateMachine;
    use fetchx_models::{DownloadId, DownloadSource, FileName};
    use fetchx_storage::{
        Database, DownloadRepository, TorrentMetadataRepository, TorrentMetadataRow,
        TorrentSeedState, TorrentSourceType,
    };
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{mpsc, watch},
        task::JoinHandle,
    };

    use super::{
        AMBIGUOUS_ADD_URI_DIAGNOSTIC, Aria2Client, Aria2ClientConfig, Aria2ClientError,
        Aria2DownloadStatus, Aria2Engine, Aria2Gid, Aria2Status, DownloadStatus, EngineKind,
        Timestamp, validate_torrent_directory,
    };
    use crate::{DownloadEngine, ManagedDownloadEngine};
    use fetchx_models::DownloadItem;

    const TEST_TOKEN: &str = "test-only-secret-token";

    enum MockReply {
        Result(Value),
        RpcError(i64),
        Raw(&'static str),
        HttpError,
        Delayed(Duration, Value),
    }

    async fn start_server(
        replies: impl IntoIterator<Item = MockReply>,
    ) -> Result<(String, mpsc::UnboundedReceiver<Value>, JoinHandle<()>), Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/jsonrpc", listener.local_addr()?);
        let (sender, requests) = mpsc::unbounded_channel();
        let mut replies = VecDeque::from_iter(replies);
        let task = tokio::spawn(async move {
            while let Some(reply) = replies.pop_front() {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let Ok(request) = read_request(&mut stream).await else {
                    return;
                };
                let _ = sender.send(request.clone());
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let (status, body, delay) = match reply {
                    MockReply::Result(result) => (
                        "200 OK",
                        json!({"jsonrpc":"2.0", "id":id, "result":result}).to_string(),
                        Duration::ZERO,
                    ),
                    MockReply::RpcError(code) => (
                        "200 OK",
                        json!({
                            "jsonrpc":"2.0",
                            "id":id,
                            "error":{"code":code,"message":"private mock error"}
                        })
                        .to_string(),
                        Duration::ZERO,
                    ),
                    MockReply::Raw(body) => ("200 OK", body.to_owned(), Duration::ZERO),
                    MockReply::HttpError => {
                        ("500 Internal Server Error", "{}".to_owned(), Duration::ZERO)
                    }
                    MockReply::Delayed(delay, result) => (
                        "200 OK",
                        json!({"jsonrpc":"2.0", "id":id, "result":result}).to_string(),
                        delay,
                    ),
                };
                tokio::time::sleep(delay).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Ok((endpoint, requests, task))
    }

    async fn read_request(
        stream: &mut tokio::net::TcpStream,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        let body_start = loop {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Err("client closed before sending request".into());
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..body_start]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .ok_or("missing content length")?;
        while bytes.len() < body_start + content_length {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Err("request body was truncated".into());
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        Ok(serde_json::from_slice(
            &bytes[body_start..body_start + content_length],
        )?)
    }

    fn make_client(endpoint: String, timeout: Duration) -> Result<Aria2Client, Aria2ClientError> {
        Aria2Client::new(Aria2ClientConfig::new(
            endpoint,
            Some(TEST_TOKEN.to_owned()),
            timeout,
        ))
    }

    fn rpc_result_version() -> Value {
        json!({"version":"1.37.0","enabledFeatures":["Async DNS","HTTPS"]})
    }

    fn raw_status(status: &str, completed: &str, total: &str) -> super::RawStatus {
        super::RawStatus {
            gid: "0123456789abcdef".to_owned(),
            status: status.to_owned(),
            completed_length: completed.to_owned(),
            total_length: total.to_owned(),
            download_speed: "12".to_owned(),
            upload_speed: None,
            upload_length: None,
            connections: "1".to_owned(),
            num_seeders: None,
            seeder: None,
            info_hash: None,
            error_code: None,
            bittorrent: None,
        }
    }

    #[tokio::test]
    async fn version_calls_correlate_ids_and_put_token_first() -> Result<(), Box<dyn Error>> {
        let (endpoint, mut requests, server) = start_server([
            MockReply::Result(rpc_result_version()),
            MockReply::Result(rpc_result_version()),
        ])
        .await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        assert_eq!(client.get_version().await?.version(), "1.37.0");
        assert_eq!(client.get_version().await?.version(), "1.37.0");

        let first = requests.recv().await.ok_or("missing first request")?;
        let second = requests.recv().await.ok_or("missing second request")?;
        assert_eq!(first["jsonrpc"], "2.0");
        assert_eq!(first["method"], "aria2.getVersion");
        assert_eq!(first["id"], "1");
        assert_eq!(second["id"], "2");
        assert_eq!(first["params"][0], format!("token:{TEST_TOKEN}"));
        assert!(!format!("{:?}", client).contains(TEST_TOKEN));
        assert!(
            !format!(
                "{:?}",
                Aria2ClientConfig::new(
                    "http://127.0.0.1:6800/jsonrpc",
                    Some(TEST_TOKEN.to_owned()),
                    Duration::from_secs(1),
                )
            )
            .contains(TEST_TOKEN)
        );
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn torrent_file_submission_uses_add_torrent_and_confined_options()
    -> Result<(), Box<dyn Error>> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        let metainfo = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
                         01234567890123456789ee";
        let root = tempfile::tempdir()?;
        let torrent_path = root.path().join("sample.torrent");
        tokio::fs::write(&torrent_path, metainfo).await?;
        let destination = root.path().join("downloads");
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(json!("0123456789abcdef"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;

        client
            .add_torrent_file_with_rate_limit(&torrent_path, &destination, Some(8192))
            .await?;

        let request = requests.recv().await.ok_or("missing addTorrent request")?;
        assert_eq!(request["method"], "aria2.addTorrent");
        assert_eq!(request["params"][0], format!("token:{TEST_TOKEN}"));
        assert_eq!(
            BASE64.decode(request["params"][1].as_str().ok_or("missing metainfo")?)?,
            metainfo
        );
        assert_eq!(request["params"][2], json!([]));
        assert_eq!(
            request["params"][3]["dir"],
            destination.to_string_lossy().as_ref()
        );
        assert_eq!(request["params"][3]["bt-max-peers"], "1");
        assert_eq!(request["params"][3]["max-download-limit"], "8192");
        assert_eq!(request["params"][3]["allow-overwrite"], "false");
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn submit_job_uses_persisted_selection_and_saves_aria2_gid() -> Result<(), Box<dyn Error>>
    {
        let metainfo = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
                         01234567890123456789ee";
        let root = tempfile::tempdir()?;
        let torrent_path = root.path().join("sample.torrent");
        tokio::fs::write(&torrent_path, metainfo).await?;
        let parsed = fetchx_models::parse_torrent_metainfo(metainfo)?;
        let info_hash = parsed.info_hash.clone();
        let database = Database::connect("sqlite::memory:").await?;
        let source = DownloadSource::new_torrent_file(&torrent_path, EngineKind::Aria2)?;
        let now = Timestamp::now()?;
        let id = DownloadId::new();
        let item = DownloadItem::new(
            id,
            source,
            FileName::new("x")?,
            root.path().join("output"),
            now,
        );
        database.downloads().create(&item).await?;
        database
            .torrent_metadata()
            .create_torrent_metadata(&TorrentMetadataRow {
                download_id: id,
                source_type: TorrentSourceType::TorrentFile,
                magnet_uri: None,
                local_torrent_path: Some(torrent_path),
                info_hash: parsed.info_hash,
                torrent_name: parsed.name,
                destination: item.destination().to_path_buf(),
                metadata_state: "ready".to_owned(),
                selected_file_indexes: vec![0],
                selected_total_bytes: 1,
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

        let (endpoint, mut requests, server) = start_server([
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(json!({
                "gid": "0123456789abcdef",
                "status": "active",
                "completedLength": "0",
                "totalLength": "1",
                "downloadSpeed": "0",
                "uploadSpeed": "0",
                "uploadLength": "0",
                "connections": "1",
                "numSeeders": "0",
                "seeder": "false",
                "infoHash": info_hash,
                "errorCode": null
            })),
        ])
        .await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let engine = Aria2Engine::new(database.downloads(), client.clone())
            .with_torrent_metadata_repository(database.torrent_metadata());
        engine.submit_job(id, &item, &client, 0, None, None).await?;

        let request = requests.recv().await.ok_or("missing addTorrent request")?;
        assert_eq!(request["method"], "aria2.addTorrent");
        assert_eq!(request["params"][3]["select-file"], "1");
        let stored_hash = database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await?
            .ok_or("missing torrent metadata")?
            .info_hash;
        let restored = engine
            .restore_persisted_gid(id, &item, &client, 1)
            .await?
            .ok_or("aria2 GID was not restored")?;
        assert_eq!(restored.as_str(), "0123456789abcdef");
        let status_request = requests.recv().await.ok_or("missing tellStatus request")?;
        assert_eq!(status_request["method"], "aria2.tellStatus");
        let persisted = database
            .torrent_metadata()
            .get_torrent_metadata(id)
            .await?
            .ok_or("missing torrent metadata")?;
        assert_eq!(persisted.aria2_gid.as_deref(), Some("0123456789abcdef"));
        assert_eq!(persisted.info_hash, stored_hash);
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn rpc_errors_are_structured_and_do_not_expose_server_messages()
    -> Result<(), Box<dyn Error>> {
        let (endpoint, _requests, server) = start_server([MockReply::RpcError(42)]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let error = client.get_version().await.unwrap_err();
        assert!(matches!(error, Aria2ClientError::Rpc { code: 42 }));
        assert!(!error.to_string().contains("private mock error"));
        assert!(!error.to_string().contains(TEST_TOKEN));
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn malformed_json_protocol_and_mismatched_ids_are_rejected() -> Result<(), Box<dyn Error>>
    {
        for response in [
            MockReply::Raw("not-json"),
            MockReply::Raw(
                r#"{"jsonrpc":"1.0","id":"1","result":{"version":"x","enabledFeatures":[]}}"#,
            ),
            MockReply::Raw(
                r#"{"jsonrpc":"2.0","id":"wrong","result":{"version":"x","enabledFeatures":[]}}"#,
            ),
            MockReply::Raw(r#"{"jsonrpc":"2.0","id":"1"}"#),
        ] {
            let (endpoint, _requests, server) = start_server([response]).await?;
            let client = make_client(endpoint, Duration::from_secs(1))?;
            assert!(matches!(
                client.get_version().await,
                Err(Aria2ClientError::InvalidJson | Aria2ClientError::InvalidResponse)
            ));
            server.await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn http_errors_timeouts_and_connection_failures_are_reported()
    -> Result<(), Box<dyn Error>> {
        let (endpoint, _requests, server) = start_server([MockReply::HttpError]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        assert!(matches!(
            client.get_version().await,
            Err(Aria2ClientError::HttpStatus(_))
        ));
        server.await?;

        let (endpoint, _requests, server) = start_server([MockReply::Delayed(
            Duration::from_millis(100),
            rpc_result_version(),
        )])
        .await?;
        let client = make_client(endpoint, Duration::from_millis(20))?;
        assert!(matches!(
            client.get_version().await,
            Err(Aria2ClientError::Timeout)
        ));
        server.await?;

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/jsonrpc", listener.local_addr()?);
        drop(listener);
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let error = client.get_version().await.unwrap_err();
        assert!(
            matches!(
                error,
                Aria2ClientError::Transport | Aria2ClientError::Timeout
            ),
            "unexpected connection-refused error: {error:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn add_uri_encodes_destination_and_never_retries_an_ambiguous_request()
    -> Result<(), Box<dyn Error>> {
        let destination = PathBuf::from(r"C:\downloads\archive.bin");
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(json!("0123456789abcdef"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let gid = client
            .add_uri_with_rate_limit("https://example.test/archive", &destination, Some(4096))
            .await?;
        assert_eq!(gid.as_str(), "0123456789abcdef");
        let request = requests.recv().await.ok_or("missing addUri request")?;
        assert_eq!(request["method"], "aria2.addUri");
        assert_eq!(request["params"][1][0], "https://example.test/archive");
        assert_eq!(request["params"][2]["out"], "archive.bin");
        assert_eq!(request["params"][2]["dir"], r"C:\downloads");
        assert_eq!(request["params"][2]["max-connection-per-server"], "1");
        assert_eq!(request["params"][2]["max-download-limit"], "4096");
        assert_eq!(request["params"][2]["allow-overwrite"], "false");
        server.await?;

        let (endpoint, mut requests, server) = start_server([MockReply::Delayed(
            Duration::from_millis(100),
            json!("0123456789abcdef"),
        )])
        .await?;
        let client = make_client(endpoint, Duration::from_millis(20))?;
        assert!(matches!(
            client
                .add_uri("https://example.test/archive", &destination)
                .await,
            Err(Aria2ClientError::AmbiguousAddUri)
        ));
        assert!(requests.recv().await.is_some());
        server.await?;
        assert!(matches!(
            make_client(
                "http://example.test/jsonrpc".to_owned(),
                Duration::from_secs(1)
            ),
            Err(Aria2ClientError::InvalidEndpoint)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn add_magnet_uses_aria2_torrent_options_and_validates_uri() -> Result<(), Box<dyn Error>>
    {
        let destination = PathBuf::from(r"C:\downloads\torrent-0123456789abcdef");
        let magnet = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=fixture";
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(json!("0123456789abcdef"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let gid = client
            .add_uri_with_rate_limit(magnet, &destination, Some(2048))
            .await?;
        assert_eq!(gid.as_str(), "0123456789abcdef");
        let request = requests
            .recv()
            .await
            .ok_or("missing magnet addUri request")?;
        assert_eq!(request["method"], "aria2.addUri");
        assert_eq!(request["params"][1][0], magnet);
        assert_eq!(
            request["params"][2]["dir"],
            r"C:\downloads\torrent-0123456789abcdef"
        );
        assert_eq!(request["params"][2]["bt-max-peers"], "1");
        assert_eq!(request["params"][2]["max-download-limit"], "2048");
        assert_eq!(request["params"][2]["allow-overwrite"], "false");
        assert!(request["params"][2].get("out").is_none());
        server.await?;

        let (endpoint, _requests, server) = start_server([]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        assert!(matches!(
            client.add_uri("magnet:?dn=no-hash", &destination).await,
            Err(Aria2ClientError::InvalidUri)
        ));
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn completed_torrent_output_is_confined_to_regular_files_with_matching_size()
    -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new(DownloadId::new())?;
        let torrent_root = directory.0.join("torrent-output");
        tokio::fs::create_dir_all(torrent_root.join("nested")).await?;
        tokio::fs::write(torrent_root.join("first.bin"), b"one").await?;
        tokio::fs::write(torrent_root.join("nested").join("second.bin"), b"two").await?;

        let status = Aria2Status {
            gid: Aria2Gid("0123456789abcdef".to_owned()),
            status: Aria2DownloadStatus::Complete,
            completed_bytes: 6,
            total_bytes: Some(6),
            download_speed: 0,
            upload_speed: None,
            uploaded_bytes: None,
            connections: 0,
            num_seeders: None,
            seeder: Some(false),
            info_hash: None,
            error_code: None,
            tracker_uris: None,
        };
        validate_torrent_directory(&torrent_root, &status).await?;

        let mismatched = Aria2Status {
            completed_bytes: 5,
            ..status
        };
        assert!(matches!(
            validate_torrent_directory(&torrent_root, &mismatched).await,
            Err(crate::DownloadEngineError::Aria2InvalidOutput)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn pause_resume_and_cancel_use_current_session_gid() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let id = DownloadId::new();
        let directory = TestDirectory::new(id)?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let (endpoint, mut requests, server) = start_server([
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("active", "1", "4")),
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("paused", "1", "4")),
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("complete", "4", "4")),
        ])
        .await?;
        let engine = std::sync::Arc::new(Aria2Engine::with_poll_interval(
            database.downloads(),
            make_client(endpoint, Duration::from_secs(1))?,
            Duration::from_millis(10),
        ));
        let (_cancel_sender, cancellation) = watch::channel(false);
        let (pause_sender, pause) = watch::channel(false);
        let (_limit_sender, connection_limit) = watch::channel(1);
        let task_engine = std::sync::Arc::clone(&engine);
        let task = tokio::spawn(async move {
            task_engine
                .download_managed(
                    id,
                    crate::ManagedDownloadControl {
                        cancellation,
                        pause,
                        connection_limit,
                        resource_demand: None,
                        bandwidth: crate::BandwidthControl::default(),
                    },
                )
                .await
        });
        assert_eq!(
            requests.recv().await.ok_or("missing addUri")?["method"],
            "aria2.addUri"
        );
        assert_eq!(
            requests.recv().await.ok_or("missing tellStatus")?["method"],
            "aria2.tellStatus"
        );
        pause_sender.send(true)?;
        assert_eq!(
            requests.recv().await.ok_or("missing pause")?["method"],
            "aria2.pause"
        );
        assert_eq!(
            requests.recv().await.ok_or("missing paused status")?["method"],
            "aria2.tellStatus"
        );
        assert!(matches!(
            task.await?,
            Err(crate::DownloadEngineError::Paused)
        ));
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("missing item")?
                .status(),
            DownloadStatus::Paused
        );

        let state_machine = DownloadStateMachine::new(database.downloads());
        state_machine.transition(id, DownloadStatus::Queued).await?;
        tokio::fs::write(directory.0.join("file.bin"), b"data").await?;
        let (_cancel_sender, cancellation) = watch::channel(false);
        let resumed = engine.download(id, cancellation).await?;
        assert_eq!(resumed.status(), DownloadStatus::Completed);
        let methods = [
            requests.recv().await.ok_or("missing unpause")?["method"].clone(),
            requests.recv().await.ok_or("missing completion status")?["method"].clone(),
        ];
        assert_eq!(methods, [json!("aria2.unpause"), json!("aria2.tellStatus")]);
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_uri_rejects_non_http_schemes_and_embedded_credentials()
    -> Result<(), Box<dyn Error>> {
        let client = make_client(
            "http://127.0.0.1:6800/jsonrpc".to_owned(),
            Duration::from_secs(1),
        )?;
        for uri in [
            "file:///etc/passwd",
            "ftp://example.test/file",
            "data:text/plain,content",
            "https://user:password@example.test/file",
            "https://example.test/file#fragment",
        ] {
            assert!(matches!(
                client.add_uri(uri, std::path::Path::new("file.bin")).await,
                Err(Aria2ClientError::InvalidUri)
            ));
        }
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_removes_active_gid_before_application_state_is_cancelled()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let id = DownloadId::new();
        let directory = TestDirectory::new(id)?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let (endpoint, mut requests, server) = start_server([
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("active", "0", "4")),
            MockReply::Result(json!("0123456789abcdef")),
        ])
        .await?;
        let engine = std::sync::Arc::new(Aria2Engine::with_poll_interval(
            database.downloads(),
            make_client(endpoint, Duration::from_secs(1))?,
            Duration::from_millis(10),
        ));
        let (cancel_sender, cancellation) = watch::channel(false);
        let (_pause_sender, pause) = watch::channel(false);
        let (_limit_sender, connection_limit) = watch::channel(1);
        let task_engine = std::sync::Arc::clone(&engine);
        let task = tokio::spawn(async move {
            task_engine
                .download_managed(
                    id,
                    crate::ManagedDownloadControl {
                        cancellation,
                        pause,
                        connection_limit,
                        resource_demand: None,
                        bandwidth: crate::BandwidthControl::default(),
                    },
                )
                .await
        });
        assert_eq!(
            requests.recv().await.ok_or("missing addUri")?["method"],
            "aria2.addUri"
        );
        assert_eq!(
            requests.recv().await.ok_or("missing tellStatus")?["method"],
            "aria2.tellStatus"
        );
        cancel_sender.send(true)?;
        assert_eq!(
            requests.recv().await.ok_or("missing remove")?["method"],
            "aria2.remove"
        );
        assert!(matches!(
            task.await?,
            Err(crate::DownloadEngineError::Cancelled)
        ));
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("missing item")?
                .status(),
            DownloadStatus::Cancelled
        );
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_a_paused_job_removes_its_runtime_gid() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let id = DownloadId::new();
        let directory = TestDirectory::new(id)?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let (endpoint, mut requests, server) = start_server([
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("paused", "0", "4")),
            MockReply::Result(json!("0123456789abcdef")),
        ])
        .await?;
        let engine = std::sync::Arc::new(Aria2Engine::with_poll_interval(
            database.downloads(),
            make_client(endpoint, Duration::from_secs(1))?,
            Duration::from_millis(10),
        ));
        let (_cancel_sender, cancellation) = watch::channel(false);
        let (pause_sender, pause) = watch::channel(false);
        let (_limit_sender, connection_limit) = watch::channel(1);
        let task_engine = std::sync::Arc::clone(&engine);
        let task = tokio::spawn(async move {
            task_engine
                .download_managed(
                    id,
                    crate::ManagedDownloadControl {
                        cancellation,
                        pause,
                        connection_limit,
                        resource_demand: None,
                        bandwidth: crate::BandwidthControl::default(),
                    },
                )
                .await
        });
        assert_eq!(
            requests.recv().await.ok_or("missing addUri")?["method"],
            "aria2.addUri"
        );
        pause_sender.send(true)?;
        assert_eq!(
            requests.recv().await.ok_or("missing pause")?["method"],
            "aria2.pause"
        );
        assert_eq!(
            requests.recv().await.ok_or("missing paused status")?["method"],
            "aria2.tellStatus"
        );
        assert!(matches!(
            task.await?,
            Err(crate::DownloadEngineError::Paused)
        ));

        engine.cancel_inactive(id).await?;
        assert_eq!(
            requests.recv().await.ok_or("missing paused-job remove")?["method"],
            "aria2.remove"
        );
        let state_machine = DownloadStateMachine::new(database.downloads());
        state_machine
            .transition(id, DownloadStatus::Cancelled)
            .await?;
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("missing item")?
                .status(),
            DownloadStatus::Cancelled
        );
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn rpc_control_methods_and_invalid_gid_are_explicit() -> Result<(), Box<dyn Error>> {
        let gid = "0123456789abcdef";
        let (endpoint, mut requests, server) = start_server([
            MockReply::Result(json!(gid)),
            MockReply::Result(json!(gid)),
            MockReply::Result(json!(gid)),
            MockReply::RpcError(1),
        ])
        .await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let gid = super::Aria2Gid::new(gid.to_owned())?;
        client.pause(&gid).await?;
        client.unpause(&gid).await?;
        client.force_remove(&gid).await?;
        assert!(matches!(
            client.remove(&gid).await,
            Err(Aria2ClientError::InvalidGid)
        ));
        for expected in [
            "aria2.pause",
            "aria2.unpause",
            "aria2.forceRemove",
            "aria2.remove",
        ] {
            assert_eq!(
                requests.recv().await.ok_or("missing RPC control request")?["method"],
                expected
            );
        }
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn status_response_must_match_the_requested_gid() -> Result<(), Box<dyn Error>> {
        let (endpoint, _requests, server) =
            start_server([MockReply::Result(status_json("active", "0", "0"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let expected = super::Aria2Gid::new("fedcba9876543210".to_owned())?;
        assert!(matches!(
            client.tell_status(&expected).await,
            Err(Aria2ClientError::InvalidResponse)
        ));
        server.await?;
        Ok(())
    }

    #[test]
    fn aria2_status_and_progress_values_are_validated() -> Result<(), Box<dyn Error>> {
        for (raw, expected) in [
            ("active", Aria2DownloadStatus::Active),
            ("waiting", Aria2DownloadStatus::Waiting),
            ("paused", Aria2DownloadStatus::Paused),
            ("complete", Aria2DownloadStatus::Complete),
            ("error", Aria2DownloadStatus::Error),
            ("removed", Aria2DownloadStatus::Removed),
        ] {
            assert_eq!(
                Aria2Status::try_from(raw_status(raw, "3", "10"))?.status,
                expected
            );
        }
        let unknown = Aria2Status::try_from(raw_status("active", "3", "0"))?;
        assert_eq!(unknown.total_bytes, None);
        assert!(matches!(
            Aria2Status::try_from(raw_status("active", "11", "10")),
            Err(Aria2ClientError::InvalidResponse)
        ));
        assert!(matches!(
            Aria2Status::try_from(raw_status("unknown", "0", "0")),
            Err(Aria2ClientError::InvalidResponse)
        ));
        Ok(())
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(id: DownloadId) -> Result<Self, std::io::Error> {
            let path =
                std::env::temp_dir().join(format!("fetchx-aria2-{}-{id}", std::process::id()));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn create_item(
        database: &Database,
        directory: &TestDirectory,
        engine: EngineKind,
    ) -> Result<DownloadId, Box<dyn Error>> {
        let id = DownloadId::new();
        let item = DownloadItem::new(
            id,
            DownloadSource::new("https://example.test/file.bin", engine)?,
            FileName::new("file.bin")?,
            directory.0.join("file.bin"),
            Timestamp::now()?,
        );
        database.downloads().create(&item).await?;
        Ok(id)
    }

    #[tokio::test]
    async fn process_generation_replaces_stale_runtime_gid() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let directory = TestDirectory::new(DownloadId::new())?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let client = Aria2Client::new(Aria2ClientConfig::new(
            "http://127.0.0.1:6800/jsonrpc",
            Some(TEST_TOKEN.to_owned()),
            Duration::from_secs(1),
        ))?;
        let engine = Aria2Engine::new(database.downloads(), client);
        engine.gids.lock().await.insert(
            id,
            super::RuntimeGid {
                generation: 1,
                gid: super::Aria2Gid::new("aaaaaaaaaaaaaaaa".to_owned())?,
            },
        );
        assert!(engine.gid_for_generation(id, 2).await.is_none());
        assert!(!engine.gids.lock().await.contains_key(&id));
        Ok(())
    }

    #[tokio::test]
    async fn resubmission_replaces_runtime_gid_mapping() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let directory = TestDirectory::new(DownloadId::new())?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let item = database
            .downloads()
            .get(id)
            .await?
            .ok_or("missing download")?;
        let (endpoint, _requests, server) = start_server([
            MockReply::Result(json!("aaaaaaaaaaaaaaaa")),
            MockReply::Result(json!("bbbbbbbbbbbbbbbb")),
        ])
        .await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let engine = Aria2Engine::new(database.downloads(), client.clone());
        engine.submit_job(id, &item, &client, 4, None, None).await?;
        engine.submit_job(id, &item, &client, 5, None, None).await?;
        let current = engine
            .gid_for_generation(id, 5)
            .await
            .ok_or("missing current runtime GID")?;
        assert_eq!(current.as_str(), "bbbbbbbbbbbbbbbb");
        assert!(engine.gid_for_generation(id, 4).await.is_none());
        server.await?;
        Ok(())
    }

    fn status_json(status: &str, completed: &str, total: &str) -> Value {
        json!({
            "gid":"0123456789abcdef",
            "status":status,
            "completedLength":completed,
            "totalLength":total,
            "downloadSpeed":"10",
            "connections":"1",
            "errorCode":"0"
        })
    }

    #[tokio::test]
    async fn engine_persists_progress_and_completes_only_after_output_validation()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let id = DownloadId::new();
        let directory = TestDirectory::new(id)?;
        tokio::fs::write(directory.0.join("file.bin"), b"data").await?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let (endpoint, _requests, server) = start_server([
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("active", "2", "4")),
            MockReply::Result(status_json("complete", "4", "4")),
        ])
        .await?;
        let engine = Aria2Engine::with_poll_interval(
            database.downloads(),
            make_client(endpoint, Duration::from_secs(1))?,
            Duration::from_millis(10),
        );
        let (_sender, cancellation) = watch::channel(false);
        let item = engine.download(id, cancellation).await?;
        assert_eq!(item.status(), DownloadStatus::Completed);
        assert_eq!(item.progress().downloaded_bytes(), 4);
        assert_eq!(item.progress().total_bytes(), Some(4));
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn engine_persists_transfer_failure_and_refuses_unreconciled_resume()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let id = DownloadId::new();
        let directory = TestDirectory::new(id)?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let (endpoint, _requests, server) = start_server([
            MockReply::Result(json!("0123456789abcdef")),
            MockReply::Result(status_json("error", "0", "0")),
        ])
        .await?;
        let engine = Aria2Engine::with_poll_interval(
            database.downloads(),
            make_client(endpoint, Duration::from_secs(1))?,
            Duration::from_millis(10),
        );
        let (_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            engine.download(id, cancellation).await,
            Err(crate::DownloadEngineError::Aria2TransferFailed)
        ));
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("missing item")?
                .status(),
            DownloadStatus::Failed
        );
        server.await?;

        let id = DownloadId::new();
        let mut item = DownloadItem::new(
            id,
            DownloadSource::new("https://example.test/resume.bin", EngineKind::Aria2)?,
            FileName::new("resume.bin")?,
            directory.0.join("resume.bin"),
            Timestamp::now()?,
        );
        item.set_progress_at(
            fetchx_models::DownloadProgress::new(10, Some(20), None, None)?,
            Timestamp::now()?,
        )?;
        database.downloads().create(&item).await?;
        let engine = Aria2Engine::new(
            database.downloads(),
            make_client(
                "http://127.0.0.1:1/jsonrpc".to_owned(),
                Duration::from_millis(50),
            )?,
        );
        let (_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            engine.download(id, cancellation).await,
            Err(crate::DownloadEngineError::Aria2ReconciliationRequired)
        ));
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("missing item")?
                .status(),
            DownloadStatus::Failed
        );
        Ok(())
    }

    #[tokio::test]
    async fn ambiguous_add_uri_is_persisted_and_never_submitted_twice() -> Result<(), Box<dyn Error>>
    {
        let database = Database::connect("sqlite::memory:").await?;
        let id = DownloadId::new();
        let directory = TestDirectory::new(id)?;
        let id = create_item(&database, &directory, EngineKind::Aria2).await?;
        let (endpoint, mut requests, server) = start_server([MockReply::Delayed(
            Duration::from_millis(100),
            json!("0123456789abcdef"),
        )])
        .await?;
        let engine = Aria2Engine::new(
            database.downloads(),
            make_client(endpoint, Duration::from_millis(20))?,
        );
        let (_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            engine.download(id, cancellation).await,
            Err(crate::DownloadEngineError::Aria2Client(
                Aria2ClientError::AmbiguousAddUri
            ))
        ));
        assert_eq!(
            requests.recv().await.ok_or("missing submitted addUri")?["method"],
            "aria2.addUri"
        );
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("missing item")?
                .error()
                .map(|error| error.message()),
            Some(AMBIGUOUS_ADD_URI_DIAGNOSTIC)
        );
        let (_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            engine.download(id, cancellation).await,
            Err(crate::DownloadEngineError::Aria2ReconciliationRequired)
        ));
        server.await?;
        Ok(())
    }

    #[test]
    fn torrent_telemetry_is_extracted_from_seeder_status_response() {
        let raw = super::RawStatus {
            gid: "0123456789abcdef".to_owned(),
            status: "active".to_owned(),
            completed_length: "500".to_owned(),
            total_length: "1000".to_owned(),
            download_speed: "100".to_owned(),
            upload_speed: Some("200".to_owned()),
            upload_length: Some("400".to_owned()),
            connections: "5".to_owned(),
            num_seeders: Some("7".to_owned()),
            seeder: Some("true".to_owned()),
            info_hash: Some("aabbcc".to_owned()),
            error_code: None,
            bittorrent: None,
        };
        let status = Aria2Status::try_from(raw).expect("valid raw status");
        assert_eq!(status.upload_speed, Some(200));
        assert_eq!(status.uploaded_bytes, Some(400));
        assert_eq!(status.num_seeders, Some(7));
        assert_eq!(status.seeder, Some(true));

        let telemetry = status.torrent_telemetry();
        assert_eq!(telemetry.download_speed_bps, Some(100));
        assert_eq!(telemetry.upload_speed_bps, Some(200));
        assert_eq!(telemetry.downloaded_bytes, Some(500));
        assert_eq!(telemetry.uploaded_bytes, Some(400));
        assert_eq!(telemetry.seed_count, Some(7));
        assert_eq!(telemetry.seeding, Some(true));
        // share_ratio = 400 / 500 = 0.8
        assert!(
            telemetry
                .share_ratio
                .is_some_and(|r| (r - 0.8).abs() < 1e-9)
        );
    }

    #[test]
    fn telemetry_share_ratio_is_none_when_nothing_downloaded() {
        let raw = super::RawStatus {
            gid: "0123456789abcdef".to_owned(),
            status: "active".to_owned(),
            completed_length: "0".to_owned(),
            total_length: "1000".to_owned(),
            download_speed: "0".to_owned(),
            upload_speed: None,
            upload_length: None,
            connections: "0".to_owned(),
            num_seeders: None,
            seeder: None,
            info_hash: None,
            error_code: None,
            bittorrent: None,
        };
        let status = Aria2Status::try_from(raw).expect("valid raw status");
        let telemetry = status.torrent_telemetry();
        assert!(telemetry.share_ratio.is_none());
        assert_eq!(telemetry.upload_speed_bps, None);
        assert_eq!(telemetry.uploaded_bytes, None);
        assert_eq!(telemetry.seed_count, None);
        assert_eq!(telemetry.seeding, None);
        assert_eq!(telemetry.tracker_uris, None);
    }

    #[tokio::test]
    async fn torrent_telemetry_fields_are_parsed_from_mocked_aria2_response()
    -> Result<(), Box<dyn Error>> {
        let gid_str = "0123456789abcdef";
        let response_json = serde_json::json!({
            "gid": gid_str,
            "status": "active",
            "completedLength": "800000",
            "totalLength": "1000000",
            "downloadSpeed": "50000",
            "uploadSpeed": "25000",
            "uploadLength": "100000",
            "connections": "8",
            "numSeeders": "12",
            "seeder": "false",
            "bittorrent": {
                "announceList": [["https://tracker.test/announce"], ["udp://tracker.test:80"]]
            },
            "infoHash": "aabbccddeeff",
            "errorCode": null
        });
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(response_json)]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        let gid = super::Aria2Gid::new(gid_str.to_owned())?;
        let status = client.tell_status(&gid).await?;
        requests.recv().await.ok_or("missing tellStatus request")?;

        let telemetry = status.torrent_telemetry();
        assert_eq!(telemetry.download_speed_bps, Some(50_000));
        assert_eq!(telemetry.upload_speed_bps, Some(25_000));
        assert_eq!(telemetry.downloaded_bytes, Some(800_000));
        assert_eq!(telemetry.uploaded_bytes, Some(100_000));
        assert_eq!(telemetry.peer_count, Some(8));
        assert_eq!(telemetry.seed_count, Some(12));
        assert_eq!(telemetry.seeding, Some(false));
        assert_eq!(
            telemetry.tracker_uris,
            Some(vec![
                "https://tracker.test/announce".to_owned(),
                "udp://tracker.test:80".to_owned(),
            ])
        );
        assert_eq!(status.info_hash.as_deref(), Some("aabbccddeeff"));
        // share_ratio = 100_000 / 800_000 = 0.125
        assert!(
            telemetry
                .share_ratio
                .is_some_and(|r| (r - 0.125).abs() < 1e-9)
        );
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn select_file_indexes_are_converted_to_1_based_and_sent_to_aria2()
    -> Result<(), Box<dyn Error>> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

        let metainfo = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
                         01234567890123456789ee";
        let root = tempfile::tempdir()?;
        let torrent_path = root.path().join("sample.torrent");
        tokio::fs::write(&torrent_path, metainfo).await?;
        let destination = root.path().join("downloads");
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(json!("0123456789abcdef"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;

        // Pass 0-based indexes [0, 2, 4] → should produce "1,3,5".
        client
            .add_torrent_file_with_options(&torrent_path, &destination, None, Some(&[0, 2, 4]))
            .await?;

        let request = requests.recv().await.ok_or("missing addTorrent request")?;
        assert_eq!(request["method"], "aria2.addTorrent");
        assert_eq!(request["params"][3]["select-file"], "1,3,5");
        // Metainfo round-trip is still intact.
        assert_eq!(
            BASE64.decode(request["params"][1].as_str().ok_or("missing metainfo")?)?,
            metainfo
        );
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn select_file_is_absent_when_no_selection_provided() -> Result<(), Box<dyn Error>> {
        let destination = std::path::PathBuf::from(r"C:\downloads\torrent-0123456789abcdef");
        let magnet = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=fixture";
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(json!("0123456789abcdef"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        client
            .add_uri_with_options(magnet, &destination, None, None)
            .await?;
        let request = requests.recv().await.ok_or("missing addUri request")?;
        assert!(
            request["params"][2].get("select-file").is_none(),
            "select-file must not be present without a selection"
        );
        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn select_file_empty_slice_is_omitted() -> Result<(), Box<dyn Error>> {
        let destination = std::path::PathBuf::from(r"C:\downloads\torrent-0123456789abcdef");
        let magnet = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=fixture";
        let (endpoint, mut requests, server) =
            start_server([MockReply::Result(json!("0123456789abcdef"))]).await?;
        let client = make_client(endpoint, Duration::from_secs(1))?;
        client
            .add_uri_with_options(magnet, &destination, None, Some(&[]))
            .await?;
        let request = requests.recv().await.ok_or("missing addUri request")?;
        assert!(
            request["params"][2].get("select-file").is_none(),
            "empty select-file slice should be omitted"
        );
        server.await?;
        Ok(())
    }
}

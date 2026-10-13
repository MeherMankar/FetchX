use std::{fmt, path::Path, sync::Arc, time::Duration};

use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
};
use fetchx_storage::DownloadRepository;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
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
        if uri.len() > MAX_SOURCE_URI_BYTES {
            return Err(Aria2ClientError::InvalidUri);
        }
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
        let options = json!({
            "dir": directory,
            "out": filename,
            "max-connection-per-server": MAX_CONNECTIONS_PER_ARIA2_DOWNLOAD.to_string(),
            "split": "1",
            "allow-overwrite": "false",
            "auto-file-renaming": "false",
        });
        let result: String = match self.call("aria2.addUri", vec![json!([uri]), options]).await {
            Ok(result) => result,
            Err(Aria2ClientError::Rpc { code }) => {
                return Err(Aria2ClientError::Rpc { code });
            }
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
            "connections",
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
    pub connections: usize,
    pub error_code: Option<String>,
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
        let connections = raw
            .connections
            .parse::<usize>()
            .map_err(|_| Aria2ClientError::InvalidResponse)?;
        Ok(Self {
            gid: Aria2Gid::new(raw.gid)?,
            status,
            completed_bytes,
            total_bytes,
            download_speed,
            connections,
            error_code: raw.error_code,
        })
    }
}

fn parse_counter(value: &str) -> Result<u64, Aria2ClientError> {
    value
        .parse::<u64>()
        .map_err(|_| Aria2ClientError::InvalidResponse)
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
    connections: String,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
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
            poll_interval: DEFAULT_POLL_INTERVAL,
            gids: Mutex::new(std::collections::HashMap::new()),
        }
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
        let mut gid = match self.gid_for_generation(id, generation).await {
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
            gid = Some(self.submit_job(id, item, &client, generation).await?);
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
                    gid = self.submit_job(id, item, &client, generation).await?;
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
                            gid = self.submit_job(id, item, &client, generation).await?;
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
                            self.validate_completed_file(item, &status).await?;
                            *item = self.state_machine.transition(id, DownloadStatus::Completed).await?;
                            self.gids.lock().await.remove(&id);
                            return Ok(item.clone());
                        }
                        Aria2DownloadStatus::Error => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            self.gids.lock().await.remove(&id);
                            return Err(DownloadEngineError::Aria2TransferFailed);
                        }
                        Aria2DownloadStatus::Removed => {
                            self.persist_progress(item, &mut expected_updated_at, &status).await?;
                            self.gids.lock().await.remove(&id);
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
    ) -> Result<Aria2Gid, DownloadEngineError> {
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
        let gid = client
            .add_uri(item.source().url(), item.destination())
            .await?;
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
            return Err(DownloadEngineError::StateMachine(
                self.current_state_error(item.id()).await?,
            ));
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

    async fn validate_completed_file(
        &self,
        item: &DownloadItem,
        status: &Aria2Status,
    ) -> Result<(), DownloadEngineError> {
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
    use fetchx_storage::{Database, DownloadRepository};
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{mpsc, watch},
        task::JoinHandle,
    };

    use super::{
        AMBIGUOUS_ADD_URI_DIAGNOSTIC, Aria2Client, Aria2ClientConfig, Aria2ClientError,
        Aria2DownloadStatus, Aria2Engine, Aria2Status, DownloadStatus, EngineKind, Timestamp,
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
            connections: "1".to_owned(),
            error_code: None,
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
            .add_uri("https://example.test/archive", &destination)
            .await?;
        assert_eq!(gid.as_str(), "0123456789abcdef");
        let request = requests.recv().await.ok_or("missing addUri request")?;
        assert_eq!(request["method"], "aria2.addUri");
        assert_eq!(request["params"][1][0], "https://example.test/archive");
        assert_eq!(request["params"][2]["out"], "archive.bin");
        assert_eq!(request["params"][2]["dir"], r"C:\downloads");
        assert_eq!(request["params"][2]["max-connection-per-server"], "1");
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
            let path = std::env::temp_dir()
                .join(format!("fetchx-aria2-{}-{id}", std::process::id()));
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
        engine.submit_job(id, &item, &client, 4).await?;
        engine.submit_job(id, &item, &client, 5).await?;
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
}


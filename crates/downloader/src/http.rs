use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use mydownloader_core::{DownloadStateMachine, StateMachineError};
use mydownloader_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
};
use mydownloader_storage::DownloadRepository;
use reqwest::{Client, Response, Url, header::CONTENT_LENGTH};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
    sync::watch,
};
use tracing::{info, warn};

use crate::{DownloadEngine, DownloadEngineError};

const DEFAULT_USER_AGENT: &str = concat!("MyDownloader/", env!("CARGO_PKG_VERSION"));

/// Timeouts, request identity, and progress checkpoint policy for HTTP downloads.
///
/// By default, progress is persisted after each MiB or one second, whichever comes
/// first. Connection and body-read timeouts default to 15 and 30 seconds.
#[derive(Clone, Debug)]
pub struct HttpEngineConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub progress_bytes_interval: u64,
    pub progress_time_interval: Duration,
    pub user_agent: String,
}

struct BodyContext {
    id: DownloadId,
    item: DownloadItem,
    total_bytes: Option<u64>,
    temporary_path: PathBuf,
}

impl Default for HttpEngineConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            progress_bytes_interval: 1024 * 1024,
            progress_time_interval: Duration::from_secs(1),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

/// Single-stream, resumeless HTTP/HTTPS downloader.
pub struct DirectHttpEngine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    client: Client,
    config: HttpEngineConfig,
}

impl<R> DirectHttpEngine<R>
where
    R: DownloadRepository + Clone,
{
    /// Creates an engine with the default client and progress configuration.
    pub fn new(repository: R) -> Result<Self, DownloadEngineError> {
        Self::with_config(repository, HttpEngineConfig::default())
    }

    /// Creates an engine with caller-selected timeouts and progress checkpoint policy.
    pub fn with_config(
        repository: R,
        config: HttpEngineConfig,
    ) -> Result<Self, DownloadEngineError> {
        if config.connect_timeout.is_zero()
            || config.read_timeout.is_zero()
            || config.progress_bytes_interval == 0
            || config.progress_time_interval.is_zero()
            || config.user_agent.trim().is_empty()
        {
            return Err(DownloadEngineError::InvalidConfiguration);
        }

        let client = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let next = attempt.url();
                let previous = attempt.previous();
                let is_http = matches!(next.scheme(), "http" | "https");
                let has_credentials = !next.username().is_empty() || next.password().is_some();
                let downgrades_tls = previous
                    .last()
                    .is_some_and(|url| url.scheme() == "https" && next.scheme() == "http");

                if previous.len() >= 10 || !is_http || has_credentials || downgrades_tls {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(DownloadEngineError::ClientConfiguration)?;
        let state_machine = DownloadStateMachine::new(repository.clone());

        Ok(Self {
            repository,
            state_machine,
            client,
            config,
        })
    }

    /// Downloads a queued item. Set the watch value to `true` to cancel cooperatively.
    ///
    /// The final destination is never overwritten. Response bytes are streamed into
    /// a sibling `.part` file and published without replacement after validation.
    pub async fn download(
        &self,
        id: DownloadId,
        mut cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        if self.repository.get(id).await?.is_none() {
            return Err(DownloadEngineError::NotFound(id));
        }

        self.state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;

        let result = self.download_prepared(id, &mut cancellation).await;
        match result {
            Ok(item) => Ok(item),
            Err(error) if error.is_cancelled() => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "download cancelled");
                Err(error)
            }
            Err(error) => {
                if let Err(state) = self.persist_failure(id, &error).await {
                    return Err(DownloadEngineError::FailurePersistence {
                        operation: Box::new(error),
                        state,
                    });
                }
                warn!(download_id = %id, "download failed");
                Err(error)
            }
        }
    }

    async fn download_prepared(
        &self,
        id: DownloadId,
        cancellation: &mut watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let mut item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        if item.source().engine() != EngineKind::DirectHttp {
            return Err(DownloadEngineError::EngineMismatch);
        }
        let url = parse_http_url(item.source().url())?;
        let destination = item.destination().to_path_buf();
        if fs::try_exists(&destination)
            .await
            .map_err(DownloadEngineError::Io)?
        {
            return Err(DownloadEngineError::DestinationExists);
        }
        let temporary_path = temporary_path(&destination);

        let initial_progress = DownloadProgress::default();
        item.set_progress_at(initial_progress, next_timestamp(item.updated_at())?)?;
        self.persist_if_status(&item, DownloadStatus::Preparing)
            .await?;

        info!(download_id = %id, "starting HTTP download");
        let response = self.send_request(url, cancellation).await?;
        let status = response.status();
        if !status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(DownloadEngineError::HttpStatus(status.as_u16()));
        }
        let total_bytes = parse_content_length(&response)?;

        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .await
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(DownloadEngineError::TemporaryFileExists);
            }
            Err(error) => return Err(DownloadEngineError::Io(error)),
        };

        self.receive_body(
            response,
            file,
            BodyContext {
                id,
                item,
                total_bytes,
                temporary_path,
            },
            cancellation,
        )
        .await
    }

    async fn send_request(
        &self,
        url: Url,
        cancellation: &mut watch::Receiver<bool>,
    ) -> Result<Response, DownloadEngineError> {
        loop {
            if *cancellation.borrow_and_update() {
                return Err(DownloadEngineError::Cancelled);
            }

            tokio::select! {
                changed = cancellation.changed() => {
                    if changed.is_err() || *cancellation.borrow_and_update() {
                        return Err(DownloadEngineError::Cancelled);
                    }
                }
                response = self.client.get(url.clone()).send() => {
                    return response.map_err(DownloadEngineError::Request);
                }
            }
        }
    }

    async fn receive_body(
        &self,
        mut response: Response,
        mut file: File,
        context: BodyContext,
        cancellation: &mut watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let BodyContext {
            id,
            mut item,
            total_bytes,
            temporary_path,
        } = context;
        let mut downloading_started = false;
        let result = async {
            item = self
                .state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
            downloading_started = true;
            item.set_progress_at(
                DownloadProgress::new(0, total_bytes, None, None)?,
                next_timestamp(item.updated_at())?,
            )?;
            self.persist_if_status(&item, DownloadStatus::Downloading)
                .await?;

            let mut downloaded_bytes = 0_u64;
            let mut last_persisted_bytes = 0_u64;
            let mut last_persisted_at = Instant::now();

            loop {
                let next_chunk = loop {
                    if *cancellation.borrow_and_update() {
                        self.persist_if_status(&item, DownloadStatus::Downloading)
                            .await?;
                        return Err(DownloadEngineError::Cancelled);
                    }

                    tokio::select! {
                        changed = cancellation.changed() => {
                            if changed.is_err() || *cancellation.borrow_and_update() {
                                self.persist_if_status(&item, DownloadStatus::Downloading)
                                    .await?;
                                return Err(DownloadEngineError::Cancelled);
                            }
                        }
                        chunk = response.chunk() => {
                            break chunk.map_err(DownloadEngineError::Request)?;
                        }
                    }
                };

                let Some(chunk) = next_chunk else {
                    break;
                };
                let chunk_size = u64::try_from(chunk.len())
                    .map_err(|_| DownloadEngineError::ByteCountOverflow)?;
                let next_downloaded = downloaded_bytes
                    .checked_add(chunk_size)
                    .ok_or(DownloadEngineError::ByteCountOverflow)?;
                if let Some(expected) = total_bytes.filter(|total| next_downloaded > *total) {
                    return Err(DownloadEngineError::ContentLengthExceeded { expected });
                }

                file.write_all(&chunk)
                    .await
                    .map_err(DownloadEngineError::Io)?;
                downloaded_bytes = next_downloaded;
                item.set_progress_at(
                    DownloadProgress::new(downloaded_bytes, total_bytes, None, None)?,
                    next_timestamp(item.updated_at())?,
                )?;

                let now = Instant::now();
                if downloaded_bytes.saturating_sub(last_persisted_bytes)
                    >= self.config.progress_bytes_interval
                    || now.duration_since(last_persisted_at) >= self.config.progress_time_interval
                {
                    self.persist_if_status(&item, DownloadStatus::Downloading)
                        .await?;
                    last_persisted_bytes = downloaded_bytes;
                    last_persisted_at = now;
                }
            }

            if let Some(expected) = total_bytes {
                if downloaded_bytes != expected {
                    return Err(DownloadEngineError::IncompleteResponse {
                        expected,
                        actual: downloaded_bytes,
                    });
                }
            }

            if *cancellation.borrow_and_update() {
                self.persist_if_status(&item, DownloadStatus::Downloading)
                    .await?;
                return Err(DownloadEngineError::Cancelled);
            }
            file.flush().await.map_err(DownloadEngineError::Io)?;
            file.sync_all().await.map_err(DownloadEngineError::Io)?;
            self.persist_if_status(&item, DownloadStatus::Downloading)
                .await?;
            Ok(())
        }
        .await;

        drop(file);
        match result {
            Ok(()) => {
                if *cancellation.borrow_and_update() {
                    return Err(cleanup_temporary_file(
                        &temporary_path,
                        DownloadEngineError::Cancelled,
                    )
                    .await);
                }
                if let Err(error) =
                    publish_without_replacement(&temporary_path, item.destination()).await
                {
                    return Err(cleanup_temporary_file(&temporary_path, error).await);
                }

                let completed = self
                    .state_machine
                    .transition(id, DownloadStatus::Completed)
                    .await?;
                info!(download_id = %id, bytes = item.progress().downloaded_bytes(), "download completed");
                Ok(completed)
            }
            Err(error) => {
                let operation = if downloading_started {
                    self.persist_if_status(&item, DownloadStatus::Downloading)
                        .await
                        .err()
                        .unwrap_or(error)
                } else {
                    error
                };
                Err(cleanup_temporary_file(&temporary_path, operation).await)
            }
        }
    }

    async fn persist_if_status(
        &self,
        item: &DownloadItem,
        expected_status: DownloadStatus,
    ) -> Result<(), DownloadEngineError> {
        if self
            .repository
            .update_if_status_current(item, expected_status)
            .await?
        {
            return Ok(());
        }

        Err(DownloadEngineError::StateMachine(
            self.current_state_error(item.id(), expected_status).await?,
        ))
    }

    async fn current_state_error(
        &self,
        id: DownloadId,
        expected: DownloadStatus,
    ) -> Result<StateMachineError, DownloadEngineError> {
        match self.repository.get(id).await? {
            None => Ok(StateMachineError::NotFound(id)),
            Some(item) => Ok(StateMachineError::ConcurrentTransition {
                from: expected,
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

    async fn persist_failure(
        &self,
        id: DownloadId,
        operation: &DownloadEngineError,
    ) -> Result<(), StateMachineError> {
        let current = self
            .repository
            .get(id)
            .await
            .map_err(StateMachineError::Storage)?
            .ok_or(StateMachineError::NotFound(id))?;
        if !matches!(
            current.status(),
            DownloadStatus::Preparing | DownloadStatus::Downloading
        ) {
            return Ok(());
        }

        let domain_error = operation
            .persisted_error()
            .map_err(StateMachineError::DomainError)?;
        self.state_machine
            .transition_with_error(id, DownloadStatus::Failed, domain_error)
            .await
            .map(|_| ())
    }
}

impl<R> DownloadEngine for DirectHttpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        DirectHttpEngine::download(self, id, cancellation).await
    }
}

fn parse_http_url(source: &str) -> Result<Url, DownloadEngineError> {
    if source.trim().is_empty() {
        return Err(DownloadEngineError::InvalidUrl);
    }

    let url = Url::parse(source).map_err(|_| DownloadEngineError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DownloadEngineError::UnsupportedScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DownloadEngineError::EmbeddedCredentials);
    }

    Ok(url)
}

fn parse_content_length(response: &Response) -> Result<Option<u64>, DownloadEngineError> {
    response
        .headers()
        .get(CONTENT_LENGTH)
        .map(|header| {
            header
                .to_str()
                .ok()
                .and_then(|value| value.parse().ok())
                .ok_or(DownloadEngineError::InvalidContentLength)
        })
        .transpose()
}

fn next_timestamp(previous: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    Ok(Timestamp::now()?.max(previous))
}

fn temporary_path(destination: &Path) -> PathBuf {
    let mut name: OsString = destination.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

async fn publish_without_replacement(
    temporary: &Path,
    destination: &Path,
) -> Result<(), DownloadEngineError> {
    match fs::hard_link(temporary, destination).await {
        Ok(()) => {
            fs::remove_file(temporary)
                .await
                .map_err(DownloadEngineError::Io)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(DownloadEngineError::DestinationExists)
        }
        Err(error) => Err(DownloadEngineError::Io(error)),
    }
}

async fn cleanup_temporary_file(
    temporary: &Path,
    operation: DownloadEngineError,
) -> DownloadEngineError {
    match fs::remove_file(temporary).await {
        Ok(()) => operation,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => operation,
        Err(cleanup) => DownloadEngineError::Cleanup {
            operation: Box::new(operation),
            cleanup,
        },
    }
}

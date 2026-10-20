#![forbid(unsafe_code)]
//! FTP and FTPS download engine.
//!
//! Uses `suppaftp 6.3.0` which provides a **synchronous** API.  The sync
//! work is bridged into the tokio async world via `tokio::task::spawn_blocking`.
//!
//! # Security
//! - Paths containing `..` components are rejected with
//!   [`DownloadEngineError::FtpPathTraversal`].
//! - Embedded passwords in the URL are rejected with
//!   [`DownloadEngineError::EmbeddedCredentials`].
//! - Credentials are never logged.
//!
//! # Resume
//! If a `.part` file already exists at the destination, the engine issues a
//! `REST <offset>` before `RETR` to resume from where it left off.

use std::{
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadStatus, EngineKind, Timestamp,
};
use fetchx_storage::DownloadRepository;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::{
    BandwidthControl, DownloadEngine, DownloadEngineError, ManagedDownloadControl,
    ManagedDownloadEngine,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_millis(500);
/// Read-loop chunk size: 64 KiB.
const CHUNK_SIZE: usize = 64 * 1024;
const DEFAULT_FTP_PORT: u16 = 21;

// ── Config ─────────────────────────────────────────────────────────────────────

/// Tunable parameters for the FTP / FTPS engine.
#[derive(Clone, Debug)]
pub struct FtpEngineConfig {
    /// TCP connect timeout. Default: 15 s.
    pub connect_timeout: Duration,
    /// TCP read timeout. Default: 30 s.
    pub read_timeout: Duration,
    /// Use PASV (passive) mode. Default: `true`.
    pub passive_mode: bool,
}

impl Default for FtpEngineConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            passive_mode: true,
        }
    }
}

// ── Engine ─────────────────────────────────────────────────────────────────────

/// FTP / FTPS download engine backed by `suppaftp 6.3.0` (sync API via
/// `tokio::task::spawn_blocking`).
pub struct FtpEngine<R> {
    repository: R,
    state_machine: DownloadStateMachine<R>,
    config: FtpEngineConfig,
}

impl<R> FtpEngine<R>
where
    R: DownloadRepository + Clone,
{
    /// Creates an engine with the default configuration.
    pub fn new(repository: R) -> Self {
        Self::with_config(repository, FtpEngineConfig::default())
    }

    /// Creates an engine with a caller-supplied configuration.
    pub fn with_config(repository: R, config: FtpEngineConfig) -> Self {
        Self {
            state_machine: DownloadStateMachine::new(repository.clone()),
            repository,
            config,
        }
    }

    // ── Core logic ─────────────────────────────────────────────────────────────

    async fn download_inner(
        &self,
        id: DownloadId,
        mut control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        // Load item and enforce EngineMismatch guard.
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;

        match item.source().engine() {
            EngineKind::Ftp | EngineKind::Ftps => {}
            _ => return Err(DownloadEngineError::EngineMismatch),
        }

        // State transition.
        let item = match item.status() {
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

        let result = self.run(id, &item, &mut control).await;

        match result {
            Ok(completed) => Ok(completed),
            Err(DownloadEngineError::Cancelled) => {
                self.transition_to_cancelled(id).await?;
                info!(download_id = %id, "FTP download cancelled");
                Err(DownloadEngineError::Cancelled)
            }
            Err(DownloadEngineError::Paused) => {
                self.transition_to_paused(id).await?;
                info!(download_id = %id, "FTP download paused");
                Err(DownloadEngineError::Paused)
            }
            Err(error) => {
                if let Err(e) = self.persist_failure(id, &error).await {
                    warn!(download_id = %id, %e, "FTP: could not persist failure state");
                }
                warn!(download_id = %id, %error, "FTP download failed");
                Err(error)
            }
        }
    }

    async fn run(
        &self,
        id: DownloadId,
        item: &DownloadItem,
        control: &mut ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        // Early cooperative check.
        if *control.cancellation.borrow_and_update() {
            return Err(DownloadEngineError::Cancelled);
        }
        if *control.pause.borrow_and_update() {
            return Err(DownloadEngineError::Paused);
        }

        let url_str = item.source().url().to_owned();
        let use_tls = item.source().engine() == EngineKind::Ftps;
        let destination = item.destination().to_path_buf();
        let tmp = temporary_path(&destination);

        let (host, port, path, username) = parse_ftp_url(&url_str)?;

        let resume_offset: u64 = std::fs::metadata(&tmp)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .unwrap_or(0);

        // Transition to Downloading.
        let mut tracked = if item.status() == DownloadStatus::Preparing {
            self.state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?
        } else {
            item.clone()
        };

        // Persist initial progress.
        let initial = DownloadProgress::new(resume_offset, None, None, None)?;
        tracked.set_progress_at(initial, next_ts(tracked.updated_at())?)?;
        self.repository
            .update(&tracked)
            .await
            .map_err(DownloadEngineError::Storage)?;

        // Shared atomic flags for bridging cancel/pause into spawn_blocking.
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let pause_flag = Arc::new(AtomicBool::new(false));

        // Progress channel: blocking thread → async driver.
        let (progress_tx, mut progress_rx) = mpsc::channel::<(u64, Option<u64>)>(64);

        let cancel_sync = Arc::clone(&cancel_flag);
        let pause_sync = Arc::clone(&pause_flag);
        let tmp_sync = tmp.clone();
        let dest_sync = destination.clone();
        let connect_timeout = self.config.connect_timeout;
        let read_timeout = self.config.read_timeout;

        let ftp_task = tokio::task::spawn_blocking(move || {
            ftp_blocking(BlockingParams {
                host,
                port,
                path,
                username,
                use_tls,
                connect_timeout,
                read_timeout,
                resume_offset,
                temporary_path: tmp_sync,
                destination: dest_sync,
                cancel_flag: cancel_sync,
                pause_flag: pause_sync,
                progress_tx,
            })
        });

        // Async driver: forward flags, persist progress.
        let bandwidth = control.bandwidth.clone();
        let mut last_persist = tokio::time::Instant::now();
        let mut last_speed_at = Instant::now();
        let mut last_speed_bytes: u64 = resume_offset;
        let mut speed: Option<u64> = None;
        let mut total_bytes: Option<u64> = None;
        // `downloaded` will be set from the first progress message; resume_offset
        // is the logical start but the first blocking_send will carry the accurate count.
        #[allow(unused_assignments)]
        let mut downloaded: u64 = resume_offset;

        let outcome: Result<(), DownloadEngineError> = loop {
            tokio::select! {
                biased;

                changed = control.cancellation.changed() => {
                    if changed.is_err() || *control.cancellation.borrow_and_update() {
                        cancel_flag.store(true, Ordering::Release);
                        let _ = ftp_task.await;
                        break Err(DownloadEngineError::Cancelled);
                    }
                }

                changed = control.pause.changed() => {
                    if changed.is_err() || *control.pause.borrow() {
                        pause_flag.store(true, Ordering::Release);
                        let _ = ftp_task.await;
                        break Err(DownloadEngineError::Paused);
                    }
                }

                msg = progress_rx.recv() => {
                    match msg {
                        Some((new_dl, new_total)) => {
                            downloaded = new_dl;
                            if new_total.is_some() {
                                total_bytes = new_total;
                            }

                            let _ = bandwidth.acquire(
                                CHUNK_SIZE as u64,
                                control.cancellation.clone(),
                                control.pause.clone(),
                            ).await;

                            let now = Instant::now();
                            let elapsed = now.duration_since(last_speed_at);
                            if elapsed >= Duration::from_secs(1) {
                                let delta = downloaded.saturating_sub(last_speed_bytes);
                                speed = compute_speed(delta, elapsed);
                                last_speed_bytes = downloaded;
                                last_speed_at = now;
                            }

                            let tok_now = tokio::time::Instant::now();
                            if tok_now.duration_since(last_persist) >= PROGRESS_PERSIST_INTERVAL {
                                let eta = total_bytes.and_then(|t| eta_secs(t, downloaded, speed));
                                let progress =
                                    DownloadProgress::new(downloaded, total_bytes, speed, eta)?;
                                tracked.set_progress_at(progress, next_ts(tracked.updated_at())?)?;
                                if let Err(e) = self.repository.update(&tracked).await {
                                    warn!(download_id = %id, %e, "FTP: progress persist error");
                                }
                                last_persist = tok_now;
                            }
                        }

                        None => {
                            match ftp_task.await {
                                Ok(Ok(())) => break Ok(()),
                                Ok(Err(e)) => break Err(e),
                                Err(join_err) => break Err(DownloadEngineError::FtpError(
                                    format!("spawn_blocking panicked: {join_err}"),
                                )),
                            }
                        }
                    }
                }
            }
        };

        outcome?;

        if !destination.exists() {
            return Err(DownloadEngineError::FtpError(
                "FTP transfer completed but destination file is missing".to_owned(),
            ));
        }

        // Final progress snapshot.
        if let Some(total) = total_bytes {
            let progress = DownloadProgress::new(total, Some(total), Some(0), Some(0))?;
            tracked.set_progress_at(progress, next_ts(tracked.updated_at())?)?;
            if let Err(e) = self.repository.update(&tracked).await {
                warn!(download_id = %id, %e, "FTP: final progress persist error");
            }
        }

        let completed = self
            .state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        info!(download_id = %id, "FTP download completed");

        if tmp.exists() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }

        Ok(completed)
    }

    // ── State helpers ──────────────────────────────────────────────────────────

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
            Err(e) => Err(e.into()),
        }
    }

    async fn transition_to_paused(&self, id: DownloadId) -> Result<(), DownloadEngineError> {
        match self
            .state_machine
            .transition(id, DownloadStatus::Paused)
            .await
        {
            Ok(_) => Ok(()),
            Err(StateMachineError::InvalidTransition {
                from: DownloadStatus::Paused,
                to: DownloadStatus::Paused,
            }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn persist_failure(
        &self,
        id: DownloadId,
        op: &DownloadEngineError,
    ) -> Result<(), DownloadEngineError> {
        let error_repr = match op.persisted_error() {
            Ok(e) => e,
            Err(_) => fetchx_models::DownloadError::new(
                fetchx_models::DownloadErrorCode::Engine,
                "The FTP download failed.",
            )
            .map_err(DownloadEngineError::DomainError)?,
        };
        for _ in 0..3 {
            let current = match self.repository.get(id).await? {
                Some(item) => item,
                None => return Ok(()),
            };
            if !matches!(
                current.status(),
                DownloadStatus::Preparing | DownloadStatus::Downloading
            ) {
                return Ok(());
            }
            match self
                .state_machine
                .transition_with_error(id, DownloadStatus::Failed, error_repr.clone())
                .await
            {
                Ok(_) => return Ok(()),
                Err(StateMachineError::ConcurrentTransition { .. }) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

// ── Trait impls ────────────────────────────────────────────────────────────────

impl<R> DownloadEngine for FtpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let (_pause_tx, pause) = watch::channel(false);
        let (_limit_tx, connection_limit) = watch::channel(1_usize);
        self.download_inner(
            id,
            ManagedDownloadControl {
                cancellation,
                pause,
                connection_limit,
                resource_demand: None,
                bandwidth: BandwidthControl::default(),
            },
        )
        .await
    }
}

impl<R> ManagedDownloadEngine for FtpEngine<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        self.download_inner(id, control).await
    }

    fn supports_active_pause(&self, _engine: EngineKind) -> bool {
        true
    }
}

// ── Blocking FTP work (runs on a threadpool thread) ────────────────────────────

struct BlockingParams {
    host: String,
    port: u16,
    path: String,
    username: Option<String>,
    use_tls: bool,
    connect_timeout: Duration,
    read_timeout: Duration,
    resume_offset: u64,
    temporary_path: PathBuf,
    destination: PathBuf,
    cancel_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    progress_tx: mpsc::Sender<(u64, Option<u64>)>,
}

/// Entry point for the blocking FTP/FTPS transfer. Dispatches to the plain or
/// TLS path, which both perform the same login → transfer → finalise sequence.
fn ftp_blocking(p: BlockingParams) -> Result<(), DownloadEngineError> {
    let addr = format!("{}:{}", p.host, p.port)
        .parse::<std::net::SocketAddr>()
        .map_err(|_| DownloadEngineError::InvalidUrl)?;

    let tcp = TcpStream::connect_timeout(&addr, p.connect_timeout)
        .map_err(|e| DownloadEngineError::FtpError(format!("connect: {e}")))?;
    tcp.set_read_timeout(Some(p.read_timeout))
        .map_err(DownloadEngineError::Io)?;

    if p.use_tls {
        ftp_blocking_tls(tcp, p)
    } else {
        ftp_blocking_plain(tcp, p)
    }
}

// ── Common transfer loop ───────────────────────────────────────────────────────

/// Runs the RETR loop shared by both the plain and TLS paths.
///
/// `data` is the data-connection reader returned by `retr_as_stream`.
/// `finalise_fn` is called after reading completes to send the FTP 226 / 250
/// acknowledgement — this must drop `data` before reading the response.
fn transfer_loop<D, F>(
    data: D,
    mut finalise_fn: F,
    p: &BlockingParams,
    total_bytes: Option<u64>,
) -> Result<(), DownloadEngineError>
where
    D: Read,
    F: FnMut(D) -> Result<(), DownloadEngineError>,
{
    // Open (or append to) the .part file.
    let mut file = if p.resume_offset > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p.temporary_path)
            .map_err(DownloadEngineError::Io)?
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&p.temporary_path)
            .map_err(DownloadEngineError::Io)?
    };

    let mut downloaded: u64 = p.resume_offset;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut data = data;

    loop {
        if p.cancel_flag.load(Ordering::Acquire) {
            return Err(DownloadEngineError::Cancelled);
        }
        if p.pause_flag.load(Ordering::Acquire) {
            return Err(DownloadEngineError::Paused);
        }

        let n = match data.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(DownloadEngineError::Io(e)),
        };

        file.write_all(&buf[..n]).map_err(DownloadEngineError::Io)?;
        downloaded = downloaded.saturating_add(n as u64);
        let _ = p.progress_tx.blocking_send((downloaded, total_bytes));
    }

    file.flush().map_err(DownloadEngineError::Io)?;
    drop(file);

    // Finalise the RETR command (sends the FTP 226 acknowledgement).
    finalise_fn(data)?;

    // Atomic publish: rename .part → destination.
    publish_part_file(&p.temporary_path, &p.destination)
}

// ── Plain FTP path ─────────────────────────────────────────────────────────────

fn ftp_blocking_plain(tcp: TcpStream, p: BlockingParams) -> Result<(), DownloadEngineError> {
    use suppaftp::FtpStream;

    let mut stream = FtpStream::connect_with_stream(tcp)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    let user = p.username.as_deref().unwrap_or("anonymous");
    stream
        .login(user, "")
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;
    stream
        .transfer_type(suppaftp::types::FileType::Binary)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    let total_bytes: Option<u64> = stream.size(&p.path).ok().map(|s| s as u64);

    if p.resume_offset > 0 {
        stream
            .resume_transfer(p.resume_offset as usize)
            .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;
    }

    let data = stream
        .retr_as_stream(&p.path)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    // The finalise closure captures `stream` by mutable reference; however,
    // we need to move `data` out of scope before using `stream` again.
    // Use a raw-pointer trick-free approach: just collect into a Vec so the
    // data reader is dropped before finalize is called.
    let result = transfer_loop(
        data,
        |d| {
            stream
                .finalize_retr_stream(d)
                .map_err(|e| DownloadEngineError::FtpError(e.to_string()))
        },
        &p,
        total_bytes,
    );

    let _ = stream.quit();
    result
}

// ── FTPS path ─────────────────────────────────────────────────────────────────

fn ftp_blocking_tls(tcp: TcpStream, p: BlockingParams) -> Result<(), DownloadEngineError> {
    use suppaftp::{RustlsConnector, RustlsFtpStream};

    let root_store = {
        let mut store = suppaftp::rustls::RootCertStore::empty();
        store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        store
    };
    let tls_config = suppaftp::rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = RustlsConnector::from(std::sync::Arc::new(tls_config));

    // RustlsFtpStream = ImplFtpStream<RustlsStream>, so into_secure with
    // RustlsConnector (Stream = RustlsStream) satisfies the TlsConnector<Stream=T> bound.
    let plain = RustlsFtpStream::connect_with_stream(tcp)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    let mut stream = plain
        .into_secure(connector, &p.host)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    let user = p.username.as_deref().unwrap_or("anonymous");
    stream
        .login(user, "")
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;
    stream
        .transfer_type(suppaftp::types::FileType::Binary)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    let total_bytes: Option<u64> = stream.size(&p.path).ok().map(|s| s as u64);

    if p.resume_offset > 0 {
        stream
            .resume_transfer(p.resume_offset as usize)
            .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;
    }

    let data = stream
        .retr_as_stream(&p.path)
        .map_err(|e| DownloadEngineError::FtpError(e.to_string()))?;

    let result = transfer_loop(
        data,
        |d| {
            stream
                .finalize_retr_stream(d)
                .map_err(|e| DownloadEngineError::FtpError(e.to_string()))
        },
        &p,
        total_bytes,
    );

    let _ = stream.quit();
    result
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Returns the `.part` sibling path of `destination`.
pub(crate) fn temporary_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

fn publish_part_file(tmp: &Path, dest: &Path) -> Result<(), DownloadEngineError> {
    if dest.exists() {
        return Err(DownloadEngineError::DestinationExists);
    }
    std::fs::rename(tmp, dest).map_err(DownloadEngineError::Io)
}

fn compute_speed(bytes: u64, elapsed: Duration) -> Option<u64> {
    let nanos = elapsed.as_nanos();
    if nanos == 0 {
        return None;
    }
    Some(
        u128::from(bytes)
            .saturating_mul(1_000_000_000)
            .checked_div(nanos)?
            .min(u128::from(u64::MAX)) as u64,
    )
}

fn eta_secs(total: u64, downloaded: u64, speed: Option<u64>) -> Option<u64> {
    let s = speed.filter(|&s| s > 0)?;
    Some(total.saturating_sub(downloaded) / s)
}

fn next_ts(current: Timestamp) -> Result<Timestamp, DownloadEngineError> {
    Timestamp::now()
        .map_err(DownloadEngineError::Timestamp)
        .map(|now| now.max(current))
}

// ── URL parsing ────────────────────────────────────────────────────────────────

/// Parse and validate an `ftp://` or `ftps://` URL.
///
/// Returns `(host, port, path, Option<username>)`.
///
/// # Errors
/// - [`DownloadEngineError::EmbeddedCredentials`] — URL has a password component.
/// - [`DownloadEngineError::FtpPathTraversal`] — URL path contains `..` segment.
/// - [`DownloadEngineError::UnsupportedScheme`] — scheme is not `ftp`/`ftps`.
/// - [`DownloadEngineError::InvalidUrl`] — malformed URL or empty/root-only path.
pub(crate) fn parse_ftp_url(
    url: &str,
) -> Result<(String, u16, String, Option<String>), DownloadEngineError> {
    // Reject path traversal sequences in the raw URL *before* parsing, since
    // URL parsers may normalize away `..` components.
    if url.split('/').any(|c| c == "..") {
        return Err(DownloadEngineError::FtpPathTraversal);
    }

    let parsed = reqwest::Url::parse(url).map_err(|_| DownloadEngineError::InvalidUrl)?;

    match parsed.scheme() {
        "ftp" | "ftps" => {}
        _ => return Err(DownloadEngineError::UnsupportedScheme),
    }

    if parsed.password().is_some() {
        return Err(DownloadEngineError::EmbeddedCredentials);
    }

    let host = parsed
        .host_str()
        .ok_or(DownloadEngineError::InvalidUrl)?
        .to_owned();

    let port = parsed.port().unwrap_or(DEFAULT_FTP_PORT);

    let path = parsed.path().to_owned();
    if path.is_empty() || path == "/" {
        return Err(DownloadEngineError::InvalidUrl);
    }

    let username = if parsed.username().is_empty() {
        None
    } else {
        Some(parsed.username().to_owned())
    };

    Ok((host, port, path, username))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use fetchx_models::{
        DownloadId, DownloadItem, DownloadSource, EngineKind, FileName, Timestamp,
    };
    use fetchx_storage::{Database, DownloadRepository};

    use super::{FtpEngine, parse_ftp_url};
    use crate::DownloadEngineError;

    // ── URL parsing ──────────────────────────────────────────────────────────

    #[test]
    fn ftp_url_parse_full() {
        let (host, port, path, user) =
            parse_ftp_url("ftp://alice@files.example.com:2121/pub/data.bin").unwrap();
        assert_eq!(host, "files.example.com");
        assert_eq!(port, 2121);
        assert_eq!(path, "/pub/data.bin");
        assert_eq!(user, Some("alice".to_owned()));
    }

    #[test]
    fn ftp_url_parse_default_port_anonymous() {
        let (host, port, path, user) = parse_ftp_url("ftp://ftp.example.com/file.bin").unwrap();
        assert_eq!(host, "ftp.example.com");
        assert_eq!(port, 21);
        assert_eq!(path, "/file.bin");
        assert!(user.is_none());
    }

    #[test]
    fn ftps_url_parse() {
        let (host, port, path, _) =
            parse_ftp_url("ftps://secure.example.com/private/archive.zip").unwrap();
        assert_eq!(host, "secure.example.com");
        assert_eq!(port, 21);
        assert_eq!(path, "/private/archive.zip");
    }

    #[test]
    fn ftp_url_rejects_embedded_password() {
        assert!(matches!(
            parse_ftp_url("ftp://user:secret@host/file.bin"),
            Err(DownloadEngineError::EmbeddedCredentials)
        ));
    }

    #[test]
    fn ftp_url_rejects_path_traversal() {
        assert!(matches!(
            parse_ftp_url("ftp://host/pub/../../../etc/passwd"),
            Err(DownloadEngineError::FtpPathTraversal)
        ));
    }

    #[test]
    fn ftp_url_rejects_wrong_scheme() {
        assert!(matches!(
            parse_ftp_url("http://host/file.bin"),
            Err(DownloadEngineError::UnsupportedScheme)
        ));
    }

    #[test]
    fn ftp_url_rejects_empty_path() {
        assert!(matches!(
            parse_ftp_url("ftp://host/"),
            Err(DownloadEngineError::InvalidUrl)
        ));
    }

    // ── EngineMismatch guard ─────────────────────────────────────────────────

    #[tokio::test]
    async fn ftp_engine_mismatch_guard() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        let repo = db.downloads();
        let engine = FtpEngine::new(repo.clone());

        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file.bin", EngineKind::DirectHttp)?,
            FileName::new("file.bin")?,
            PathBuf::from("/tmp/file.bin"),
            Timestamp::from_unix_millis(1),
        );
        let id = item.id();
        repo.create(&item).await?;

        let (_tx, rx) = tokio::sync::watch::channel(false);
        let result = crate::DownloadEngine::download(&engine, id, rx).await;
        assert!(matches!(result, Err(DownloadEngineError::EngineMismatch)));
        Ok(())
    }

    #[tokio::test]
    async fn ftp_engine_not_found_for_unknown_id() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        let engine = FtpEngine::new(db.downloads());
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let result = crate::DownloadEngine::download(&engine, DownloadId::new(), rx).await;
        assert!(matches!(result, Err(DownloadEngineError::NotFound(_))));
        Ok(())
    }

    #[tokio::test]
    async fn ftps_engine_mismatch_guard_rejects_non_ftp() -> Result<(), Box<dyn Error>> {
        let db = Database::connect("sqlite::memory:").await?;
        let repo = db.downloads();
        let engine = FtpEngine::new(repo.clone());

        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/video.mp4", EngineKind::YtDlp)?,
            FileName::new("video.mp4")?,
            PathBuf::from("/tmp/video.mp4"),
            Timestamp::from_unix_millis(1),
        );
        let id = item.id();
        repo.create(&item).await?;

        let (_tx, rx) = tokio::sync::watch::channel(false);
        let result = crate::DownloadEngine::download(&engine, id, rx).await;
        assert!(matches!(result, Err(DownloadEngineError::EngineMismatch)));
        Ok(())
    }

    /// Full FTP download against an in-process minimal FTP server.
    ///
    /// Run with: `cargo test -p fetchx-downloader -- --ignored ftp_integration_download`
    #[tokio::test]
    #[ignore = "requires local FTP server fixture"]
    async fn ftp_integration_download() -> Result<(), Box<dyn Error>> {
        use crate::tests::start_ftp_server;

        let content = b"Hello from FTP!".to_vec();
        let (addr, _server) = start_ftp_server(content.clone()).await?;

        let db = Database::connect("sqlite::memory:").await?;
        let repo = db.downloads();
        let engine = FtpEngine::new(repo.clone());

        let dir = tempfile::tempdir()?;
        let dest = dir.path().join("result.bin");
        let url = format!("ftp://{addr}/file.bin");

        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(&url, EngineKind::Ftp)?,
            FileName::new("result.bin")?,
            dest.clone(),
            Timestamp::from_unix_millis(1),
        );
        let id = item.id();
        repo.create(&item).await?;

        let (_tx, rx) = tokio::sync::watch::channel(false);
        let completed = crate::DownloadEngine::download(&engine, id, rx).await?;

        assert_eq!(completed.status(), fetchx_models::DownloadStatus::Completed);
        assert_eq!(tokio::fs::read(&dest).await?, content);
        Ok(())
    }
}

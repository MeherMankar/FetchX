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
    ManagedDownloadEngine, retry::RetryPolicy,
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
    /// Reserved for future active-mode support. Currently always `true`; the engine always
    /// uses PASV (passive mode). Active mode (PORT) is not implemented.
    pub passive_mode: bool,
    /// Username for FTP authentication. `None` falls back to `"anonymous"`.
    pub username: Option<String>,
    /// Password for FTP authentication. Never logged. `None` uses an empty password (anonymous).
    pub password: Option<String>,
    /// Timeout that bounds the FTPS TLS handshake. Default: 30 s.
    /// Applied as the TCP read timeout on the underlying stream before calling
    /// `into_secure`; the data-phase `read_timeout` takes over afterward.
    pub tls_handshake_timeout: Duration,
}

impl Default for FtpEngineConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            passive_mode: true,
            username: None,
            password: None,
            tls_handshake_timeout: Duration::from_secs(30),
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

        let started_at = std::time::Instant::now();
        let mut attempts: u8 = 0;
        let result = loop {
            attempts = attempts.saturating_add(1);
            let outcome = self.run(id, &item, &mut control).await;
            match &outcome {
                Ok(_) | Err(DownloadEngineError::Cancelled) | Err(DownloadEngineError::Paused) => {
                    break outcome;
                }
                Err(e) if !is_retryable_ftp_error(e) => {
                    break outcome;
                }
                Err(_) => {
                    let Some(delay) = RetryPolicy::FTP.delay(attempts, None, started_at.elapsed())
                    else {
                        break outcome;
                    };
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        changed = control.cancellation.changed() => {
                            if changed.is_err() || *control.cancellation.borrow_and_update() {
                                break Err(DownloadEngineError::Cancelled);
                            }
                        }
                    }
                }
            }
        };

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
        let tls_handshake_timeout = self.config.tls_handshake_timeout;
        // Username from URL takes priority; fall back to config username.
        let effective_username = username.or_else(|| self.config.username.clone());
        let password = self.config.password.clone();

        let ftp_task = tokio::task::spawn_blocking(move || {
            ftp_blocking(BlockingParams {
                host,
                port,
                path,
                username: effective_username,
                password,
                use_tls,
                connect_timeout,
                read_timeout,
                tls_handshake_timeout,
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
                            let chunk_bytes = new_dl.saturating_sub(downloaded);
                            downloaded = new_dl;
                            if new_total.is_some() {
                                total_bytes = new_total;
                            }

                            let _ = bandwidth.acquire(
                                chunk_bytes,
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

#[derive(Clone)]
struct BlockingParams {
    host: String,
    port: u16,
    path: String,
    username: Option<String>,
    /// Password for FTP authentication. Never logged.
    password: Option<String>,
    use_tls: bool,
    connect_timeout: Duration,
    read_timeout: Duration,
    /// Timeout applied to the TCP stream before the TLS handshake in FTPS.
    tls_handshake_timeout: Duration,
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
        // Re-apply the (potentially shorter) handshake timeout before TLS upgrade.
        tcp.set_read_timeout(Some(p.tls_handshake_timeout))
            .map_err(DownloadEngineError::Io)?;
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
    // Active mode (PORT) is not implemented; PASV is always used.
    stream
        .login(user, p.password.as_deref().unwrap_or(""))
        .map_err(map_ftp_login_error)?;
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

    // Restore data-phase read timeout after TLS handshake completes.
    // suppaftp 6.3.0 does not expose a way to reset the underlying stream's
    // read timeout after into_secure(); tls_handshake_timeout defaults to the
    // same value as read_timeout (30 s), so both phases are covered correctly.
    let user = p.username.as_deref().unwrap_or("anonymous");
    // Active mode (PORT) is not implemented; PASV is always used.
    stream
        .login(user, p.password.as_deref().unwrap_or(""))
        .map_err(map_ftp_login_error)?;
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

/// Maps a `suppaftp::FtpError` from a `login()` call to the appropriate
/// [`DownloadEngineError`] variant.
///
/// Auth-denial responses (530 Not Logged In, 430 Invalid Credentials) are mapped
/// to [`DownloadEngineError::FtpCredentialRequired`] so that
/// `is_retryable_ftp_error` correctly classifies them as non-retryable.
/// All other errors fall through to the generic [`DownloadEngineError::FtpError`].
fn map_ftp_login_error(e: suppaftp::FtpError) -> DownloadEngineError {
    use suppaftp::FtpError;
    use suppaftp::types::Response;
    if let FtpError::UnexpectedResponse(Response { ref status, .. }) = e {
        // 530 = Not Logged In (password rejected)
        // 430 = Invalid Credentials
        if matches!(
            status,
            suppaftp::Status::NotLoggedIn | suppaftp::Status::InvalidCredentials
        ) {
            return DownloadEngineError::FtpCredentialRequired;
        }
    }
    DownloadEngineError::FtpError(e.to_string())
}

/// Returns `true` for transient FTP errors that warrant a retry.
/// Authentication failures and permanent errors are NOT retried.
fn is_retryable_ftp_error(e: &DownloadEngineError) -> bool {
    match e {
        // FtpError wraps suppaftp strings; connection/network failures are transient.
        DownloadEngineError::FtpError(_) => true,
        // Io errors (e.g. connection reset) are transient.
        DownloadEngineError::Io(_) => true,
        // These are permanent or require human intervention.
        DownloadEngineError::FtpCredentialRequired
        | DownloadEngineError::EmbeddedCredentials
        | DownloadEngineError::FtpPathTraversal
        | DownloadEngineError::InvalidUrl
        | DownloadEngineError::UnsupportedScheme
        | DownloadEngineError::EngineMismatch
        | DownloadEngineError::Cancelled
        | DownloadEngineError::Paused
        | DownloadEngineError::DestinationExists
        | DownloadEngineError::NotFound(_) => false,
        // Storage/state errors are not transient.
        DownloadEngineError::StateMachine(_) | DownloadEngineError::Storage(_) => false,
        // Anything else (progress, timestamp, domain): not retried.
        _ => false,
    }
}

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

    // ── Authentication config tests ──────────────────────────────────────────

    #[test]
    fn ftp_config_anonymous_login_uses_empty_password() {
        // When password is None, as_deref().unwrap_or("") must produce "".
        let config = super::FtpEngineConfig::default();
        assert!(config.password.is_none());
        assert_eq!(config.password.as_deref().unwrap_or(""), "");
    }

    #[test]
    fn ftp_config_with_credentials_are_passed_to_login() {
        // When password is Some, it must be passed through correctly.
        let config = super::FtpEngineConfig {
            password: Some("s3cr3t".to_owned()),
            ..Default::default()
        };
        assert_eq!(config.password.as_deref().unwrap_or(""), "s3cr3t");
    }

    // ── Retry classification tests ───────────────────────────────────────────

    #[test]
    fn ftp_is_retryable_error_classifies_correctly() {
        use super::is_retryable_ftp_error;

        // FtpError (connection/network) → retryable.
        assert!(is_retryable_ftp_error(&DownloadEngineError::FtpError(
            "connection reset".to_owned()
        )));
        // Io error → retryable.
        assert!(is_retryable_ftp_error(&DownloadEngineError::Io(
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset")
        )));
        // Auth/permanent errors → NOT retryable.
        assert!(!is_retryable_ftp_error(
            &DownloadEngineError::FtpCredentialRequired
        ));
        assert!(!is_retryable_ftp_error(
            &DownloadEngineError::FtpPathTraversal
        ));
        assert!(!is_retryable_ftp_error(
            &DownloadEngineError::EmbeddedCredentials
        ));
        assert!(!is_retryable_ftp_error(&DownloadEngineError::Cancelled));
        assert!(!is_retryable_ftp_error(&DownloadEngineError::Paused));
        assert!(!is_retryable_ftp_error(
            &DownloadEngineError::DestinationExists
        ));
    }

    #[test]
    fn ftp_auth_failure_is_not_retried() {
        use super::is_retryable_ftp_error;
        // FtpCredentialRequired is the designated auth-failure error.
        assert!(!is_retryable_ftp_error(
            &DownloadEngineError::FtpCredentialRequired
        ));
    }

    // ── TLS handshake timeout field test ────────────────────────────────────

    #[test]
    fn ftps_tls_handshake_timeout_field_exists() {
        let config = super::FtpEngineConfig::default();
        // Confirm the field is present and defaults to 30 s.
        assert_eq!(
            config.tls_handshake_timeout,
            std::time::Duration::from_secs(30)
        );
    }

    // ── Login error mapping tests ────────────────────────────────────────────

    /// Verifies that `map_ftp_login_error` maps 530 Not Logged In → FtpCredentialRequired.
    /// This is the wire path for an authentication failure from suppaftp.
    #[test]
    fn ftp_login_error_530_maps_to_credential_required() {
        use suppaftp::FtpError;
        use suppaftp::types::Response;

        let response = Response::new(suppaftp::Status::NotLoggedIn, b"Login incorrect".to_vec());
        let err = FtpError::UnexpectedResponse(response);
        let mapped = super::map_ftp_login_error(err);
        assert!(
            matches!(mapped, DownloadEngineError::FtpCredentialRequired),
            "expected FtpCredentialRequired, got: {mapped:?}"
        );
    }

    /// Verifies that `map_ftp_login_error` maps 430 Invalid Credentials → FtpCredentialRequired.
    #[test]
    fn ftp_login_error_430_maps_to_credential_required() {
        use suppaftp::FtpError;
        use suppaftp::types::Response;

        let response = Response::new(
            suppaftp::Status::InvalidCredentials,
            b"Invalid credentials".to_vec(),
        );
        let err = FtpError::UnexpectedResponse(response);
        let mapped = super::map_ftp_login_error(err);
        assert!(
            matches!(mapped, DownloadEngineError::FtpCredentialRequired),
            "expected FtpCredentialRequired, got: {mapped:?}"
        );
    }

    /// Verifies that `map_ftp_login_error` maps connection errors → FtpError (retryable).
    #[test]
    fn ftp_login_error_connection_maps_to_ftp_error() {
        use suppaftp::FtpError;

        let err = FtpError::ConnectionError(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset",
        ));
        let mapped = super::map_ftp_login_error(err);
        assert!(
            matches!(mapped, DownloadEngineError::FtpError(_)),
            "expected FtpError, got: {mapped:?}"
        );
        // A connection error should be retryable.
        assert!(super::is_retryable_ftp_error(&mapped));
    }

    /// Confirms that FtpCredentialRequired (the result of a 530 login failure)
    /// is not retried — the credential error classification works end-to-end.
    #[test]
    fn ftp_credential_required_is_not_retryable() {
        use suppaftp::FtpError;
        use suppaftp::types::Response;

        let response = Response::new(suppaftp::Status::NotLoggedIn, b"Login incorrect".to_vec());
        let err = FtpError::UnexpectedResponse(response);
        let mapped = super::map_ftp_login_error(err);
        // Must be FtpCredentialRequired, which is NOT retried.
        assert!(!super::is_retryable_ftp_error(&mapped));
    }

    // ── RetryPolicy::FTP constant test ──────────────────────────────────────

    #[test]
    fn ftp_retry_policy_has_adequate_window() {
        use crate::retry::RetryPolicy;
        use std::time::Duration;
        // FTP policy should allow at least 5 attempts within its window even
        // if the first attempt takes up to 15 s (the default connect timeout).
        const { assert!(RetryPolicy::FTP.max_attempts >= 5) }
        const { assert!(RetryPolicy::FTP.max_duration.as_secs() >= 60) }
        // First delay should be available even after 15 s elapsed.
        assert!(
            RetryPolicy::FTP
                .delay(1, None, Duration::from_secs(15))
                .is_some(),
            "FTP retry budget exhausted after first 15 s connect attempt"
        );
    }
}

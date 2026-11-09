use std::{
    ffi::OsString,
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{Arc, Weak},
    time::Duration,
};

use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState, get_sockets_info};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    sync::Mutex,
    task::JoinHandle,
    time::{Instant, sleep, timeout, timeout_at},
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{Aria2Client, Aria2ClientConfig, Aria2ClientError, Aria2Version};

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FORCED_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

type ProcessFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + Send + 'a>>;

/// Exit information independent of platform-specific `ExitStatus` details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessExit {
    pub success: bool,
    pub code: Option<i32>,
}

/// An owned child process controlled by an aria2 launcher.
pub trait SupervisedProcess: Send {
    fn process_id(&self) -> Option<u32>;
    fn try_wait(&mut self) -> ProcessFuture<'_, Option<ProcessExit>>;
    fn wait(&mut self) -> ProcessFuture<'_, ProcessExit>;
    fn kill(&mut self) -> ProcessFuture<'_, ()>;
}

/// Injectable process creation boundary; executable arguments remain structured.
pub trait Aria2ProcessLauncher: Send + Sync {
    fn launch<'a>(
        &'a self,
        executable: &'a Path,
        arguments: &'a [OsString],
    ) -> ProcessFuture<'a, Box<dyn SupervisedProcess>>;
}

#[derive(Clone, Debug)]
pub struct Aria2SupervisorConfig {
    executable: PathBuf,
    runtime_directory: Option<PathBuf>,
    startup_timeout: Duration,
    rpc_timeout: Duration,
    shutdown_timeout: Duration,
}

impl Aria2SupervisorConfig {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            runtime_directory: None,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }

    /// Overrides the parent directory used for the private runtime config directory.
    pub fn with_runtime_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.runtime_directory = Some(directory.into());
        self
    }

    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn with_rpc_timeout(mut self, timeout: Duration) -> Self {
        self.rpc_timeout = timeout;
        self
    }

    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Aria2SupervisorState {
    Starting,
    Ready {
        generation: u64,
    },
    Crashed {
        generation: u64,
        exit_code: Option<i32>,
    },
    Stopping,
    Stopped,
}

#[derive(Debug, Error)]
pub enum Aria2SupervisorError {
    #[error("aria2 executable is missing or is not a regular file")]
    ExecutableMissing,
    #[error("aria2 executable is not executable")]
    ExecutableNotRunnable,
    #[error("aria2 executable could not be started")]
    ProcessStart,
    #[error("aria2 process exited before RPC became ready")]
    ProcessExitedDuringStartup,
    #[error("aria2 RPC startup timed out")]
    ReadinessTimeout,
    #[error("aria2 RPC endpoint did not require authentication")]
    AuthenticationUnavailable,
    #[error("aria2 RPC authentication failed")]
    AuthenticationFailed,
    #[error("aria2 RPC endpoint returned an invalid readiness response")]
    InvalidReadinessResponse,
    #[error("aria2 RPC listener does not belong to the supervised process")]
    RpcListenerOwnershipMismatch,
    #[error("aria2 RPC endpoint could not be reserved")]
    PortUnavailable,
    #[error("aria2 runtime configuration could not be created")]
    RuntimeConfiguration,
    #[error("aria2 supervisor is shutting down")]
    ShuttingDown,
    #[error("aria2 process did not exit after forced termination")]
    ForcedTerminationFailed,
    #[error("aria2 process status could not be inspected")]
    ProcessStatus,
    #[error("aria2 process restart failed")]
    RestartFailed,
}

struct SupervisorInner {
    state: Aria2SupervisorState,
    generation_counter: u64,
    process: Option<Arc<Mutex<Box<dyn SupervisedProcess>>>>,
    client: Option<Aria2Client>,
    runtime_config: Option<RuntimeConfig>,
}

/// Owns one private aria2c process and its runtime-only RPC credentials.
pub struct Aria2Supervisor {
    config: Aria2SupervisorConfig,
    launcher: Arc<dyn Aria2ProcessLauncher>,
    lifecycle: Mutex<()>,
    inner: Mutex<SupervisorInner>,
}

impl Aria2Supervisor {
    /// Starts the configured executable and returns only after authenticated RPC is ready.
    pub async fn start(config: Aria2SupervisorConfig) -> Result<Arc<Self>, Aria2SupervisorError> {
        Self::start_with_launcher(config, Arc::new(TokioProcessLauncher)).await
    }

    /// Starts aria2 using an injected launcher, primarily for deterministic tests.
    pub async fn start_with_launcher(
        config: Aria2SupervisorConfig,
        launcher: Arc<dyn Aria2ProcessLauncher>,
    ) -> Result<Arc<Self>, Aria2SupervisorError> {
        if config.startup_timeout.is_zero()
            || config.rpc_timeout.is_zero()
            || config.shutdown_timeout.is_zero()
        {
            return Err(Aria2SupervisorError::RuntimeConfiguration);
        }
        let supervisor = Arc::new(Self {
            config,
            launcher,
            lifecycle: Mutex::new(()),
            inner: Mutex::new(SupervisorInner {
                state: Aria2SupervisorState::Stopped,
                generation_counter: 0,
                process: None,
                client: None,
                runtime_config: None,
            }),
        });
        supervisor.ensure_running().await?;
        supervisor.spawn_monitor();
        Ok(supervisor)
    }

    /// Starts a replacement process after a failed RPC operation unless another
    /// concurrent worker has already advanced the process generation.
    pub async fn restart_after_failure(
        &self,
        failed_generation: u64,
    ) -> Result<u64, Aria2SupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        self.refresh_state().await?;
        {
            let inner = self.inner.lock().await;
            if let Aria2SupervisorState::Ready { generation } = inner.state
                && generation != failed_generation
            {
                return Ok(generation);
            }
            if matches!(
                inner.state,
                Aria2SupervisorState::Stopping | Aria2SupervisorState::Stopped
            ) {
                return Err(Aria2SupervisorError::ShuttingDown);
            }
        }
        self.stop_process(true).await?;
        self.start_process_locked().await?;
        let inner = self.inner.lock().await;
        match inner.state {
            Aria2SupervisorState::Ready { generation } => Ok(generation),
            _ => Err(Aria2SupervisorError::RestartFailed),
        }
    }

    pub async fn is_running(&self) -> Result<bool, Aria2SupervisorError> {
        self.refresh_state().await?;
        Ok(matches!(
            self.inner.lock().await.state,
            Aria2SupervisorState::Ready { .. }
        ))
    }

    pub async fn state(&self) -> Result<Aria2SupervisorState, Aria2SupervisorError> {
        self.refresh_state().await?;
        Ok(self.inner.lock().await.state)
    }

    /// Stops work from acquiring a client and performs bounded graceful/forced shutdown.
    pub async fn shutdown(&self) -> Result<(), Aria2SupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        {
            let mut inner = self.inner.lock().await;
            if matches!(inner.state, Aria2SupervisorState::Stopped) {
                return Ok(());
            }
            inner.state = Aria2SupervisorState::Stopping;
        }
        self.stop_process(false).await?;
        let mut inner = self.inner.lock().await;
        inner.client = None;
        inner.state = Aria2SupervisorState::Stopped;
        inner.runtime_config = None;
        info!("private aria2 process stopped");
        Ok(())
    }

    pub(crate) async fn client_runtime(&self) -> Result<(Aria2Client, u64), Aria2SupervisorError> {
        self.ensure_running().await?;
        self.refresh_state().await?;
        let inner = self.inner.lock().await;
        match (inner.state, inner.client.as_ref()) {
            (Aria2SupervisorState::Ready { generation }, Some(client)) => {
                Ok((client.clone(), generation))
            }
            (Aria2SupervisorState::Stopping | Aria2SupervisorState::Stopped, _) => {
                Err(Aria2SupervisorError::ShuttingDown)
            }
            _ => Err(Aria2SupervisorError::ProcessExitedDuringStartup),
        }
    }

    pub(crate) async fn existing_client_runtime(&self) -> Option<(Aria2Client, u64)> {
        let inner = self.inner.lock().await;
        match (inner.state, inner.client.as_ref()) {
            (Aria2SupervisorState::Ready { generation }, Some(client)) => {
                Some((client.clone(), generation))
            }
            _ => None,
        }
    }

    async fn ensure_running(&self) -> Result<(), Aria2SupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        self.refresh_state().await?;
        let state = { self.inner.lock().await.state };
        match state {
            Aria2SupervisorState::Ready { .. } => Ok(()),
            Aria2SupervisorState::Stopping => Err(Aria2SupervisorError::ShuttingDown),
            Aria2SupervisorState::Stopped | Aria2SupervisorState::Crashed { .. } => {
                self.start_process_locked().await
            }
            Aria2SupervisorState::Starting => self.start_process_locked().await,
        }
    }

    async fn start_process_locked(&self) -> Result<(), Aria2SupervisorError> {
        {
            let mut inner = self.inner.lock().await;
            inner.state = Aria2SupervisorState::Starting;
        }
        let result = self.launch_and_wait().await;
        match result {
            Ok((process, client, runtime_config)) => {
                let mut inner = self.inner.lock().await;
                let generation = inner
                    .generation_counter
                    .checked_add(1)
                    .ok_or(Aria2SupervisorError::RestartFailed)?;
                inner.generation_counter = generation;
                inner.process = Some(Arc::new(Mutex::new(process)));
                inner.client = Some(client);
                inner.runtime_config = Some(runtime_config);
                inner.state = Aria2SupervisorState::Ready { generation };
                info!(generation, "private aria2 process is ready");
                Ok(())
            }
            Err(error) => {
                self.inner.lock().await.state = Aria2SupervisorState::Stopped;
                Err(error)
            }
        }
    }

    async fn launch_and_wait(
        &self,
    ) -> Result<(Box<dyn SupervisedProcess>, Aria2Client, RuntimeConfig), Aria2SupervisorError>
    {
        let executable = tokio::fs::canonicalize(&self.config.executable)
            .await
            .map_err(|_| Aria2SupervisorError::ExecutableMissing)?;
        let metadata = tokio::fs::metadata(&executable)
            .await
            .map_err(|_| Aria2SupervisorError::ExecutableMissing)?;
        if !metadata.is_file() {
            return Err(Aria2SupervisorError::ExecutableMissing);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(Aria2SupervisorError::ExecutableNotRunnable);
            }
        }

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| Aria2SupervisorError::PortUnavailable)?;
        let port = listener
            .local_addr()
            .map_err(|_| Aria2SupervisorError::PortUnavailable)?
            .port();
        drop(listener);

        let token = Uuid::new_v4().simple().to_string();
        let runtime_config =
            RuntimeConfig::create(self.config.runtime_directory.as_deref(), port, &token)?;
        let endpoint = format!("http://127.0.0.1:{port}/jsonrpc");
        let unauthenticated = Aria2Client::new(Aria2ClientConfig::new(
            endpoint.clone(),
            None,
            self.config.rpc_timeout,
        ))
        .map_err(|_| Aria2SupervisorError::RuntimeConfiguration)?;
        let client = Aria2Client::new(Aria2ClientConfig::new(
            endpoint,
            Some(token),
            self.config.rpc_timeout,
        ))
        .map_err(|_| Aria2SupervisorError::RuntimeConfiguration)?;
        let arguments = [
            OsString::from("--conf-path"),
            runtime_config.path().as_os_str().to_owned(),
        ];
        let mut process = self
            .launcher
            .launch(&executable, &arguments)
            .await
            .map_err(|_| Aria2SupervisorError::ProcessStart)?;

        let deadline = Instant::now() + self.config.startup_timeout;
        let version = timeout_at(
            deadline,
            wait_until_ready(process.as_mut(), port, &unauthenticated, &client),
        )
        .await
        .map_err(|_| Aria2SupervisorError::ReadinessTimeout)??;
        runtime_config.remove_secret_file()?;
        info!(
            aria2_version = version.version(),
            "authenticated aria2 RPC is ready"
        );
        Ok((process, client, runtime_config))
    }

    fn spawn_monitor(self: &Arc<Self>) {
        let weak: Weak<Self> = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                sleep(PROCESS_POLL_INTERVAL).await;
                let Some(supervisor) = weak.upgrade() else {
                    break;
                };
                match supervisor.refresh_state().await {
                    Ok(()) => {
                        if matches!(
                            supervisor.inner.lock().await.state,
                            Aria2SupervisorState::Stopped | Aria2SupervisorState::Stopping
                        ) {
                            break;
                        }
                    }
                    Err(_) => {
                        warn!("could not inspect private aria2 process state");
                    }
                }
            }
        });
    }

    async fn refresh_state(&self) -> Result<(), Aria2SupervisorError> {
        let process = {
            let inner = self.inner.lock().await;
            if !matches!(inner.state, Aria2SupervisorState::Ready { .. }) {
                return Ok(());
            }
            inner.process.clone()
        };
        let Some(process) = process else {
            return Ok(());
        };
        let exit = process
            .lock()
            .await
            .try_wait()
            .await
            .map_err(|_| Aria2SupervisorError::ProcessStatus)?;
        if let Some(exit) = exit {
            let mut inner = self.inner.lock().await;
            let generation = match inner.state {
                Aria2SupervisorState::Ready { generation } => generation,
                _ => return Ok(()),
            };
            inner.state = Aria2SupervisorState::Crashed {
                generation,
                exit_code: exit.code,
            };
            inner.process = None;
            inner.client = None;
            inner.runtime_config = None;
            warn!(generation, exit_code = ?exit.code, "private aria2 process exited unexpectedly");
        }
        Ok(())
    }

    async fn stop_process(&self, restart: bool) -> Result<(), Aria2SupervisorError> {
        let (process, client) = {
            let mut inner = self.inner.lock().await;
            (inner.process.take(), inner.client.take())
        };
        let Some(process) = process else {
            let mut inner = self.inner.lock().await;
            inner.runtime_config = None;
            if restart {
                inner.state = Aria2SupervisorState::Crashed {
                    generation: match inner.state {
                        Aria2SupervisorState::Ready { generation }
                        | Aria2SupervisorState::Crashed { generation, .. } => generation,
                        _ => 0,
                    },
                    exit_code: None,
                };
            }
            return Ok(());
        };

        if let Some(client) = client
            && let Err(error) = client.shutdown().await
        {
            warn!(error = %error, "aria2 graceful RPC shutdown failed; forcing process termination");
        }
        let mut child = process.lock().await;
        let mut forced_termination_failed = false;
        match timeout(self.config.shutdown_timeout, child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => {
                let killed = timeout(FORCED_SHUTDOWN_TIMEOUT, child.kill())
                    .await
                    .is_ok_and(|result| result.is_ok());
                let reaped = killed
                    && timeout(FORCED_SHUTDOWN_TIMEOUT, child.wait())
                        .await
                        .is_ok_and(|result| result.is_ok());
                forced_termination_failed = !reaped;
            }
        }
        drop(child);
        let mut inner = self.inner.lock().await;
        inner.runtime_config = None;
        if restart {
            let generation = match inner.state {
                Aria2SupervisorState::Ready { generation }
                | Aria2SupervisorState::Crashed { generation, .. } => generation,
                _ => 0,
            };
            inner.state = Aria2SupervisorState::Crashed {
                generation,
                exit_code: None,
            };
        }
        if forced_termination_failed {
            Err(Aria2SupervisorError::ForcedTerminationFailed)
        } else {
            Ok(())
        }
    }
}

async fn wait_until_ready(
    process: &mut dyn SupervisedProcess,
    port: u16,
    unauthenticated: &Aria2Client,
    authenticated: &Aria2Client,
) -> Result<Aria2Version, Aria2SupervisorError> {
    loop {
        if process
            .try_wait()
            .await
            .map_err(|_| Aria2SupervisorError::ProcessStatus)?
            .is_some()
        {
            return Err(Aria2SupervisorError::ProcessExitedDuringStartup);
        }
        match unauthenticated.get_version().await {
            Ok(_) => return Err(Aria2SupervisorError::AuthenticationUnavailable),
            Err(Aria2ClientError::Rpc { .. }) => {
                let Some(process_id) = process.process_id() else {
                    return Err(Aria2SupervisorError::ProcessStatus);
                };
                if !rpc_listener_belongs_to_process(port, process_id).await? {
                    return Err(Aria2SupervisorError::RpcListenerOwnershipMismatch);
                }
            }
            Err(Aria2ClientError::Timeout | Aria2ClientError::Transport) => {
                sleep(PROCESS_POLL_INTERVAL).await;
                continue;
            }
            Err(_) => return Err(Aria2SupervisorError::InvalidReadinessResponse),
        }
        if process
            .try_wait()
            .await
            .map_err(|_| Aria2SupervisorError::ProcessStatus)?
            .is_some()
        {
            return Err(Aria2SupervisorError::ProcessExitedDuringStartup);
        }
        return match authenticated.get_version().await {
            Ok(version) => Ok(version),
            Err(Aria2ClientError::Rpc { .. }) => Err(Aria2SupervisorError::AuthenticationFailed),
            Err(Aria2ClientError::Timeout | Aria2ClientError::Transport) => {
                sleep(PROCESS_POLL_INTERVAL).await;
                continue;
            }
            Err(_) => Err(Aria2SupervisorError::InvalidReadinessResponse),
        };
    }
}

async fn rpc_listener_belongs_to_process(
    port: u16,
    process_id: u32,
) -> Result<bool, Aria2SupervisorError> {
    tokio::task::spawn_blocking(move || {
        let sockets = get_sockets_info(
            AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
            ProtocolFlags::TCP,
        )
        .map_err(|_| Aria2SupervisorError::ProcessStatus)?;
        Ok(sockets.into_iter().any(|socket| {
            matches!(
                socket.protocol_socket_info,
                ProtocolSocketInfo::Tcp(ref info)
                    if info.local_port == port
                        && info.local_addr.is_loopback()
                        && info.state == TcpState::Listen
            ) && socket.associated_pids.contains(&process_id)
        }))
    })
    .await
    .map_err(|_| Aria2SupervisorError::ProcessStatus)?
}

struct RuntimeConfig {
    directory: PathBuf,
    file: PathBuf,
}

impl RuntimeConfig {
    fn create(
        configured_parent: Option<&Path>,
        port: u16,
        token: &str,
    ) -> Result<Self, Aria2SupervisorError> {
        let parent = configured_parent
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(default_runtime_parent)?;
        std::fs::create_dir_all(&parent).map_err(|_| Aria2SupervisorError::RuntimeConfiguration)?;
        let directory = parent.join(format!("aria2-{}", Uuid::new_v4().simple()));
        create_private_directory(&directory)?;
        let file = directory.join("aria2.conf");
        let contents = format!(
            "enable-rpc=true\nrpc-listen-all=false\nrpc-listen-port={port}\nrpc-secret={token}\n\
             max-concurrent-downloads=32\nmax-connection-per-server=1\nsplit=1\ncontinue=true\n\
             allow-overwrite=false\nauto-file-renaming=false\ndaemon=false\n"
        );
        if write_private_file(&file, contents.as_bytes()).is_err() {
            let _ = std::fs::remove_file(&file);
            let _ = std::fs::remove_dir(&directory);
            return Err(Aria2SupervisorError::RuntimeConfiguration);
        }
        Ok(Self { directory, file })
    }

    fn path(&self) -> &Path {
        &self.file
    }

    fn remove_secret_file(&self) -> Result<(), Aria2SupervisorError> {
        std::fs::remove_file(&self.file).map_err(|_| Aria2SupervisorError::RuntimeConfiguration)
    }
}

impl Drop for RuntimeConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), Aria2SupervisorError> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|_| Aria2SupervisorError::RuntimeConfiguration)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), Aria2SupervisorError> {
    std::fs::create_dir(path).map_err(|_| Aria2SupervisorError::RuntimeConfiguration)
}

#[cfg(unix)]
fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(contents)
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(contents)
}

fn default_runtime_parent() -> Result<PathBuf, Aria2SupervisorError> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("FetchX").join("aria2-runtime"))
            .ok_or(Aria2SupervisorError::RuntimeConfiguration)
    }
    #[cfg(not(windows))]
    {
        Ok(std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("FetchX"))
    }
}

struct TokioProcessLauncher;

impl Aria2ProcessLauncher for TokioProcessLauncher {
    fn launch<'a>(
        &'a self,
        executable: &'a Path,
        arguments: &'a [OsString],
    ) -> ProcessFuture<'a, Box<dyn SupervisedProcess>> {
        Box::pin(async move {
            let mut command = Command::new(executable);
            command
                .args(arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            // Suppress the console window that would otherwise flash on Windows.
            #[cfg(windows)]
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let mut child = command.spawn()?;
            let mut drains = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                drains.push(drain_output(stdout));
            }
            if let Some(stderr) = child.stderr.take() {
                drains.push(drain_output(stderr));
            }
            Ok(Box::new(TokioProcess { child, drains }) as Box<dyn SupervisedProcess>)
        })
    }
}

struct TokioProcess {
    child: Child,
    drains: Vec<JoinHandle<()>>,
}

impl SupervisedProcess for TokioProcess {
    fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    fn try_wait(&mut self) -> ProcessFuture<'_, Option<ProcessExit>> {
        Box::pin(async move {
            let exit = self.child.try_wait()?.map(ProcessExit::from);
            if exit.is_some() {
                self.finish_drains().await;
            }
            Ok(exit)
        })
    }

    fn wait(&mut self) -> ProcessFuture<'_, ProcessExit> {
        Box::pin(async move {
            let exit = self.child.wait().await.map(ProcessExit::from)?;
            self.finish_drains().await;
            Ok(exit)
        })
    }

    fn kill(&mut self) -> ProcessFuture<'_, ()> {
        Box::pin(async move { self.child.kill().await })
    }
}

impl TokioProcess {
    async fn finish_drains(&mut self) {
        for mut drain in self.drains.drain(..) {
            match timeout(Duration::from_secs(1), &mut drain).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => warn!("aria2 output drain task failed"),
                Err(_) => drain.abort(),
            }
        }
    }
}

impl From<std::process::ExitStatus> for ProcessExit {
    fn from(status: std::process::ExitStatus) -> Self {
        Self {
            success: status.success(),
            code: status.code(),
        }
    }
}

fn drain_output<R>(mut reader: R) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        error::Error,
        fs, io,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::Notify,
        task::JoinHandle,
        time::sleep,
    };

    use super::{
        Aria2ProcessLauncher, Aria2Supervisor, Aria2SupervisorConfig, Aria2SupervisorError,
        Aria2SupervisorState, ProcessExit, ProcessFuture, SupervisedProcess,
    };

    #[derive(Clone, Copy)]
    enum Behavior {
        Ready,
        AuthenticationFailure,
        Malformed,
        NeverReady,
        ExitImmediately,
        ForeignListener,
        NoGracefulShutdown,
        LaunchFailure,
    }

    #[derive(Default)]
    struct FakeProcessState {
        process_id: u32,
        exit: StdMutex<Option<ProcessExit>>,
        stopped: Notify,
        server: StdMutex<Option<JoinHandle<()>>>,
        gids_seen: StdMutex<Vec<String>>,
    }

    impl FakeProcessState {
        fn exit(&self) -> Option<ProcessExit> {
            self.exit.lock().ok().and_then(|exit| *exit)
        }

        fn finish(&self, exit: ProcessExit) {
            if let Ok(mut current) = self.exit.lock()
                && current.is_none()
            {
                *current = Some(exit);
                self.stopped.notify_waiters();
            }
        }

        fn abort_server(&self) {
            if let Ok(mut server) = self.server.lock()
                && let Some(server) = server.take()
            {
                server.abort();
            }
        }
    }

    struct FakeChild(Arc<FakeProcessState>);

    impl SupervisedProcess for FakeChild {
        fn process_id(&self) -> Option<u32> {
            Some(self.0.process_id)
        }

        fn try_wait(&mut self) -> ProcessFuture<'_, Option<ProcessExit>> {
            Box::pin(async move { Ok(self.0.exit()) })
        }

        fn wait(&mut self) -> ProcessFuture<'_, ProcessExit> {
            Box::pin(async move {
                loop {
                    let notified = self.0.stopped.notified();
                    if let Some(exit) = self.0.exit() {
                        return Ok(exit);
                    }
                    notified.await;
                }
            })
        }

        fn kill(&mut self) -> ProcessFuture<'_, ()> {
            Box::pin(async move {
                self.0.finish(ProcessExit {
                    success: false,
                    code: Some(9),
                });
                self.0.abort_server();
                Ok(())
            })
        }
    }

    impl Drop for FakeChild {
        fn drop(&mut self) {
            if self.0.exit().is_none() {
                self.0.finish(ProcessExit {
                    success: false,
                    code: Some(9),
                });
                self.0.abort_server();
            }
        }
    }

    struct FakeLauncher {
        behaviors: StdMutex<VecDeque<Behavior>>,
        starts: AtomicUsize,
        states: StdMutex<Vec<Arc<FakeProcessState>>>,
        next_gid: Arc<AtomicUsize>,
    }

    impl FakeLauncher {
        fn new(behaviors: impl IntoIterator<Item = Behavior>) -> Self {
            Self {
                behaviors: StdMutex::new(behaviors.into_iter().collect()),
                starts: AtomicUsize::new(0),
                states: StdMutex::new(Vec::new()),
                next_gid: Arc::new(AtomicUsize::new(1)),
            }
        }

        fn starts(&self) -> usize {
            self.starts.load(Ordering::SeqCst)
        }

        fn crash_latest(&self) {
            if let Ok(states) = self.states.lock()
                && let Some(state) = states.last()
            {
                state.finish(ProcessExit {
                    success: false,
                    code: Some(23),
                });
                state.abort_server();
            }
        }
    }

    impl Aria2ProcessLauncher for FakeLauncher {
        fn launch<'a>(
            &'a self,
            _executable: &'a Path,
            arguments: &'a [std::ffi::OsString],
        ) -> ProcessFuture<'a, Box<dyn SupervisedProcess>> {
            Box::pin(async move {
                self.starts.fetch_add(1, Ordering::SeqCst);
                let behavior = self
                    .behaviors
                    .lock()
                    .map_err(|_| io::Error::other("poisoned fake launcher"))?
                    .pop_front()
                    .unwrap_or(Behavior::Ready);
                if matches!(behavior, Behavior::LaunchFailure) {
                    return Err(io::Error::other("deliberate fake launch failure"));
                }
                let config_path = arguments
                    .get(1)
                    .ok_or_else(|| io::Error::other("missing config argument"))?;
                let config = tokio::fs::read_to_string(config_path).await?;
                let port = config
                    .lines()
                    .find_map(|line| line.strip_prefix("rpc-listen-port="))
                    .ok_or_else(|| io::Error::other("missing rpc port"))?
                    .parse::<u16>()
                    .map_err(io::Error::other)?;
                let token = config
                    .lines()
                    .find_map(|line| line.strip_prefix("rpc-secret="))
                    .ok_or_else(|| io::Error::other("missing rpc token"))?
                    .to_owned();
                let state = Arc::new(FakeProcessState {
                    process_id: if matches!(behavior, Behavior::ForeignListener) {
                        u32::MAX
                    } else {
                        std::process::id()
                    },
                    ..FakeProcessState::default()
                });
                if matches!(behavior, Behavior::ExitImmediately) {
                    state.finish(ProcessExit {
                        success: false,
                        code: Some(4),
                    });
                } else if !matches!(behavior, Behavior::NeverReady) {
                    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
                    let server_state = Arc::clone(&state);
                    let next_gid = Arc::clone(&self.next_gid);
                    let server = tokio::spawn(async move {
                        fake_rpc_server(listener, token, behavior, server_state, next_gid).await;
                    });
                    if let Ok(mut handle) = state.server.lock() {
                        *handle = Some(server);
                    }
                }
                if let Ok(mut states) = self.states.lock() {
                    states.push(Arc::clone(&state));
                }
                Ok(Box::new(FakeChild(state)) as Box<dyn SupervisedProcess>)
            })
        }
    }

    async fn fake_rpc_server(
        listener: TcpListener,
        token: String,
        behavior: Behavior,
        state: Arc<FakeProcessState>,
        next_gid: Arc<AtomicUsize>,
    ) {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(request) = read_request(&mut stream).await else {
                return;
            };
            if matches!(behavior, Behavior::Malformed) {
                let _ = write_response(&mut stream, "not json").await;
                continue;
            }
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let method = request.get("method").and_then(Value::as_str).unwrap_or("");
            let params = request
                .get("params")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let supplied_token = params.first().and_then(Value::as_str);
            let auth_required = supplied_token == Some(format!("token:{token}").as_str());
            let response = if !auth_required || matches!(behavior, Behavior::AuthenticationFailure)
            {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":1,"message":"unauthorized"}})
            } else if method == "aria2.getVersion" {
                json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "result":{"version":"1.37.0","enabledFeatures":[]}
                })
            } else if method == "aria2.addUri" {
                let gid = format!("{:016x}", next_gid.fetch_add(1, Ordering::SeqCst));
                json!({"jsonrpc":"2.0","id":id,"result":gid})
            } else if method == "aria2.tellStatus" {
                let gid = params.get(1).and_then(Value::as_str).unwrap_or("");
                if let Ok(mut seen) = state.gids_seen.lock() {
                    seen.push(gid.to_owned());
                }
                json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "result":{
                        "gid":gid,
                        "status":"active",
                        "completedLength":"0",
                        "totalLength":"0",
                        "downloadSpeed":"0",
                        "connections":"1",
                        "errorCode":"0"
                    }
                })
            } else if matches!(
                method,
                "aria2.pause" | "aria2.unpause" | "aria2.remove" | "aria2.forceRemove"
            ) {
                let gid = params.get(1).and_then(Value::as_str).unwrap_or("");
                json!({"jsonrpc":"2.0","id":id,"result":gid})
            } else if method == "aria2.shutdown" {
                if !matches!(behavior, Behavior::NoGracefulShutdown) {
                    state.finish(ProcessExit {
                        success: true,
                        code: Some(0),
                    });
                }
                json!({"jsonrpc":"2.0","id":id,"result":"OK"})
            } else {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":1,"message":"unsupported"}})
            };
            if write_response(&mut stream, &response.to_string())
                .await
                .is_err()
            {
                return;
            }
            if method == "aria2.shutdown" && !matches!(behavior, Behavior::NoGracefulShutdown) {
                return;
            }
        }
    }

    async fn read_request(stream: &mut TcpStream) -> io::Result<Value> {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        let body_start = loop {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "request ended",
                ));
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..body_start]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing content length"))?;
        while bytes.len() < body_start + length {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "body ended"));
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        serde_json::from_slice(&bytes[body_start..body_start + length]).map_err(io::Error::other)
    }

    async fn write_response(stream: &mut TcpStream, body: &str) -> io::Result<()> {
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Result<Self, Box<dyn Error>> {
            let path = std::env::temp_dir().join(format!("fetchx-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_config(directory: &TestDirectory) -> Aria2SupervisorConfig {
        Aria2SupervisorConfig::new(
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("missing")),
        )
        .with_runtime_directory(&directory.0)
        .with_startup_timeout(Duration::from_millis(500))
        .with_rpc_timeout(Duration::from_millis(100))
        .with_shutdown_timeout(Duration::from_millis(50))
    }

    #[tokio::test]
    async fn starts_authenticated_private_rpc_and_shuts_down_idempotently()
    -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new()?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::Ready]));
        let supervisor =
            Aria2Supervisor::start_with_launcher(test_config(&directory), launcher.clone()).await?;
        assert!(supervisor.is_running().await?);
        assert!(matches!(
            supervisor.state().await?,
            Aria2SupervisorState::Ready { generation: 1 }
        ));
        assert_eq!(launcher.starts(), 1);
        assert_eq!(fs::read_dir(&directory.0)?.count(), 1);
        let runtime_dir = fs::read_dir(&directory.0)?
            .next()
            .ok_or("missing runtime dir")??
            .path();
        assert_eq!(fs::read_dir(runtime_dir)?.count(), 0);
        let (first, second) = tokio::join!(supervisor.shutdown(), supervisor.shutdown());
        first?;
        second?;
        assert_eq!(supervisor.state().await?, Aria2SupervisorState::Stopped);
        assert!(supervisor.existing_client_runtime().await.is_none());
        assert_eq!(launcher.starts(), 1);
        assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn missing_and_non_executable_paths_are_rejected() -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new()?;
        let missing = Aria2Supervisor::start_with_launcher(
            Aria2SupervisorConfig::new(directory.0.join("missing")),
            Arc::new(FakeLauncher::new([])),
        )
        .await
        .err()
        .ok_or("missing executable unexpectedly started")?;
        assert!(matches!(missing, Aria2SupervisorError::ExecutableMissing));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = directory.0.join("not-executable");
            fs::write(&path, b"not executable")?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            let error = Aria2Supervisor::start_with_launcher(
                Aria2SupervisorConfig::new(&path),
                Arc::new(FakeLauncher::new([])),
            )
            .await
            .err()
            .ok_or("non-executable file unexpectedly started")?;
            assert!(matches!(error, Aria2SupervisorError::ExecutableNotRunnable));
            fs::remove_file(&path)?;
        }
        let error = Aria2Supervisor::start_with_launcher(
            test_config(&directory),
            Arc::new(FakeLauncher::new([Behavior::LaunchFailure])),
        )
        .await
        .err()
        .ok_or("failing launcher unexpectedly started")?;
        assert!(matches!(error, Aria2SupervisorError::ProcessStart));
        assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn detects_immediate_exit_authentication_failure_and_malformed_rpc()
    -> Result<(), Box<dyn Error>> {
        for (behavior, expected) in [
            (Behavior::ExitImmediately, "exit"),
            (Behavior::AuthenticationFailure, "auth"),
            (Behavior::Malformed, "malformed"),
            (Behavior::ForeignListener, "foreign"),
        ] {
            let directory = TestDirectory::new()?;
            let error = Aria2Supervisor::start_with_launcher(
                test_config(&directory),
                Arc::new(FakeLauncher::new([behavior])),
            )
            .await
            .err()
            .ok_or("invalid fake process unexpectedly started")?;
            match expected {
                "exit" => assert!(matches!(
                    error,
                    Aria2SupervisorError::ProcessExitedDuringStartup
                )),
                "auth" => assert!(matches!(error, Aria2SupervisorError::AuthenticationFailed)),
                "foreign" => assert!(matches!(
                    error,
                    Aria2SupervisorError::RpcListenerOwnershipMismatch
                )),
                _ => assert!(matches!(
                    error,
                    Aria2SupervisorError::InvalidReadinessResponse
                )),
            }
            assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn readiness_timeout_cleans_configuration_and_process() -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new()?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::NeverReady]));
        let error = Aria2Supervisor::start_with_launcher(
            test_config(&directory).with_startup_timeout(Duration::from_millis(100)),
            launcher.clone(),
        )
        .await
        .err()
        .ok_or("unresponsive process unexpectedly became ready")?;
        assert!(matches!(error, Aria2SupervisorError::ReadinessTimeout));
        assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
        assert_eq!(launcher.states.lock().map_err(|_| "poisoned")?.len(), 1);
        assert!(
            launcher.states.lock().map_err(|_| "poisoned")?[0]
                .exit()
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn crash_detection_and_restart_advance_generation() -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new()?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::Ready, Behavior::Ready]));
        let supervisor =
            Aria2Supervisor::start_with_launcher(test_config(&directory), launcher.clone()).await?;
        launcher.crash_latest();
        for _ in 0..20 {
            if matches!(
                supervisor.state().await?,
                Aria2SupervisorState::Crashed {
                    exit_code: Some(23),
                    ..
                }
            ) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(matches!(
            supervisor.state().await?,
            Aria2SupervisorState::Crashed {
                exit_code: Some(23),
                ..
            }
        ));
        let (client, generation) = supervisor.client_runtime().await?;
        assert_eq!(client.get_version().await?.version(), "1.37.0");
        assert_eq!(generation, 2);
        assert_eq!(launcher.starts(), 2);
        supervisor.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn restarts_after_rpc_failure_without_duplicate_processes() -> Result<(), Box<dyn Error>>
    {
        let directory = TestDirectory::new()?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::Ready, Behavior::Ready]));
        let supervisor =
            Aria2Supervisor::start_with_launcher(test_config(&directory), launcher.clone()).await?;
        let (_, initial_generation) = supervisor.client_runtime().await?;
        let generation = supervisor.restart_after_failure(initial_generation).await?;
        assert_eq!(generation, initial_generation + 1);
        assert_eq!(launcher.starts(), 2);
        supervisor.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn forced_shutdown_terminates_unresponsive_child() -> Result<(), Box<dyn Error>> {
        let directory = TestDirectory::new()?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::NoGracefulShutdown]));
        let supervisor =
            Aria2Supervisor::start_with_launcher(test_config(&directory), launcher.clone()).await?;
        supervisor.shutdown().await?;
        assert_eq!(supervisor.state().await?, Aria2SupervisorState::Stopped);
        assert!(
            launcher.states.lock().map_err(|_| "poisoned")?[0]
                .exit()
                .is_some()
        );
        assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn startup_cancellation_drops_child_and_runtime_secret_file() -> Result<(), Box<dyn Error>>
    {
        let directory = TestDirectory::new()?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::NeverReady]));
        let config = test_config(&directory).with_startup_timeout(Duration::from_secs(5));
        let start = tokio::spawn(Aria2Supervisor::start_with_launcher(
            config,
            launcher.clone(),
        ));
        for _ in 0..50 {
            if launcher.starts() == 1
                && launcher
                    .states
                    .lock()
                    .is_ok_and(|states| !states.is_empty())
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        start.abort();
        let _ = start.await;
        assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
        assert!(
            launcher.states.lock().map_err(|_| "poisoned")?[0]
                .exit()
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_download_is_resubmitted_with_a_new_gid_after_process_crash()
    -> Result<(), Box<dyn Error>> {
        use std::path::PathBuf;

        use fetchx_models::{
            DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName,
            Timestamp,
        };
        use fetchx_storage::{Database, DownloadRepository};
        use tokio::sync::watch;

        use crate::{
            Aria2Engine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
        };

        let directory = TestDirectory::new()?;
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let id = DownloadId::new();
        let item = DownloadItem::new(
            id,
            DownloadSource::new("https://example.test/file.bin", EngineKind::Aria2)?,
            FileName::new("file.bin")?,
            PathBuf::from("file.bin"),
            Timestamp::now()?,
        );
        repository.create(&item).await?;
        let launcher = Arc::new(FakeLauncher::new([Behavior::Ready, Behavior::Ready]));
        let supervisor =
            Aria2Supervisor::start_with_launcher(test_config(&directory), launcher.clone()).await?;
        let engine = Arc::new(Aria2Engine::with_supervisor(
            repository.clone(),
            Arc::clone(&supervisor),
        ));
        let (cancel_sender, cancellation) = watch::channel(false);
        let (_pause_sender, pause) = watch::channel(false);
        let (_limit_sender, connection_limit) = watch::channel(1);
        let worker_engine = Arc::clone(&engine);
        let worker = tokio::spawn(async move {
            worker_engine
                .download_managed(
                    id,
                    ManagedDownloadControl {
                        cancellation,
                        pause,
                        connection_limit,
                        resource_demand: None,
                        bandwidth: crate::BandwidthControl::default(),
                    },
                )
                .await
        });
        for _ in 0..100 {
            let active = repository
                .get(id)
                .await?
                .is_some_and(|item| item.status() == DownloadStatus::Downloading);
            let submitted = launcher
                .states
                .lock()
                .map_err(|_| "poisoned")?
                .first()
                .is_some_and(|state| state.gids_seen.lock().is_ok_and(|gids| !gids.is_empty()));
            if active && submitted {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        launcher.crash_latest();
        for _ in 0..400 {
            if launcher.starts() >= 2
                && launcher.states.lock().map_err(|_| "poisoned")?.len() >= 2
                && launcher.states.lock().map_err(|_| "poisoned")?[1]
                    .gids_seen
                    .lock()
                    .is_ok_and(|gids| !gids.is_empty())
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(launcher.starts(), 2, "worker did not restart aria2");
        assert!(
            launcher.states.lock().map_err(|_| "poisoned")?[1]
                .gids_seen
                .lock()
                .is_ok_and(|gids| !gids.is_empty()),
            "replacement process did not receive a status request; worker finished: {}; supervisor state: {:?}",
            worker.is_finished(),
            supervisor.state().await?
        );
        cancel_sender.send(true)?;
        let result = tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .map_err(|_| "supervised engine did not observe cancellation")??;
        assert!(matches!(result, Err(DownloadEngineError::Cancelled)));
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("missing download")?
                .status(),
            DownloadStatus::Cancelled
        );
        {
            let states = launcher.states.lock().map_err(|_| "poisoned")?;
            let old_gids = states[0].gids_seen.lock().map_err(|_| "poisoned")?;
            let new_gids = states[1].gids_seen.lock().map_err(|_| "poisoned")?;
            assert!(!old_gids.is_empty());
            assert!(!new_gids.is_empty());
            assert_ne!(old_gids[0], new_gids[0]);
        }
        supervisor.shutdown().await?;
        Ok(())
    }
}

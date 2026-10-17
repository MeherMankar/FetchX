use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use fetchx_core::{DownloadStateMachine, StateMachineError};
use fetchx_downloader::{
    BandwidthControl, BandwidthLimiter, DownloadEngineError, ManagedDownloadControl,
    ManagedDownloadEngine, ResourceDemand,
};
use fetchx_models::{
    DownloadError, DownloadErrorCode, DownloadId, DownloadItem, DownloadPriority, DownloadStatus,
    EngineKind, ResourcePolicy, ResourcePressure,
};
use fetchx_storage::{DownloadRepository, StorageError};
use thiserror::Error;
use tokio::{
    sync::{Mutex, mpsc, oneshot, watch},
    task::{Id as TaskId, JoinHandle, JoinSet},
};
use tracing::{info, warn};

const MAX_ACTIVE_DOWNLOADS: usize = 32;
const MAX_GLOBAL_CONNECTIONS: usize = 64;
const DEFAULT_DESIRED_CONNECTIONS: usize = 4;
const FAIRNESS_STARTS: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueueConfig {
    pub max_active_downloads: usize,
    pub global_connection_limit: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_active_downloads: 3,
            global_connection_limit: 16,
        }
    }
}

impl QueueConfig {
    fn validate(&self) -> Result<(), QueueError> {
        if self.max_active_downloads == 0
            || self.max_active_downloads > MAX_ACTIVE_DOWNLOADS
            || self.global_connection_limit == 0
            || self.global_connection_limit > MAX_GLOBAL_CONNECTIONS
        {
            return Err(QueueError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveDownload {
    pub item: DownloadItem,
    pub desired_connections: usize,
    pub allocated_connections: usize,
    pub active_connections: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueueSnapshot {
    pub items: Vec<DownloadItem>,
    pub queued: Vec<DownloadId>,
    pub paused: Vec<DownloadId>,
    pub active: Vec<ActiveDownload>,
    pub global_connection_limit: usize,
    pub resource_pressure: ResourcePressure,
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("queue configuration is invalid or exceeds its safety bound")]
    InvalidConfiguration,
    #[error("queue manager has shut down")]
    Stopped,
    #[error("download {id} cannot perform {action} while in {status:?}")]
    InvalidStatus {
        id: DownloadId,
        action: &'static str,
        status: DownloadStatus,
    },
    #[error("download {0} was not found")]
    DownloadNotFound(DownloadId),
    #[error("pause request was superseded by resume")]
    PauseSuperseded,
    #[error("active {engine:?} downloads do not support {operation}")]
    UnsupportedEngineOperation {
        engine: EngineKind,
        operation: &'static str,
    },
    #[error("global connection limit is below currently allocated or active connections")]
    ResourceLimitInUse,
    #[error("queue resource invariant was violated")]
    ResourceInvariant,
    #[error("resource bandwidth policy is invalid")]
    InvalidBandwidthPolicy,
    #[error("download state transition failed: {0}")]
    StateMachine(#[from] StateMachineError),
    #[error("download persistence failed: {0}")]
    Storage(#[from] StorageError),
    #[error("download engine failed: {0}")]
    Engine(#[from] DownloadEngineError),
    #[error("queue worker task failed: {0}")]
    Worker(#[source] tokio::task::JoinError),
    #[error("queue actor task failed: {0}")]
    Actor(#[source] tokio::task::JoinError),
    #[error("queue shutdown failed because its completion channel closed")]
    ShutdownChannelClosed,
}

#[derive(Clone)]
pub struct QueueManager {
    commands: mpsc::Sender<Command>,
    actor: SharedActorHandle,
}

type SharedActorHandle = Arc<Mutex<Option<JoinHandle<Result<(), QueueError>>>>>;

impl QueueManager {
    pub async fn new<R, E>(
        repository: R,
        engine: Arc<E>,
        config: QueueConfig,
    ) -> Result<Self, QueueError>
    where
        R: DownloadRepository + Clone + 'static,
        E: ManagedDownloadEngine + 'static,
    {
        let mut policy = ResourcePolicy::default();
        policy.max_active_downloads = config.max_active_downloads;
        policy.global_connection_limit = config.global_connection_limit;
        policy.per_download_connection_limit = policy
            .per_download_connection_limit
            .min(config.global_connection_limit);
        Self::new_with_deferred_policy(repository, engine, policy, HashSet::new()).await
    }

    /// Starts the queue while keeping the specified persisted downloads out of
    /// worker scheduling until [`Self::release_download`] is called.
    pub async fn new_with_deferred<R, E>(
        repository: R,
        engine: Arc<E>,
        config: QueueConfig,
        deferred: HashSet<DownloadId>,
    ) -> Result<Self, QueueError>
    where
        R: DownloadRepository + Clone + 'static,
        E: ManagedDownloadEngine + 'static,
    {
        let mut policy = ResourcePolicy::default();
        policy.max_active_downloads = config.max_active_downloads;
        policy.global_connection_limit = config.global_connection_limit;
        policy.per_download_connection_limit = policy
            .per_download_connection_limit
            .min(config.global_connection_limit);
        Self::new_with_deferred_policy(repository, engine, policy, deferred).await
    }

    pub async fn new_with_resource_policy<R, E>(
        repository: R,
        engine: Arc<E>,
        policy: ResourcePolicy,
    ) -> Result<Self, QueueError>
    where
        R: DownloadRepository + Clone + 'static,
        E: ManagedDownloadEngine + 'static,
    {
        Self::new_with_deferred_policy(repository, engine, policy, HashSet::new()).await
    }

    pub async fn new_with_deferred_policy<R, E>(
        repository: R,
        engine: Arc<E>,
        policy: ResourcePolicy,
        deferred: HashSet<DownloadId>,
    ) -> Result<Self, QueueError>
    where
        R: DownloadRepository + Clone + 'static,
        E: ManagedDownloadEngine + 'static,
    {
        policy
            .validate()
            .map_err(|_| QueueError::InvalidConfiguration)?;
        let state_machine = DownloadStateMachine::new(repository.clone());
        let mut items = repository.list().await?;
        for item in &mut items {
            if let Some(recovered) = state_machine.recover_interrupted(item.id()).await? {
                *item = recovered;
            }
        }

        let entries = items
            .into_iter()
            .enumerate()
            .map(|(position, item)| {
                Ok((
                    item.id(),
                    Entry {
                        sequence: u64::try_from(position)
                            .map_err(|_| QueueError::ResourceInvariant)?,
                        item,
                    },
                ))
            })
            .collect::<Result<HashMap<_, _>, QueueError>>()?;
        let next_sequence =
            u64::try_from(entries.len()).map_err(|_| QueueError::ResourceInvariant)?;
        let (command_sender, command_receiver) = mpsc::channel(128);
        let actor = tokio::spawn(run_actor(
            repository,
            engine,
            policy,
            entries,
            next_sequence,
            deferred,
            command_receiver,
        ));

        Ok(Self {
            commands: command_sender,
            actor: Arc::new(Mutex::new(Some(actor))),
        })
    }

    pub async fn enqueue(&self, item: DownloadItem) -> Result<(), QueueError> {
        self.request(|reply| Command::Enqueue { item, reply }).await
    }

    /// Persists a new item and defers it in one actor operation.
    pub async fn enqueue_deferred(&self, item: DownloadItem) -> Result<(), QueueError> {
        self.request(|reply| Command::EnqueueDeferred { item, reply })
            .await
    }

    /// Defers a queued/paused download without changing its lifecycle state.
    /// Repeating the operation is safe. Active work cannot be deferred.
    pub async fn defer_download(&self, id: DownloadId) -> Result<(), QueueError> {
        self.request(|reply| Command::Defer { id, reply }).await
    }

    /// Makes a deferred download eligible for ordinary queue scheduling.
    /// Repeating the operation is safe and never changes a paused/terminal status.
    pub async fn release_download(&self, id: DownloadId) -> Result<(), QueueError> {
        self.request(|reply| Command::Release { id, reply }).await
    }

    /// Releases multiple deferred downloads atomically with respect to queue
    /// dispatch, allowing priority ordering to apply across the entire batch.
    pub async fn release_downloads(&self, ids: Vec<DownloadId>) -> Result<(), QueueError> {
        self.request(|reply| Command::ReleaseMany { ids, reply })
            .await
    }

    pub async fn pause(&self, id: DownloadId) -> Result<(), QueueError> {
        self.request(|reply| Command::Pause { id, reply }).await
    }

    pub async fn resume(&self, id: DownloadId) -> Result<(), QueueError> {
        self.request(|reply| Command::Resume { id, reply }).await
    }

    pub async fn cancel(&self, id: DownloadId) -> Result<(), QueueError> {
        self.request(|reply| Command::Cancel { id, reply }).await
    }

    /// Removes a download record after any active worker has stopped.
    ///
    /// Download files and partial data are preserved.
    pub async fn remove_download(&self, id: DownloadId) -> Result<(), QueueError> {
        self.request(|reply| Command::Remove { id, reply }).await
    }

    pub async fn set_priority(
        &self,
        id: DownloadId,
        priority: DownloadPriority,
    ) -> Result<(), QueueError> {
        self.request(|reply| Command::SetPriority {
            id,
            priority,
            reply,
        })
        .await
    }

    pub async fn set_global_connection_limit(&self, limit: usize) -> Result<(), QueueError> {
        self.request(|reply| Command::SetConnectionLimit { limit, reply })
            .await
    }

    pub async fn set_resource_pressure(
        &self,
        pressure: ResourcePressure,
    ) -> Result<(), QueueError> {
        self.request(|reply| Command::SetResourcePressure { pressure, reply })
            .await
    }

    pub async fn snapshot(&self) -> Result<QueueSnapshot, QueueError> {
        self.request(|reply| Command::Snapshot { reply }).await
    }

    pub async fn shutdown(&self) -> Result<(), QueueError> {
        let (reply, result) = oneshot::channel();
        if self
            .commands
            .send(Command::Shutdown { reply })
            .await
            .is_err()
        {
            return Err(self.actor_failure().await);
        }
        match result.await {
            Ok(result) => result?,
            Err(_) => return Err(self.actor_failure().await),
        }

        if let Some(actor) = self.actor.lock().await.take() {
            actor.await.map_err(QueueError::Actor)??;
        }
        Ok(())
    }

    async fn request<T>(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<T, QueueError>>) -> Command,
    ) -> Result<T, QueueError> {
        let (reply, result) = oneshot::channel();
        if self.commands.send(command(reply)).await.is_err() {
            return Err(self.actor_failure().await);
        }
        match result.await {
            Ok(result) => result,
            Err(_) => Err(self.actor_failure().await),
        }
    }

    async fn actor_failure(&self) -> QueueError {
        let actor = self.actor.lock().await.take();
        if let Some(actor) = actor {
            match actor.await {
                Ok(Err(error)) => error,
                Err(error) => QueueError::Actor(error),
                Ok(Ok(())) => QueueError::Stopped,
            }
        } else {
            QueueError::Stopped
        }
    }
}

struct RuntimeResourceManager {
    policy: ResourcePolicy,
    global_bandwidth: Option<Arc<BandwidthLimiter>>,
    engine_bandwidth: HashMap<EngineKind, Arc<BandwidthLimiter>>,
}

impl RuntimeResourceManager {
    fn new(policy: &ResourcePolicy) -> Result<Self, QueueError> {
        let limiter = |rate: Option<u64>| {
            rate.map(BandwidthLimiter::new)
                .transpose()
                .map(|limiter| limiter.map(Arc::new))
                .map_err(|_| QueueError::InvalidBandwidthPolicy)
        };
        let global_bandwidth = limiter(policy.global_bandwidth_bytes_per_second)?;
        let mut engine_bandwidth = HashMap::new();
        for (engine, rate) in [
            (
                EngineKind::DirectHttp,
                policy.engine_bandwidth_limits.direct_http_bytes_per_second,
            ),
            (
                EngineKind::Hls,
                policy.engine_bandwidth_limits.hls_bytes_per_second,
            ),
            (
                EngineKind::Aria2,
                policy.engine_bandwidth_limits.aria2_bytes_per_second,
            ),
            (
                EngineKind::YtDlp,
                policy.engine_bandwidth_limits.yt_dlp_bytes_per_second,
            ),
        ] {
            if let Some(limiter) = limiter(rate)? {
                engine_bandwidth.insert(engine, limiter);
            }
        }
        Ok(Self {
            policy: policy.clone(),
            global_bandwidth,
            engine_bandwidth,
        })
    }

    fn bandwidth_for(&self, engine: EngineKind) -> Result<BandwidthControl, QueueError> {
        let engine_limiter = self.engine_bandwidth.get(&engine).cloned();
        let global_download_share = self
            .policy
            .global_bandwidth_bytes_per_second
            .map(|rate| rate / self.policy.max_active_downloads as u64);
        let per_download_rate = [
            self.policy.per_download_bandwidth_bytes_per_second,
            global_download_share,
        ]
        .into_iter()
        .flatten()
        .min();
        let effective_external_rate = [
            per_download_rate,
            self.policy.engine_bandwidth_limit(engine),
        ]
        .into_iter()
        .flatten()
        .min();
        Ok(BandwidthControl {
            global: self.global_bandwidth.clone(),
            per_download: per_download_rate
                .map(BandwidthLimiter::new)
                .transpose()
                .map_err(|_| QueueError::InvalidBandwidthPolicy)?
                .map(Arc::new),
            engine: engine_limiter,
            external_download_rate_bytes_per_second: effective_external_rate,
        })
    }
}

struct Entry {
    item: DownloadItem,
    sequence: u64,
}

struct ActiveWorker {
    cancellation: watch::Sender<bool>,
    pause: watch::Sender<bool>,
    connection_limit: watch::Sender<usize>,
    desired: usize,
    active: usize,
    allocated: usize,
    sequence: u64,
    pausing: bool,
    cancelling: bool,
    shutdown_requeue: bool,
    pause_waiters: Vec<oneshot::Sender<Result<(), QueueError>>>,
    cancel_waiters: Vec<oneshot::Sender<Result<(), QueueError>>>,
    removing: bool,
    remove_waiters: Vec<oneshot::Sender<Result<(), QueueError>>>,
}

enum Command {
    Enqueue {
        item: DownloadItem,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    EnqueueDeferred {
        item: DownloadItem,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Defer {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Release {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    ReleaseMany {
        ids: Vec<DownloadId>,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Pause {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Resume {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Cancel {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Remove {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    SetPriority {
        id: DownloadId,
        priority: DownloadPriority,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    SetConnectionLimit {
        limit: usize,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    SetResourcePressure {
        pressure: ResourcePressure,
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
    Snapshot {
        reply: oneshot::Sender<Result<QueueSnapshot, QueueError>>,
    },
    Shutdown {
        reply: oneshot::Sender<Result<(), QueueError>>,
    },
}

async fn run_actor<R, E>(
    repository: R,
    engine: Arc<E>,
    policy: ResourcePolicy,
    mut entries: HashMap<DownloadId, Entry>,
    mut next_sequence: u64,
    mut deferred: HashSet<DownloadId>,
    mut commands: mpsc::Receiver<Command>,
) -> Result<(), QueueError>
where
    R: DownloadRepository + Clone + 'static,
    E: ManagedDownloadEngine + 'static,
{
    let state_machine = DownloadStateMachine::new(repository.clone());
    let mut config = QueueConfig {
        max_active_downloads: policy.max_active_downloads,
        global_connection_limit: policy.global_connection_limit,
    };
    config.validate()?;
    let resources = RuntimeResourceManager::new(&policy)?;
    let (resource_sender, mut resource_receiver) = mpsc::channel::<ResourceDemand>(256);
    let mut active = HashMap::<DownloadId, ActiveWorker>::new();
    let mut workers = JoinSet::<(DownloadId, Result<DownloadItem, DownloadEngineError>)>::new();
    let mut task_downloads = HashMap::<TaskId, DownloadId>::new();
    let mut last_priority = None;
    let mut consecutive_priority_starts = 0_u8;
    let mut stopping = false;
    let mut pressure = ResourcePressure::Normal;
    let mut shutdown_waiters = Vec::<oneshot::Sender<Result<(), QueueError>>>::new();

    loop {
        if !stopping {
            start_eligible(
                Arc::clone(&engine),
                &config,
                &mut entries,
                &deferred,
                &mut active,
                &mut workers,
                &mut task_downloads,
                &resource_sender,
                &resources,
                pressure,
                &mut last_priority,
                &mut consecutive_priority_starts,
            )
            .await?;
        }
        allocate_resources(
            &mut active,
            config.global_connection_limit,
            &entries,
            &deferred,
            config.max_active_downloads,
            pressure,
        );

        if stopping && active.is_empty() {
            for waiter in shutdown_waiters {
                let _ = waiter.send(Ok(()));
            }
            return Ok(());
        }

        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    stopping = true;
                    stop_workers_for_shutdown(engine.as_ref(), &entries, &mut active);
                    continue;
                };
                handle_command(
                    command,
                    &repository,
                    engine.as_ref(),
                    &state_machine,
                    &mut config,
                    &mut entries,
                    &mut deferred,
                    &mut active,
                    &mut stopping,
                    &mut pressure,
                    &mut shutdown_waiters,
                    &mut next_sequence,
                ).await?;
                allocate_resources(
                    &mut active,
                    config.global_connection_limit,
                    &entries,
                    &deferred,
                    config.max_active_downloads,
                    pressure,
                );
            }
            joined = workers.join_next_with_id(), if !workers.is_empty() => {
                match joined {
                    Some(Ok((task_id, (id, result)))) => {
                        task_downloads.remove(&task_id);
                        finish_worker(
                            id,
                            result,
                            &repository,
                            &state_machine,
                            &mut entries,
                            &mut deferred,
                            &mut active,
                            &mut next_sequence,
                        ).await?;
                    }
                    Some(Err(error)) => {
                        if let Some(id) = task_downloads.remove(&error.id()) {
                            fail_panicked_worker(
                                id,
                                &repository,
                                &state_machine,
                                &mut entries,
                                &mut deferred,
                                &mut active,
                            ).await?;
                        }
                        warn!(error = %error, "queue worker task panicked and was isolated");
                        continue;
                    }
                    None => {}
                }
            }
            demand = resource_receiver.recv() => {
                if let Some(demand) = demand {
                    if let Some(worker) = active.get_mut(&demand.download_id) {
                        worker.desired = demand
                            .desired_concurrency
                            .clamp(1, resources.policy.per_download_connection_limit);
                        worker.active = demand.active_concurrency.min(worker.desired);
                    }
                    allocate_resources(
                        &mut active,
                        config.global_connection_limit,
                        &entries,
                        &deferred,
                        config.max_active_downloads,
                        pressure,
                    );
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn start_eligible<E>(
    engine: Arc<E>,
    config: &QueueConfig,
    entries: &mut HashMap<DownloadId, Entry>,
    deferred: &HashSet<DownloadId>,
    active: &mut HashMap<DownloadId, ActiveWorker>,
    workers: &mut JoinSet<(DownloadId, Result<DownloadItem, DownloadEngineError>)>,
    task_downloads: &mut HashMap<TaskId, DownloadId>,
    resource_sender: &mpsc::Sender<ResourceDemand>,
    resources: &RuntimeResourceManager,
    pressure: ResourcePressure,
    last_priority: &mut Option<DownloadPriority>,
    consecutive_priority_starts: &mut u8,
) -> Result<(), QueueError>
where
    E: ManagedDownloadEngine + 'static,
{
    let pressure_active_limit = match pressure {
        ResourcePressure::Normal => config.max_active_downloads,
        ResourcePressure::Elevated => config
            .max_active_downloads
            .saturating_mul(3)
            .div_ceil(4)
            .max(1),
        ResourcePressure::High => config.max_active_downloads.div_ceil(2).max(1),
        ResourcePressure::Critical => 0,
    };
    while active.len() < pressure_active_limit
        && used_connections(active) < config.global_connection_limit
    {
        let Some(id) = choose_next(
            entries,
            deferred,
            active,
            last_priority,
            *consecutive_priority_starts,
        ) else {
            break;
        };
        let entry = entries.get(&id).ok_or(QueueError::ResourceInvariant)?;
        let priority = entry.item.priority();
        let bandwidth = resources.bandwidth_for(entry.item.source().engine())?;
        match last_priority {
            Some(previous) if *previous == priority => {
                *consecutive_priority_starts = consecutive_priority_starts.saturating_add(1);
            }
            _ => {
                *last_priority = Some(priority);
                *consecutive_priority_starts = 1;
            }
        }

        let (cancellation_sender, cancellation) = watch::channel(false);
        let (pause_sender, pause) = watch::channel(false);
        let (connection_limit, limit_receiver) = watch::channel(1_usize);
        let id_for_worker = id;
        let engine = Arc::clone(&engine);
        let resource_demand = resource_sender.clone();
        let abort = workers.spawn(async move {
            let result = engine
                .download_managed(
                    id_for_worker,
                    ManagedDownloadControl {
                        cancellation,
                        pause,
                        connection_limit: limit_receiver,
                        resource_demand: Some(resource_demand),
                        bandwidth,
                    },
                )
                .await;
            (id_for_worker, result)
        });
        task_downloads.insert(abort.id(), id);
        let sequence = entry.sequence;
        active.insert(
            id,
            ActiveWorker {
                cancellation: cancellation_sender,
                pause: pause_sender,
                connection_limit,
                desired: DEFAULT_DESIRED_CONNECTIONS
                    .min(resources.policy.per_download_connection_limit),
                active: 0,
                allocated: 1,
                sequence,
                pausing: false,
                cancelling: false,
                shutdown_requeue: false,
                pause_waiters: Vec::new(),
                cancel_waiters: Vec::new(),
                removing: false,
                remove_waiters: Vec::new(),
            },
        );
    }
    Ok(())
}

fn choose_next(
    entries: &HashMap<DownloadId, Entry>,
    deferred: &HashSet<DownloadId>,
    active: &HashMap<DownloadId, ActiveWorker>,
    last_priority: &Option<DownloadPriority>,
    consecutive_priority_starts: u8,
) -> Option<DownloadId> {
    let mut queued = entries
        .iter()
        .filter(|(id, entry)| {
            entry.item.status() == DownloadStatus::Queued
                && !active.contains_key(id)
                && !deferred.contains(id)
        })
        .map(|(id, entry)| (*id, entry.item.priority(), entry.sequence))
        .collect::<Vec<_>>();
    if queued.is_empty() {
        return None;
    }
    queued.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.2.cmp(&right.2)));
    let highest = queued.first()?.1;
    let fairness_due = consecutive_priority_starts >= FAIRNESS_STARTS
        && last_priority.is_some_and(|priority| priority == highest);
    if fairness_due
        && let Some((id, _, _)) = queued
            .iter()
            .filter(|(_, priority, _)| *priority < highest)
            .min_by_key(|(_, _, sequence)| *sequence)
    {
        return Some(*id);
    }
    queued.first().map(|(id, _, _)| *id)
}

fn used_connections(active: &HashMap<DownloadId, ActiveWorker>) -> usize {
    active
        .values()
        .map(|worker| worker.active.max(worker.allocated).max(1))
        .sum()
}

fn allocate_resources(
    active: &mut HashMap<DownloadId, ActiveWorker>,
    limit: usize,
    entries: &HashMap<DownloadId, Entry>,
    deferred: &HashSet<DownloadId>,
    max_active_downloads: usize,
    pressure: ResourcePressure,
) {
    let active_count = active.len();
    let queued_count = entries
        .iter()
        .filter(|(id, entry)| {
            entry.item.status() == DownloadStatus::Queued
                && !active.contains_key(id)
                && !deferred.contains(id)
        })
        .count();
    let reserved_for_queued = queued_count.min(max_active_downloads.saturating_sub(active_count));
    let mut workers = active.iter_mut().collect::<Vec<_>>();
    workers.sort_by_key(|(id, worker)| {
        let priority = entries
            .get(id)
            .map_or(DownloadPriority::Normal, |entry| entry.item.priority());
        (Reverse(priority), worker.sequence)
    });
    let mut allocation = workers
        .iter()
        .map(|(_, worker)| worker.active.max(1))
        .collect::<Vec<_>>();
    let mut used = allocation.iter().sum::<usize>();
    let reserved_for_queued = reserved_for_queued.min(limit.saturating_sub(used));
    let allocation_limit = limit.saturating_sub(reserved_for_queued);

    while used < allocation_limit {
        let mut changed = false;
        for (index, (_, worker)) in workers.iter_mut().enumerate() {
            if used >= allocation_limit {
                break;
            }
            let pressure_target = match pressure {
                ResourcePressure::Normal => worker.desired,
                ResourcePressure::Elevated => worker.desired.saturating_mul(3).div_ceil(4),
                ResourcePressure::High | ResourcePressure::Critical => worker.active,
            };
            let target = pressure_target.max(worker.active).max(1);
            if allocation[index] < target {
                allocation[index] += 1;
                used += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for (index, (_, worker)) in workers.iter_mut().enumerate() {
        let allowed = allocation[index].max(worker.active);
        worker.allocated = allowed;
        worker.connection_limit.send_replace(allowed);
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_command<R, E>(
    command: Command,
    repository: &R,
    engine: &E,
    state_machine: &DownloadStateMachine<R>,
    config: &mut QueueConfig,
    entries: &mut HashMap<DownloadId, Entry>,
    deferred: &mut HashSet<DownloadId>,
    active: &mut HashMap<DownloadId, ActiveWorker>,
    stopping: &mut bool,
    pressure: &mut ResourcePressure,
    shutdown_waiters: &mut Vec<oneshot::Sender<Result<(), QueueError>>>,
    next_sequence: &mut u64,
) -> Result<(), QueueError>
where
    R: DownloadRepository + Clone,
    E: ManagedDownloadEngine,
{
    match command {
        Command::Enqueue { item, reply } => {
            let result = async {
                if item.status() != DownloadStatus::Queued {
                    return Err(QueueError::InvalidStatus {
                        id: item.id(),
                        action: "enqueue",
                        status: item.status(),
                    });
                }
                repository.create(&item).await?;
                let sequence = *next_sequence;
                *next_sequence = next_sequence
                    .checked_add(1)
                    .ok_or(QueueError::ResourceInvariant)?;
                entries.insert(item.id(), Entry { item, sequence });
                Ok(())
            }
            .await;
            let _ = reply.send(result);
        }
        Command::EnqueueDeferred { item, reply } => {
            let result = async {
                if item.status() != DownloadStatus::Queued {
                    return Err(QueueError::InvalidStatus {
                        id: item.id(),
                        action: "enqueue",
                        status: item.status(),
                    });
                }
                repository.create(&item).await?;
                deferred.insert(item.id());
                let sequence = *next_sequence;
                *next_sequence = next_sequence
                    .checked_add(1)
                    .ok_or(QueueError::ResourceInvariant)?;
                entries.insert(item.id(), Entry { item, sequence });
                Ok(())
            }
            .await;
            let _ = reply.send(result);
        }
        Command::Defer { id, reply } => {
            let result = async {
                if active.contains_key(&id) {
                    let item = repository
                        .get(id)
                        .await?
                        .ok_or(StateMachineError::NotFound(id))?;
                    return Err(QueueError::InvalidStatus {
                        id,
                        action: "defer",
                        status: item.status(),
                    });
                }
                let item = repository
                    .get(id)
                    .await?
                    .ok_or(StateMachineError::NotFound(id))?;
                if !matches!(
                    item.status(),
                    DownloadStatus::Queued | DownloadStatus::Paused
                ) {
                    return Err(QueueError::InvalidStatus {
                        id,
                        action: "defer",
                        status: item.status(),
                    });
                }
                deferred.insert(id);
                if let Some(entry) = entries.get_mut(&id) {
                    entry.item = item;
                }
                Ok(())
            }
            .await;
            let _ = reply.send(result);
        }
        Command::Release { id, reply } => {
            deferred.remove(&id);
            let _ = reply.send(Ok(()));
        }
        Command::ReleaseMany { ids, reply } => {
            for id in ids {
                deferred.remove(&id);
            }
            let _ = reply.send(Ok(()));
        }
        Command::Pause { id, reply } => {
            if active.contains_key(&id) {
                let engine_kind = entries
                    .get(&id)
                    .map(|entry| entry.item.source().engine())
                    .unwrap_or(EngineKind::DirectHttp);
                if !engine.supports_active_pause(engine_kind) {
                    let _ = reply.send(Err(QueueError::UnsupportedEngineOperation {
                        engine: engine_kind,
                        operation: "active pause",
                    }));
                    return Ok(());
                }
                if let Some(worker) = active.get_mut(&id) {
                    if worker.cancelling {
                        let _ = reply.send(Err(QueueError::InvalidStatus {
                            id,
                            action: "pause",
                            status: DownloadStatus::Downloading,
                        }));
                    } else {
                        worker.pausing = true;
                        let _ = worker.pause.send(true);
                        worker.pause_waiters.push(reply);
                    }
                }
                return Ok(());
            } else {
                let result = async {
                    let item = repository
                        .get(id)
                        .await?
                        .ok_or(StateMachineError::NotFound(id))?;
                    match item.status() {
                        DownloadStatus::Paused => Ok(()),
                        DownloadStatus::Queued => {
                            let paused =
                                state_machine.transition(id, DownloadStatus::Paused).await?;
                            if let Some(entry) = entries.get_mut(&id) {
                                entry.item = paused;
                            }
                            Ok(())
                        }
                        status => Err(QueueError::InvalidStatus {
                            id,
                            action: "pause",
                            status,
                        }),
                    }
                }
                .await;
                let _ = reply.send(result);
            }
        }
        Command::Resume { id, reply } => {
            if let Some(worker) = active.get_mut(&id) {
                if worker.pausing && !worker.cancelling {
                    worker.pausing = false;
                    let _ = worker.pause.send(false);
                    for waiter in worker.pause_waiters.drain(..) {
                        let _ = waiter.send(Err(QueueError::PauseSuperseded));
                    }
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(QueueError::InvalidStatus {
                        id,
                        action: "resume",
                        status: DownloadStatus::Downloading,
                    }));
                }
                return Ok(());
            } else {
                let result = async {
                    let item = repository
                        .get(id)
                        .await?
                        .ok_or(StateMachineError::NotFound(id))?;
                    match item.status() {
                        DownloadStatus::Queued => Ok(()),
                        DownloadStatus::Paused => {
                            repository.move_to_queue_tail(id).await?;
                            let queued =
                                state_machine.transition(id, DownloadStatus::Queued).await?;
                            let sequence = *next_sequence;
                            *next_sequence = next_sequence
                                .checked_add(1)
                                .ok_or(QueueError::ResourceInvariant)?;
                            entries.insert(
                                id,
                                Entry {
                                    item: queued,
                                    sequence,
                                },
                            );
                            Ok(())
                        }
                        status => Err(QueueError::InvalidStatus {
                            id,
                            action: "resume",
                            status,
                        }),
                    }
                }
                .await;
                let _ = reply.send(result);
            }
        }
        Command::Cancel { id, reply } => {
            if let Some(worker) = active.get_mut(&id) {
                worker.shutdown_requeue = false;
                worker.cancelling = true;
                let _ = worker.cancellation.send(true);
                worker.cancel_waiters.push(reply);
                return Ok(());
            } else {
                let result = async {
                    let item = repository
                        .get(id)
                        .await?
                        .ok_or(StateMachineError::NotFound(id))?;
                    match item.status() {
                        DownloadStatus::Cancelled => Ok(()),
                        DownloadStatus::Queued | DownloadStatus::Paused => {
                            if item.status() == DownloadStatus::Paused {
                                engine.cancel_inactive(id).await?;
                            }
                            let cancelled = state_machine
                                .transition(id, DownloadStatus::Cancelled)
                                .await?;
                            if let Some(entry) = entries.get_mut(&id) {
                                entry.item = cancelled;
                            }
                            Ok(())
                        }
                        status => Err(QueueError::InvalidStatus {
                            id,
                            action: "cancel",
                            status,
                        }),
                    }
                }
                .await;
                let _ = reply.send(result);
            }
        }
        Command::Remove { id, reply } => {
            if let Some(worker) = active.get_mut(&id) {
                worker.shutdown_requeue = false;
                worker.cancelling = true;
                worker.removing = true;
                let _ = worker.cancellation.send(true);
                worker.remove_waiters.push(reply);
                return Ok(());
            }

            let result = async {
                let Some(item) = repository.get(id).await? else {
                    entries.remove(&id);
                    deferred.remove(&id);
                    return Err(QueueError::DownloadNotFound(id));
                };

                if item.status() == DownloadStatus::Paused {
                    engine.cancel_inactive(id).await?;
                }

                if matches!(
                    item.status(),
                    DownloadStatus::Queued
                        | DownloadStatus::Preparing
                        | DownloadStatus::Downloading
                        | DownloadStatus::Paused
                ) {
                    state_machine
                        .transition(id, DownloadStatus::Cancelled)
                        .await?;
                }

                if !repository.delete(id).await? {
                    entries.remove(&id);
                    deferred.remove(&id);
                    return Err(QueueError::DownloadNotFound(id));
                }
                entries.remove(&id);
                deferred.remove(&id);
                Ok(())
            }
            .await;
            let _ = reply.send(result);
        }
        Command::SetPriority {
            id,
            priority,
            reply,
        } => {
            let result = async {
                if !repository.set_priority(id, priority).await? {
                    return Err(QueueError::StateMachine(StateMachineError::NotFound(id)));
                }
                if let Some(entry) = entries.get_mut(&id) {
                    entry
                        .item
                        .set_priority_at(priority, entry.item.updated_at())
                        .map_err(|error| {
                            QueueError::StateMachine(StateMachineError::Domain(error))
                        })?;
                }
                Ok(())
            }
            .await;
            let _ = reply.send(result);
        }
        Command::SetConnectionLimit { limit, reply } => {
            let result = if limit == 0 || limit > MAX_GLOBAL_CONNECTIONS {
                Err(QueueError::InvalidConfiguration)
            } else if limit < used_connections(active) {
                Err(QueueError::ResourceLimitInUse)
            } else {
                config.global_connection_limit = limit;
                Ok(())
            };
            let _ = reply.send(result);
        }
        Command::SetResourcePressure {
            pressure: requested,
            reply,
        } => {
            *pressure = requested;
            let _ = reply.send(Ok(()));
        }
        Command::Snapshot { reply } => {
            let result = async {
                let items = repository.list().await?;
                let by_id = items
                    .iter()
                    .map(|item| (item.id(), item))
                    .collect::<HashMap<_, _>>();
                let queued = items
                    .iter()
                    .filter(|item| {
                        item.status() == DownloadStatus::Queued
                            && !active.contains_key(&item.id())
                            && !deferred.contains(&item.id())
                    })
                    .map(DownloadItem::id)
                    .collect();
                let paused = items
                    .iter()
                    .filter(|item| item.status() == DownloadStatus::Paused)
                    .map(DownloadItem::id)
                    .collect();
                let mut active_items = active
                    .iter()
                    .filter_map(|(id, worker)| {
                        by_id.get(id).map(|item| ActiveDownload {
                            item: (*item).clone(),
                            desired_connections: worker.desired,
                            allocated_connections: worker.allocated,
                            active_connections: worker.active,
                        })
                    })
                    .collect::<Vec<_>>();
                active_items.sort_by_key(|worker| {
                    active
                        .get(&worker.item.id())
                        .map_or(u64::MAX, |value| value.sequence)
                });
                Ok(QueueSnapshot {
                    items,
                    queued,
                    paused,
                    active: active_items,
                    global_connection_limit: config.global_connection_limit,
                    resource_pressure: *pressure,
                })
            }
            .await;
            let _ = reply.send(result);
        }
        Command::Shutdown { reply } => {
            *stopping = true;
            stop_workers_for_shutdown(engine, entries, active);
            shutdown_waiters.push(reply);
        }
    }
    Ok(())
}

fn stop_workers_for_shutdown<E>(
    engine: &E,
    entries: &HashMap<DownloadId, Entry>,
    active: &mut HashMap<DownloadId, ActiveWorker>,
) where
    E: ManagedDownloadEngine,
{
    for (id, worker) in active {
        if worker.cancelling {
            continue;
        }
        let engine_kind = entries
            .get(id)
            .map(|entry| entry.item.source().engine())
            .unwrap_or(EngineKind::DirectHttp);
        if engine.supports_active_pause(engine_kind) {
            worker.pausing = true;
            let _ = worker.pause.send(true);
        } else {
            worker.cancelling = true;
            worker.shutdown_requeue = true;
            let _ = worker.cancellation.send(true);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish_worker<R>(
    id: DownloadId,
    result: Result<DownloadItem, DownloadEngineError>,
    repository: &R,
    state_machine: &DownloadStateMachine<R>,
    entries: &mut HashMap<DownloadId, Entry>,
    deferred: &mut HashSet<DownloadId>,
    active: &mut HashMap<DownloadId, ActiveWorker>,
    next_sequence: &mut u64,
) -> Result<(), QueueError>
where
    R: DownloadRepository,
{
    let Some(worker) = active.remove(&id) else {
        return Ok(());
    };
    let Some(mut current) = repository.get(id).await? else {
        if worker.removing {
            entries.remove(&id);
            deferred.remove(&id);
            for waiter in worker.remove_waiters {
                let _ = waiter.send(Err(QueueError::DownloadNotFound(id)));
            }
            return Ok(());
        }
        return Err(QueueError::StateMachine(StateMachineError::NotFound(id)));
    };
    if let Err(error) = &result {
        warn!(download_id = %id, error = %error, "queued download worker returned an error");
    }

    if worker.shutdown_requeue && matches!(&result, Err(DownloadEngineError::Cancelled)) {
        if matches!(
            current.status(),
            DownloadStatus::Preparing | DownloadStatus::Downloading
        ) {
            current = state_machine
                .transition(id, DownloadStatus::Cancelled)
                .await?;
        }
        if current.status() == DownloadStatus::Cancelled {
            state_machine
                .transition(id, DownloadStatus::Preparing)
                .await?;
            current = state_machine.transition(id, DownloadStatus::Queued).await?;
        }
    }

    if matches!(
        current.status(),
        DownloadStatus::Preparing | DownloadStatus::Downloading
    ) {
        let target = if worker.cancelling || matches!(&result, Err(DownloadEngineError::Cancelled))
        {
            Some(DownloadStatus::Cancelled)
        } else if worker.pausing || matches!(&result, Err(DownloadEngineError::Paused)) {
            Some(DownloadStatus::Paused)
        } else {
            Some(DownloadStatus::Failed)
        };
        if let Some(target) = target {
            if target == DownloadStatus::Failed {
                let diagnostic = DownloadError::new(
                    DownloadErrorCode::Internal,
                    "The download worker stopped before reaching a terminal state.",
                )
                .map_err(|error| QueueError::StateMachine(StateMachineError::DomainError(error)))?;
                current = state_machine
                    .transition_with_error(id, target, diagnostic)
                    .await?;
            } else {
                current = state_machine.transition(id, target).await?;
            }
        }
    } else if current.status() == DownloadStatus::Queued
        && result.is_err()
        && !worker.shutdown_requeue
    {
        if worker.cancelling || matches!(&result, Err(DownloadEngineError::Cancelled)) {
            current = state_machine
                .transition(id, DownloadStatus::Cancelled)
                .await?;
        } else if worker.pausing || matches!(&result, Err(DownloadEngineError::Paused)) {
            current = state_machine.transition(id, DownloadStatus::Paused).await?;
        } else {
            state_machine
                .transition(id, DownloadStatus::Preparing)
                .await?;
            let diagnostic = DownloadError::new(
                DownloadErrorCode::Engine,
                "The selected download engine could not start the work.",
            )
            .map_err(|error| QueueError::StateMachine(StateMachineError::DomainError(error)))?;
            current = state_machine
                .transition_with_error(id, DownloadStatus::Failed, diagnostic)
                .await?;
        }
    }
    if result.is_ok() && current.status() != DownloadStatus::Completed {
        return Err(QueueError::InvalidStatus {
            id,
            action: "complete",
            status: current.status(),
        });
    }
    let resume_after_pause = !worker.pausing
        && !worker.cancelling
        && matches!(&result, Err(DownloadEngineError::Paused))
        && current.status() == DownloadStatus::Paused;
    if resume_after_pause {
        current = state_machine.transition(id, DownloadStatus::Queued).await?;
        repository.move_to_queue_tail(id).await?;
        let sequence = *next_sequence;
        *next_sequence = next_sequence
            .checked_add(1)
            .ok_or(QueueError::ResourceInvariant)?;
        if let Some(entry) = entries.get_mut(&id) {
            entry.sequence = sequence;
        }
    }
    if let Some(entry) = entries.get_mut(&id) {
        entry.item = current.clone();
    }

    for waiter in worker.pause_waiters {
        let response = if current.status() == DownloadStatus::Paused {
            Ok(())
        } else {
            Err(QueueError::InvalidStatus {
                id,
                action: "pause",
                status: current.status(),
            })
        };
        let _ = waiter.send(response);
    }
    for waiter in worker.cancel_waiters {
        let response = if current.status() == DownloadStatus::Cancelled {
            Ok(())
        } else {
            Err(QueueError::InvalidStatus {
                id,
                action: "cancel",
                status: current.status(),
            })
        };
        let _ = waiter.send(response);
    }
    if worker.removing {
        let deleted = repository.delete(id).await?;
        entries.remove(&id);
        deferred.remove(&id);
        let result = if deleted {
            Ok(())
        } else {
            Err(QueueError::DownloadNotFound(id))
        };
        for waiter in worker.remove_waiters {
            let _ = waiter.send(match &result {
                Ok(()) => Ok(()),
                Err(_) => Err(QueueError::DownloadNotFound(id)),
            });
        }
        return Ok(());
    }
    info!(download_id = %id, status = ?current.status(), "queued download worker released");
    Ok(())
}

async fn fail_panicked_worker<R>(
    id: DownloadId,
    repository: &R,
    state_machine: &DownloadStateMachine<R>,
    entries: &mut HashMap<DownloadId, Entry>,
    deferred: &mut HashSet<DownloadId>,
    active: &mut HashMap<DownloadId, ActiveWorker>,
) -> Result<(), QueueError>
where
    R: DownloadRepository,
{
    let Some(worker) = active.remove(&id) else {
        return Ok(());
    };
    let Some(mut item) = repository.get(id).await? else {
        if worker.removing {
            entries.remove(&id);
            deferred.remove(&id);
            for waiter in worker.remove_waiters {
                let _ = waiter.send(Err(QueueError::DownloadNotFound(id)));
            }
            return Ok(());
        }
        return Err(QueueError::StateMachine(StateMachineError::NotFound(id)));
    };
    if matches!(
        item.status(),
        DownloadStatus::Preparing | DownloadStatus::Downloading
    ) {
        if worker.cancelling {
            item = state_machine
                .transition(id, DownloadStatus::Cancelled)
                .await?;
        } else if worker.pausing {
            item = state_machine.transition(id, DownloadStatus::Paused).await?;
        } else {
            let diagnostic = DownloadError::new(
                DownloadErrorCode::Internal,
                "The download worker terminated unexpectedly.",
            )
            .map_err(|error| QueueError::StateMachine(StateMachineError::DomainError(error)))?;
            item = state_machine
                .transition_with_error(id, DownloadStatus::Failed, diagnostic)
                .await?;
        }
    } else if item.status() == DownloadStatus::Queued {
        if worker.cancelling {
            item = state_machine
                .transition(id, DownloadStatus::Cancelled)
                .await?;
        } else if worker.pausing {
            item = state_machine.transition(id, DownloadStatus::Paused).await?;
        } else {
            state_machine
                .transition(id, DownloadStatus::Preparing)
                .await?;
            let diagnostic = DownloadError::new(
                DownloadErrorCode::Internal,
                "The download worker terminated before it could start.",
            )
            .map_err(|error| QueueError::StateMachine(StateMachineError::DomainError(error)))?;
            item = state_machine
                .transition_with_error(id, DownloadStatus::Failed, diagnostic)
                .await?;
        }
    }
    if worker.shutdown_requeue && item.status() == DownloadStatus::Cancelled {
        state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        item = state_machine.transition(id, DownloadStatus::Queued).await?;
    }
    if let Some(entry) = entries.get_mut(&id) {
        entry.item = item.clone();
    }
    for waiter in worker.pause_waiters {
        let result = if item.status() == DownloadStatus::Paused {
            Ok(())
        } else {
            Err(QueueError::InvalidStatus {
                id,
                action: "pause",
                status: item.status(),
            })
        };
        let _ = waiter.send(result);
    }
    for waiter in worker.cancel_waiters {
        let result = if item.status() == DownloadStatus::Cancelled {
            Ok(())
        } else {
            Err(QueueError::InvalidStatus {
                id,
                action: "cancel",
                status: item.status(),
            })
        };
        let _ = waiter.send(result);
    }
    if worker.removing {
        let deleted = repository.delete(id).await?;
        entries.remove(&id);
        deferred.remove(&id);
        let result = if deleted {
            Ok(())
        } else {
            Err(QueueError::DownloadNotFound(id))
        };
        for waiter in worker.remove_waiters {
            let _ = waiter.send(match &result {
                Ok(()) => Ok(()),
                Err(_) => Err(QueueError::DownloadNotFound(id)),
            });
        }
        return Ok(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        error::Error,
        ffi::OsString,
        path::{Path, PathBuf},
        sync::Arc,
        time::Duration,
    };

    use fetchx_core::DownloadStateMachine;
    use fetchx_downloader::{
        Aria2Client, Aria2ClientConfig, Aria2Engine, DirectHttpEngine, DownloadEngine,
        DownloadEngineError, EngineDispatcher, ManagedDownloadControl, ManagedDownloadEngine,
        ResourceDemand, YtDlpConfig, YtDlpEngine, YtDlpError, YtDlpProcessExit, YtDlpProcessRunner,
        YtDlpRunnerEvent, YtDlpRunnerFuture,
    };
    use fetchx_models::{
        DownloadId, DownloadItem, DownloadPriority, DownloadSource, DownloadStatus, EngineKind,
        FileName, ResourcePolicy, ResourcePressure, ResourceProfile, Timestamp,
    };
    use fetchx_storage::{Database, DownloadRepository, SqliteDownloadRepository};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{mpsc, watch},
        task::JoinHandle,
    };

    use super::{
        ActiveWorker, Entry, QueueConfig, QueueError, QueueManager, RuntimeResourceManager,
        allocate_resources, choose_next, used_connections,
    };

    struct WaitingEngine {
        repository: SqliteDownloadRepository,
        release: watch::Receiver<bool>,
        supports_yt_dlp_pause: bool,
        desired_connections: usize,
    }

    impl DownloadEngine for WaitingEngine {
        async fn download(
            &self,
            _id: DownloadId,
            _cancellation: watch::Receiver<bool>,
        ) -> Result<DownloadItem, DownloadEngineError> {
            Err(DownloadEngineError::InvalidConfiguration)
        }
    }

    impl ManagedDownloadEngine for WaitingEngine {
        async fn download_managed(
            &self,
            id: DownloadId,
            mut control: ManagedDownloadControl,
        ) -> Result<DownloadItem, DownloadEngineError> {
            let state_machine = DownloadStateMachine::new(self.repository.clone());
            let item = self
                .repository
                .get(id)
                .await?
                .ok_or(DownloadEngineError::NotFound(id))?;
            if item.status() == DownloadStatus::Queued {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
            }
            state_machine
                .transition(id, DownloadStatus::Downloading)
                .await?;
            control
                .resource_demand
                .ok_or(DownloadEngineError::ResourceManagerUnavailable)?
                .send(ResourceDemand {
                    download_id: id,
                    desired_concurrency: self.desired_connections,
                    active_concurrency: 1,
                })
                .await
                .map_err(|_| DownloadEngineError::ResourceManagerUnavailable)?;
            let mut release = self.release.clone();
            loop {
                if *control.cancellation.borrow() {
                    let item = state_machine
                        .transition(id, DownloadStatus::Cancelled)
                        .await?;
                    let _ = item;
                    return Err(DownloadEngineError::Cancelled);
                }
                if *control.pause.borrow() {
                    state_machine.transition(id, DownloadStatus::Paused).await?;
                    return Err(DownloadEngineError::Paused);
                }
                if *release.borrow() {
                    return state_machine
                        .transition(id, DownloadStatus::Completed)
                        .await
                        .map_err(Into::into);
                }
                tokio::select! {
                    changed = control.cancellation.changed() => {
                        if changed.is_err() || *control.cancellation.borrow() {
                            state_machine.transition(id, DownloadStatus::Cancelled).await?;
                            return Err(DownloadEngineError::Cancelled);
                        }
                    }
                    changed = control.pause.changed() => {
                        if changed.is_err() || *control.pause.borrow() {
                            state_machine.transition(id, DownloadStatus::Paused).await?;
                            return Err(DownloadEngineError::Paused);
                        }
                    }
                    changed = release.changed() => {
                        if changed.is_err() {
                            return Err(DownloadEngineError::Cancelled);
                        }
                    }
                }
            }
        }

        fn supports_active_pause(&self, engine: EngineKind) -> bool {
            engine != EngineKind::YtDlp || self.supports_yt_dlp_pause
        }
    }

    struct RejectInactiveCancelEngine;

    struct QueueYtDlpRunner;

    impl YtDlpProcessRunner for QueueYtDlpRunner {
        fn validate_version<'a>(
            &'a self,
            _executable: &'a Path,
            _timeout: Duration,
        ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
            Box::pin(async { Ok("test-version".to_owned()) })
        }

        fn run<'a>(
            &'a self,
            _executable: &'a Path,
            arguments: &'a [OsString],
            _stop: watch::Receiver<bool>,
            events: mpsc::Sender<YtDlpRunnerEvent>,
        ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
            Box::pin(async move {
                let workspace = arguments
                    .iter()
                    .position(|argument| argument == "--paths")
                    .and_then(|index| arguments.get(index + 1))
                    .and_then(|argument| argument.to_str())
                    .and_then(|value| value.strip_prefix("home:"))
                    .ok_or(YtDlpError::ProcessRunner)?;
                tokio::fs::write(Path::new(workspace).join("output.webm"), b"video")
                    .await
                    .map_err(|_| YtDlpError::ProcessRunner)?;
                events
                    .send(YtDlpRunnerEvent::ProgressLine(
                        "FETCHX_PROGRESS|100%|5|5|NA|1|0".to_owned(),
                    ))
                    .await
                    .map_err(|_| YtDlpError::ProcessRunner)?;
                Ok(YtDlpProcessExit {
                    success: true,
                    code: Some(0),
                })
            })
        }
    }

    impl DownloadEngine for RejectInactiveCancelEngine {
        async fn download(
            &self,
            _id: DownloadId,
            _cancellation: watch::Receiver<bool>,
        ) -> Result<DownloadItem, DownloadEngineError> {
            Err(DownloadEngineError::InvalidConfiguration)
        }
    }

    impl ManagedDownloadEngine for RejectInactiveCancelEngine {
        async fn download_managed(
            &self,
            _id: DownloadId,
            _control: ManagedDownloadControl,
        ) -> Result<DownloadItem, DownloadEngineError> {
            Err(DownloadEngineError::EngineNotAvailable)
        }

        #[allow(clippy::manual_async_fn)]
        fn cancel_inactive(
            &self,
            _id: DownloadId,
        ) -> impl std::future::Future<Output = Result<(), DownloadEngineError>> + Send {
            async { Err(DownloadEngineError::Aria2ReconciliationRequired) }
        }
    }

    fn item() -> Result<DownloadItem, Box<dyn std::error::Error>> {
        Ok(DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file", EngineKind::DirectHttp)?,
            FileName::new("file.bin")?,
            std::env::temp_dir()
                .join(format!("fetchx-queue-{}", DownloadId::new()))
                .join("file.bin"),
            Timestamp::from_unix_millis(1),
        ))
    }

    async fn setup_engine() -> Result<
        (
            SqliteDownloadRepository,
            Arc<WaitingEngine>,
            watch::Sender<bool>,
        ),
        Box<dyn std::error::Error>,
    > {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let (release_sender, release) = watch::channel(false);
        let engine = Arc::new(WaitingEngine {
            repository: repository.clone(),
            release,
            supports_yt_dlp_pause: true,
            desired_connections: 4,
        });
        Ok((repository, engine, release_sender))
    }

    async fn persist_item_in_status(
        repository: &SqliteDownloadRepository,
        status: DownloadStatus,
    ) -> Result<DownloadId, Box<dyn std::error::Error>> {
        let download = item()?;
        let id = download.id();
        repository.create(&download).await?;
        let state_machine = DownloadStateMachine::new(repository.clone());
        match status {
            DownloadStatus::Queued => {}
            DownloadStatus::Preparing => {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
            }
            DownloadStatus::Downloading => {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
                state_machine
                    .transition(id, DownloadStatus::Downloading)
                    .await?;
            }
            DownloadStatus::Paused => {
                state_machine.transition(id, DownloadStatus::Paused).await?;
            }
            DownloadStatus::Completed => {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
                state_machine
                    .transition(id, DownloadStatus::Downloading)
                    .await?;
                state_machine
                    .transition(id, DownloadStatus::Completed)
                    .await?;
            }
            DownloadStatus::Failed => {
                state_machine
                    .transition(id, DownloadStatus::Preparing)
                    .await?;
                state_machine.transition(id, DownloadStatus::Failed).await?;
            }
            DownloadStatus::Cancelled => {
                state_machine
                    .transition(id, DownloadStatus::Cancelled)
                    .await?;
            }
            status => {
                return Err(format!("persist_item_in_status does not support {status:?}").into());
            }
        }
        Ok(id)
    }

    async fn wait_for_active(
        manager: &QueueManager,
        expected: usize,
    ) -> Result<super::QueueSnapshot, Box<dyn std::error::Error>> {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot.active.len() == expected {
                    return Ok(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?
    }

    async fn start_aria2_mock()
    -> Result<(String, mpsc::UnboundedReceiver<String>, JoinHandle<()>), Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/jsonrpc", listener.local_addr()?);
        let (sender, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let responses = [
                r#"{"jsonrpc":"2.0","id":"1","result":"0123456789abcdef"}"#,
                r#"{"jsonrpc":"2.0","id":"2","result":{"gid":"0123456789abcdef","status":"error","totalLength":"0","completedLength":"0","downloadSpeed":"0","connections":"0","errorCode":"1"}}"#,
            ];
            for body in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 1024];
                let body_start = loop {
                    let Ok(read) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..body_start]);
                let Some(content_length) = headers.lines().find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                }) else {
                    return;
                };
                while bytes.len() < body_start + content_length {
                    let Ok(read) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let request =
                    String::from_utf8_lossy(&bytes[body_start..body_start + content_length])
                        .into_owned();
                let _ = sender.send(request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                if stream.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        Ok((endpoint, requests, task))
    }

    #[tokio::test]
    async fn limits_active_downloads_and_allocates_only_within_global_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, release) = setup_engine().await?;
        let manager = QueueManager::new(
            repository,
            engine,
            QueueConfig {
                max_active_downloads: 2,
                global_connection_limit: 3,
            },
        )
        .await?;
        for _ in 0..4 {
            manager.enqueue(item()?).await?;
        }

        let snapshot = match wait_for_active(&manager, 2).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(format!("{error}; queue state: {:?}", manager.snapshot().await).into());
            }
        };
        assert!(
            snapshot
                .active
                .iter()
                .map(|worker| worker.allocated_connections)
                .sum::<usize>()
                <= 3
        );
        assert_eq!(snapshot.items.len(), 4);
        assert_eq!(snapshot.queued.len(), 4 - snapshot.active.len());
        assert!(matches!(
            manager.set_global_connection_limit(1).await,
            Err(QueueError::ResourceLimitInUse)
        ));

        release.send(true)?;
        let completion = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot
                    .items
                    .iter()
                    .all(|download| download.status() == DownloadStatus::Completed)
                {
                    break Ok::<_, QueueError>(());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(
            completion.is_ok(),
            "downloads did not complete: {:?}",
            manager.snapshot().await?
        );
        completion??;
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn global_budget_bounds_extreme_workers_across_queue_and_priority_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let (release_sender, release) = watch::channel(false);
        let engine = Arc::new(WaitingEngine {
            repository: repository.clone(),
            release,
            supports_yt_dlp_pause: true,
            desired_connections: 32,
        });
        let mut policy = ResourcePolicy::for_profile(ResourceProfile::Unlimited);
        policy.max_active_downloads = 3;
        policy.global_connection_limit = 48;
        let manager = QueueManager::new_with_resource_policy(repository, engine, policy).await?;
        let downloads = [item()?, item()?, item()?];
        for download in downloads {
            manager.enqueue(download).await?;
        }

        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot.active.len() == 3
                    && snapshot
                        .active
                        .iter()
                        .all(|worker| worker.desired_connections == 32)
                {
                    return Ok::<_, QueueError>(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert!(
            snapshot
                .active
                .iter()
                .map(|worker| worker.allocated_connections)
                .sum::<usize>()
                <= snapshot.global_connection_limit
        );
        let cancelled_id = snapshot.active[0].item.id();
        manager
            .set_priority(cancelled_id, DownloadPriority::Low)
            .await?;

        let mut high_priority = item()?;
        let high_priority_id = high_priority.id();
        high_priority.set_priority_at(DownloadPriority::High, high_priority.updated_at())?;
        manager.enqueue(high_priority).await?;
        manager.cancel(cancelled_id).await?;

        let replacement_snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot
                    .active
                    .iter()
                    .any(|worker| worker.item.id() == high_priority_id)
                    && snapshot
                        .active
                        .iter()
                        .all(|worker| worker.desired_connections == 32)
                {
                    return Ok::<_, QueueError>(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert_eq!(replacement_snapshot.active.len(), 3);
        assert!(
            replacement_snapshot
                .active
                .iter()
                .map(|worker| worker.allocated_connections)
                .sum::<usize>()
                <= replacement_snapshot.global_connection_limit
        );
        assert!(
            replacement_snapshot
                .active
                .iter()
                .all(|worker| worker.desired_connections == 32)
        );

        manager.shutdown().await?;
        drop(release_sender);
        Ok(())
    }

    #[tokio::test]
    async fn critical_pressure_blocks_new_work_and_high_pressure_limits_starts()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, release) = setup_engine().await?;
        let manager = QueueManager::new(
            repository,
            engine,
            QueueConfig {
                max_active_downloads: 4,
                global_connection_limit: 16,
            },
        )
        .await?;
        manager
            .set_resource_pressure(ResourcePressure::Critical)
            .await?;
        for _ in 0..4 {
            manager.enqueue(item()?).await?;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(manager.snapshot().await?.active.is_empty());

        manager
            .set_resource_pressure(ResourcePressure::High)
            .await?;
        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot.active.len() == 2 {
                    return Ok::<_, QueueError>(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert_eq!(snapshot.queued.len(), 2);
        manager.shutdown().await?;
        drop(release);
        Ok(())
    }

    #[tokio::test]
    async fn elevated_pressure_reduces_new_active_downloads_without_stopping_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, release) = setup_engine().await?;
        let manager = QueueManager::new(
            repository,
            engine,
            QueueConfig {
                max_active_downloads: 4,
                global_connection_limit: 16,
            },
        )
        .await?;
        manager
            .set_resource_pressure(ResourcePressure::Elevated)
            .await?;
        for _ in 0..4 {
            manager.enqueue(item()?).await?;
        }
        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot.active.len() == 3 {
                    return Ok::<_, QueueError>(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert_eq!(snapshot.queued.len(), 1);
        manager.shutdown().await?;
        drop(release);
        Ok(())
    }

    #[test]
    fn external_engines_receive_a_share_of_the_global_bandwidth_ceiling() {
        let mut policy = ResourcePolicy::for_profile(ResourceProfile::Unlimited);
        policy.max_active_downloads = 4;
        policy.global_bandwidth_bytes_per_second = Some(8000);
        policy.engine_bandwidth_limits.aria2_bytes_per_second = Some(1500);
        let resources = RuntimeResourceManager::new(&policy).expect("valid resource policy");
        let aria2 = resources
            .bandwidth_for(EngineKind::Aria2)
            .expect("aria2 limits should be created");
        let yt_dlp = resources
            .bandwidth_for(EngineKind::YtDlp)
            .expect("yt-dlp limits should be created");
        assert_eq!(aria2.external_download_rate_bytes_per_second, Some(1500));
        assert_eq!(yt_dlp.external_download_rate_bytes_per_second, Some(2000));
        assert_eq!(
            resources
                .bandwidth_for(EngineKind::DirectHttp)
                .expect("Direct HTTP limits should be created")
                .per_download
                .expect("global share limiter")
                .bytes_per_second(),
            2000
        );
        assert!(Arc::ptr_eq(
            aria2.global.as_ref().expect("global limiter"),
            yt_dlp.global.as_ref().expect("global limiter"),
        ));
    }

    #[tokio::test]
    async fn queue_dispatches_mixed_direct_http_aria2_and_ytdlp_items()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let (endpoint, mut requests, server) = start_aria2_mock().await?;
        let yt_dlp_base =
            std::env::temp_dir().join(format!("fetchx-queue-ytdlp-test-{}", DownloadId::new()));
        tokio::fs::create_dir_all(&yt_dlp_base).await?;
        let direct_http = Arc::new(DirectHttpEngine::new(repository.clone())?);
        let aria2 = Arc::new(Aria2Engine::with_poll_interval(
            repository.clone(),
            Aria2Client::new(Aria2ClientConfig::new(
                endpoint,
                Some("queue-test-token".to_owned()),
                Duration::from_secs(1),
            ))?,
            Duration::from_millis(10),
        ));
        let yt_dlp = Arc::new(
            YtDlpEngine::initialize_with_runner(
                repository.clone(),
                YtDlpConfig::new().with_executable(std::env::current_exe()?),
                Arc::new(QueueYtDlpRunner),
            )
            .await?,
        );
        let dispatcher = Arc::new(
            EngineDispatcher::new(repository.clone(), direct_http, aria2).with_yt_dlp(yt_dlp),
        );
        let manager =
            QueueManager::new(repository.clone(), dispatcher, QueueConfig::default()).await?;

        let direct = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("not-a-url", EngineKind::DirectHttp)?,
            FileName::new("direct.bin")?,
            PathBuf::from("direct.bin"),
            Timestamp::now()?,
        );
        let aria_item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/aria.bin", EngineKind::Aria2)?,
            FileName::new("aria.bin")?,
            PathBuf::from("aria.bin"),
            Timestamp::now()?,
        );
        let yt_dlp_item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/video", EngineKind::YtDlp)?,
            FileName::new("video.webm")?,
            yt_dlp_base.join("video.webm"),
            Timestamp::now()?,
        );
        let direct_id = direct.id();
        let aria_id = aria_item.id();
        let yt_dlp_id = yt_dlp_item.id();
        manager.enqueue(direct).await?;
        manager.enqueue(aria_item).await?;
        manager.enqueue(yt_dlp_item).await?;

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let direct_status = repository.get(direct_id).await?.map(|item| item.status());
                let aria_status = repository.get(aria_id).await?.map(|item| item.status());
                let yt_dlp_status = repository.get(yt_dlp_id).await?.map(|item| item.status());
                if direct_status == Some(DownloadStatus::Failed)
                    && aria_status == Some(DownloadStatus::Failed)
                    && yt_dlp_status == Some(DownloadStatus::Completed)
                {
                    return Ok::<_, fetchx_storage::StorageError>(());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;

        let add = requests
            .recv()
            .await
            .ok_or("aria2 addUri was not dispatched")?;
        let status = requests
            .recv()
            .await
            .ok_or("aria2 tellStatus was not dispatched")?;
        assert!(add.contains("\"method\":\"aria2.addUri\""));
        assert!(add.contains("\"token:queue-test-token\""));
        assert!(status.contains("\"method\":\"aria2.tellStatus\""));
        manager.shutdown().await?;
        server.await?;
        assert_eq!(
            tokio::fs::read(yt_dlp_base.join("video.webm")).await?,
            b"video"
        );
        tokio::fs::remove_dir_all(yt_dlp_base).await?;
        Ok(())
    }

    #[tokio::test]
    async fn pause_resume_and_cancel_follow_queue_lifecycle()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(
            repository.clone(),
            engine,
            QueueConfig {
                max_active_downloads: 1,
                global_connection_limit: 2,
            },
        )
        .await?;
        let download = item()?;
        let id = download.id();
        manager.enqueue(download).await?;
        wait_for_active(&manager, 1).await?;

        manager.pause(id).await?;
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("download missing")?
                .status(),
            DownloadStatus::Paused
        );
        manager.resume(id).await?;
        wait_for_active(&manager, 1).await?;
        manager.cancel(id).await?;
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("download missing")?
                .status(),
            DownloadStatus::Cancelled
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_queued_download_deletes_storage_and_queue_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;
        let download = item()?;
        let id = download.id();
        let destination = download.destination().to_path_buf();
        let test_directory = destination
            .parent()
            .ok_or("missing test directory")?
            .to_path_buf();
        let partial = PathBuf::from(format!("{}.part", destination.display()));
        let segment = PathBuf::from(format!("{}.segment-0", destination.display()));
        tokio::fs::create_dir_all(destination.parent().ok_or("missing parent")?).await?;
        tokio::fs::write(&destination, b"completed user data").await?;
        tokio::fs::write(&partial, b"partial user data").await?;
        tokio::fs::write(&segment, b"segment user data").await?;
        manager.enqueue_deferred(download).await?;

        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert!(
            manager
                .snapshot()
                .await?
                .items
                .iter()
                .all(|item| item.id() != id)
        );
        assert_eq!(tokio::fs::read(destination).await?, b"completed user data");
        assert_eq!(tokio::fs::read(partial).await?, b"partial user data");
        assert_eq!(tokio::fs::read(segment).await?, b"segment user data");
        tokio::fs::remove_dir_all(test_directory).await?;
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_paused_download_cancels_inactive_engine_then_deletes_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let id = persist_item_in_status(&repository, DownloadStatus::Queued).await?;
        let manager = QueueManager::new_with_deferred(
            repository.clone(),
            engine,
            QueueConfig::default(),
            HashSet::from([id]),
        )
        .await?;
        manager.pause(id).await?;

        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert!(manager.snapshot().await?.paused.is_empty());
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn failed_inactive_engine_cancellation_preserves_paused_download()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let id = persist_item_in_status(&repository, DownloadStatus::Paused).await?;
        let manager = QueueManager::new(
            repository.clone(),
            Arc::new(RejectInactiveCancelEngine),
            QueueConfig::default(),
        )
        .await?;

        assert!(matches!(
            manager.remove_download(id).await,
            Err(QueueError::Engine(
                DownloadEngineError::Aria2ReconciliationRequired
            ))
        ));
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("missing download")?
                .status(),
            DownloadStatus::Paused
        );
        assert!(
            manager
                .snapshot()
                .await?
                .items
                .iter()
                .any(|item| item.id() == id)
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_completed_download_preserves_destination_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let id = persist_item_in_status(&repository, DownloadStatus::Completed).await?;
        let destination = repository
            .get(id)
            .await?
            .ok_or("missing item")?
            .destination()
            .to_path_buf();
        let test_directory = destination
            .parent()
            .ok_or("missing test directory")?
            .to_path_buf();
        tokio::fs::create_dir_all(&test_directory).await?;
        tokio::fs::write(&destination, b"completed file").await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;

        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert_eq!(tokio::fs::read(destination).await?, b"completed file");
        tokio::fs::remove_dir_all(test_directory).await?;
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_failed_download_deletes_persisted_and_in_memory_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let id = persist_item_in_status(&repository, DownloadStatus::Failed).await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;

        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert!(
            manager
                .snapshot()
                .await?
                .items
                .iter()
                .all(|item| item.id() != id)
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_cancelled_download_deletes_persisted_and_in_memory_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let id = persist_item_in_status(&repository, DownloadStatus::Cancelled).await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;

        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert!(
            manager
                .snapshot()
                .await?
                .items
                .iter()
                .all(|item| item.id() != id)
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_unknown_download_reports_not_found() -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(repository, engine, QueueConfig::default()).await?;
        let id = DownloadId::new();

        assert!(matches!(
            manager.remove_download(id).await,
            Err(QueueError::DownloadNotFound(missing)) if missing == id
        ));

        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_active_download_waits_for_cancellation_before_deleting_and_dispatching()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(
            repository.clone(),
            engine,
            QueueConfig {
                max_active_downloads: 1,
                global_connection_limit: 1,
            },
        )
        .await?;
        let active = item()?;
        let active_id = active.id();
        let next = item()?;
        let next_id = next.id();
        manager.enqueue(active).await?;
        manager.enqueue(next).await?;
        wait_for_active(&manager, 1).await?;

        manager.remove_download(active_id).await?;

        assert!(repository.get(active_id).await?.is_none());
        let snapshot = wait_for_active(&manager, 1).await?;
        assert_eq!(snapshot.active[0].item.id(), next_id);
        assert!(snapshot.items.iter().all(|item| item.id() != active_id));
        manager.remove_download(next_id).await?;
        assert!(repository.get(next_id).await?.is_none());
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn removal_racing_worker_completion_never_leaves_or_resurrects_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, release) = setup_engine().await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;
        let download = item()?;
        let id = download.id();
        manager.enqueue(download.clone()).await?;
        wait_for_active(&manager, 1).await?;

        release.send(true)?;
        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert!(
            manager
                .snapshot()
                .await?
                .items
                .iter()
                .all(|item| item.id() != id)
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_then_reenqueue_same_id_does_not_resurrect_old_worker()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;
        let download = item()?;
        let id = download.id();
        manager.enqueue(download.clone()).await?;
        wait_for_active(&manager, 1).await?;

        manager.remove_download(id).await?;
        manager.enqueue(download).await?;

        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if repository
                    .get(id)
                    .await?
                    .is_some_and(|item| item.status() == DownloadStatus::Downloading)
                {
                    return Ok::<_, QueueError>(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert_eq!(snapshot.active[0].item.id(), id);
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("re-added item missing")?
                .status(),
            DownloadStatus::Downloading
        );
        manager.remove_download(id).await?;
        assert!(repository.get(id).await?.is_none());
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn remove_racing_queue_dispatch_leaves_no_worker_or_stale_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;
        let download = item()?;
        let id = download.id();
        manager.enqueue(download).await?;

        manager.remove_download(id).await?;

        assert!(repository.get(id).await?.is_none());
        assert!(manager.snapshot().await?.active.is_empty());
        assert!(
            manager
                .snapshot()
                .await?
                .items
                .iter()
                .all(|item| item.id() != id)
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn inactive_engine_cancellation_must_succeed_before_paused_state_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let item = item()?;
        let id = item.id();
        repository.create(&item).await?;
        DownloadStateMachine::new(repository.clone())
            .transition(id, DownloadStatus::Paused)
            .await?;
        let manager = QueueManager::new(
            repository.clone(),
            Arc::new(RejectInactiveCancelEngine),
            QueueConfig::default(),
        )
        .await?;

        assert!(matches!(
            manager.cancel(id).await,
            Err(QueueError::Engine(
                DownloadEngineError::Aria2ReconciliationRequired
            ))
        ));
        assert_eq!(
            repository.get(id).await?.ok_or("missing item")?.status(),
            DownloadStatus::Paused
        );
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn engine_rejection_before_preparing_fails_queue_item_without_retry_loop()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let manager = QueueManager::new(
            repository.clone(),
            Arc::new(RejectInactiveCancelEngine),
            QueueConfig::default(),
        )
        .await?;
        let item = item()?;
        let id = item.id();
        manager.enqueue(item).await?;

        let failed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(item) = repository.get(id).await?
                    && item.status() == DownloadStatus::Failed
                {
                    return Ok::<_, fetchx_storage::StorageError>(item);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert_eq!(failed.status(), DownloadStatus::Failed);
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn startup_requeues_interrupted_items_without_exceeding_active_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let state_machine = DownloadStateMachine::new(repository.clone());
        let first = item()?;
        let second = item()?;
        for download in [&first, &second] {
            repository.create(download).await?;
            state_machine
                .transition(download.id(), DownloadStatus::Preparing)
                .await?;
            state_machine
                .transition(download.id(), DownloadStatus::Downloading)
                .await?;
        }

        let manager = QueueManager::new(
            repository.clone(),
            engine,
            QueueConfig {
                max_active_downloads: 1,
                global_connection_limit: 1,
            },
        )
        .await?;
        let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = manager.snapshot().await?;
                if snapshot.active.len() == 1 && snapshot.queued.len() == 1 {
                    break Ok::<_, QueueError>(snapshot);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert_eq!(snapshot.queued.len(), 1);
        assert!(snapshot.items.iter().all(|download| {
            matches!(
                download.status(),
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        }));
        let active_id = snapshot.active[0].item.id();
        manager.cancel(active_id).await?;
        wait_for_active(&manager, 1).await?;
        let remaining = manager.snapshot().await?.active[0].item.id();
        manager.cancel(remaining).await?;
        manager.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_pauses_active_downloads() -> Result<(), Box<dyn std::error::Error>> {
        let (repository, engine, _release) = setup_engine().await?;
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;
        let download = item()?;
        let id = download.id();
        manager.enqueue(download).await?;
        wait_for_active(&manager, 1).await?;
        manager.shutdown().await?;
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("download missing")?
                .status(),
            DownloadStatus::Paused
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_ytdlp_pause_is_rejected_and_shutdown_requeues_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let (release_sender, release) = watch::channel(false);
        let engine = Arc::new(WaitingEngine {
            repository: repository.clone(),
            release,
            supports_yt_dlp_pause: false,
            desired_connections: 4,
        });
        let manager = QueueManager::new(repository.clone(), engine, QueueConfig::default()).await?;
        let download = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/video", EngineKind::YtDlp)?,
            FileName::new("video.webm")?,
            PathBuf::from("downloads/video.webm"),
            Timestamp::now()?,
        );
        manager.enqueue(download.clone()).await?;
        wait_for_active(&manager, 1).await?;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if repository
                    .get(download.id())
                    .await?
                    .is_some_and(|item| item.status() == DownloadStatus::Downloading)
                {
                    return Ok::<(), Box<dyn std::error::Error>>(());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;

        assert!(matches!(
            manager.pause(download.id()).await,
            Err(QueueError::UnsupportedEngineOperation {
                engine: EngineKind::YtDlp,
                operation: "active pause",
            })
        ));
        assert_eq!(
            repository
                .get(download.id())
                .await?
                .ok_or("missing queued item")?
                .status(),
            DownloadStatus::Downloading
        );

        manager.shutdown().await?;
        assert_eq!(
            repository
                .get(download.id())
                .await?
                .ok_or("missing queued item after shutdown")?
                .status(),
            DownloadStatus::Queued
        );
        drop(release_sender);
        Ok(())
    }

    #[test]
    fn priority_fifo_and_fairness_are_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let mut high = item()?;
        high.set_priority_at(DownloadPriority::High, high.updated_at())?;
        let mut low_first = item()?;
        low_first.set_priority_at(DownloadPriority::Low, low_first.updated_at())?;
        let mut low_second = item()?;
        low_second.set_priority_at(DownloadPriority::Low, low_second.updated_at())?;
        let high_id = high.id();
        let low_first_id = low_first.id();
        let low_second_id = low_second.id();
        let entries = [
            (high_id, high),
            (low_first_id, low_first),
            (low_second_id, low_second),
        ]
        .into_iter()
        .enumerate()
        .map(|(sequence, (id, item))| {
            (
                id,
                Entry {
                    item,
                    sequence: sequence as u64,
                },
            )
        })
        .collect::<HashMap<_, _>>();
        let active = HashMap::new();
        assert_eq!(
            choose_next(&entries, &HashSet::new(), &active, &None, 0),
            Some(high_id)
        );
        assert_eq!(
            choose_next(
                &entries,
                &HashSet::new(),
                &active,
                &Some(DownloadPriority::High),
                3
            ),
            Some(low_first_id)
        );
        Ok(())
    }

    #[test]
    fn allocator_shares_global_budget_fairly_and_preserves_live_workers() {
        let id1 = DownloadId::new();
        let id2 = DownloadId::new();
        let (cancel1, _) = watch::channel(false);
        let (pause1, _) = watch::channel(false);
        let (limit1, limit_rx1) = watch::channel(1);
        let (cancel2, _) = watch::channel(false);
        let (pause2, _) = watch::channel(false);
        let (limit2, limit_rx2) = watch::channel(1);
        let mut workers = HashMap::from([
            (
                id1,
                ActiveWorker {
                    cancellation: cancel1,
                    pause: pause1,
                    connection_limit: limit1,
                    desired: 4,
                    active: 1,
                    allocated: 1,
                    sequence: 0,
                    pausing: false,
                    cancelling: false,
                    shutdown_requeue: false,
                    pause_waiters: Vec::new(),
                    cancel_waiters: Vec::new(),
                    removing: false,
                    remove_waiters: Vec::new(),
                },
            ),
            (
                id2,
                ActiveWorker {
                    cancellation: cancel2,
                    pause: pause2,
                    connection_limit: limit2,
                    desired: 4,
                    active: 1,
                    allocated: 1,
                    sequence: 1,
                    pausing: false,
                    cancelling: false,
                    shutdown_requeue: false,
                    pause_waiters: Vec::new(),
                    cancel_waiters: Vec::new(),
                    removing: false,
                    remove_waiters: Vec::new(),
                },
            ),
        ]);
        allocate_resources(
            &mut workers,
            3,
            &HashMap::new(),
            &HashSet::new(),
            2,
            ResourcePressure::Normal,
        );
        assert_eq!(*limit_rx1.borrow(), 2);
        assert_eq!(*limit_rx2.borrow(), 1);
        assert_eq!(used_connections(&workers), 3);
    }
}

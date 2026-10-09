#![forbid(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use mydownloader_downloader::ManagedDownloadEngine;
use mydownloader_models::{
    DownloadId, DownloadItem, DownloadSchedule, DownloadScheduleError, DownloadStatus, Timestamp,
    TimestampError,
};
use mydownloader_queue::{QueueConfig, QueueError, QueueManager};
use mydownloader_storage::{DownloadRepository, StorageError};
use thiserror::Error;
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, sleep_until},
};
use tracing::debug;

const MAX_TIMER_INTERVAL: Duration = Duration::from_secs(60);

/// Persistent one-shot scheduling errors.
#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("download {0} was not found")]
    DownloadNotFound(DownloadId),
    #[error("download {id} cannot be scheduled while in {status:?}")]
    InvalidState {
        id: DownloadId,
        status: DownloadStatus,
    },
    #[error("download {0} does not have a schedule")]
    ScheduleNotFound(DownloadId),
    #[error("scheduler has shut down")]
    Stopped,
    #[error("system timestamp could not be read: {0}")]
    Timestamp(#[from] TimestampError),
    #[error("schedule domain data is invalid: {0}")]
    InvalidSchedule(#[from] DownloadScheduleError),
    #[error("schedule persistence failed: {0}")]
    Storage(#[from] StorageError),
    #[error("queue operation failed: {0}")]
    Queue(#[from] QueueError),
    #[error("scheduler task failed: {0}")]
    Actor(#[source] tokio::task::JoinError),
}

#[derive(Clone)]
pub struct Scheduler<R> {
    repository: R,
    queue: QueueManager,
    commands: mpsc::Sender<Command>,
    actor: SharedActorHandle,
}

type SharedActorHandle = Arc<Mutex<Option<JoinHandle<Result<(), ScheduleError>>>>>;

enum Command {
    Schedule {
        id: DownloadId,
        scheduled_at: Timestamp,
        reply: oneshot::Sender<Result<(), ScheduleError>>,
    },
    Remove {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), ScheduleError>>,
    },
    Enable {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), ScheduleError>>,
    },
    Disable {
        id: DownloadId,
        reply: oneshot::Sender<Result<(), ScheduleError>>,
    },
    Shutdown {
        reply: oneshot::Sender<Result<(), ScheduleError>>,
    },
}

impl<R> Scheduler<R>
where
    R: DownloadRepository + Clone + 'static,
{
    /// Starts a scheduler and queue manager, recovering persisted schedules first.
    ///
    /// Enabled schedules are passed to the queue as deferred before its worker
    /// actor starts, preventing startup dispatch from racing schedule recovery.
    pub async fn start<E>(
        repository: R,
        engine: Arc<E>,
        queue_config: QueueConfig,
    ) -> Result<Self, ScheduleError>
    where
        E: ManagedDownloadEngine + 'static,
    {
        let schedules = repository.list_schedules().await?;
        let deferred = schedules
            .iter()
            .filter(|schedule| schedule.enabled())
            .map(|schedule| schedule.download_id())
            .collect::<HashSet<_>>();
        let queue =
            QueueManager::new_with_deferred(repository.clone(), engine, queue_config, deferred)
                .await?;
        let (commands, receiver) = mpsc::channel(128);
        let actor_queue = queue.clone();
        let actor_repository = repository.clone();
        let actor =
            tokio::spawn(
                async move { run_scheduler(actor_repository, actor_queue, receiver).await },
            );

        Ok(Self {
            repository,
            queue,
            commands,
            actor: Arc::new(Mutex::new(Some(actor))),
        })
    }

    /// Creates or updates a one-shot schedule. Repeated calls replace its time
    /// and enable it while preserving its original creation timestamp.
    pub async fn schedule_download(
        &self,
        id: DownloadId,
        scheduled_at: Timestamp,
    ) -> Result<(), ScheduleError> {
        self.command(|reply| Command::Schedule {
            id,
            scheduled_at,
            reply,
        })
        .await
    }

    /// Removes a schedule. Removing a nonexistent schedule is idempotent.
    pub async fn remove_schedule(&self, id: DownloadId) -> Result<(), ScheduleError> {
        self.command(|reply| Command::Remove { id, reply }).await
    }

    /// Enables a persisted schedule. Enabling an already enabled schedule is safe.
    pub async fn enable_schedule(&self, id: DownloadId) -> Result<(), ScheduleError> {
        self.command(|reply| Command::Enable { id, reply }).await
    }

    /// Disables a persisted schedule and releases it as an ordinary queued item.
    pub async fn disable_schedule(&self, id: DownloadId) -> Result<(), ScheduleError> {
        self.command(|reply| Command::Disable { id, reply }).await
    }

    pub async fn get_schedule(
        &self,
        id: DownloadId,
    ) -> Result<Option<DownloadSchedule>, ScheduleError> {
        Ok(self.repository.get_schedule(id).await?)
    }

    pub async fn list_scheduled_downloads(&self) -> Result<Vec<DownloadSchedule>, ScheduleError> {
        Ok(self.repository.list_schedules().await?)
    }

    /// Returns true only for a queued item whose schedule is absent, disabled,
    /// or due. Paused and terminal items are never eligible.
    pub async fn is_eligible(&self, id: DownloadId, now: Timestamp) -> Result<bool, ScheduleError> {
        let Some(item) = self.repository.get(id).await? else {
            return Err(ScheduleError::DownloadNotFound(id));
        };
        if item.status() != DownloadStatus::Queued {
            return Ok(false);
        }
        let Some(schedule) = self.repository.get_schedule(id).await? else {
            return Ok(true);
        };
        Ok(!schedule.enabled() || schedule.scheduled_at() <= now)
    }

    /// Exposes ordinary queue controls; scheduling itself never bypasses the queue.
    #[must_use]
    pub fn queue_manager(&self) -> QueueManager {
        self.queue.clone()
    }

    pub async fn shutdown(&self) -> Result<(), ScheduleError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command::Shutdown { reply })
            .await
            .map_err(|_| ScheduleError::Stopped)?;
        result.await.map_err(|_| ScheduleError::Stopped)??;
        if let Some(actor) = self.actor.lock().await.take() {
            actor.await.map_err(ScheduleError::Actor)??;
        }
        Ok(())
    }

    async fn command(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<(), ScheduleError>>) -> Command,
    ) -> Result<(), ScheduleError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(command(reply))
            .await
            .map_err(|_| ScheduleError::Stopped)?;
        result.await.map_err(|_| ScheduleError::Stopped)?
    }
}

async fn run_scheduler<R>(
    repository: R,
    queue: QueueManager,
    mut commands: mpsc::Receiver<Command>,
) -> Result<(), ScheduleError>
where
    R: DownloadRepository + Clone,
{
    let result = run_scheduler_loop(&repository, &queue, &mut commands).await;
    if result.is_err() {
        if let Err(error) = queue.shutdown().await {
            tracing::error!(%error, "queue failed to shut down after scheduler error");
        }
    } else if let Err(error) = queue.shutdown().await {
        return Err(ScheduleError::Queue(error));
    }
    result
}

async fn run_scheduler_loop<R>(
    repository: &R,
    queue: &QueueManager,
    commands: &mut mpsc::Receiver<Command>,
) -> Result<(), ScheduleError>
where
    R: DownloadRepository,
{
    loop {
        let now = Timestamp::now()?;
        let next = reconcile(repository, queue, now).await?;
        let delay = next.map_or(MAX_TIMER_INTERVAL, |scheduled_at| {
            Duration::from_millis(
                scheduled_at
                    .as_unix_millis()
                    .saturating_sub(now.as_unix_millis())
                    .min(MAX_TIMER_INTERVAL.as_millis() as u64),
            )
        });
        let wake_at = Instant::now() + delay;
        tokio::select! {
            biased;
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                if handle_command(command, repository, queue, Timestamp::now()?).await? {
                    return Ok(());
                }
            }
            _ = sleep_until(wake_at) => {
                debug!(scheduled_wake = true, "scheduler reevaluating due downloads");
            }
        }
    }
}

async fn reconcile<R>(
    repository: &R,
    queue: &QueueManager,
    now: Timestamp,
) -> Result<Option<Timestamp>, ScheduleError>
where
    R: DownloadRepository,
{
    let schedules = repository.list_schedules().await?;
    let downloads = repository
        .list()
        .await?
        .into_iter()
        .map(|item| (item.id(), item))
        .collect::<HashMap<_, DownloadItem>>();
    let mut next = None;
    let mut release = Vec::new();

    for schedule in schedules {
        let Some(item) = downloads.get(&schedule.download_id()) else {
            continue;
        };
        if !schedule.enabled()
            || !matches!(
                item.status(),
                DownloadStatus::Queued | DownloadStatus::Paused
            )
            || schedule.scheduled_at() <= now
        {
            release.push(schedule.download_id());
        } else {
            match queue.defer_download(schedule.download_id()).await {
                Ok(()) => {
                    next = Some(next.map_or(schedule.scheduled_at(), |current: Timestamp| {
                        current.min(schedule.scheduled_at())
                    }));
                }
                Err(QueueError::InvalidStatus {
                    action: "defer", ..
                }) => {
                    release.push(schedule.download_id());
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    queue.release_downloads(release).await?;
    Ok(next)
}

async fn handle_command<R>(
    command: Command,
    repository: &R,
    queue: &QueueManager,
    now: Timestamp,
) -> Result<bool, ScheduleError>
where
    R: DownloadRepository,
{
    match command {
        Command::Schedule {
            id,
            scheduled_at,
            reply,
        } => {
            let result = schedule_download(repository, queue, id, scheduled_at, now).await;
            let _ = reply.send(result);
        }
        Command::Remove { id, reply } => {
            let result = async {
                repository.delete_schedule(id).await?;
                queue.release_download(id).await?;
                Ok(())
            }
            .await;
            let _ = reply.send(result);
        }
        Command::Enable { id, reply } => {
            let result = set_schedule_enabled(repository, queue, id, true, now).await;
            let _ = reply.send(result);
        }
        Command::Disable { id, reply } => {
            let result = set_schedule_enabled(repository, queue, id, false, now).await;
            let _ = reply.send(result);
        }
        Command::Shutdown { reply } => {
            let _ = reply.send(Ok(()));
            return Ok(true);
        }
    }
    Ok(false)
}

async fn schedule_download<R>(
    repository: &R,
    queue: &QueueManager,
    id: DownloadId,
    scheduled_at: Timestamp,
    now: Timestamp,
) -> Result<(), ScheduleError>
where
    R: DownloadRepository,
{
    let item = repository
        .get(id)
        .await?
        .ok_or(ScheduleError::DownloadNotFound(id))?;
    validate_schedulable(&item)?;
    if let Err(error) = queue.defer_download(id).await {
        let current = repository
            .get(id)
            .await?
            .ok_or(ScheduleError::DownloadNotFound(id))?;
        validate_schedulable(&current)?;
        return Err(ScheduleError::Queue(error));
    }
    let existing = repository.get_schedule(id).await?;
    let was_deferred = existing
        .as_ref()
        .is_some_and(|schedule| schedule.enabled() && schedule.scheduled_at() > now);
    let mut schedule = if let Some(existing) = existing {
        existing
    } else {
        DownloadSchedule::new(id, scheduled_at, true, now)
    };
    let updated_at = schedule.updated_at().max(now);
    schedule.set_scheduled_at(scheduled_at, updated_at)?;
    schedule.set_enabled(true, updated_at)?;
    if let Err(error) = repository.save_schedule(&schedule).await {
        if was_deferred {
            reconcile(repository, queue, now).await?;
        } else {
            queue.release_download(id).await?;
        }
        return Err(ScheduleError::Storage(error));
    }
    if scheduled_at <= now {
        queue.release_download(id).await?;
    }
    Ok(())
}

async fn set_schedule_enabled<R>(
    repository: &R,
    queue: &QueueManager,
    id: DownloadId,
    enabled: bool,
    now: Timestamp,
) -> Result<(), ScheduleError>
where
    R: DownloadRepository,
{
    let mut schedule = repository
        .get_schedule(id)
        .await?
        .ok_or(ScheduleError::ScheduleNotFound(id))?;
    let item = repository
        .get(id)
        .await?
        .ok_or(ScheduleError::DownloadNotFound(id))?;
    if schedule.enabled() == enabled {
        if enabled {
            validate_schedulable(&item)?;
            if schedule.scheduled_at() > now {
                queue.defer_download(id).await?;
            } else {
                queue.release_download(id).await?;
            }
        } else {
            queue.release_download(id).await?;
        }
        return Ok(());
    }
    if enabled {
        validate_schedulable(&item)?;
        if schedule.scheduled_at() > now {
            queue.defer_download(id).await?;
        }
    }
    schedule.set_enabled(enabled, schedule.updated_at().max(now))?;
    if let Err(error) = repository.save_schedule(&schedule).await {
        reconcile(repository, queue, now).await?;
        return Err(ScheduleError::Storage(error));
    }
    if !enabled || schedule.scheduled_at() <= now {
        queue.release_download(id).await?;
    }
    Ok(())
}

fn validate_schedulable(item: &DownloadItem) -> Result<(), ScheduleError> {
    if matches!(
        item.status(),
        DownloadStatus::Queued | DownloadStatus::Paused
    ) {
        Ok(())
    } else {
        Err(ScheduleError::InvalidState {
            id: item.id(),
            status: item.status(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use mydownloader_core::DownloadStateMachine;
    use mydownloader_downloader::{
        DownloadEngine, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    };
    use mydownloader_models::{
        DownloadId, DownloadItem, DownloadPriority, DownloadProgress, DownloadSchedule,
        DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use mydownloader_queue::{QueueConfig, QueueError};
    use mydownloader_storage::{Database, DownloadRepository, SqliteDownloadRepository};
    use tokio::{sync::watch, time::timeout};

    use super::{ScheduleError, Scheduler};

    struct ControlledEngine {
        repository: SqliteDownloadRepository,
        completion: watch::Receiver<bool>,
        started: Arc<AtomicUsize>,
    }

    impl DownloadEngine for ControlledEngine {
        async fn download(
            &self,
            _id: DownloadId,
            _cancellation: watch::Receiver<bool>,
        ) -> Result<DownloadItem, DownloadEngineError> {
            Err(DownloadEngineError::InvalidConfiguration)
        }
    }

    impl ManagedDownloadEngine for ControlledEngine {
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
            self.started.fetch_add(1, Ordering::SeqCst);
            let mut completion = self.completion.clone();
            loop {
                if *control.cancellation.borrow() {
                    state_machine
                        .transition(id, DownloadStatus::Cancelled)
                        .await?;
                    return Err(DownloadEngineError::Cancelled);
                }
                if *control.pause.borrow() {
                    state_machine.transition(id, DownloadStatus::Paused).await?;
                    return Err(DownloadEngineError::Paused);
                }
                if *completion.borrow() {
                    return Ok(state_machine
                        .transition(id, DownloadStatus::Completed)
                        .await?);
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
                    changed = completion.changed() => {
                        if changed.is_err() {
                            return Err(DownloadEngineError::Cancelled);
                        }
                    }
                }
            }
        }
    }

    async fn repository() -> Result<SqliteDownloadRepository, Box<dyn Error>> {
        Ok(Database::connect("sqlite::memory:").await?.downloads())
    }

    fn new_item() -> Result<DownloadItem, Box<dyn Error>> {
        Ok(DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file", EngineKind::DirectHttp)?,
            FileName::new("file.bin")?,
            PathBuf::from("downloads/file.bin"),
            Timestamp::from_unix_millis(1),
        ))
    }

    async fn insert_item(
        repository: &SqliteDownloadRepository,
        status: DownloadStatus,
    ) -> Result<DownloadItem, Box<dyn Error>> {
        let mut item = new_item()?;
        if status != DownloadStatus::Queued {
            item = DownloadItem::from_parts(
                item.id(),
                item.source().clone(),
                FileName::new(item.filename().as_str())?,
                item.destination().to_path_buf(),
                status,
                DownloadPriority::Normal,
                DownloadProgress::default(),
                item.created_at(),
                item.updated_at(),
                None,
            )?;
        }
        repository.create(&item).await?;
        Ok(item)
    }

    async fn save_schedule(
        repository: &SqliteDownloadRepository,
        id: DownloadId,
        scheduled_at: Timestamp,
        enabled: bool,
    ) -> Result<DownloadSchedule, Box<dyn Error>> {
        let schedule =
            DownloadSchedule::new(id, scheduled_at, enabled, Timestamp::from_unix_millis(10));
        repository.save_schedule(&schedule).await?;
        Ok(schedule)
    }

    async fn start(
        repository: SqliteDownloadRepository,
        limit: usize,
    ) -> Result<
        (
            Scheduler<SqliteDownloadRepository>,
            Arc<AtomicUsize>,
            watch::Sender<bool>,
        ),
        Box<dyn Error>,
    > {
        let (completion_sender, completion) = watch::channel(false);
        let started = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(ControlledEngine {
            repository: repository.clone(),
            completion,
            started: Arc::clone(&started),
        });
        let scheduler = Scheduler::start(
            repository,
            engine,
            QueueConfig {
                max_active_downloads: limit,
                global_connection_limit: limit,
            },
        )
        .await?;
        Ok((scheduler, started, completion_sender))
    }

    async fn wait_started(started: &AtomicUsize, expected: usize) -> Result<(), Box<dyn Error>> {
        timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    async fn wait_status(
        repository: &SqliteDownloadRepository,
        id: DownloadId,
        status: DownloadStatus,
    ) -> Result<(), Box<dyn Error>> {
        timeout(Duration::from_secs(2), async {
            loop {
                if repository
                    .get(id)
                    .await?
                    .is_some_and(|item| item.status() == status)
                {
                    break Ok::<(), Box<dyn Error>>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        Ok(())
    }

    #[tokio::test]
    async fn future_schedules_wait_without_using_queue_slots_or_connections()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let scheduled = insert_item(&repository, DownloadStatus::Queued).await?;
        let ordinary = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            scheduled.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            true,
        )
        .await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;

        wait_started(&started, 1).await?;
        let snapshot = scheduler
            .queue_manager()
            .snapshot()
            .await
            .map_err(|error| format!("queue snapshot failed: {error}"))?;
        assert_eq!(snapshot.active.len(), 1);
        assert_eq!(snapshot.active[0].item.id(), ordinary.id());
        assert_eq!(snapshot.queued, []);
        assert_eq!(
            repository
                .get(scheduled.id())
                .await?
                .ok_or("missing")?
                .status(),
            DownloadStatus::Queued
        );
        assert!(
            !scheduler
                .is_eligible(
                    scheduled.id(),
                    Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis())
                )
                .await?
        );
        scheduler
            .shutdown()
            .await
            .map_err(|error| format!("scheduler shutdown failed: {error}"))?;
        Ok(())
    }

    #[tokio::test]
    async fn overdue_and_exact_time_schedules_are_eligible_at_startup() -> Result<(), Box<dyn Error>>
    {
        let repository = repository().await?;
        let overdue = insert_item(&repository, DownloadStatus::Queued).await?;
        let exact = insert_item(&repository, DownloadStatus::Queued).await?;
        let now = Timestamp::now()?;
        save_schedule(
            &repository,
            overdue.id(),
            Timestamp::from_unix_millis(now.as_unix_millis().saturating_sub(1)),
            true,
        )
        .await?;
        save_schedule(&repository, exact.id(), now, true).await?;

        let (scheduler, started, _completion) = start(repository, 2).await?;
        wait_started(&started, 2).await?;
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn schedule_api_is_persistent_idempotent_and_supports_modification()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        let later = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 90_000);
        let original = save_schedule(&repository, item.id(), later, true).await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;

        let updated_time =
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 120_000);
        scheduler.schedule_download(item.id(), updated_time).await?;
        let updated = scheduler
            .get_schedule(item.id())
            .await?
            .ok_or("schedule missing")?;
        assert_eq!(updated.scheduled_at(), updated_time);
        assert_eq!(updated.created_at(), original.created_at());
        assert!(updated.updated_at() >= original.updated_at());
        scheduler.schedule_download(item.id(), updated_time).await?;
        assert_eq!(scheduler.list_scheduled_downloads().await?.len(), 1);
        assert_eq!(started.load(Ordering::SeqCst), 0);

        scheduler.remove_schedule(item.id()).await?;
        scheduler.remove_schedule(DownloadId::new()).await?;
        wait_started(&started, 1).await?;
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn changing_a_schedule_to_the_past_wakes_and_releases_it() -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            true,
        )
        .await?;
        let (scheduler, started, _completion) = start(repository, 1).await?;
        assert_eq!(started.load(Ordering::SeqCst), 0);
        let past = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() - 1);
        scheduler.schedule_download(item.id(), past).await?;
        wait_started(&started, 1).await?;
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn disabling_or_removing_a_future_schedule_releases_the_item()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let disabled = insert_item(&repository, DownloadStatus::Queued).await?;
        let removed = insert_item(&repository, DownloadStatus::Queued).await?;
        let future = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000);
        save_schedule(&repository, disabled.id(), future, true).await?;
        save_schedule(&repository, removed.id(), future, true).await?;
        let (scheduler, started, _completion) = start(repository, 2).await?;

        scheduler.disable_schedule(disabled.id()).await?;
        wait_started(&started, 1).await?;
        scheduler.remove_schedule(removed.id()).await?;
        wait_started(&started, 2).await?;
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn pausing_a_waiting_download_is_never_overridden_by_schedule_due()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            true,
        )
        .await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;
        let queue = scheduler.queue_manager();
        queue.pause(item.id()).await?;
        let past = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() - 1);
        scheduler.schedule_download(item.id(), past).await?;
        assert_eq!(
            repository.get(item.id()).await?.ok_or("missing")?.status(),
            DownloadStatus::Paused
        );
        assert_eq!(started.load(Ordering::SeqCst), 0);
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn missing_and_terminal_downloads_have_deterministic_errors() -> Result<(), Box<dyn Error>>
    {
        let repository = repository().await?;
        let _queued = insert_item(&repository, DownloadStatus::Queued).await?;
        let completed = insert_item(&repository, DownloadStatus::Completed).await?;
        let cancelled = insert_item(&repository, DownloadStatus::Cancelled).await?;
        let failed = insert_item(&repository, DownloadStatus::Failed).await?;
        let paused = insert_item(&repository, DownloadStatus::Paused).await?;
        let waiting = insert_item(&repository, DownloadStatus::Queued).await?;
        let waiting_schedule =
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000);
        save_schedule(&repository, waiting.id(), waiting_schedule, true).await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;

        for item in [&completed, &cancelled, &failed] {
            assert!(matches!(
                scheduler
                    .schedule_download(
                        item.id(),
                        Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis())
                    )
                    .await,
                Err(ScheduleError::InvalidState { .. })
            ));
        }
        assert!(matches!(
            scheduler
                .schedule_download(
                    DownloadId::new(),
                    Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis())
                )
                .await,
            Err(ScheduleError::DownloadNotFound(_))
        ));
        scheduler
            .schedule_download(
                paused.id(),
                Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            )
            .await?;
        assert!(
            !scheduler
                .is_eligible(waiting.id(), Timestamp::now()?)
                .await?
        );
        assert!(
            scheduler
                .is_eligible(waiting.id(), waiting_schedule)
                .await?
        );
        assert!(
            !scheduler
                .is_eligible(paused.id(), Timestamp::now()?)
                .await?
        );
        scheduler.enable_schedule(paused.id()).await?;
        assert_eq!(started.load(Ordering::SeqCst), 1);
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn disabled_schedule_can_be_enabled_and_missing_enable_is_reported()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            false,
        )
        .await?;
        let (scheduler, started, _completion) = start(repository, 1).await?;
        wait_started(&started, 1).await?;
        scheduler.queue_manager().pause(item.id()).await?;
        scheduler.enable_schedule(item.id()).await?;
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert!(
            scheduler
                .queue_manager()
                .snapshot()
                .await?
                .paused
                .contains(&item.id())
        );
        assert!(matches!(
            scheduler.enable_schedule(DownloadId::new()).await,
            Err(ScheduleError::ScheduleNotFound(_))
        ));
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_item_is_not_restarted_when_its_schedule_becomes_due()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            true,
        )
        .await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;
        scheduler.queue_manager().cancel(item.id()).await?;
        let past = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() - 1);
        assert!(matches!(
            scheduler.schedule_download(item.id(), past).await,
            Err(ScheduleError::InvalidState {
                status: DownloadStatus::Cancelled,
                ..
            })
        ));
        assert_eq!(
            repository.get(item.id()).await?.ok_or("missing")?.status(),
            DownloadStatus::Cancelled
        );
        assert_eq!(started.load(Ordering::SeqCst), 0);
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn completion_while_scheduled_does_not_restart_the_download() -> Result<(), Box<dyn Error>>
    {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis().saturating_sub(1)),
            true,
        )
        .await?;
        let (scheduler, started, completion) = start(repository.clone(), 1).await?;
        wait_started(&started, 1).await?;
        completion.send(true)?;
        wait_status(&repository, item.id(), DownloadStatus::Completed).await?;
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert!(matches!(
            scheduler
                .schedule_download(
                    item.id(),
                    Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis())
                )
                .await,
            Err(ScheduleError::InvalidState {
                status: DownloadStatus::Completed,
                ..
            })
        ));
        assert_eq!(started.load(Ordering::SeqCst), 1);
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_racing_a_due_schedule_never_restarts_the_item()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000),
            true,
        )
        .await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;
        let queue = scheduler.queue_manager();
        let due = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis());
        let (schedule_result, cancel_result) = tokio::join!(
            scheduler.schedule_download(item.id(), due),
            queue.cancel(item.id())
        );
        cancel_result?;
        assert!(
            schedule_result.is_ok()
                || matches!(
                    &schedule_result,
                    Err(ScheduleError::InvalidState {
                        status: DownloadStatus::Cancelled,
                        ..
                    }) | Err(ScheduleError::Queue(QueueError::InvalidStatus { .. }))
                ),
            "unexpected scheduling race result: {schedule_result:?}"
        );
        wait_status(&repository, item.id(), DownloadStatus::Cancelled).await?;
        assert!(started.load(Ordering::SeqCst) <= 1);
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn timer_wakes_at_due_time_and_releases_deferred_download() -> Result<(), Box<dyn Error>>
    {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        let start_time = Timestamp::now()?.as_unix_millis();
        save_schedule(
            &repository,
            item.id(),
            Timestamp::from_unix_millis(start_time + 100),
            true,
        )
        .await?;
        let (completion_sender, completion) = watch::channel(false);
        let started = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(ControlledEngine {
            repository: repository.clone(),
            completion,
            started: Arc::clone(&started),
        });
        let scheduler =
            Scheduler::start(repository.clone(), engine, QueueConfig::default()).await?;
        assert_eq!(started.load(Ordering::SeqCst), 0);
        wait_started(&started, 1).await?;
        completion_sender.send(true)?;
        wait_status(&repository, item.id(), DownloadStatus::Completed).await?;
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn equal_due_time_jobs_are_released_together_for_queue_priority()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let low = insert_item(&repository, DownloadStatus::Queued).await?;
        let mut high = insert_item(&repository, DownloadStatus::Queued).await?;
        high.set_priority_at(DownloadPriority::High, high.updated_at())?;
        repository.update(&high).await?;
        let now = Timestamp::now()?.as_unix_millis();
        save_schedule(
            &repository,
            low.id(),
            Timestamp::from_unix_millis(now + 100),
            true,
        )
        .await?;
        save_schedule(
            &repository,
            high.id(),
            Timestamp::from_unix_millis(now + 100),
            true,
        )
        .await?;
        let (_completion_sender, completion) = watch::channel(false);
        let started = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(ControlledEngine {
            repository: repository.clone(),
            completion,
            started: Arc::clone(&started),
        });
        let scheduler = Scheduler::start(
            repository,
            engine,
            QueueConfig {
                max_active_downloads: 1,
                global_connection_limit: 1,
            },
        )
        .await?;
        wait_started(&started, 1).await?;
        assert_eq!(
            scheduler.queue_manager().snapshot().await?.active[0]
                .item
                .id(),
            high.id()
        );
        scheduler.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_stops_scheduler_and_pauses_active_worker() -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;
        wait_started(&started, 1).await?;
        scheduler.shutdown().await?;
        assert_eq!(
            repository.get(item.id()).await?.ok_or("missing")?.status(),
            DownloadStatus::Paused
        );
        assert!(matches!(
            scheduler.remove_schedule(item.id()).await,
            Err(ScheduleError::Stopped)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn restart_requeues_interrupted_download_but_keeps_future_schedule_deferred()
    -> Result<(), Box<dyn Error>> {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Downloading).await?;
        let future = Timestamp::from_unix_millis(Timestamp::now()?.as_unix_millis() + 60_000);
        save_schedule(&repository, item.id(), future, true).await?;
        let (scheduler, started, _completion) = start(repository.clone(), 1).await?;
        assert_eq!(started.load(Ordering::SeqCst), 0);
        assert_eq!(
            repository.get(item.id()).await?.ok_or("missing")?.status(),
            DownloadStatus::Queued
        );
        scheduler.shutdown().await?;
        Ok(())
    }

    #[test]
    fn schedule_time_comparison_treats_equal_as_due_and_future_as_waiting() {
        let now = Timestamp::from_unix_millis(100);
        let equal = Timestamp::from_unix_millis(100);
        let future = Timestamp::from_unix_millis(101);
        assert!(equal <= now);
        assert!(future > now);
    }

    #[tokio::test]
    async fn queue_errors_remain_structured_for_invalid_queue_actions() -> Result<(), Box<dyn Error>>
    {
        let repository = repository().await?;
        let item = insert_item(&repository, DownloadStatus::Queued).await?;
        let (scheduler, _started, _completion) = start(repository.clone(), 1).await?;
        scheduler.shutdown().await?;
        assert!(matches!(
            scheduler.queue_manager().defer_download(item.id()).await,
            Err(QueueError::Stopped)
        ));
        Ok(())
    }
}

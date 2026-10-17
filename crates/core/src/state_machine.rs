use fetchx_models::{
    DownloadId, DownloadItem, DownloadItemError, DownloadStatus, Timestamp, TimestampError,
};
use fetchx_storage::{DownloadRepository, StorageError};
use thiserror::Error;

/// Validates a lifecycle transition without performing I/O or changing any state.
pub fn validate_transition(
    from: DownloadStatus,
    to: DownloadStatus,
) -> Result<(), StateMachineError> {
    let allowed = matches!(
        (from, to),
        (DownloadStatus::Queued, DownloadStatus::Preparing)
            | (DownloadStatus::Queued, DownloadStatus::Paused)
            | (DownloadStatus::Queued, DownloadStatus::Cancelled)
            | (DownloadStatus::Queued, DownloadStatus::MetadataAcquiring)
            | (DownloadStatus::Preparing, DownloadStatus::Queued)
            | (DownloadStatus::Preparing, DownloadStatus::Downloading)
            | (DownloadStatus::Preparing, DownloadStatus::Failed)
            | (DownloadStatus::Preparing, DownloadStatus::Cancelled)
            | (DownloadStatus::Preparing, DownloadStatus::Paused)
            | (DownloadStatus::Preparing, DownloadStatus::MetadataAcquiring)
            | (DownloadStatus::Downloading, DownloadStatus::Queued)
            | (DownloadStatus::Downloading, DownloadStatus::Paused)
            | (DownloadStatus::Downloading, DownloadStatus::Completed)
            | (DownloadStatus::Downloading, DownloadStatus::Failed)
            | (DownloadStatus::Downloading, DownloadStatus::Cancelled)
            | (DownloadStatus::Downloading, DownloadStatus::Seeding)
            | (DownloadStatus::Downloading, DownloadStatus::Recovering)
            | (DownloadStatus::Downloading, DownloadStatus::MetadataAcquiring)
            | (DownloadStatus::Completed, DownloadStatus::Seeding)
            | (DownloadStatus::Paused, DownloadStatus::Downloading)
            | (DownloadStatus::Paused, DownloadStatus::Queued)
            | (DownloadStatus::Paused, DownloadStatus::Cancelled)
            | (DownloadStatus::Failed, DownloadStatus::Preparing)
            | (DownloadStatus::Cancelled, DownloadStatus::Preparing)
            // Torrent-specific lifecycle edges.
            | (DownloadStatus::MetadataAcquiring, DownloadStatus::Downloading)
            | (DownloadStatus::MetadataAcquiring, DownloadStatus::Failed)
            | (DownloadStatus::MetadataAcquiring, DownloadStatus::Cancelled)
            | (DownloadStatus::MetadataAcquiring, DownloadStatus::Paused)
            | (DownloadStatus::MetadataAcquiring, DownloadStatus::Queued)
            | (DownloadStatus::Seeding, DownloadStatus::Stopped)
            | (DownloadStatus::Seeding, DownloadStatus::Paused)
            | (DownloadStatus::Seeding, DownloadStatus::Cancelled)
            | (DownloadStatus::Seeding, DownloadStatus::Recovering)
            | (DownloadStatus::Stopped, DownloadStatus::Queued)
            | (DownloadStatus::Stopped, DownloadStatus::Cancelled)
            | (DownloadStatus::Cancelled, DownloadStatus::Stopped)
            | (DownloadStatus::Recovering, DownloadStatus::Queued)
            | (DownloadStatus::Recovering, DownloadStatus::Downloading)
            | (DownloadStatus::Recovering, DownloadStatus::Seeding)
            | (DownloadStatus::Recovering, DownloadStatus::Failed)
            | (DownloadStatus::Recovering, DownloadStatus::Cancelled)
    );

    if allowed {
        Ok(())
    } else {
        Err(StateMachineError::InvalidTransition { from, to })
    }
}

/// Applies validated lifecycle transitions and persists them through a repository.
pub struct DownloadStateMachine<R> {
    repository: R,
}

impl<R> DownloadStateMachine<R>
where
    R: DownloadRepository,
{
    #[must_use]
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    /// Transitions a persisted download, returning the updated domain item.
    ///
    /// The repository performs a status-conditional update, so concurrent transitions
    /// from the same prior status cannot both succeed.
    pub async fn transition(
        &self,
        id: DownloadId,
        to: DownloadStatus,
    ) -> Result<DownloadItem, StateMachineError> {
        self.transition_inner(id, to, None).await
    }

    /// Transitions a download and atomically records its failure diagnostic.
    pub async fn transition_with_error(
        &self,
        id: DownloadId,
        to: DownloadStatus,
        error: fetchx_models::DownloadError,
    ) -> Result<DownloadItem, StateMachineError> {
        self.transition_inner(id, to, Some(error)).await
    }

    /// Recovers a process-interrupted download whose former worker no longer exists.
    ///
    /// Only `Preparing` and `Downloading` items are requeued. Their filesystem
    /// partials are left untouched for the downloader's existing resume logic.
    pub async fn recover_interrupted(
        &self,
        id: DownloadId,
    ) -> Result<Option<DownloadItem>, StateMachineError> {
        let Some(item) = self.repository.get(id).await? else {
            return Err(StateMachineError::NotFound(id));
        };
        if !matches!(
            item.status(),
            DownloadStatus::Preparing
                | DownloadStatus::Downloading
                | DownloadStatus::MetadataAcquiring
                | DownloadStatus::Recovering
                | DownloadStatus::Seeding
        ) {
            return Ok(None);
        }
        if item.status() == DownloadStatus::Seeding {
            self.transition(id, DownloadStatus::Recovering).await?;
        }
        self.transition(id, DownloadStatus::Queued).await.map(Some)
    }

    async fn transition_inner(
        &self,
        id: DownloadId,
        to: DownloadStatus,
        error: Option<fetchx_models::DownloadError>,
    ) -> Result<DownloadItem, StateMachineError> {
        let mut item = self
            .repository
            .get(id)
            .await?
            .ok_or(StateMachineError::NotFound(id))?;
        let from = item.status();
        validate_transition(from, to)?;

        let updated_at = Timestamp::now()?.max(item.updated_at());
        item.set_status_at(to, updated_at)?;
        if let Some(error) = error {
            item.set_error_at(Some(error), updated_at)?;
        }

        if self
            .repository
            .update_if_status_current(&item, from)
            .await?
        {
            return Ok(item);
        }

        match self.repository.get(id).await? {
            None => Err(StateMachineError::NotFound(id)),
            Some(current) => Err(StateMachineError::ConcurrentTransition {
                from,
                to,
                actual: current.status(),
            }),
        }
    }
}

#[derive(Debug, Error)]
pub enum StateMachineError {
    #[error("download {0} was not found")]
    NotFound(DownloadId),
    #[error("cannot transition download from {from:?} to {to:?}")]
    InvalidTransition {
        from: DownloadStatus,
        to: DownloadStatus,
    },
    #[error(
        "concurrent transition changed download from {from:?}; requested {to:?}, current state is {actual:?}"
    )]
    ConcurrentTransition {
        from: DownloadStatus,
        to: DownloadStatus,
        actual: DownloadStatus,
    },
    #[error("download timestamp could not be updated: {0}")]
    Timestamp(#[from] TimestampError),
    #[error("download state could not be updated: {0}")]
    Domain(#[from] DownloadItemError),
    #[error("download failure diagnostic is invalid: {0}")]
    DomainError(#[from] fetchx_models::DownloadErrorMessageError),
    #[error("download state persistence failed: {0}")]
    Storage(#[from] StorageError),
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use fetchx_models::{
        DownloadId, DownloadItem, DownloadPriority, DownloadProgress, DownloadSource,
        DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use fetchx_storage::{Database, DownloadRepository};

    use super::{DownloadStateMachine, StateMachineError, validate_transition};

    const STATES: [DownloadStatus; 11] = [
        DownloadStatus::Queued,
        DownloadStatus::Preparing,
        DownloadStatus::Downloading,
        DownloadStatus::Paused,
        DownloadStatus::Completed,
        DownloadStatus::Failed,
        DownloadStatus::Cancelled,
        DownloadStatus::MetadataAcquiring,
        DownloadStatus::Seeding,
        DownloadStatus::Recovering,
        DownloadStatus::Stopped,
    ];

    const LEGAL_TRANSITIONS: [(DownloadStatus, DownloadStatus); 41] = [
        (DownloadStatus::Queued, DownloadStatus::Preparing),
        (DownloadStatus::Queued, DownloadStatus::Paused),
        (DownloadStatus::Preparing, DownloadStatus::Paused),
        (DownloadStatus::Preparing, DownloadStatus::Queued),
        (DownloadStatus::Downloading, DownloadStatus::Queued),
        (DownloadStatus::Preparing, DownloadStatus::Downloading),
        (DownloadStatus::Downloading, DownloadStatus::Paused),
        (DownloadStatus::Paused, DownloadStatus::Downloading),
        (DownloadStatus::Paused, DownloadStatus::Queued),
        (DownloadStatus::Downloading, DownloadStatus::Completed),
        (DownloadStatus::Downloading, DownloadStatus::Failed),
        (DownloadStatus::Downloading, DownloadStatus::Cancelled),
        (DownloadStatus::Preparing, DownloadStatus::Failed),
        (DownloadStatus::Preparing, DownloadStatus::Cancelled),
        (DownloadStatus::Preparing, DownloadStatus::MetadataAcquiring),
        (DownloadStatus::Queued, DownloadStatus::Cancelled),
        (DownloadStatus::Paused, DownloadStatus::Cancelled),
        (DownloadStatus::Failed, DownloadStatus::Preparing),
        (DownloadStatus::Cancelled, DownloadStatus::Preparing),
        // Torrent lifecycle.
        (DownloadStatus::Queued, DownloadStatus::MetadataAcquiring),
        (
            DownloadStatus::Downloading,
            DownloadStatus::MetadataAcquiring,
        ),
        (
            DownloadStatus::MetadataAcquiring,
            DownloadStatus::Downloading,
        ),
        (DownloadStatus::MetadataAcquiring, DownloadStatus::Paused),
        (DownloadStatus::MetadataAcquiring, DownloadStatus::Failed),
        (DownloadStatus::MetadataAcquiring, DownloadStatus::Cancelled),
        (DownloadStatus::MetadataAcquiring, DownloadStatus::Queued),
        (DownloadStatus::Downloading, DownloadStatus::Seeding),
        (DownloadStatus::Downloading, DownloadStatus::Recovering),
        (DownloadStatus::Completed, DownloadStatus::Seeding),
        (DownloadStatus::Seeding, DownloadStatus::Stopped),
        (DownloadStatus::Seeding, DownloadStatus::Paused),
        (DownloadStatus::Seeding, DownloadStatus::Cancelled),
        (DownloadStatus::Seeding, DownloadStatus::Recovering),
        (DownloadStatus::Stopped, DownloadStatus::Queued),
        (DownloadStatus::Stopped, DownloadStatus::Cancelled),
        (DownloadStatus::Cancelled, DownloadStatus::Stopped),
        (DownloadStatus::Recovering, DownloadStatus::Queued),
        (DownloadStatus::Recovering, DownloadStatus::Downloading),
        (DownloadStatus::Recovering, DownloadStatus::Seeding),
        (DownloadStatus::Recovering, DownloadStatus::Failed),
        (DownloadStatus::Recovering, DownloadStatus::Cancelled),
    ];

    fn item(status: DownloadStatus) -> Result<DownloadItem, Box<dyn Error>> {
        Ok(DownloadItem::from_parts(
            DownloadId::new(),
            DownloadSource::new("https://example.test/file", EngineKind::DirectHttp)?,
            FileName::new("file.zip")?,
            PathBuf::from("downloads/file.zip"),
            status,
            DownloadPriority::High,
            DownloadProgress::new(128, Some(1024), Some(64), Some(14))?,
            Timestamp::from_unix_millis(1),
            Timestamp::from_unix_millis(2),
            None,
        )?)
    }

    #[test]
    fn transition_matrix_allows_only_documented_edges() {
        for from in STATES {
            for to in STATES {
                let should_be_allowed = LEGAL_TRANSITIONS.contains(&(from, to));
                assert_eq!(
                    validate_transition(from, to).is_ok(),
                    should_be_allowed,
                    "unexpected transition rule for {from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn invalid_transition_reports_both_states() {
        assert!(matches!(
            validate_transition(DownloadStatus::Completed, DownloadStatus::Downloading),
            Err(StateMachineError::InvalidTransition {
                from: DownloadStatus::Completed,
                to: DownloadStatus::Downloading
            })
        ));
    }

    #[tokio::test]
    async fn transitions_persist_and_preserve_other_domain_data() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());
        let queued = item(DownloadStatus::Queued)?;
        let id = queued.id();
        repository.create(&queued).await?;

        let preparing = state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;
        let loaded = repository.get(id).await?.ok_or("download not found")?;
        assert_eq!(loaded.status(), DownloadStatus::Preparing);
        assert_eq!(loaded, preparing);

        let downloading = state_machine
            .transition(id, DownloadStatus::Downloading)
            .await?;
        let loaded = repository.get(id).await?.ok_or("download not found")?;
        assert_eq!(loaded.status(), DownloadStatus::Downloading);

        let completed = state_machine
            .transition(id, DownloadStatus::Completed)
            .await?;
        let loaded = repository.get(id).await?.ok_or("download not found")?;
        assert_eq!(loaded.status(), DownloadStatus::Completed);
        assert_eq!(loaded, completed);

        assert_eq!(completed.id(), queued.id());
        assert_eq!(completed.source(), queued.source());
        assert_eq!(completed.filename(), queued.filename());
        assert_eq!(completed.destination(), queued.destination());
        assert_eq!(completed.priority(), queued.priority());
        assert_eq!(completed.progress(), queued.progress());
        assert_eq!(completed.created_at(), queued.created_at());
        assert!(completed.updated_at() > queued.updated_at());
        assert!(downloading.updated_at() >= preparing.updated_at());

        Ok(())
    }

    #[tokio::test]
    async fn restart_requeues_interrupted_seeding_through_recovering() -> Result<(), Box<dyn Error>>
    {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());
        let seeding = item(DownloadStatus::Seeding)?;
        let id = seeding.id();
        repository.create(&seeding).await?;

        let recovered = state_machine
            .recover_interrupted(id)
            .await?
            .ok_or("seeding task was not recovered")?;

        assert_eq!(recovered.status(), DownloadStatus::Queued);
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("download not found")?
                .status(),
            DownloadStatus::Queued
        );
        Ok(())
    }

    #[tokio::test]
    async fn recovery_does_not_resurrect_terminal_or_stopped_downloads()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());

        for status in [
            DownloadStatus::Completed,
            DownloadStatus::Failed,
            DownloadStatus::Cancelled,
            DownloadStatus::Stopped,
        ] {
            let download = item(status)?;
            let id = download.id();
            repository.create(&download).await?;

            assert!(
                state_machine.recover_interrupted(id).await?.is_none(),
                "{status:?} downloads must not be resurrected"
            );
            assert_eq!(
                repository
                    .get(id)
                    .await?
                    .ok_or("download not found")?
                    .status(),
                status
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn torrent_lifecycle_persists_completion_seeding_stop_and_failure()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());

        let downloading = item(DownloadStatus::Downloading)?;
        let seeding_id = downloading.id();
        repository.create(&downloading).await?;
        state_machine
            .transition(seeding_id, DownloadStatus::Completed)
            .await?;
        state_machine
            .transition(seeding_id, DownloadStatus::Seeding)
            .await?;
        state_machine
            .transition(seeding_id, DownloadStatus::Stopped)
            .await?;
        assert_eq!(
            repository
                .get(seeding_id)
                .await?
                .ok_or("seeding download not found")?
                .status(),
            DownloadStatus::Stopped
        );

        let failed = item(DownloadStatus::Downloading)?;
        let failed_id = failed.id();
        repository.create(&failed).await?;
        state_machine
            .transition(failed_id, DownloadStatus::Failed)
            .await?;
        assert_eq!(
            repository
                .get(failed_id)
                .await?
                .ok_or("failed download not found")?
                .status(),
            DownloadStatus::Failed
        );
        Ok(())
    }

    #[tokio::test]
    async fn transition_timestamp_does_not_precede_a_future_progress_checkpoint()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());
        let mut queued = item(DownloadStatus::Queued)?;
        let future_timestamp = Timestamp::from_unix_millis(
            Timestamp::now()?
                .as_unix_millis()
                .checked_add(60_000)
                .ok_or("timestamp overflow")?,
        );
        queued.set_progress_at(queued.progress().clone(), future_timestamp)?;
        let id = queued.id();
        repository.create(&queued).await?;

        let preparing = state_machine
            .transition(id, DownloadStatus::Preparing)
            .await?;

        assert_eq!(preparing.updated_at(), future_timestamp);
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("download not found")?
                .updated_at(),
            future_timestamp
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_download_returns_not_found() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let state_machine = DownloadStateMachine::new(database.downloads());
        let id = DownloadId::new();

        assert!(matches!(
            state_machine
                .transition(id, DownloadStatus::Preparing)
                .await,
            Err(StateMachineError::NotFound(found)) if found == id
        ));
        Ok(())
    }

    #[tokio::test]
    async fn invalid_transition_is_not_persisted() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());
        let completed = item(DownloadStatus::Completed)?;
        repository.create(&completed).await?;

        assert!(matches!(
            state_machine
                .transition(completed.id(), DownloadStatus::Downloading)
                .await,
            Err(StateMachineError::InvalidTransition {
                from: DownloadStatus::Completed,
                to: DownloadStatus::Downloading
            })
        ));
        assert_eq!(repository.get(completed.id()).await?, Some(completed));
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_transitions_from_one_state_cannot_both_succeed()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let state_machine = DownloadStateMachine::new(repository.clone());
        let downloading = item(DownloadStatus::Downloading)?;
        let id = downloading.id();
        repository.create(&downloading).await?;

        let (completed, failed) = tokio::join!(
            state_machine.transition(id, DownloadStatus::Completed),
            state_machine.transition(id, DownloadStatus::Failed)
        );

        assert_ne!(completed.is_ok(), failed.is_ok());
        let persisted = repository.get(id).await?.ok_or("download not found")?;
        assert!(matches!(
            persisted.status(),
            DownloadStatus::Completed | DownloadStatus::Failed
        ));
        Ok(())
    }
}

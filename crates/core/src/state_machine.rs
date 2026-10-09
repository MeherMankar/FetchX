use mydownloader_models::{
    DownloadId, DownloadItem, DownloadItemError, DownloadStatus, Timestamp, TimestampError,
};
use mydownloader_storage::{DownloadRepository, StorageError};
use thiserror::Error;

/// Validates a lifecycle transition without performing I/O or changing any state.
pub fn validate_transition(
    from: DownloadStatus,
    to: DownloadStatus,
) -> Result<(), StateMachineError> {
    let allowed = matches!(
        (from, to),
        (DownloadStatus::Queued, DownloadStatus::Preparing)
            | (DownloadStatus::Queued, DownloadStatus::Cancelled)
            | (DownloadStatus::Preparing, DownloadStatus::Downloading)
            | (DownloadStatus::Preparing, DownloadStatus::Failed)
            | (DownloadStatus::Preparing, DownloadStatus::Cancelled)
            | (DownloadStatus::Downloading, DownloadStatus::Paused)
            | (DownloadStatus::Downloading, DownloadStatus::Completed)
            | (DownloadStatus::Downloading, DownloadStatus::Failed)
            | (DownloadStatus::Downloading, DownloadStatus::Cancelled)
            | (DownloadStatus::Paused, DownloadStatus::Downloading)
            | (DownloadStatus::Paused, DownloadStatus::Cancelled)
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
        error: mydownloader_models::DownloadError,
    ) -> Result<DownloadItem, StateMachineError> {
        self.transition_inner(id, to, Some(error)).await
    }

    async fn transition_inner(
        &self,
        id: DownloadId,
        to: DownloadStatus,
        error: Option<mydownloader_models::DownloadError>,
    ) -> Result<DownloadItem, StateMachineError> {
        let mut item = self
            .repository
            .get(id)
            .await?
            .ok_or(StateMachineError::NotFound(id))?;
        let from = item.status();
        validate_transition(from, to)?;

        let updated_at = Timestamp::now()?;
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
    DomainError(#[from] mydownloader_models::DownloadErrorMessageError),
    #[error("download state persistence failed: {0}")]
    Storage(#[from] StorageError),
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use mydownloader_models::{
        DownloadId, DownloadItem, DownloadPriority, DownloadProgress, DownloadSource,
        DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use mydownloader_storage::{Database, DownloadRepository};

    use super::{DownloadStateMachine, StateMachineError, validate_transition};

    const STATES: [DownloadStatus; 7] = [
        DownloadStatus::Queued,
        DownloadStatus::Preparing,
        DownloadStatus::Downloading,
        DownloadStatus::Paused,
        DownloadStatus::Completed,
        DownloadStatus::Failed,
        DownloadStatus::Cancelled,
    ];

    const LEGAL_TRANSITIONS: [(DownloadStatus, DownloadStatus); 11] = [
        (DownloadStatus::Queued, DownloadStatus::Preparing),
        (DownloadStatus::Preparing, DownloadStatus::Downloading),
        (DownloadStatus::Downloading, DownloadStatus::Paused),
        (DownloadStatus::Paused, DownloadStatus::Downloading),
        (DownloadStatus::Downloading, DownloadStatus::Completed),
        (DownloadStatus::Downloading, DownloadStatus::Failed),
        (DownloadStatus::Downloading, DownloadStatus::Cancelled),
        (DownloadStatus::Preparing, DownloadStatus::Failed),
        (DownloadStatus::Preparing, DownloadStatus::Cancelled),
        (DownloadStatus::Queued, DownloadStatus::Cancelled),
        (DownloadStatus::Paused, DownloadStatus::Cancelled),
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

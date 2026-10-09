use std::sync::Arc;

use mydownloader_models::{DownloadId, DownloadItem, EngineKind};
use mydownloader_storage::DownloadRepository;
use tokio::sync::watch;

use crate::{
    Aria2Engine, DirectHttpEngine, DownloadEngine, DownloadEngineError, ManagedDownloadControl,
    ManagedDownloadEngine,
};

/// Routes work to the selected backend without moving engine selection into the queue.
pub struct EngineDispatcher<R> {
    repository: R,
    direct_http: Arc<DirectHttpEngine<R>>,
    aria2: Arc<Aria2Engine<R>>,
}

impl<R> EngineDispatcher<R>
where
    R: DownloadRepository + Clone,
{
    pub fn new(
        repository: R,
        direct_http: Arc<DirectHttpEngine<R>>,
        aria2: Arc<Aria2Engine<R>>,
    ) -> Self {
        Self {
            repository,
            direct_http,
            aria2,
        }
    }

    async fn engine_for(&self, id: DownloadId) -> Result<EngineKind, DownloadEngineError> {
        self.repository
            .get(id)
            .await?
            .map(|item| item.source().engine())
            .ok_or(DownloadEngineError::NotFound(id))
    }
}

impl<R> DownloadEngine for EngineDispatcher<R>
where
    R: DownloadRepository + Clone,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        match self.engine_for(id).await? {
            EngineKind::DirectHttp => self.direct_http.download(id, cancellation).await,
            EngineKind::Aria2 => self.aria2.download(id, cancellation).await,
            EngineKind::YtDlp => Err(DownloadEngineError::EngineNotAvailable),
        }
    }
}

impl<R> ManagedDownloadEngine for EngineDispatcher<R>
where
    R: DownloadRepository + Clone,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        match self.engine_for(id).await? {
            EngineKind::DirectHttp => self.direct_http.download_managed(id, control).await,
            EngineKind::Aria2 => self.aria2.download_managed(id, control).await,
            EngineKind::YtDlp => Err(DownloadEngineError::EngineNotAvailable),
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn cancel_inactive(
        &self,
        id: DownloadId,
    ) -> impl std::future::Future<Output = Result<(), DownloadEngineError>> + Send {
        async move {
            match self.engine_for(id).await? {
                EngineKind::Aria2 => self.aria2.cancel_inactive(id).await,
                EngineKind::DirectHttp | EngineKind::YtDlp => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

    use mydownloader_models::{
        DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use mydownloader_storage::{Database, DownloadRepository};
    use tokio::sync::watch;

    use super::EngineDispatcher;
    use crate::{
        Aria2Client, Aria2ClientConfig, Aria2Engine, DirectHttpEngine, DownloadEngine,
        DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    };

    fn item(engine: EngineKind, url: &str) -> Result<DownloadItem, Box<dyn Error>> {
        Ok(DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(url, engine)?,
            FileName::new("file.bin")?,
            PathBuf::from("file.bin"),
            Timestamp::now()?,
        ))
    }

    #[tokio::test]
    async fn dispatcher_routes_by_persisted_engine_kind() -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let direct_item = item(EngineKind::DirectHttp, "not-a-url")?;
        let direct_id = direct_item.id();
        let aria2_item = item(EngineKind::Aria2, "https://example.test/file")?;
        let aria2_id = aria2_item.id();
        let unsupported_item = item(EngineKind::YtDlp, "https://example.test/video")?;
        let unsupported_id = unsupported_item.id();
        for download in [&direct_item, &aria2_item, &unsupported_item] {
            repository.create(download).await?;
        }
        let direct = Arc::new(DirectHttpEngine::new(repository.clone())?);
        let aria2 = Arc::new(Aria2Engine::new(
            repository.clone(),
            Aria2Client::new(Aria2ClientConfig::new(
                "http://127.0.0.1:6800/jsonrpc",
                Some("runtime-only-test-secret".to_owned()),
                Duration::from_millis(50),
            ))?,
        ));
        let dispatcher = EngineDispatcher::new(repository.clone(), direct, aria2);

        assert_eq!(dispatcher.engine_for(aria2_id).await?, EngineKind::Aria2);
        let (_cancel_sender, cancellation) = watch::channel(false);
        let (_pause_sender, pause) = watch::channel(false);
        let (_limit_sender, connection_limit) = watch::channel(1);
        assert!(matches!(
            dispatcher
                .download_managed(
                    direct_id,
                    ManagedDownloadControl {
                        cancellation,
                        pause,
                        connection_limit,
                        resource_demand: None,
                    },
                )
                .await,
            Err(DownloadEngineError::InvalidUrl)
        ));
        assert_eq!(
            repository
                .get(direct_id)
                .await?
                .ok_or("missing direct item")?
                .status(),
            DownloadStatus::Failed
        );

        let (_cancel_sender, cancellation) = watch::channel(false);
        assert!(matches!(
            dispatcher.download(unsupported_id, cancellation).await,
            Err(DownloadEngineError::EngineNotAvailable)
        ));
        Ok(())
    }
}

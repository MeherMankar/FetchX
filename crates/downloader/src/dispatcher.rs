use std::sync::Arc;
use std::time::Duration;

use fetchx_models::{DownloadId, DownloadItem, EngineKind};
use fetchx_storage::DownloadRepository;
use reqwest::{Client, Url, header::CONTENT_TYPE};
use tokio::sync::watch;

use crate::{
    Aria2Engine, DirectHttpEngine, DownloadEngine, DownloadEngineError, FtpEngine, HlsEngine,
    ManagedDownloadControl, ManagedDownloadEngine, NativeTorrentEngine, YtDlpEngine, YtDlpFormat,
    is_hls_url,
};

/// Routes work to the selected backend without moving engine selection into the queue.
pub struct EngineDispatcher<R> {
    repository: R,
    direct_http: Arc<DirectHttpEngine<R>>,
    aria2: Arc<Aria2Engine<R>>,
    yt_dlp: Option<Arc<YtDlpEngine<R>>>,
    hls: Option<Arc<HlsEngine<R>>>,
    native_torrent: Option<Arc<NativeTorrentEngine<R>>>,
    ftp: Option<Arc<FtpEngine<R>>>,
    probe_client: Client,
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
            yt_dlp: None,
            hls: None,
            native_torrent: None,
            ftp: None,
            probe_client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("default HLS probe client configuration is valid"),
        }
    }

    pub fn with_yt_dlp(mut self, engine: Arc<YtDlpEngine<R>>) -> Self {
        self.yt_dlp = Some(engine);
        self
    }

    pub fn with_hls(mut self, engine: Arc<HlsEngine<R>>) -> Self {
        self.hls = Some(engine);
        self
    }

    pub fn with_native_torrent(mut self, engine: Arc<NativeTorrentEngine<R>>) -> Self {
        self.native_torrent = Some(engine);
        self
    }

    pub fn with_ftp(mut self, engine: Arc<FtpEngine<R>>) -> Self {
        self.ftp = Some(engine);
        self
    }

    pub async fn discover_ytdlp_formats(
        &self,
        url: &str,
    ) -> Result<Vec<YtDlpFormat>, DownloadEngineError> {
        self.yt_dlp
            .as_ref()
            .ok_or(DownloadEngineError::EngineNotAvailable)?
            .discover_formats(url)
            .await
            .map_err(Into::into)
    }

    async fn engine_for(&self, id: DownloadId) -> Result<EngineKind, DownloadEngineError> {
        self.repository
            .get(id)
            .await?
            .map(|item| item.source().engine())
            .ok_or(DownloadEngineError::NotFound(id))
    }

    async fn selected_engine_for(&self, id: DownloadId) -> Result<EngineKind, DownloadEngineError> {
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;
        let source = item.source();
        if source.engine() == EngineKind::Hls
            || (source.engine() == EngineKind::DirectHttp && is_hls_url(source.url()))
        {
            return Ok(EngineKind::Hls);
        }
        if source.engine() != EngineKind::DirectHttp {
            return Ok(source.engine());
        }

        let Ok(url) = Url::parse(source.url()) else {
            return Ok(EngineKind::DirectHttp);
        };
        let response = self.probe_client.head(url).send().await;
        if response.is_ok_and(|response| {
            response.status().is_success()
                && response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(is_hls_content_type)
        }) {
            Ok(EngineKind::Hls)
        } else {
            Ok(EngineKind::DirectHttp)
        }
    }
}

fn is_hls_content_type(content_type: &str) -> bool {
    matches!(
        content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "application/x-mpegurl" | "application/vnd.apple.mpegurl"
    )
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
        match self.selected_engine_for(id).await? {
            EngineKind::DirectHttp => Box::pin(self.direct_http.download(id, cancellation)).await,
            EngineKind::Aria2 => Box::pin(self.aria2.download(id, cancellation)).await,
            EngineKind::YtDlp => match &self.yt_dlp {
                Some(engine) => Box::pin(engine.download(id, cancellation)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::Hls => match &self.hls {
                Some(engine) => Box::pin(engine.download(id, cancellation)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::NativeTorrent => match &self.native_torrent {
                Some(engine) => Box::pin(engine.download(id, cancellation)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::Ftp | EngineKind::Ftps => match &self.ftp {
                Some(engine) => Box::pin(engine.download(id, cancellation)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::Sftp | EngineKind::Metalink => {
                Err(DownloadEngineError::EngineNotAvailable)
            }
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
        match self.selected_engine_for(id).await? {
            EngineKind::DirectHttp => {
                Box::pin(self.direct_http.download_managed(id, control)).await
            }
            EngineKind::Aria2 => Box::pin(self.aria2.download_managed(id, control)).await,
            EngineKind::YtDlp => match &self.yt_dlp {
                Some(engine) => Box::pin(engine.download_managed(id, control)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::Hls => match &self.hls {
                Some(engine) => Box::pin(engine.download_managed(id, control)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::NativeTorrent => match &self.native_torrent {
                Some(engine) => Box::pin(engine.download_managed(id, control)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::Ftp | EngineKind::Ftps => match &self.ftp {
                Some(engine) => Box::pin(engine.download_managed(id, control)).await,
                None => Err(DownloadEngineError::EngineNotAvailable),
            },
            EngineKind::Sftp | EngineKind::Metalink => {
                Err(DownloadEngineError::EngineNotAvailable)
            }
        }
    }

    fn supports_active_pause(&self, engine: EngineKind) -> bool {
        match engine {
            EngineKind::YtDlp => self
                .yt_dlp
                .as_ref()
                .is_some_and(|engine| engine.supports_active_pause(EngineKind::YtDlp)),
            EngineKind::DirectHttp
            | EngineKind::Aria2
            | EngineKind::Hls
            | EngineKind::NativeTorrent
            | EngineKind::Ftp
            | EngineKind::Ftps
            | EngineKind::Sftp
            | EngineKind::Metalink => true,
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
                EngineKind::NativeTorrent => match &self.native_torrent {
                    Some(engine) => engine.cancel_inactive(id).await,
                    None => Ok(()),
                },
                EngineKind::DirectHttp
                | EngineKind::YtDlp
                | EngineKind::Hls
                | EngineKind::Ftp
                | EngineKind::Ftps
                | EngineKind::Sftp
                | EngineKind::Metalink => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        ffi::OsString,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use fetchx_models::{
        DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
    };
    use fetchx_storage::{Database, DownloadRepository};
    use tokio::sync::{mpsc, watch};

    use super::EngineDispatcher;
    use crate::{
        Aria2Client, Aria2ClientConfig, Aria2Engine, DirectHttpEngine, DownloadEngine,
        DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine, YtDlpConfig,
        YtDlpEngine, YtDlpError, YtDlpProcessExit, YtDlpProcessRunner, YtDlpRunnerEvent,
        YtDlpRunnerFuture,
    };

    struct CountingRunner(Arc<AtomicUsize>);

    impl YtDlpProcessRunner for CountingRunner {
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
            _arguments: &'a [OsString],
            _stop: watch::Receiver<bool>,
            _events: mpsc::Sender<YtDlpRunnerEvent>,
        ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(YtDlpProcessExit {
                    success: true,
                    code: Some(0),
                })
            })
        }
    }

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
                        bandwidth: crate::BandwidthControl::default(),
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

    #[tokio::test]
    async fn dispatcher_invokes_configured_ytdlp_runner_without_system_install()
    -> Result<(), Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let base =
            std::env::temp_dir().join(format!("fetchx-dispatcher-test-{}", DownloadId::new()));
        tokio::fs::create_dir_all(&base).await?;
        let video = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new("https://example.test/video", EngineKind::YtDlp)?,
            FileName::new("video.webm")?,
            base.join("video.webm"),
            Timestamp::now()?,
        );
        let id = video.id();
        repository.create(&video).await?;
        let direct = Arc::new(DirectHttpEngine::new(repository.clone())?);
        let aria2 = Arc::new(Aria2Engine::new(
            repository.clone(),
            Aria2Client::new(Aria2ClientConfig::new(
                "http://127.0.0.1:6800/jsonrpc",
                None,
                Duration::from_millis(50),
            ))?,
        ));
        let run_count = Arc::new(AtomicUsize::new(0));
        let yt_dlp = Arc::new(
            YtDlpEngine::initialize_with_runner(
                repository.clone(),
                YtDlpConfig::new().with_executable(std::env::current_exe()?),
                Arc::new(CountingRunner(Arc::clone(&run_count))),
            )
            .await?,
        );
        let dispatcher =
            EngineDispatcher::new(repository.clone(), direct, aria2).with_yt_dlp(yt_dlp);
        let (_cancel_sender, cancellation) = watch::channel(false);

        assert!(matches!(
            dispatcher.download(id, cancellation).await,
            Err(DownloadEngineError::YtDlp(YtDlpError::OutputUnavailable))
        ));
        assert_eq!(run_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            repository
                .get(id)
                .await?
                .ok_or("missing dispatched item")?
                .status(),
            DownloadStatus::Failed
        );
        tokio::fs::remove_dir_all(base).await?;
        Ok(())
    }
}

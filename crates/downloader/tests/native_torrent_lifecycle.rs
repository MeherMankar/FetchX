use std::{error::Error, path::Path, sync::Arc, time::Duration};

use fetchx_core::DownloadStateMachine;
use fetchx_downloader::{
    BandwidthControl, DownloadEngineError, ManagedDownloadControl, ManagedDownloadEngine,
    NativeTorrentConfig, NativeTorrentEngine,
};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
};
use fetchx_storage::{Database, DownloadRepository};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use tokio::sync::watch;

fn valid_private_torrent_bytes(payload: &[u8]) -> Vec<u8> {
    let piece_hash = Sha1::digest(payload);
    let tracker = "http://127.0.0.1:1/announce";
    let mut bytes = format!("d8:announce{}:{}4:infod", tracker.len(), tracker).into_bytes();
    bytes.extend_from_slice(format!("6:lengthi{}e", payload.len()).as_bytes());
    bytes.extend_from_slice(b"4:name9:hello.txt12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&piece_hash);
    bytes.extend_from_slice(b"7:privatei1eee");
    bytes
}

fn managed_control() -> (
    watch::Sender<bool>,
    watch::Sender<bool>,
    ManagedDownloadControl,
) {
    let (cancellation, cancellation_rx) = watch::channel(false);
    let (pause, pause_rx) = watch::channel(false);
    (
        cancellation,
        pause,
        ManagedDownloadControl {
            cancellation: cancellation_rx,
            pause: pause_rx,
            connection_limit: watch::channel(1).1,
            resource_demand: None,
            bandwidth: BandwidthControl::default(),
        },
    )
}

async fn create_native_download(
    repository: &impl DownloadRepository,
    directory: &Path,
) -> Result<DownloadItem, Box<dyn Error>> {
    let torrent_path = directory.join("fixture.torrent");
    tokio::fs::write(&torrent_path, valid_private_torrent_bytes(b"hello")).await?;
    let source = DownloadSource::new_torrent_file(&torrent_path, EngineKind::NativeTorrent)?;
    let item = DownloadItem::new(
        DownloadId::new(),
        source,
        FileName::new("hello.txt")?,
        directory.join("payload"),
        Timestamp::now()?,
    );
    repository.create(&item).await?;
    Ok(item)
}

async fn wait_until_status(
    repository: &impl DownloadRepository,
    id: DownloadId,
    status: DownloadStatus,
) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if repository
                .get(id)
                .await?
                .is_some_and(|item| item.status() == status)
            {
                return Ok::<(), Box<dyn Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    Ok(())
}

fn active_torrent_count<R>(engine: &NativeTorrentEngine<R>) -> usize
where
    R: DownloadRepository + Clone,
{
    engine.session().with_torrents(|torrents| torrents.count())
}

#[tokio::test]
async fn native_pause_resume_cancel_releases_torrent_session_entries() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let item = create_native_download(&repository, directory.path()).await?;
    let engine = Arc::new(
        NativeTorrentEngine::new(
            repository.clone(),
            NativeTorrentConfig {
                listen_port: 0,
                session_dir: Some(directory.path().join("session")),
                ..NativeTorrentConfig::default()
            },
        )
        .await?,
    );

    let (_cancel, pause, control) = managed_control();
    let worker_engine = Arc::clone(&engine);
    let id = item.id();
    let worker = tokio::spawn(async move { worker_engine.download_managed(id, control).await });
    wait_until_status(&repository, id, DownloadStatus::Downloading).await?;
    assert_eq!(active_torrent_count(&engine), 1);

    pause.send(true)?;
    let result = tokio::time::timeout(Duration::from_secs(5), worker).await??;
    assert!(matches!(result, Err(DownloadEngineError::Paused)));
    assert_eq!(
        repository.get(id).await?.unwrap().status(),
        DownloadStatus::Paused
    );
    assert_eq!(active_torrent_count(&engine), 0);

    DownloadStateMachine::new(repository.clone())
        .transition(id, DownloadStatus::Queued)
        .await?;
    let (cancel, _pause, control) = managed_control();
    let worker_engine = Arc::clone(&engine);
    let worker = tokio::spawn(async move { worker_engine.download_managed(id, control).await });
    wait_until_status(&repository, id, DownloadStatus::Downloading).await?;
    assert_eq!(active_torrent_count(&engine), 1);

    cancel.send(true)?;
    let result = tokio::time::timeout(Duration::from_secs(5), worker).await??;
    assert!(matches!(result, Err(DownloadEngineError::Cancelled)));
    assert_eq!(
        repository.get(id).await?.unwrap().status(),
        DownloadStatus::Cancelled
    );
    assert_eq!(active_torrent_count(&engine), 0);
    Ok(())
}

#[tokio::test]
async fn completed_native_torrent_seeds_then_stops_without_deleting_payload()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let item = create_native_download(&repository, directory.path()).await?;
    let payload = b"hello";
    tokio::fs::create_dir_all(item.destination()).await?;
    tokio::fs::write(item.destination().join("hello.txt"), payload).await?;
    let expected_sha256 = hex::encode(Sha256::digest(payload));
    let engine = NativeTorrentEngine::new(
        repository.clone(),
        NativeTorrentConfig {
            listen_port: 0,
            session_dir: Some(directory.path().join("session")),
            ..NativeTorrentConfig::default()
        },
    )
    .await?;

    let (_, _, control) = managed_control();
    let completed = tokio::time::timeout(
        Duration::from_secs(10),
        engine.download_managed(item.id(), control),
    )
    .await??;
    assert_eq!(completed.status(), DownloadStatus::Seeding);
    assert_eq!(
        completed.progress().downloaded_bytes(),
        payload.len() as u64
    );
    assert_eq!(
        completed.progress().total_bytes(),
        Some(payload.len() as u64)
    );

    let payload_path = item.destination().join("hello.txt");
    let actual_payload = tokio::fs::read(&payload_path).await?;
    assert_eq!(actual_payload, payload);
    assert_eq!(
        hex::encode(Sha256::digest(&actual_payload)),
        expected_sha256
    );
    assert_eq!(active_torrent_count(&engine), 1);

    engine.stop_seeding(item.id()).await?;
    assert_eq!(active_torrent_count(&engine), 0);
    assert_eq!(tokio::fs::read(payload_path).await?, payload);
    Ok(())
}

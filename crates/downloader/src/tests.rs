use std::{
    error::Error,
    path::{Path, PathBuf},
    time::Duration,
};

use mydownloader_models::{
    DownloadId, DownloadItem, DownloadSource, DownloadStatus, EngineKind, FileName, Timestamp,
};
use mydownloader_storage::{Database, DownloadRepository};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch},
    task::JoinHandle,
    time::sleep,
};

use crate::{DirectHttpEngine, DownloadEngineError, HttpEngineConfig};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Result<Self, std::io::Error> {
        let path = std::env::temp_dir().join(format!("mydownloader-{}", DownloadId::new()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn destination(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ServerResponse {
    status: u16,
    body_chunks: Vec<Vec<u8>>,
    content_length: Option<u64>,
    delay_between_chunks: Duration,
    signal_after_first_chunk: Option<oneshot::Sender<()>>,
}

impl ServerResponse {
    fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_length: Some(body.len() as u64),
            body_chunks: vec![body],
            delay_between_chunks: Duration::ZERO,
            signal_after_first_chunk: None,
        }
    }
}

async fn start_server(
    mut response: ServerResponse,
) -> Result<(String, JoinHandle<()>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            if !serve_connection(stream, &mut response).await {
                break;
            }
        }
    });

    Ok((format!("http://{address}/file"), task))
}

async fn serve_connection(mut stream: TcpStream, response: &mut ServerResponse) -> bool {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let Ok(read) = stream.read(&mut buffer).await else {
            return false;
        };
        if read == 0 {
            return false;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = String::from_utf8_lossy(&request);
    let path = request.lines().next().unwrap_or_default();
    if path.contains("/redirect ") {
        let _ = stream
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /file\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        let _ = stream.flush().await;
        return true;
    }

    let reason = if response.status == 200 {
        "OK"
    } else {
        "Failure"
    };
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        response.status, reason
    );
    if let Some(content_length) = response.content_length {
        headers.push_str(&format!("Content-Length: {content_length}\r\n"));
    } else {
        headers.push_str("Transfer-Encoding: chunked\r\n");
    }
    headers.push_str("\r\n");
    if stream.write_all(headers.as_bytes()).await.is_err() {
        return false;
    }

    let mut signalled = false;
    for chunk in response.body_chunks.drain(..) {
        if response.content_length.is_none() {
            let prefix = format!("{:X}\r\n", chunk.len());
            if stream.write_all(prefix.as_bytes()).await.is_err()
                || stream.write_all(&chunk).await.is_err()
                || stream.write_all(b"\r\n").await.is_err()
            {
                return false;
            }
        } else if stream.write_all(&chunk).await.is_err() {
            return false;
        }
        if stream.flush().await.is_err() {
            return false;
        }

        if !signalled {
            if let Some(signal) = response.signal_after_first_chunk.take() {
                let _ = signal.send(());
            }
            signalled = true;
        }
        if !response.delay_between_chunks.is_zero() {
            sleep(response.delay_between_chunks).await;
        }
    }

    if response.content_length.is_none() {
        let _ = stream.write_all(b"0\r\n\r\n").await;
        let _ = stream.flush().await;
    }
    false
}

fn make_item(url: &str, destination: &Path) -> Result<DownloadItem, Box<dyn Error>> {
    Ok(DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(url, EngineKind::DirectHttp)?,
        FileName::new("download.bin")?,
        destination.to_path_buf(),
        Timestamp::from_unix_millis(1),
    ))
}

async fn setup_engine(
    config: HttpEngineConfig,
) -> Result<
    (
        Database,
        DirectHttpEngine<mydownloader_storage::SqliteDownloadRepository>,
        TestDirectory,
    ),
    Box<dyn Error>,
> {
    let database = Database::connect("sqlite::memory:").await?;
    let engine = DirectHttpEngine::with_config(database.downloads(), config)?;
    let directory = TestDirectory::new()?;
    Ok((database, engine, directory))
}

async fn run_download(
    engine: &DirectHttpEngine<mydownloader_storage::SqliteDownloadRepository>,
    database: &Database,
    item: DownloadItem,
) -> Result<DownloadItem, Box<dyn Error>> {
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);
    Ok(engine.download(id, receiver).await?)
}

#[tokio::test]
async fn downloads_small_empty_unknown_length_and_large_files() -> Result<(), Box<dyn Error>> {
    let config = HttpEngineConfig {
        progress_bytes_interval: 32 * 1024,
        ..HttpEngineConfig::default()
    };
    let (database, engine, directory) = setup_engine(config).await?;

    for (name, body, content_length) in [
        ("small.bin", b"small response".to_vec(), Some(14)),
        ("empty.bin", Vec::new(), Some(0)),
        ("unknown.bin", b"chunked data".to_vec(), None),
        (
            "large.bin",
            (0..(3 * 1024 * 1024))
                .map(|index| (index % 251) as u8)
                .collect(),
            Some(3 * 1024 * 1024),
        ),
    ] {
        let mut response = ServerResponse::ok(body.clone());
        response.content_length = content_length;
        response.body_chunks = body.chunks(16 * 1024).map(<[u8]>::to_vec).collect();
        let (url, server) = start_server(response).await?;
        let destination = directory.destination(name);
        let item = make_item(&url, &destination)?;
        let completed = run_download(&engine, &database, item).await?;
        server.await?;

        assert_eq!(tokio::fs::read(&destination).await?, body);
        assert_eq!(
            completed.progress().downloaded_bytes(),
            tokio::fs::metadata(&destination).await?.len()
        );
        assert_eq!(completed.progress().total_bytes(), content_length);
        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert!(!PathBuf::from(format!("{}.part", destination.display())).exists());
    }

    Ok(())
}

#[tokio::test]
async fn follows_a_normal_redirect() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let (file_url, server) = start_server(ServerResponse::ok(b"redirected".to_vec())).await?;
    let url = file_url.replace("/file", "/redirect");
    let destination = directory.destination("redirect.bin");
    let item = make_item(&url, &destination)?;
    let completed = run_download(&engine, &database, item).await?;
    server.await?;

    assert_eq!(tokio::fs::read(destination).await?, b"redirected");
    assert_eq!(completed.status(), DownloadStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn http_failures_persist_failed_state_and_safe_error() -> Result<(), Box<dyn Error>> {
    for status in [404, 403, 429, 500, 502, 503] {
        let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
        let mut response = ServerResponse::ok(b"failure body".to_vec());
        response.status = status;
        let (url, server) = start_server(response).await?;
        let item = make_item(&url, &directory.destination("failed.bin"))?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);

        assert!(matches!(
            engine.download(id, receiver).await,
            Err(DownloadEngineError::HttpStatus(found)) if found == status
        ));
        server.await?;
        let persisted = database
            .downloads()
            .get(id)
            .await?
            .ok_or("failed download not found")?;
        assert_eq!(persisted.status(), DownloadStatus::Failed);
        assert!(persisted.error().is_some());
        assert!(!directory.destination("failed.bin").exists());
    }

    Ok(())
}

#[tokio::test]
async fn truncated_response_fails_and_cleans_up_the_partial_file() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let mut response = ServerResponse::ok(b"partial".to_vec());
    response.content_length = Some(64);
    let (url, server) = start_server(response).await?;
    let destination = directory.destination("truncated.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    assert!(engine.download(id, receiver).await.is_err());
    server.await?;
    let persisted = database
        .downloads()
        .get(id)
        .await?
        .ok_or("failed download not found")?;
    assert_eq!(persisted.status(), DownloadStatus::Failed);
    assert!(persisted.progress().downloaded_bytes() <= 7);
    assert!(!destination.exists());
    assert!(!PathBuf::from(format!("{}.part", destination.display())).exists());
    Ok(())
}

#[tokio::test]
async fn rejects_invalid_urls_and_existing_destinations() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let invalid = make_item("not a url", &directory.destination("invalid.bin"))?;
    let invalid_id = invalid.id();
    database.downloads().create(&invalid).await?;
    let (_sender, receiver) = watch::channel(false);
    assert!(matches!(
        engine.download(invalid_id, receiver).await,
        Err(DownloadEngineError::InvalidUrl)
    ));
    assert_eq!(
        database
            .downloads()
            .get(invalid_id)
            .await?
            .ok_or("download not found")?
            .status(),
        DownloadStatus::Failed
    );

    let existing_path = directory.destination("existing.bin");
    tokio::fs::write(&existing_path, b"keep me").await?;
    let (url, server) = start_server(ServerResponse::ok(b"overwrite".to_vec())).await?;
    let existing = make_item(&url, &existing_path)?;
    let existing_id = existing.id();
    database.downloads().create(&existing).await?;
    let (_sender, receiver) = watch::channel(false);
    assert!(matches!(
        engine.download(existing_id, receiver).await,
        Err(DownloadEngineError::DestinationExists)
    ));
    server.abort();
    assert_eq!(tokio::fs::read(existing_path).await?, b"keep me");
    Ok(())
}

#[tokio::test]
async fn unsupported_schemes_and_embedded_credentials_are_rejected() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    for (url, expected) in [
        ("ftp://example.test/file", "unsupported"),
        ("http://user:password@example.test/file", "credentials"),
    ] {
        let item = make_item(url, &directory.destination("rejected.bin"))?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);
        let result = engine.download(id, receiver).await;
        assert!(matches!(
            (result, expected),
            (Err(DownloadEngineError::UnsupportedScheme), "unsupported")
                | (Err(DownloadEngineError::EmbeddedCredentials), "credentials")
        ));
        assert_eq!(
            database
                .downloads()
                .get(id)
                .await?
                .ok_or("download not found")?
                .status(),
            DownloadStatus::Failed
        );
    }
    Ok(())
}

#[tokio::test]
async fn rejects_downloads_assigned_to_another_engine() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new("https://example.test/file", EngineKind::YtDlp)?,
        FileName::new("download.bin")?,
        directory.destination("mismatch.bin"),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    assert!(matches!(
        engine.download(id, receiver).await,
        Err(DownloadEngineError::EngineMismatch)
    ));
    assert_eq!(
        database
            .downloads()
            .get(id)
            .await?
            .ok_or("download not found")?
            .status(),
        DownloadStatus::Failed
    );
    Ok(())
}

#[tokio::test]
async fn cancellation_removes_partial_file_and_persists_cancelled_state()
-> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let body = vec![8_u8; 1024 * 1024];
    let (signal, notified) = oneshot::channel();
    let mut response = ServerResponse::ok(body.clone());
    response.body_chunks = body.chunks(1024).map(<[u8]>::to_vec).collect();
    response.delay_between_chunks = Duration::from_millis(2);
    response.signal_after_first_chunk = Some(signal);
    let (url, server) = start_server(response).await?;
    let destination = directory.destination("cancelled.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (sender, receiver) = watch::channel(false);
    let engine_task = tokio::spawn(async move { engine.download(id, receiver).await });

    notified.await?;
    sender.send(true)?;
    assert!(matches!(
        engine_task.await?,
        Err(DownloadEngineError::Cancelled)
    ));
    server.await?;

    let cancelled = database
        .downloads()
        .get(id)
        .await?
        .ok_or("cancelled download not found")?;
    assert_eq!(cancelled.status(), DownloadStatus::Cancelled);
    assert!(!destination.exists());
    assert!(!PathBuf::from(format!("{}.part", destination.display())).exists());
    Ok(())
}

#[tokio::test]
async fn progress_is_persisted_during_transfer_and_final_state_is_complete()
-> Result<(), Box<dyn Error>> {
    let config = HttpEngineConfig {
        progress_bytes_interval: 1024,
        progress_time_interval: Duration::from_secs(60),
        ..HttpEngineConfig::default()
    };
    let (database, engine, directory) = setup_engine(config).await?;
    let body = vec![17_u8; 128 * 1024];
    let mut response = ServerResponse::ok(body.clone());
    response.body_chunks = body.chunks(1024).map(<[u8]>::to_vec).collect();
    response.delay_between_chunks = Duration::from_millis(3);
    let (url, server) = start_server(response).await?;
    let destination = directory.destination("progress.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);
    let engine_task = tokio::spawn(async move { engine.download(id, receiver).await });

    let mut observed_intermediate_progress = false;
    for _ in 0..100 {
        let current = database.downloads().get(id).await?;
        if current.is_some_and(|item| {
            item.status() == DownloadStatus::Downloading
                && item.progress().downloaded_bytes() > 0
                && item.progress().downloaded_bytes() < body.len() as u64
        }) {
            observed_intermediate_progress = true;
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }

    assert!(observed_intermediate_progress);
    let completed = engine_task.await??;
    server.await?;
    let persisted = database
        .downloads()
        .get(id)
        .await?
        .ok_or("completed download not found")?;
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(persisted.progress().downloaded_bytes(), body.len() as u64);
    assert_eq!(persisted.progress().total_bytes(), Some(body.len() as u64));
    assert_eq!(tokio::fs::read(destination).await?, body);
    Ok(())
}

use std::{
    error::Error,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use fetchx_models::{
    DownloadId, DownloadItem, DownloadProgress, DownloadSource, DownloadStatus, EngineKind,
    FileName, Timestamp,
};
use fetchx_storage::{Database, DownloadRepository};
use sha2::Digest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::{Mutex as AsyncMutex, oneshot, watch},
    task::JoinHandle,
    time::sleep,
};

use crate::{
    AdaptiveConfig, BandwidthControl, BandwidthLimiter, DirectHttpEngine, DownloadEngineError,
    HttpEngineConfig, ManagedDownloadControl, ManagedDownloadEngine,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Result<Self, std::io::Error> {
        let path = std::env::temp_dir().join(format!("FetchX-{}", DownloadId::new()));
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

async fn attach_http_multi_source_set(
    destination: &Path,
    urls: &[String],
) -> Result<(), Box<dyn Error>> {
    let mut path = destination.as_os_str().to_os_string();
    path.push(".fetchx.sources");
    let bytes = serde_json::to_vec(&serde_json::json!({ "version": 1, "urls": urls }))?;
    tokio::fs::write(PathBuf::from(path), bytes).await?;
    Ok(())
}

async fn attach_expected_sha256(destination: &Path, body: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut path = destination.as_os_str().to_os_string();
    path.push(".fetchx.sha256");
    tokio::fs::write(PathBuf::from(path), hex::encode(sha2::Sha256::digest(body))).await?;
    Ok(())
}

struct ServerResponse {
    status: u16,
    body_chunks: Vec<Vec<u8>>,
    content_length: Option<u64>,
    delay_between_chunks: Duration,
    signal_after_first_chunk: Option<oneshot::Sender<()>>,
}

pub(crate) struct ScriptedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Clone, Copy)]
enum SegmentServerMode {
    Valid,
    NoValidator,
    WrongStart,
    WrongEnd,
    WrongTotal,
    ShortBody,
    IgnoreRange,
    Fail,
    FailAfterProbe,
    FailAboveConcurrency(usize),
    RateLimit,
}

struct SegmentServerReport {
    requests: Vec<String>,
    max_active_segments: usize,
    completion_order: Vec<usize>,
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

async fn start_process_recovery_server(
    body: Vec<u8>,
) -> Result<(String, JoinHandle<Vec<String>>, oneshot::Sender<()>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown, mut shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let accepted = tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((mut stream, _)) = accepted else {
                break;
            };
            let Some(request) = read_request(&mut stream).await else {
                continue;
            };
            requests.push(request);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\nETag: \"process-recovery-v1\"\r\n\r\n",
                body.len()
            );
            if stream.write_all(headers.as_bytes()).await.is_err() {
                continue;
            }
            for chunk in body.chunks(16 * 1024) {
                if stream.write_all(chunk).await.is_err() || stream.flush().await.is_err() {
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
        }
        requests
    });

    Ok((format!("http://{address}/process-recovery"), task, shutdown))
}

pub(crate) async fn start_scripted_server(
    responses: Vec<ScriptedResponse>,
) -> Result<(String, JoinHandle<Vec<String>>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while let Ok(read) = stream.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            requests.push(String::from_utf8_lossy(&request).into_owned());

            let mut headers = format!(
                "HTTP/1.1 {} Test Response\r\nConnection: close\r\n",
                response.status
            );
            for (name, value) in response.headers {
                headers.push_str(&format!("{name}: {value}\r\n"));
            }
            if !headers.to_ascii_lowercase().contains("content-length:") {
                headers.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
            }
            headers.push_str("\r\n");
            if stream.write_all(headers.as_bytes()).await.is_err()
                || stream.write_all(&response.body).await.is_err()
            {
                break;
            }
            let _ = stream.flush().await;
        }
        requests
    });

    Ok((format!("http://{address}/file"), task))
}

async fn start_retry_control_server(
    body: Vec<u8>,
) -> Result<
    (
        String,
        oneshot::Receiver<()>,
        Arc<AsyncMutex<Vec<String>>>,
        JoinHandle<()>,
    ),
    std::io::Error,
> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (retry_sender, retry_started) = oneshot::channel();
    let requests = Arc::new(AsyncMutex::new(Vec::new()));
    let requests_for_server = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        let Ok((mut first, _)) = listener.accept().await else {
            return;
        };
        if let Some(request) = read_request(&mut first).await {
            requests_for_server.lock().await.push(request);
        }
        let response = b"HTTP/1.1 503 Retry\r\nContent-Length: 0\r\nRetry-After: 5\r\nConnection: close\r\n\r\n";
        if first.write_all(response).await.is_err() || first.flush().await.is_err() {
            return;
        }
        let _ = retry_sender.send(());

        let Ok((mut second, _)) = listener.accept().await else {
            return;
        };
        if let Some(request) = read_request(&mut second).await {
            requests_for_server.lock().await.push(request);
        }
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        if second.write_all(headers.as_bytes()).await.is_ok() {
            let _ = second.write_all(&body).await;
            let _ = second.flush().await;
        }
    });

    Ok((
        format!("http://{address}/retry-control"),
        retry_started,
        requests,
        task,
    ))
}

async fn start_redirect_server(
    redirect_target: Option<String>,
) -> Result<(String, JoinHandle<Vec<String>>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        let Ok((mut first_stream, _)) = listener.accept().await else {
            return requests;
        };
        if let Some(request) = read_request(&mut first_stream).await {
            requests.push(request);
        }
        let location = redirect_target.unwrap_or_else(|| format!("http://{address}/private"));
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        if first_stream.write_all(redirect.as_bytes()).await.is_err() {
            return requests;
        }
        let _ = first_stream.flush().await;
        drop(first_stream);
        requests
    });

    Ok((format!("http://{address}/redirect"), task))
}

async fn start_disconnect_then_success_server(
    body: Vec<u8>,
) -> Result<(String, JoinHandle<Vec<String>>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for attempt in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            if let Some(request) = read_request(&mut stream).await {
                requests.push(request);
            }
            if attempt == 1 {
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes()).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.flush().await;
            }
        }
        requests
    });

    Ok((format!("http://{address}/file"), task))
}

async fn start_cancel_resume_server(
    body: Vec<u8>,
) -> Result<
    (
        String,
        oneshot::Receiver<()>,
        oneshot::Sender<()>,
        JoinHandle<Vec<String>>,
    ),
    std::io::Error,
> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (first_chunk_sent, first_chunk_received) = oneshot::channel();
    let (continue_server, wait_for_continue) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        let Ok((mut first_stream, _)) = listener.accept().await else {
            return requests;
        };
        let Some(first_request) = read_request(&mut first_stream).await else {
            return requests;
        };
        requests.push(first_request);
        let first_chunk_size = 64 * 1024;
        let first_headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"test-v1\"\r\nConnection: close\r\n\r\n",
            body.len()
        );
        if first_stream
            .write_all(first_headers.as_bytes())
            .await
            .is_err()
            || first_stream
                .write_all(&body[..first_chunk_size.min(body.len())])
                .await
                .is_err()
            || first_stream.flush().await.is_err()
        {
            return requests;
        }
        let _ = first_chunk_sent.send(());
        let _ = wait_for_continue.await;
        drop(first_stream);

        let Ok((mut resume_stream, _)) = listener.accept().await else {
            return requests;
        };
        let Some(resume_request) = read_request(&mut resume_stream).await else {
            return requests;
        };
        let Some(range_header) = range_header(&resume_request) else {
            return requests;
        };
        let Some(start) = range_header
            .strip_prefix("bytes=")
            .and_then(|value| value.strip_suffix('-'))
            .and_then(|value| value.parse::<usize>().ok())
        else {
            return requests;
        };
        requests.push(resume_request);
        if start >= body.len() {
            return requests;
        }

        let remaining = &body[start..];
        let response_headers = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nETag: \"test-v1\"\r\nConnection: close\r\n\r\n",
            remaining.len(),
            start,
            body.len() - 1,
            body.len()
        );
        let _ = resume_stream.write_all(response_headers.as_bytes()).await;
        let _ = resume_stream.write_all(remaining).await;
        let _ = resume_stream.flush().await;
        requests
    });

    Ok((
        format!("http://{address}/file"),
        first_chunk_received,
        continue_server,
        task,
    ))
}

async fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&request).into_owned());
        }
    }
    None
}

async fn start_segment_server(
    body: Vec<u8>,
    segment_count: usize,
    mode: SegmentServerMode,
    total_connections: usize,
    delayed: bool,
) -> Result<(String, JoinHandle<SegmentServerReport>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let requests = Arc::new(AsyncMutex::new(Vec::new()));
    let completion_order = Arc::new(AsyncMutex::new(Vec::new()));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let no_range_requests = Arc::new(AtomicUsize::new(0));
    let accept_idle_timeout = if matches!(
        mode,
        SegmentServerMode::Fail
            | SegmentServerMode::ShortBody
            | SegmentServerMode::FailAboveConcurrency(_)
            | SegmentServerMode::FailAfterProbe
            | SegmentServerMode::WrongStart
            | SegmentServerMode::WrongEnd
            | SegmentServerMode::WrongTotal
            | SegmentServerMode::IgnoreRange
    ) {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(5)
    };
    let task = tokio::spawn(async move {
        let mut connections = Vec::with_capacity(total_connections);
        while connections.len() < total_connections {
            let accepted = tokio::time::timeout(accept_idle_timeout, listener.accept()).await;
            let Ok(Ok((stream, _))) = accepted else {
                break;
            };
            let body = body.clone();
            let requests = Arc::clone(&requests);
            let completion_order = Arc::clone(&completion_order);
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            let no_range_requests = Arc::clone(&no_range_requests);
            connections.push(tokio::spawn(async move {
                serve_segment_connection(
                    stream,
                    body,
                    segment_count,
                    mode,
                    delayed,
                    requests,
                    completion_order,
                    active,
                    max_active,
                    no_range_requests,
                )
                .await
            }));
        }
        for connection in connections {
            let _ = connection.await;
        }
        SegmentServerReport {
            requests: Arc::try_unwrap(requests)
                .map(|requests| requests.into_inner())
                .unwrap_or_default(),
            max_active_segments: max_active.load(Ordering::Acquire),
            completion_order: Arc::try_unwrap(completion_order)
                .map(|order| order.into_inner())
                .unwrap_or_default(),
        }
    });

    Ok((format!("http://{address}/file"), task))
}

#[allow(clippy::too_many_arguments)]
async fn serve_segment_connection(
    mut stream: TcpStream,
    body: Vec<u8>,
    segment_count: usize,
    mode: SegmentServerMode,
    delayed: bool,
    requests: Arc<AsyncMutex<Vec<String>>>,
    completion_order: Arc<AsyncMutex<Vec<usize>>>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    no_range_requests: Arc<AtomicUsize>,
) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let range = range_header(&request).map(str::to_owned);
    requests.lock().await.push(request);

    let Some(range) = range else {
        let _ = no_range_requests.fetch_add(1, Ordering::AcqRel);
        if matches!(mode, SegmentServerMode::FailAfterProbe) {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return;
        }
        let validator = if matches!(mode, SegmentServerMode::NoValidator) {
            ""
        } else {
            "ETag: \"test-v1\"\r\n"
        };
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{validator}Connection: close\r\n\r\n",
            body.len(),
        );
        if stream.write_all(headers.as_bytes()).await.is_err() || stream.flush().await.is_err() {
            return;
        }
        if !delayed {
            let _ = stream.write_all(&body).await;
        } else {
            for chunk in body.chunks(16 * 1024) {
                if stream.write_all(chunk).await.is_err() {
                    return;
                }
                sleep(Duration::from_millis(1)).await;
            }
        }
        let _ = stream.flush().await;
        return;
    };

    let Some((start, end)) = range
        .strip_prefix("bytes=")
        .and_then(|value| value.split_once('-'))
        .and_then(|(start, end)| Some((start.parse::<usize>().ok()?, end.parse::<usize>().ok()?)))
    else {
        return;
    };
    let segment_size = body.len().div_ceil(segment_count);
    let segment_index = start / segment_size.max(1);
    let current = active.fetch_add(1, Ordering::AcqRel) + 1;
    max_active.fetch_max(current, Ordering::AcqRel);

    if delayed {
        sleep(Duration::from_millis(
            u64::try_from(segment_count.saturating_sub(segment_index) * 20).unwrap_or(0),
        ))
        .await;
    }

    let (status, response_start, response_end, response_total, response_body, content_length) =
        match mode {
            SegmentServerMode::IgnoreRange => (200, 0, 0, 0, body.clone(), body.len()),
            SegmentServerMode::Fail => (500, start, end, body.len(), Vec::new(), 0),
            SegmentServerMode::FailAfterProbe if start == 0 && end == 0 => {
                (206, 0, 0, body.len(), body[..1].to_vec(), 1)
            }
            SegmentServerMode::FailAfterProbe => (500, start, end, body.len(), Vec::new(), 0),
            SegmentServerMode::FailAboveConcurrency(limit) if current > limit => {
                (503, start, end, body.len(), Vec::new(), 0)
            }
            SegmentServerMode::RateLimit => (429, start, end, body.len(), Vec::new(), 0),
            SegmentServerMode::WrongStart => (
                206,
                start.saturating_add(1),
                end,
                body.len(),
                body.get(start..=end).unwrap_or_default().to_vec(),
                end.saturating_sub(start).saturating_add(1),
            ),
            SegmentServerMode::WrongEnd => (
                206,
                start,
                start.saturating_sub(1),
                body.len(),
                Vec::new(),
                0,
            ),
            SegmentServerMode::WrongTotal => (
                206,
                start,
                end,
                body.len().saturating_sub(1),
                body.get(start..=end).unwrap_or_default().to_vec(),
                end.saturating_sub(start).saturating_add(1),
            ),
            SegmentServerMode::ShortBody => (
                206,
                start,
                end,
                body.len(),
                body.get(start..end).unwrap_or_default().to_vec(),
                end.saturating_sub(start).saturating_add(1),
            ),
            SegmentServerMode::FailAboveConcurrency(_)
            | SegmentServerMode::Valid
            | SegmentServerMode::NoValidator => (
                206,
                start,
                end,
                body.len(),
                body.get(start..=end).unwrap_or_default().to_vec(),
                end.saturating_sub(start).saturating_add(1),
            ),
        };

    let validator = if matches!(mode, SegmentServerMode::NoValidator) {
        ""
    } else {
        "ETag: \"test-v1\"\r\n"
    };
    let mut headers = format!(
        "HTTP/1.1 {} Segment\r\nContent-Length: {content_length}\r\n{validator}Connection: close\r\n",
        status
    );
    if status == 206 {
        headers.push_str(&format!(
            "Content-Range: bytes {response_start}-{response_end}/{response_total}\r\n"
        ));
    }
    if matches!(mode, SegmentServerMode::RateLimit) {
        headers.push_str("Retry-After: 0\r\n");
    }
    headers.push_str("\r\n");
    if stream.write_all(headers.as_bytes()).await.is_ok() {
        let chunks = response_body.chunks(16 * 1024);
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            if stream.write_all(chunk).await.is_err() {
                break;
            }
            if delayed && index + 1 < chunk_count {
                sleep(Duration::from_millis(1)).await;
            }
        }
        let _ = stream.flush().await;
    }
    let _ = active.fetch_sub(1, Ordering::AcqRel);
    completion_order.lock().await.push(segment_index);
}

pub(crate) fn scripted_response(
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
) -> ScriptedResponse {
    ScriptedResponse {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.to_vec(),
    }
}

fn range_header(request: &str) -> Option<&str> {
    request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range").then(|| value.trim())
    })
}

async fn write_resume_identity(
    destination: &Path,
    url: &str,
    validator: &str,
) -> Result<(), Box<dyn Error>> {
    let metadata = serde_json::to_vec(&serde_json::json!({
        "url": url,
        "validator": validator,
    }))?;
    tokio::fs::write(
        PathBuf::from(format!("{}.part.meta", destination.display())),
        metadata,
    )
    .await?;
    Ok(())
}

async fn write_resume_identity_without_validator(
    destination: &Path,
    url: &str,
) -> Result<(), Box<dyn Error>> {
    let metadata = serde_json::to_vec(&serde_json::json!({
        "url": url,
        "validator": null,
    }))?;
    tokio::fs::write(
        PathBuf::from(format!("{}.part.meta", destination.display())),
        metadata,
    )
    .await?;
    Ok(())
}

async fn write_finalizing_identity(
    destination: &Path,
    url: &str,
    validator: &str,
    total_size: u64,
) -> Result<(), Box<dyn Error>> {
    let metadata = serde_json::to_vec(&serde_json::json!({
        "url": url,
        "validator": validator,
        "finalizing": true,
        "total_size": total_size,
    }))?;
    tokio::fs::write(
        PathBuf::from(format!("{}.part.meta", destination.display())),
        metadata,
    )
    .await?;
    Ok(())
}

fn segmented_config(minimum_size: u64) -> HttpEngineConfig {
    HttpEngineConfig {
        segment_count: 4,
        minimum_segmented_file_size: minimum_size,
        progress_bytes_interval: 16 * 1024,
        progress_time_interval: Duration::from_millis(50),
        adaptive: AdaptiveConfig {
            enabled: false,
            initial_concurrency: 4,
            max_concurrency: 4,
            ..AdaptiveConfig::default()
        },
        ..HttpEngineConfig::default()
    }
}

fn adaptive_segmented_config(minimum_size: u64) -> HttpEngineConfig {
    HttpEngineConfig {
        segment_count: 8,
        minimum_segmented_file_size: minimum_size,
        progress_bytes_interval: 16 * 1024,
        progress_time_interval: Duration::from_millis(50),
        adaptive: AdaptiveConfig {
            min_concurrency: 1,
            initial_concurrency: 2,
            max_concurrency: 4,
            measurement_window: Duration::from_millis(20),
            improvement_threshold_percent: 10,
            regression_threshold_percent: 20,
            observations_to_increase: 2,
            cooldown_windows: 1,
            failures_to_decrease: 2,
            ..AdaptiveConfig::default()
        },
        ..HttpEngineConfig::default()
    }
}

/// Starts a minimal in-process FTP server that serves a single file at any
/// path, supporting the subset of FTP commands that `FtpEngine` uses:
/// `USER`, `PASS`, `TYPE`, `PASV`, `SIZE`, `REST`, `RETR`, `QUIT`.
///
/// Returns the bound `SocketAddr` (for building the `ftp://` URL) and a
/// `JoinHandle` that resolves when the server has handled one connection.
pub(crate) async fn start_ftp_server(
    content: Vec<u8>,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        let Ok((mut ctrl, _)) = listener.accept().await else {
            return;
        };

        // 220 Welcome
        if ctrl
            .write_all(b"220 FetchX-test FTP ready\r\n")
            .await
            .is_err()
        {
            return;
        }

        let mut rest_offset: u64 = 0;
        let mut pasv_addr: Option<std::net::SocketAddr> = None;

        let mut buf = vec![0u8; 4096];
        loop {
            let n = match ctrl.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            let cmd = String::from_utf8_lossy(&buf[..n])
                .trim()
                .to_ascii_uppercase();

            if cmd.starts_with("USER") {
                let _ = ctrl.write_all(b"331 Password required\r\n").await;
            } else if cmd.starts_with("PASS") {
                let _ = ctrl.write_all(b"230 Logged in\r\n").await;
            } else if cmd.starts_with("TYPE") {
                let _ = ctrl.write_all(b"200 Type set\r\n").await;
            } else if cmd.starts_with("SIZE") {
                let _ = ctrl
                    .write_all(format!("213 {}\r\n", content.len()).as_bytes())
                    .await;
            } else if cmd.starts_with("REST") {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                rest_offset = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let _ = ctrl.write_all(b"350 Rest offset accepted\r\n").await;
            } else if cmd.starts_with("PASV") {
                // Open a data listener on a random port.
                let data_listener = match TcpListener::bind("127.0.0.1:0").await {
                    Ok(l) => l,
                    Err(_) => {
                        let _ = ctrl.write_all(b"425 Can't open data connection\r\n").await;
                        continue;
                    }
                };
                let data_addr = match data_listener.local_addr() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                pasv_addr = Some(data_addr);
                // PASV response encodes addr as h1,h2,h3,h4,p1,p2
                let port = data_addr.port();
                let p1 = port >> 8;
                let p2 = port & 0xFF;
                let response = format!("227 Entering Passive Mode (127,0,0,1,{p1},{p2})\r\n");
                let _ = ctrl.write_all(response.as_bytes()).await;

                // Accept the data connection in a background task.
                let content_clone = content.clone();
                let rest = rest_offset;
                tokio::spawn(async move {
                    let Ok((mut data_stream, _)) = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        data_listener.accept(),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.ok()) // flatten Result
                    .map(Ok::<_, std::io::Error>)
                    .unwrap_or_else(|| Err(std::io::Error::other("accept timeout")))
                    .map(Ok::<_, std::io::Error>)
                    .unwrap_or_else(|_| Err(std::io::Error::other("error")))
                    .map(Ok)
                    .unwrap_or_else(|_e: std::io::Error| Err(())) else {
                        return;
                    };
                    let slice = if rest as usize <= content_clone.len() {
                        &content_clone[rest as usize..]
                    } else {
                        &[]
                    };
                    let _ = data_stream.write_all(slice).await;
                    let _ = data_stream.flush().await;
                });
            } else if cmd.starts_with("RETR") {
                let _ = ctrl.write_all(b"150 Opening data connection\r\n").await;
                // Wait a moment for the data to be sent by the PASV task.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let _ = ctrl.write_all(b"226 Transfer complete\r\n").await;
                let _ = pasv_addr.take(); // Reset for next RETR
            } else if cmd.starts_with("QUIT") {
                let _ = ctrl.write_all(b"221 Goodbye\r\n").await;
                return;
            } else {
                let _ = ctrl.write_all(b"502 Command not implemented\r\n").await;
            }
        }
    });

    Ok((addr, handle))
}

struct FtpsServer {
    address: std::net::SocketAddr,
    certificate: Vec<u8>,
    task: JoinHandle<()>,
    data_ready: oneshot::Receiver<()>,
}

async fn read_ftp_line<S>(stream: &mut S) -> Result<String, std::io::Error>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            line.push(byte[0]);
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

async fn start_ftps_server(
    content: Vec<u8>,
    advertised_size: Option<u64>,
    username: &'static str,
    password: &'static str,
    handshake_delay: Duration,
    data_delay: Duration,
) -> Result<FtpsServer, Box<dyn Error>> {
    use std::{net::IpAddr, sync::Arc};

    use rcgen::{CertificateParams, KeyPair, SanType};
    use tokio_rustls::{
        TlsAcceptor,
        rustls::{
            ServerConfig,
            pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        },
    };

    let mut certificate_params = CertificateParams::new(Vec::new())?;
    certificate_params.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(
        std::net::Ipv4Addr::LOCALHOST,
    ))];
    let key_pair = KeyPair::generate()?;
    let certificate = certificate_params.self_signed(&key_pair)?;
    let certificate_der = certificate.der().to_vec();
    let tls_config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(certificate_der.clone())],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der())),
    )?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (data_ready_sender, data_ready) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut data_ready_tx = Some(data_ready_sender);
        let Ok((mut control, _)) = listener.accept().await else {
            return;
        };
        if control
            .write_all(b"220 FetchX hermetic FTPS fixture\r\n")
            .await
            .is_err()
        {
            return;
        }
        if !matches!(
            read_ftp_line(&mut control).await,
            Ok(command) if command.eq_ignore_ascii_case("AUTH TLS")
        ) {
            return;
        }
        if control
            .write_all(b"234 Proceed with TLS\r\n")
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(handshake_delay).await;
        let Ok(mut control) = tls_acceptor.accept(control).await else {
            return;
        };

        let mut rest_offset = 0_usize;
        let mut data_listener = None;
        loop {
            let Ok(command) = read_ftp_line(&mut control).await else {
                return;
            };
            let mut fields = command.split_whitespace();
            let verb = fields.next().unwrap_or_default().to_ascii_uppercase();
            let argument = fields.next().unwrap_or_default();
            match verb.as_str() {
                "PBSZ" | "PROT" | "TYPE" => {
                    if control
                        .write_all(b"200 Command accepted\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                "USER" => {
                    let response = if argument == username {
                        b"331 Password required\r\n".as_slice()
                    } else {
                        b"530 Login incorrect\r\n".as_slice()
                    };
                    if control.write_all(response).await.is_err() {
                        return;
                    }
                }
                "PASS" => {
                    let response = if argument == password {
                        b"230 Logged in\r\n".as_slice()
                    } else {
                        b"530 Login incorrect\r\n".as_slice()
                    };
                    if control.write_all(response).await.is_err() {
                        return;
                    }
                }
                "SIZE" => {
                    let size = advertised_size.unwrap_or(content.len() as u64);
                    if control
                        .write_all(format!("213 {size}\r\n").as_bytes())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                "REST" => {
                    rest_offset = argument.parse().unwrap_or(0);
                    if control
                        .write_all(b"350 Restart accepted\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                "PASV" => {
                    let Ok(listener) = TcpListener::bind("127.0.0.1:0").await else {
                        let _ = control.write_all(b"425 Data listener failed\r\n").await;
                        continue;
                    };
                    let Ok(data_address) = listener.local_addr() else {
                        return;
                    };
                    let port = data_address.port();
                    let response = format!(
                        "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                        port >> 8,
                        port & 0xff
                    );
                    if control.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                    data_listener = Some(listener);
                }
                "RETR" => {
                    if control
                        .write_all(b"150 Opening protected data connection\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let Some(data_listener) = data_listener.take() else {
                        let _ = control.write_all(b"425 No passive listener\r\n").await;
                        continue;
                    };
                    let Ok((data_stream, _)) = data_listener.accept().await else {
                        return;
                    };
                    let Ok(mut data_stream) = tls_acceptor.accept(data_stream).await else {
                        return;
                    };
                    if let Some(data_ready_tx) = data_ready_tx.take() {
                        let _ = data_ready_tx.send(());
                    }
                    tokio::time::sleep(data_delay).await;
                    let bytes = content.get(rest_offset..).unwrap_or_default();
                    if data_stream.write_all(bytes).await.is_err()
                        || data_stream.shutdown().await.is_err()
                        || control
                            .write_all(b"226 Transfer complete\r\n")
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                "QUIT" => {
                    let _ = control.write_all(b"221 Goodbye\r\n").await;
                    return;
                }
                _ => {
                    if control
                        .write_all(b"502 Command not implemented\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    });

    Ok(FtpsServer {
        address,
        certificate: certificate_der,
        task,
        data_ready,
    })
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
        DirectHttpEngine<fetchx_storage::SqliteDownloadRepository>,
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
    engine: &DirectHttpEngine<fetchx_storage::SqliteDownloadRepository>,
    database: &Database,
    item: DownloadItem,
) -> Result<DownloadItem, Box<dyn Error>> {
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);
    Ok(engine.download(id, receiver).await?)
}

#[tokio::test]
async fn managed_direct_http_download_observes_shared_bandwidth_control()
-> Result<(), Box<dyn Error>> {
    let body = vec![0x5a; 64 * 1024];
    let (url, server) = start_scripted_server(vec![scripted_response(200, &[], &body)]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("bandwidth-limited.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;

    let (_cancel_sender, cancellation) = watch::channel(false);
    let (_pause_sender, pause) = watch::channel(false);
    let (_limit_sender, connection_limit) = watch::channel(1);
    let control = ManagedDownloadControl {
        cancellation,
        pause,
        connection_limit,
        resource_demand: None,
        bandwidth: BandwidthControl {
            global: Some(Arc::new(BandwidthLimiter::new(32 * 1024)?)),
            per_download: Some(Arc::new(BandwidthLimiter::new(32 * 1024)?)),
            ..BandwidthControl::default()
        },
    };
    let start = tokio::time::Instant::now();
    let completed = engine.download_managed(id, control).await?;
    assert!(start.elapsed() >= Duration::from_millis(800));
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(server.await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn segmented_download_uses_four_ranges_and_assembles_exact_content()
-> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 1024 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 251) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 5, true).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("segmented.bin");
    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let report = server.await?;

    assert_eq!(report.requests.len(), 5);
    assert_eq!(range_header(&report.requests[0]), None);
    let ranges = report.requests[1..]
        .iter()
        .filter_map(|request| range_header(request))
        .collect::<Vec<_>>();
    for expected in [
        "bytes=0-262143",
        "bytes=262144-524287",
        "bytes=524288-786431",
        "bytes=786432-1048575",
    ] {
        assert!(
            ranges.contains(&expected),
            "missing request range {expected}"
        );
    }
    assert!(report.max_active_segments > 1);
    assert!(
        report.max_active_segments <= 4,
        "observed {} active ranged requests",
        report.max_active_segments
    );
    assert_ne!(report.completion_order.first(), Some(&0));
    assert_eq!(tokio::fs::read(&destination).await?, body);
    assert_eq!(tokio::fs::metadata(destination).await?.len(), TOTAL as u64);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(completed.progress().downloaded_bytes(), TOTAL as u64);
    let part_path = PathBuf::from(format!(
        "{}.part",
        directory.destination("segmented.bin").display()
    ));
    for index in 0..4 {
        assert!(
            !PathBuf::from(format!(
                "{}.segments-4-{TOTAL}-{index}",
                part_path.display()
            ))
            .exists()
        );
    }
    Ok(())
}

#[tokio::test]
async fn segmented_download_shares_global_bandwidth_across_workers() -> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 128 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 251) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 5, true).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("segmented-bandwidth.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_cancel_sender, cancellation) = watch::channel(false);
    let (_pause_sender, pause) = watch::channel(false);
    let (_limit_sender, connection_limit) = watch::channel(4);
    let control = ManagedDownloadControl {
        cancellation,
        pause,
        connection_limit,
        resource_demand: None,
        bandwidth: BandwidthControl {
            global: Some(Arc::new(BandwidthLimiter::new(64 * 1024)?)),
            per_download: Some(Arc::new(BandwidthLimiter::new(64 * 1024)?)),
            ..BandwidthControl::default()
        },
    };

    let start = tokio::time::Instant::now();
    let completed = engine.download_managed(id, control).await?;
    let report = server.await?;
    assert!(start.elapsed() >= Duration::from_millis(800));
    assert!(report.max_active_segments > 1);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    Ok(())
}

#[tokio::test]
async fn adaptive_scheduler_completes_within_worker_bounds_and_assembles_all_ranges()
-> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 512 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 251) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 8, SegmentServerMode::Valid, 20, true).await?;
    let (database, engine, directory) = setup_engine(adaptive_segmented_config(1024)).await?;
    let destination = directory.destination("adaptive-segments.bin");

    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let report = server.await?;
    let ranges = report
        .requests
        .iter()
        .filter_map(|request| range_header(request))
        .collect::<Vec<_>>();

    assert_eq!(ranges.len(), 8);
    assert!(report.max_active_segments >= 2);
    assert!(
        report.max_active_segments <= 4,
        "observed {} active ranged requests",
        report.max_active_segments
    );
    assert_ne!(report.completion_order.first(), Some(&0));
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(completed.progress().downloaded_bytes(), TOTAL as u64);
    Ok(())
}

#[tokio::test]
async fn adaptive_scheduler_backs_off_when_server_rejects_excess_concurrency()
-> Result<(), Box<dyn Error>> {
    // Use a smaller body (64 KB) and a generous connection budget (4096) so
    // this test doesn't exhaust the server on slow CI runners regardless of
    // how many 503-triggered retries the adaptive controller issues.
    const TOTAL: usize = 64 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 239) as u8).collect();
    let (url, server) = start_segment_server(
        body.clone(),
        8,
        SegmentServerMode::FailAboveConcurrency(1),
        4096,
        true,
    )
    .await?;
    let mut config = adaptive_segmented_config(1024);
    config.adaptive.initial_concurrency = 2;
    config.adaptive.max_concurrency = 2;
    let (database, engine, directory) = setup_engine(config).await?;
    let destination = directory.destination("adaptive-backoff.bin");

    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let report = server.await?;
    let ranges = report
        .requests
        .iter()
        .filter_map(|request| range_header(request))
        .collect::<Vec<_>>();

    assert!(report.max_active_segments <= 3);
    assert!(ranges.len() > 8);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn extreme_segmented_profile_respects_configured_thirty_two_connection_ceiling()
-> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 1024 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 251) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 32, SegmentServerMode::Valid, 64, true).await?;
    let mut config = adaptive_segmented_config(1024);
    config.segment_count = 32;
    config.adaptive.min_concurrency = 8;
    config.adaptive.initial_concurrency = 32;
    config.adaptive.max_concurrency = 32;
    config.adaptive.enabled = false;
    let (database, engine, directory) = setup_engine(config).await?;
    let destination = directory.destination("extreme-profile.bin");

    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let report = server.await?;
    let ranges = report
        .requests
        .iter()
        .filter_map(|request| range_header(request))
        .collect::<Vec<_>>();

    assert_eq!(ranges.len(), 32);
    assert!(report.max_active_segments > 8);
    assert!(report.max_active_segments <= 32);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn rate_limited_segments_retry_with_a_bounded_attempt_count() -> Result<(), Box<dyn Error>> {
    let body = vec![31_u8; 256 * 1024];
    let (url, server) =
        start_segment_server(body, 4, SegmentServerMode::RateLimit, 64, false).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("rate-limited.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    let result = engine.download(id, receiver).await;
    assert!(
        matches!(result, Err(DownloadEngineError::HttpStatus(429))),
        "{result:?}"
    );
    let report = server.await?;
    let range_requests = report
        .requests
        .iter()
        .filter_map(|request| range_header(request))
        .collect::<Vec<_>>();
    let unique_ranges = range_requests
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let persisted = database
        .downloads()
        .get(id)
        .await?
        .ok_or("rate-limited download missing")?;

    assert!(range_requests.len() > 4);
    assert!(range_requests.len() <= 16);
    assert!(unique_ranges.len() <= 4);
    assert!(!destination.exists());
    assert_eq!(persisted.status(), DownloadStatus::Failed);
    Ok(())
}

#[tokio::test]
async fn cancellation_during_adaptive_transfer_preserves_partial_segments()
-> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 8 * 1024 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 223) as u8).collect();
    let (url, server) = start_segment_server(body, 8, SegmentServerMode::Valid, 20, true).await?;
    let (database, engine, directory) = setup_engine(adaptive_segmented_config(1024)).await?;
    let destination = directory.destination("adaptive-cancel.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (sender, receiver) = watch::channel(false);
    let engine_task = tokio::spawn(async move { engine.download(id, receiver).await });

    let mut observed_progress = false;
    for _ in 0..300 {
        let current = database.downloads().get(id).await?;
        if current.is_some_and(|item| item.progress().downloaded_bytes() > 0) {
            observed_progress = true;
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }
    assert!(observed_progress);
    sleep(Duration::from_millis(150)).await;
    sender.send(true)?;
    assert!(matches!(
        engine_task.await?,
        Err(DownloadEngineError::Cancelled)
    ));
    let report = server.await?;
    let current = database
        .downloads()
        .get(id)
        .await?
        .ok_or("cancelled adaptive download missing")?;
    let mut partial_bytes = 0_u64;
    for index in 0..8 {
        let path = PathBuf::from(format!(
            "{}.part.segments-8-{TOTAL}-{index}",
            destination.display()
        ));
        match tokio::fs::metadata(path).await {
            Ok(metadata) => partial_bytes += metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    assert_eq!(current.status(), DownloadStatus::Cancelled);
    assert!(report.max_active_segments >= 2);
    assert!(report.max_active_segments <= 4);
    assert!(partial_bytes > 0 && partial_bytes < TOTAL as u64);
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn invalid_adaptive_configuration_is_rejected_by_engine() -> Result<(), Box<dyn Error>> {
    let database = Database::connect("sqlite::memory:").await?;
    let config = HttpEngineConfig {
        adaptive: AdaptiveConfig {
            max_concurrency: 65,
            ..AdaptiveConfig::default()
        },
        ..HttpEngineConfig::default()
    };

    assert!(matches!(
        DirectHttpEngine::with_config(database.downloads(), config),
        Err(DownloadEngineError::InvalidConfiguration)
    ));
    Ok(())
}

#[tokio::test]
async fn invalid_segment_ranges_fall_back_to_a_fresh_single_stream() -> Result<(), Box<dyn Error>> {
    let body: Vec<u8> = (0..256 * 1024).map(|index| (index % 239) as u8).collect();
    for mode in [
        SegmentServerMode::WrongStart,
        SegmentServerMode::WrongEnd,
        SegmentServerMode::WrongTotal,
    ] {
        let (url, server) = start_segment_server(body.clone(), 4, mode, 6, false).await?;
        let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
        let destination = directory.destination("invalid-range.bin");
        let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
        let report = server.await?;

        let range_requests = report
            .requests
            .iter()
            .filter(|request| range_header(request).is_some())
            .count();
        assert!((1..=4).contains(&range_requests));
        assert_eq!(
            report
                .requests
                .iter()
                .filter(|request| range_header(request).is_none())
                .count(),
            2
        );
        assert_eq!(report.requests.len(), range_requests + 2);
        assert_eq!(tokio::fs::read(destination).await?, body);
        assert_eq!(completed.status(), DownloadStatus::Completed);
    }
    Ok(())
}

#[tokio::test]
async fn ignored_segment_ranges_fall_back_without_mixing_segment_bodies()
-> Result<(), Box<dyn Error>> {
    let body: Vec<u8> = (0..256 * 1024).map(|index| (index % 233) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::IgnoreRange, 6, false).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("ignored-segment-ranges.bin");
    run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let report = server.await?;

    let range_requests = report
        .requests
        .iter()
        .filter(|request| range_header(request).is_some())
        .count();
    assert!((1..=4).contains(&range_requests));
    assert_eq!(
        report
            .requests
            .iter()
            .filter(|request| range_header(request).is_none())
            .count(),
        2
    );
    assert_eq!(report.requests.len(), range_requests + 2);
    assert_eq!(tokio::fs::read(destination).await?, body);
    Ok(())
}

#[tokio::test]
async fn short_segment_body_fails_without_publishing_destination() -> Result<(), Box<dyn Error>> {
    let body: Vec<u8> = (0..256 * 1024).map(|index| (index % 229) as u8).collect();
    let (url, server) =
        start_segment_server(body, 4, SegmentServerMode::ShortBody, 20, false).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("short-segment.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    let result = engine.download(id, receiver).await;
    assert!(
        matches!(
            result,
            Err(DownloadEngineError::SegmentFailed { .. })
                | Err(DownloadEngineError::IncompleteResponse { .. })
        ),
        "{result:?}"
    );
    let report = server.await?;
    assert!(report.requests.len() >= 2);
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn failed_segment_does_not_publish_any_destination() -> Result<(), Box<dyn Error>> {
    let body: Vec<u8> = (0..256 * 1024).map(|index| (index % 227) as u8).collect();
    let (url, server) = start_segment_server(body, 4, SegmentServerMode::Fail, 20, false).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("failed-segment.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    let result = engine.download(id, receiver).await;
    assert!(
        matches!(result, Err(DownloadEngineError::SegmentFailed { .. })),
        "{result:?}"
    );
    let report = server.await?;
    assert!(report.requests.len() >= 2);
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn resumed_segments_without_a_server_validator_are_restarted() -> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 256 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 239) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::NoValidator, 5, false).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("unverified-segment-resume.bin");
    let temporary_path = PathBuf::from(format!("{}.part", destination.display()));

    for index in 0..4 {
        let segment_path = PathBuf::from(format!(
            "{}.segments-4-{TOTAL}-{index}",
            temporary_path.display()
        ));
        tokio::fs::write(segment_path, vec![0x11; TOTAL / 4]).await?;
    }
    write_resume_identity_without_validator(&destination, &url).await?;

    let source_item = make_item(&url, &destination)?;
    let item = DownloadItem::from_parts(
        source_item.id(),
        source_item.source().clone(),
        source_item.filename().clone(),
        source_item.destination().to_path_buf(),
        DownloadStatus::Cancelled,
        source_item.priority(),
        DownloadProgress::new(TOTAL as u64, Some(TOTAL as u64), None, None)?,
        source_item.created_at(),
        Timestamp::from_unix_millis(2),
        None,
    )?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    let completed = engine.download(id, receiver).await?;
    let report = server.await?;
    let mut ranges = report
        .requests
        .iter()
        .filter_map(|request| range_header(request))
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    let mut expected_ranges = vec![
        "bytes=0-65535",
        "bytes=65536-131071",
        "bytes=131072-196607",
        "bytes=196608-262143",
    ];
    expected_ranges.sort_unstable();

    assert_eq!(ranges, expected_ranges);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn cancellation_keeps_segment_bytes_and_later_invocation_resumes_them()
-> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 8 * 1024 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 223) as u8).collect();
    let (url, server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 5, true).await?;
    let config = HttpEngineConfig {
        progress_bytes_interval: 16 * 1024,
        progress_time_interval: Duration::from_secs(60),
        ..segmented_config(1024)
    };
    let (database, engine, directory) = setup_engine(config).await?;
    let destination = directory.destination("cancelled-segments.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (sender, receiver) = watch::channel(false);
    let engine_task = tokio::spawn(async move { engine.download(id, receiver).await });

    let mut observed_downloaded_bytes = false;
    for _ in 0..300 {
        let current = database.downloads().get(id).await?;
        if current.is_some_and(|item| item.progress().downloaded_bytes() > 0) {
            observed_downloaded_bytes = true;
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }
    assert!(observed_downloaded_bytes);
    sender.send(true)?;
    assert!(matches!(
        engine_task.await?,
        Err(DownloadEngineError::Cancelled)
    ));
    let first_report = server.await?;
    assert_eq!(first_report.requests.len(), 5);
    assert!(!destination.exists());

    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    let mut preserved_bytes = 0_u64;
    let mut incomplete_segments = 0_usize;
    for index in 0..4 {
        let path = PathBuf::from(format!(
            "{}.segments-4-{TOTAL}-{index}",
            partial_path.display()
        ));
        if tokio::fs::try_exists(&path).await? {
            let size = tokio::fs::metadata(path).await?.len();
            preserved_bytes += size;
            if size < (TOTAL / 4) as u64 {
                incomplete_segments += 1;
            }
        } else {
            incomplete_segments += 1;
        }
    }
    assert!(preserved_bytes > 0 && preserved_bytes < TOTAL as u64);

    let (resume_url, resume_server) = start_segment_server(
        body.clone(),
        4,
        SegmentServerMode::Valid,
        incomplete_segments + 1,
        false,
    )
    .await?;
    write_resume_identity(&destination, &resume_url, "\"test-v1\"").await?;
    let cancelled = database
        .downloads()
        .get(id)
        .await?
        .ok_or("cancelled segmented download missing")?;
    let resumed_item = DownloadItem::from_parts(
        cancelled.id(),
        DownloadSource::new(resume_url, EngineKind::DirectHttp)?,
        cancelled.filename().clone(),
        cancelled.destination().to_path_buf(),
        DownloadStatus::Cancelled,
        cancelled.priority(),
        cancelled.progress().clone(),
        cancelled.created_at(),
        Timestamp::from_unix_millis(cancelled.updated_at().as_unix_millis() + 1),
        None,
    )?;
    database.downloads().update(&resumed_item).await?;
    let resume_engine = DirectHttpEngine::new(database.downloads())?;
    let (_sender, receiver) = watch::channel(false);
    let completed = resume_engine.download(id, receiver).await?;
    let resume_report = resume_server.await?;

    assert_eq!(resume_report.requests.len(), incomplete_segments + 1);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn files_below_threshold_use_one_non_range_request() -> Result<(), Box<dyn Error>> {
    let body = b"small file".to_vec();
    let (url, server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 1, false).await?;
    let (database, engine, directory) = setup_engine(segmented_config(1024)).await?;
    let destination = directory.destination("small-not-segmented.bin");
    run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let report = server.await?;

    assert_eq!(report.requests.len(), 1);
    assert_eq!(range_header(&report.requests[0]), None);
    assert_eq!(tokio::fs::read(destination).await?, body);
    Ok(())
}

#[tokio::test]
async fn resumes_using_actual_partial_size_and_validates_final_content()
-> Result<(), Box<dyn Error>> {
    const TOTAL: usize = 10 * 1024 * 1024;
    const PARTIAL: usize = 4 * 1024 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 251) as u8).collect();
    let remaining = &body[PARTIAL..];
    let response = scripted_response(
        206,
        &[
            (
                "Content-Range",
                &format!("bytes {PARTIAL}-{}/{}", TOTAL - 1, TOTAL),
            ),
            ("ETag", "\"test-v1\""),
        ],
        remaining,
    );
    let (url, server) = start_scripted_server(vec![response]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("resumed.bin");
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    tokio::fs::write(&partial_path, &body[..PARTIAL]).await?;
    write_resume_identity(&destination, &url, "\"test-v1\"").await?;

    let mut item = make_item(&url, &destination)?;
    item.set_progress_at(
        DownloadProgress::new(1024, None, None, None)?,
        Timestamp::from_unix_millis(2),
    )?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);
    let completed = engine.download(id, receiver).await?;
    let requests = server.await?;

    assert_eq!(range_header(&requests[0]), Some("bytes=4194304-"));
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(completed.progress().downloaded_bytes(), TOTAL as u64);
    assert_eq!(completed.progress().total_bytes(), Some(TOTAL as u64));
    Ok(())
}

#[tokio::test]
async fn reopens_file_database_and_recovers_interrupted_http_download() -> Result<(), Box<dyn Error>>
{
    const TOTAL: usize = 2 * 1024 * 1024;
    const PARTIAL: usize = 768 * 1024;
    let body: Vec<u8> = (0..TOTAL).map(|index| (index % 251) as u8).collect();
    let remaining = &body[PARTIAL..];
    let response = scripted_response(
        206,
        &[
            (
                "Content-Range",
                &format!("bytes {PARTIAL}-{}/{TOTAL}", TOTAL - 1),
            ),
            ("ETag", "\"crash-recovery-v1\""),
        ],
        remaining,
    );
    let (url, server) = start_scripted_server(vec![response]).await?;
    let directory = TestDirectory::new()?;
    let database_path = directory.destination("recovery.sqlite");
    let destination = directory.destination("recovered.bin");
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));

    let database = Database::connect_path(&database_path).await?;
    let repository = database.downloads();
    let mut item = make_item(&url, &destination)?;
    item.set_progress_at(
        DownloadProgress::new(PARTIAL as u64, Some(TOTAL as u64), None, None)?,
        Timestamp::from_unix_millis(2),
    )?;
    let id = item.id();
    repository.create(&item).await?;
    let state_machine = fetchx_core::DownloadStateMachine::new(repository.clone());
    state_machine
        .transition(id, DownloadStatus::Preparing)
        .await?;
    state_machine
        .transition(id, DownloadStatus::Downloading)
        .await?;
    tokio::fs::write(&partial_path, &body[..PARTIAL]).await?;
    write_resume_identity(&destination, &url, "\"crash-recovery-v1\"").await?;

    drop(database);
    drop(state_machine);
    drop(repository);

    let reopened_database = Database::connect_path(&database_path).await?;
    let repository = reopened_database.downloads();
    let state_machine = fetchx_core::DownloadStateMachine::new(repository.clone());
    let recovered = state_machine
        .recover_interrupted(id)
        .await?
        .ok_or("downloading item was not recovered")?;
    assert_eq!(recovered.status(), DownloadStatus::Queued);
    assert_eq!(recovered.progress().downloaded_bytes(), PARTIAL as u64);

    let engine = DirectHttpEngine::new(repository.clone())?;
    let (_sender, cancellation) = watch::channel(false);
    let completed = engine.download(id, cancellation).await?;
    let requests = server.await?;
    assert_eq!(range_header(&requests[0]), Some("bytes=786432-"));
    assert_eq!(completed.status(), DownloadStatus::Completed);
    let actual_body = tokio::fs::read(&destination).await?;
    assert_eq!(actual_body, body);
    let actual_hash = hex::encode(sha2::Sha256::digest(&actual_body));
    let expected_hash = hex::encode(sha2::Sha256::digest(&body));
    assert_eq!(actual_hash, expected_hash);
    assert_eq!(
        repository
            .get(id)
            .await?
            .ok_or("recovered download disappeared from persistence")?
            .status(),
        DownloadStatus::Completed
    );

    drop(engine);
    drop(state_machine);
    drop(repository);
    drop(reopened_database);
    Ok(())
}

#[tokio::test]
async fn missing_partial_file_is_not_resumed_and_download_restarts_from_zero()
-> Result<(), Box<dyn Error>> {
    let body = b"fresh content after partial loss".to_vec();
    let (url, server) = start_scripted_server(vec![scripted_response(200, &[], &body)]).await?;
    let config = HttpEngineConfig {
        minimum_segmented_file_size: u64::MAX,
        ..HttpEngineConfig::default()
    };
    let (database, engine, directory) = setup_engine(config).await?;
    let destination = directory.destination("missing-partial.bin");
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let state_machine = fetchx_core::DownloadStateMachine::new(database.downloads());
    state_machine
        .transition(id, DownloadStatus::Preparing)
        .await?;
    state_machine
        .transition(id, DownloadStatus::Downloading)
        .await?;
    tokio::fs::write(&partial_path, b"stale partial bytes").await?;
    write_resume_identity(&destination, &url, "\"stale-v1\"").await?;
    tokio::fs::remove_file(&partial_path).await?;

    let recovered = state_machine
        .recover_interrupted(id)
        .await?
        .ok_or("downloading item was not recovered")?;
    assert_eq!(recovered.status(), DownloadStatus::Queued);

    let (_sender, cancellation) = watch::channel(false);
    let completed = engine.download(id, cancellation).await?;
    let requests = server.await?;
    assert_eq!(requests.len(), 1);
    assert_eq!(range_header(&requests[0]), None);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(
        database.downloads().get(id).await?.unwrap().status(),
        DownloadStatus::Completed
    );
    Ok(())
}

#[tokio::test]
async fn abrupt_subprocess_termination_recovers_and_completes_http_download()
-> Result<(), Box<dyn Error>> {
    const CHILD_MODE: &str = "FETCHX_PROCESS_RECOVERY_CHILD";
    const DATABASE_PATH: &str = "FETCHX_PROCESS_RECOVERY_DATABASE";
    const DOWNLOAD_ID: &str = "FETCHX_PROCESS_RECOVERY_DOWNLOAD";

    if std::env::var_os(CHILD_MODE).is_some() {
        let database_path = std::env::var_os(DATABASE_PATH).ok_or("missing child database path")?;
        let id = std::env::var(DOWNLOAD_ID)?.parse::<DownloadId>()?;
        let database = Database::connect_path(PathBuf::from(database_path)).await?;
        let engine = DirectHttpEngine::new(database.downloads())?;
        let (_sender, cancellation) = watch::channel(false);
        let _ = engine.download(id, cancellation).await;
        return Ok(());
    }

    let directory = TestDirectory::new()?;
    let database_path = directory.destination("process-recovery.sqlite");
    let destination = directory.destination("process-recovery.bin");
    let body: Vec<u8> = (0..2 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let expected_hash = hex::encode(sha2::Sha256::digest(&body));
    let (url, server, stop_server) = start_process_recovery_server(body.clone()).await?;

    let database = Database::connect_path(&database_path).await?;
    let repository = database.downloads();
    let item = make_item(&url, &destination)?;
    let id = item.id();
    repository.create(&item).await?;

    let executable = std::env::current_exe()?;
    let mut child = Command::new(executable)
        .arg("--exact")
        .arg("tests::abrupt_subprocess_termination_recovers_and_completes_http_download")
        .arg("--nocapture")
        .env(CHILD_MODE, "1")
        .env(DATABASE_PATH, &database_path)
        .env(DOWNLOAD_ID, id.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    let (interrupted_item, partial_size) = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(status) = child.try_wait()? {
                return Err::<_, Box<dyn Error>>(
                    std::io::Error::other(format!(
                        "download subprocess exited before interruption: {status}"
                    ))
                    .into(),
                );
            }
            let current = repository.get(id).await?;
            if current
                .as_ref()
                .is_some_and(|item| item.status() == DownloadStatus::Completed)
            {
                return Err::<_, Box<dyn Error>>(
                    std::io::Error::other("download completed before it could be interrupted")
                        .into(),
                );
            }
            if let Some(current) =
                current.filter(|item| item.status() == DownloadStatus::Downloading)
                && let Ok(metadata) = tokio::fs::metadata(&partial_path).await
                && metadata.len() > 0
            {
                return Ok::<_, Box<dyn Error>>((current, metadata.len()));
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    assert_eq!(interrupted_item.status(), DownloadStatus::Downloading);
    assert!(partial_size > 0 && partial_size < body.len() as u64);

    child.kill().await?;
    let child_status = child.wait().await?;
    assert!(
        !child_status.success(),
        "subprocess termination must be abrupt"
    );
    assert_eq!(
        repository.get(id).await?.unwrap().status(),
        DownloadStatus::Downloading,
        "the database must retain the interrupted active state"
    );
    let persisted_partial_size = tokio::fs::metadata(&partial_path).await?.len();
    assert!(
        persisted_partial_size >= partial_size && persisted_partial_size < body.len() as u64,
        "the interrupted partial file must retain at least the observed bytes and remain incomplete"
    );

    let state_machine = fetchx_core::DownloadStateMachine::new(repository.clone());
    let recovered = state_machine
        .recover_interrupted(id)
        .await?
        .ok_or("interrupted download was not requeued")?;
    assert_eq!(recovered.status(), DownloadStatus::Queued);

    let engine = DirectHttpEngine::new(repository.clone())?;
    let (_sender, cancellation) = watch::channel(false);
    let completed = engine.download(id, cancellation).await?;
    assert_eq!(completed.status(), DownloadStatus::Completed);
    let _ = stop_server.send(());
    let requests = server.await?;
    assert_eq!(requests.len(), 2);
    let expected_range = format!("bytes={persisted_partial_size}-");
    assert_eq!(range_header(&requests[1]), Some(expected_range.as_str()));
    let actual_body = tokio::fs::read(&destination).await?;
    assert_eq!(actual_body, body);
    assert_eq!(
        hex::encode(sha2::Sha256::digest(&actual_body)),
        expected_hash
    );
    assert_eq!(
        repository.get(id).await?.unwrap().status(),
        DownloadStatus::Completed
    );
    Ok(())
}

#[tokio::test]
async fn injected_write_failures_persist_failed_without_retry_or_publication()
-> Result<(), Box<dyn Error>> {
    for kind in [
        std::io::ErrorKind::Other,
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::StorageFull,
    ] {
        let body = b"write fault payload";
        let (url, server) = start_scripted_server(vec![scripted_response(200, &[], body)]).await?;
        let config = HttpEngineConfig {
            minimum_segmented_file_size: u64::MAX,
            ..HttpEngineConfig::default()
        };
        let (database, engine, directory) = setup_engine(config).await?;
        let destination = directory.destination("write-failure.bin");
        let item = make_item(&url, &destination)?;
        let id = item.id();
        database.downloads().create(&item).await?;
        crate::http::inject_next_write_failure(id, kind);
        let (_sender, cancellation) = watch::channel(false);

        assert!(matches!(
            engine.download(id, cancellation).await,
            Err(DownloadEngineError::Io(error)) if error.kind() == kind
        ));
        assert_eq!(server.await?.len(), 1, "write errors are not retried");
        assert!(
            !destination.exists(),
            "failed payload must not be published"
        );
        let partial_path = PathBuf::from(format!("{}.part", destination.display()));
        assert!(partial_path.exists(), "partial data remains recoverable");
        let persisted = database
            .downloads()
            .get(id)
            .await?
            .ok_or("failed download not found")?;
        assert_eq!(persisted.status(), DownloadStatus::Failed);
        assert!(persisted.error().is_some());
    }
    Ok(())
}

#[tokio::test]
async fn invalid_partial_responses_never_append_or_publish() -> Result<(), Box<dyn Error>> {
    let config = HttpEngineConfig {
        progress_bytes_interval: 1,
        progress_time_interval: Duration::from_secs(60),
        ..HttpEngineConfig::default()
    };
    let (database, engine, directory) = setup_engine(config).await?;
    for (name, headers, expected_error) in [
        (
            "wrong-start.bin",
            vec![("Content-Range", "bytes 0-4/10")],
            "range",
        ),
        ("missing-range.bin", vec![], "content-range"),
        (
            "malformed-range.bin",
            vec![("Content-Range", "not a range")],
            "content-range",
        ),
        (
            "invalid-total.bin",
            vec![("Content-Range", "bytes 5-9/8")],
            "content-range",
        ),
        (
            "length-mismatch.bin",
            vec![("Content-Range", "bytes 5-9/10"), ("Content-Length", "4")],
            "length",
        ),
    ] {
        let original = b"known";
        let (url, server) =
            start_scripted_server(vec![scripted_response(206, &headers, b"extra")]).await?;
        let destination = directory.destination(name);
        let partial_path = PathBuf::from(format!("{}.part", destination.display()));
        tokio::fs::write(&partial_path, original).await?;
        write_resume_identity(&destination, &url, "\"test-v1\"").await?;
        let item = make_item(&url, &destination)?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);

        let result = engine.download(id, receiver).await;
        match (result, expected_error) {
            (Err(DownloadEngineError::RangeMismatch), "range")
            | (Err(DownloadEngineError::InvalidContentRange), "content-range") => {}
            (Err(DownloadEngineError::InvalidContentLength), "length") => {}
            (unexpected, _) => {
                return Err(format!("unexpected resume result: {unexpected:?}").into());
            }
        }
        assert!(range_header(&server.await?[0]).is_some());
        assert_eq!(tokio::fs::read(partial_path).await?, original);
        assert!(!destination.exists());
    }
    Ok(())
}

#[tokio::test]
async fn resumed_partial_is_preserved_when_redirect_targets_private_address()
-> Result<(), Box<dyn Error>> {
    let (url, redirect_server) = start_redirect_server(None).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("redirected-resume.bin");
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    tokio::fs::write(&partial_path, b"hello").await?;
    write_resume_identity(&destination, &url, "\"test-v1\"").await?;
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    assert!(matches!(
        engine.download(id, receiver).await,
        Err(DownloadEngineError::InvalidUrl)
    ));
    let requests = redirect_server.await?;
    assert_eq!(requests.len(), 1);
    assert!(range_header(&requests[0]).is_some());
    assert_eq!(tokio::fs::read(partial_path).await?, b"hello");
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn ignored_range_restarts_from_zero_without_duplicate_bytes() -> Result<(), Box<dyn Error>> {
    let body: Vec<u8> = (0..128 * 1024).map(|index| (index % 239) as u8).collect();
    let (url, server) = start_scripted_server(vec![scripted_response(200, &[], &body)]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("ignored-range.bin");
    tokio::fs::write(
        PathBuf::from(format!("{}.part", destination.display())),
        &body[..4096],
    )
    .await?;
    let item = make_item(&url, &destination)?;

    let completed = run_download(&engine, &database, item).await?;
    let requests = server.await?;

    assert_eq!(range_header(&requests[0]), None);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.progress().downloaded_bytes(), 128 * 1024);
    Ok(())
}

#[tokio::test]
async fn changed_etag_prevents_appending_stale_partial_bytes() -> Result<(), Box<dyn Error>> {
    let original = b"old!";
    let replacement = b"new!";
    let response = scripted_response(
        206,
        &[("Content-Range", "bytes 4-7/8"), ("ETag", "\"test-v2\"")],
        replacement,
    );
    let (url, server) = start_scripted_server(vec![response]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("changed-etag.bin");
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    tokio::fs::write(&partial_path, original).await?;
    write_resume_identity(&destination, &url, "\"test-v1\"").await?;
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    assert!(matches!(
        engine.download(id, receiver).await,
        Err(DownloadEngineError::RangeMismatch)
    ));
    let requests = server.await?;
    assert_eq!(range_header(&requests[0]), Some("bytes=4-"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("if-range: \"test-v1\"")
    );
    assert_eq!(tokio::fs::read(partial_path).await?, original);
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn changed_last_modified_prevents_appending_stale_partial_bytes() -> Result<(), Box<dyn Error>>
{
    let response = scripted_response(
        206,
        &[
            ("Content-Range", "bytes 4-7/8"),
            ("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
        ],
        b"new!",
    );
    let (url, server) = start_scripted_server(vec![response]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("changed-last-modified.bin");
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    tokio::fs::write(&partial_path, b"old!").await?;
    write_resume_identity(&destination, &url, "Wed, 21 Oct 2015 07:27:00 GMT").await?;
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    assert!(matches!(
        engine.download(id, receiver).await,
        Err(DownloadEngineError::RangeMismatch)
    ));
    let requests = server.await?;
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("if-range: wed, 21 oct 2015 07:27:00 gmt")
    );
    assert_eq!(tokio::fs::read(partial_path).await?, b"old!");
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn if_range_mismatch_restarts_from_a_full_response() -> Result<(), Box<dyn Error>> {
    let body = b"updated resource";
    let response = scripted_response(200, &[("ETag", "\"test-v2\"")], body);
    let (url, server) = start_scripted_server(vec![response]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("if-range-restart.bin");
    tokio::fs::write(
        PathBuf::from(format!("{}.part", destination.display())),
        b"old partial",
    )
    .await?;
    write_resume_identity(&destination, &url, "\"test-v1\"").await?;
    let item = make_item(&url, &destination)?;
    let completed = run_download(&engine, &database, item).await?;
    let requests = server.await?;

    assert_eq!(range_header(&requests[0]), Some("bytes=11-"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("if-range: \"test-v1\"")
    );
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn recovers_a_valid_file_after_crash_during_finalization() -> Result<(), Box<dyn Error>> {
    for destination_already_published in [false, true] {
        let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
        let destination = directory.destination("recovered-finalization.bin");
        let partial_path = PathBuf::from(format!("{}.part", destination.display()));
        let url = "http://127.0.0.1:1/recovery-test";
        let content = b"fully verified before publication";
        tokio::fs::write(&partial_path, content).await?;
        if destination_already_published {
            tokio::fs::write(&destination, content).await?;
        }
        write_finalizing_identity(&destination, url, "\"test-v1\"", content.len() as u64).await?;

        let item = make_item(url, &destination)?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);
        let completed = engine.download(id, receiver).await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(
            completed.progress().downloaded_bytes(),
            content.len() as u64
        );
        assert_eq!(tokio::fs::read(&destination).await?, content);
        assert!(!partial_path.exists());
    }
    Ok(())
}

#[tokio::test]
async fn final_publication_does_not_overwrite_a_concurrent_destination()
-> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig {
        minimum_segmented_file_size: u64::MAX,
        ..HttpEngineConfig::default()
    })
    .await?;
    let (published_first_chunk, wait_for_first_chunk) = oneshot::channel();
    let mut response = ServerResponse::ok(b"replacement".to_vec());
    response.body_chunks = vec![b"replace".to_vec(), b"ment".to_vec()];
    response.delay_between_chunks = Duration::from_millis(100);
    response.signal_after_first_chunk = Some(published_first_chunk);
    let (url, server) = start_server(response).await?;
    let destination = directory.destination("concurrent-target.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);
    let task = tokio::spawn(async move { engine.download(id, receiver).await });

    wait_for_first_chunk.await?;
    tokio::fs::write(&destination, b"existing owner").await?;
    let result = task.await?;
    assert!(matches!(
        result,
        Err(DownloadEngineError::DestinationExists)
    ));
    server.await?;
    assert_eq!(tokio::fs::read(&destination).await?, b"existing owner");
    let persisted = database
        .downloads()
        .get(id)
        .await?
        .ok_or("failed finalization download not found")?;
    assert_eq!(persisted.status(), DownloadStatus::Failed);
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    assert!(
        partial_path.exists(),
        "failed publication keeps recoverable data"
    );
    assert!(
        persisted.status() != DownloadStatus::Completed,
        "publication failure must not mark the download completed"
    );
    Ok(())
}

#[tokio::test]
async fn handles_416_by_restarting_only_when_local_size_is_not_short() -> Result<(), Box<dyn Error>>
{
    for (name, partial_size, remote_size, should_restart) in [
        ("short.bin", 4, 10, false),
        ("equal.bin", 10, 10, true),
        ("oversized.bin", 12, 10, true),
    ] {
        let body = b"0123456789";
        let responses = if should_restart {
            vec![
                scripted_response(
                    416,
                    &[("Content-Range", &format!("bytes */{remote_size}"))],
                    b"",
                ),
                scripted_response(200, &[], body),
            ]
        } else {
            vec![scripted_response(
                416,
                &[("Content-Range", &format!("bytes */{remote_size}"))],
                b"",
            )]
        };
        let (url, server) = start_scripted_server(responses).await?;
        let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
        let destination = directory.destination(name);
        let partial_path = PathBuf::from(format!("{}.part", destination.display()));
        let mut partial_content = body[..partial_size.min(body.len())].to_vec();
        partial_content.resize(partial_size, b'x');
        tokio::fs::write(&partial_path, &partial_content).await?;
        write_resume_identity(&destination, &url, "\"test-v1\"").await?;
        let item = make_item(&url, &destination)?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);

        let result = engine.download(id, receiver).await;
        let requests = server.await?;
        assert_eq!(
            range_header(&requests[0]),
            Some(format!("bytes={partial_size}-").as_str())
        );
        if should_restart {
            assert_eq!(requests.len(), 2);
            assert_eq!(range_header(&requests[1]), None);
            assert_eq!(result?.status(), DownloadStatus::Completed);
            assert_eq!(tokio::fs::read(destination).await?, body);
        } else {
            assert!(matches!(
                result,
                Err(DownloadEngineError::RangeNotSatisfiable)
            ));
            assert_eq!(tokio::fs::read(partial_path).await?, partial_content);
            assert!(!destination.exists());
        }
    }
    Ok(())
}

#[tokio::test]
async fn resumes_unknown_total_from_valid_content_range() -> Result<(), Box<dyn Error>> {
    let body = b"unknown-total-body";
    let partial_size = 7;
    let response = scripted_response(
        206,
        &[
            (
                "Content-Range",
                &format!("bytes {partial_size}-{}/*", body.len() - 1),
            ),
            ("ETag", "\"test-v1\""),
        ],
        &body[partial_size..],
    );
    let (url, server) = start_scripted_server(vec![response]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("unknown-total.bin");
    tokio::fs::write(
        PathBuf::from(format!("{}.part", destination.display())),
        &body[..partial_size],
    )
    .await?;
    write_resume_identity(&destination, &url, "\"test-v1\"").await?;
    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;

    server.await?;
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.progress().downloaded_bytes(), body.len() as u64);
    assert_eq!(completed.progress().total_bytes(), None);
    Ok(())
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
async fn single_stream_download_persists_measured_speed_and_eta() -> Result<(), Box<dyn Error>> {
    let body = vec![42_u8; 128 * 1024];
    let mut response = ServerResponse::ok(body.clone());
    response.body_chunks = body.chunks(16 * 1024).map(<[u8]>::to_vec).collect();
    response.delay_between_chunks = Duration::from_millis(25);
    let (url, server) = start_server(response).await?;
    let config = HttpEngineConfig {
        minimum_segmented_file_size: u64::MAX,
        progress_bytes_interval: 16 * 1024,
        progress_time_interval: Duration::from_millis(20),
        ..HttpEngineConfig::default()
    };
    let (database, engine, directory) = setup_engine(config).await?;
    let destination = directory.destination("single-stream-speed.bin");
    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
    server.await?;

    assert_eq!(tokio::fs::read(destination).await?, body);
    assert!(
        completed
            .progress()
            .speed_bytes_per_second()
            .is_some_and(|speed| speed > 0)
    );
    assert_eq!(completed.progress().estimated_remaining_seconds(), Some(0));
    Ok(())
}

#[tokio::test]
#[ignore = "requires direct public network access to verify a pinned safe redirect"]
async fn follows_public_redirect_and_downloads_known_content() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let expected_body = b"User-agent: *\nDisallow:\n";
    let (url, server) =
        start_redirect_server(Some("https://www.iana.org/robots.txt".to_owned())).await?;
    let destination = directory.destination("redirect.bin");
    let item = make_item(&url, &destination)?;
    let completed = run_download(&engine, &database, item).await?;
    let requests = server.await?;

    let body = tokio::fs::read(destination).await?;
    assert_eq!(body, expected_body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /redirect "));
    Ok(())
}

#[tokio::test]
async fn follows_first_request_then_rejects_redirect_to_private_address()
-> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let (url, server) = start_redirect_server(None).await?;
    let destination = directory.destination("private-redirect.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    assert!(matches!(
        engine.download(id, receiver).await,
        Err(DownloadEngineError::InvalidUrl)
    ));
    let requests = server.await?;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /redirect "));
    assert!(!destination.exists());
    Ok(())
}

#[tokio::test]
async fn transient_connection_failure_is_retried() -> Result<(), Box<dyn Error>> {
    let body = b"retried successfully".to_vec();
    let (url, server) = start_disconnect_then_success_server(body.clone()).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("connection-retry.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    let completed = run_download(&engine, &database, item).await?;
    let requests = server.await?;

    assert_eq!(requests.len(), 2);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
    let id_string = id.to_string();
    let retry_events = fetchx_models::global_diagnostics()
        .recent_events()
        .into_iter()
        .filter(|event| event.download_id.as_deref() == Some(id_string.as_str()))
        .collect::<Vec<_>>();
    assert!(retry_events.iter().any(|event| {
        event.event == "retry_started"
            && event.error_code.as_deref() == Some("HTTP_TRANSIENT_REQUEST")
    }));
    let serialized = serde_json::to_string(&retry_events)?;
    assert!(!serialized.contains(&url));
    Ok(())
}

#[tokio::test]
async fn cancellation_during_retry_backoff_does_not_resurrect_download()
-> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("cancel-retry.bin");
    let (url, retry_started, requests, server) =
        start_retry_control_server(b"must not be downloaded".to_vec()).await?;
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;

    let engine = Arc::new(engine);
    let (cancel_sender, cancellation) = watch::channel(false);
    let (_pause_sender, pause) = watch::channel(false);
    let (_limit_sender, connection_limit) = watch::channel(4);
    let download = tokio::spawn(async move {
        engine
            .download_managed(
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
    });

    tokio::time::timeout(Duration::from_secs(2), retry_started).await??;
    cancel_sender.send(true)?;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), download).await??,
        Err(DownloadEngineError::Cancelled)
    ));
    assert_eq!(
        database.downloads().get(id).await?.unwrap().status(),
        DownloadStatus::Cancelled
    );
    assert_eq!(requests.lock().await.len(), 1);
    assert!(!destination.exists());
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test]
async fn pause_during_retry_backoff_stays_paused_until_explicit_resume()
-> Result<(), Box<dyn Error>> {
    let body = b"completed only after explicit resume".to_vec();
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("pause-retry.bin");
    let (url, retry_started, requests, server) = start_retry_control_server(body.clone()).await?;
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;

    let engine = Arc::new(engine);
    let (_cancel_sender, cancellation) = watch::channel(false);
    let (pause_sender, pause) = watch::channel(false);
    let (_limit_sender, connection_limit) = watch::channel(4);
    let first_engine = Arc::clone(&engine);
    let download = tokio::spawn(async move {
        first_engine
            .download_managed(
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
    });

    tokio::time::timeout(Duration::from_secs(2), retry_started).await??;
    pause_sender.send(true)?;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), download).await??,
        Err(DownloadEngineError::Paused)
    ));
    assert_eq!(
        database.downloads().get(id).await?.unwrap().status(),
        DownloadStatus::Paused
    );
    assert_eq!(requests.lock().await.len(), 1);
    assert!(!destination.exists());

    let (_resume_cancel_sender, resume_cancellation) = watch::channel(false);
    let completed = engine.download(id, resume_cancellation).await?;
    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(requests.lock().await.len(), 2);
    server.await?;
    Ok(())
}

#[tokio::test]
async fn http_failures_persist_failed_state_and_safe_error() -> Result<(), Box<dyn Error>> {
    for status in [404, 403, 429, 500, 502, 503] {
        let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
        let attempts = if crate::retry::RetryPolicy::is_transient_status(status) {
            4
        } else {
            1
        };
        let retry_headers = if attempts > 1 {
            vec![("Retry-After", "0")]
        } else {
            vec![]
        };
        let responses = (0..attempts)
            .map(|_| scripted_response(status, &retry_headers, b"failure body"))
            .collect();
        let (url, server) = start_scripted_server(responses).await?;
        let item = make_item(&url, &directory.destination("failed.bin"))?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);

        assert!(matches!(
            engine.download(id, receiver).await,
            Err(DownloadEngineError::HttpStatus(found)) if found == status
        ));
        assert_eq!(server.await?.len(), attempts);
        let persisted = database
            .downloads()
            .get(id)
            .await?
            .ok_or("failed download not found")?;
        assert_eq!(persisted.status(), DownloadStatus::Failed);
        assert!(persisted.error().is_some());
        assert!(!directory.destination("failed.bin").exists());
        if status == 503 {
            let id_string = id.to_string();
            let retry_events = fetchx_models::global_diagnostics()
                .recent_events()
                .into_iter()
                .filter(|event| event.download_id.as_deref() == Some(id_string.as_str()))
                .collect::<Vec<_>>();
            assert_eq!(
                retry_events
                    .iter()
                    .filter(|event| {
                        event.event == "retry_started"
                            && event.error_code.as_deref() == Some("HTTP_TRANSIENT_STATUS")
                    })
                    .count(),
                3
            );
            assert_eq!(
                retry_events
                    .iter()
                    .filter(|event| {
                        event.event == "retry_exhausted"
                            && event.error_code.as_deref() == Some("HTTP_TRANSIENT_STATUS")
                    })
                    .count(),
                1
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn truncated_response_fails_and_preserves_the_partial_file() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let response = scripted_response(200, &[("Content-Length", "64")], b"partial");
    let (url, server) = start_scripted_server(vec![response]).await?;
    let destination = directory.destination("truncated.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_sender, receiver) = watch::channel(false);

    let error = engine
        .download(id, receiver)
        .await
        .expect_err("truncated response must fail without an automatic retry");
    match &error {
        DownloadEngineError::Request(request_error) => {
            assert!(
                request_error.is_decode(),
                "truncated response should be classified as a decode failure"
            );
            assert!(
                !crate::retry::RetryPolicy::is_transient_request(request_error),
                "decode failures are intentionally excluded from automatic request retries"
            );
        }
        other => panic!("unexpected truncation error: {other:?}"),
    }
    let requests = server.await?;
    assert_eq!(
        requests.len(),
        1,
        "body interruption is not automatically retried"
    );
    let persisted = database
        .downloads()
        .get(id)
        .await?
        .ok_or("failed download not found")?;
    assert_eq!(persisted.status(), DownloadStatus::Failed);
    assert!(persisted.progress().downloaded_bytes() <= 7);
    assert!(!destination.exists());
    let partial = PathBuf::from(format!("{}.part", destination.display()));
    assert!(partial.exists());
    assert_eq!(tokio::fs::metadata(partial).await?.len(), 7);
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
    // FTP URLs can no longer be created with the DirectHttp engine (FEAT-001
    // introduced SchemeMismatch validation in DownloadSource).  Only test the
    // embedded-credentials case which still applies to HTTP URLs.
    for (url, expected) in [("http://user:password@example.test/file", "credentials")] {
        let item = make_item(url, &directory.destination("rejected.bin"))?;
        let id = item.id();
        database.downloads().create(&item).await?;
        let (_sender, receiver) = watch::channel(false);
        let result = engine.download(id, receiver).await;
        assert!(matches!(
            (result, expected),
            (Err(DownloadEngineError::EmbeddedCredentials), "credentials")
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
async fn cancellation_preserves_partial_file_for_a_later_resume() -> Result<(), Box<dyn Error>> {
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let body = vec![8_u8; 1024 * 1024];
    let (url, notified, continue_server, server) = start_cancel_resume_server(body.clone()).await?;
    let destination = directory.destination("cancelled.bin");
    let item = make_item(&url, &destination)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (sender, receiver) = watch::channel(false);
    let engine_task = tokio::spawn(async move { engine.download(id, receiver).await });

    notified.await?;
    for _ in 0..100 {
        let current = database.downloads().get(id).await?;
        if current.is_some_and(|item| item.progress().downloaded_bytes() > 0) {
            break;
        }
        sleep(Duration::from_millis(2)).await;
    }
    sender.send(true)?;
    let engine_result = engine_task.await?;
    assert!(
        matches!(engine_result, Err(DownloadEngineError::Cancelled)),
        "{engine_result:?}"
    );
    let _ = continue_server.send(());

    let cancelled = database
        .downloads()
        .get(id)
        .await?
        .ok_or("cancelled download not found")?;
    assert_eq!(cancelled.status(), DownloadStatus::Cancelled);
    assert!(!destination.exists());
    let partial_path = PathBuf::from(format!("{}.part", destination.display()));
    let partial_size = tokio::fs::metadata(&partial_path).await?.len() as usize;
    assert!(partial_size > 0 && partial_size < body.len());
    assert_eq!(cancelled.progress().downloaded_bytes(), partial_size as u64);

    let resume_engine = DirectHttpEngine::new(database.downloads())?;
    let (_sender, receiver) = watch::channel(false);
    let completed = resume_engine.download(id, receiver).await?;

    let requests = server.await?;
    assert_eq!(range_header(&requests[0]), None);
    assert_eq!(
        range_header(&requests[1]),
        Some(format!("bytes={partial_size}-").as_str())
    );
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
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

// ── FTP regression tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn multi_source_download_fails_over_then_covers_segments_without_overlap()
-> Result<(), Box<dyn Error>> {
    let body = (0..4096).map(|byte| (byte % 251) as u8).collect::<Vec<_>>();
    let (failed_url, failed_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Fail, 32, false).await?;
    let (working_url, working_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 5, false).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("multi-segmented.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&failed_url, EngineKind::DirectHttp)?,
        FileName::new("multi-segmented.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[failed_url.clone(), working_url]).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(
        repository,
        HttpEngineConfig {
            segment_count: 4,
            minimum_segmented_file_size: 1,
            ..HttpEngineConfig::default()
        },
    )?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let completed = engine.download(id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    failed_server.abort();
    let _ = failed_server.await;
    let report = working_server.await?;
    let mut coverage = vec![0_u8; body.len()];
    let mut ranged = 0;
    for request in &report.requests {
        if let Some(value) = range_header(request) {
            ranged += 1;
            let (start, end) = value
                .strip_prefix("bytes=")
                .and_then(|value| value.split_once('-'))
                .ok_or("invalid range")?;
            let start = start.parse::<usize>()?;
            let end = end.parse::<usize>()?;
            for byte in coverage.get_mut(start..=end).ok_or("range outside file")? {
                *byte += 1;
            }
        }
    }
    assert_eq!(ranged, 4);
    assert!(coverage.iter().all(|count| *count == 1));
    Ok(())
}

#[tokio::test]
async fn multi_source_ranges_are_assigned_across_validated_mirrors() -> Result<(), Box<dyn Error>> {
    let body = (0..4096).map(|byte| (byte % 251) as u8).collect::<Vec<_>>();
    let expected_sha256 = hex::encode(sha2::Sha256::digest(&body));
    let (first_url, first_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 3, true).await?;
    let (second_url, second_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 3, true).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("cross-source.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("cross-source.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[first_url, second_url]).await?;
    attach_expected_sha256(&destination, &body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(repository, segmented_config(1))?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let completed = engine.download(id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    let reports = [first_server.await?, second_server.await?];
    let mut coverage = vec![0_u8; body.len()];
    let mut mirror_ranges = [0_usize; 2];
    for (mirror_index, report) in reports.iter().enumerate() {
        for request in &report.requests {
            let Some(range) = range_header(request) else {
                continue;
            };
            if range == "bytes=0-0" {
                continue;
            }
            mirror_ranges[mirror_index] += 1;
            let (start, end) = range
                .strip_prefix("bytes=")
                .and_then(|value| value.split_once('-'))
                .ok_or("invalid range")?;
            for byte in coverage
                .get_mut(start.parse::<usize>()?..=end.parse::<usize>()?)
                .ok_or("range outside file")?
            {
                *byte += 1;
            }
        }
    }
    assert!(mirror_ranges.iter().all(|count| *count > 0));
    assert_eq!(mirror_ranges.iter().sum::<usize>(), 4);
    assert!(coverage.iter().all(|count| *count == 1));
    assert_eq!(expected_sha256, hex::encode(sha2::Sha256::digest(&body)));
    Ok(())
}

#[tokio::test]
async fn multi_source_reassigns_failed_active_ranges_to_another_valid_mirror()
-> Result<(), Box<dyn Error>> {
    let body = (0..4096).map(|byte| (byte % 233) as u8).collect::<Vec<_>>();
    let (failing_url, failing_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::FailAfterProbe, 3, false).await?;
    let (healthy_url, healthy_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 5, true).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("range-reassignment.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&failing_url, EngineKind::DirectHttp)?,
        FileName::new("range-reassignment.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[failing_url, healthy_url]).await?;
    attach_expected_sha256(&destination, &body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(repository, segmented_config(1))?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let completed = engine.download(id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    let failing_report = failing_server.await?;
    let healthy_report = healthy_server.await?;
    assert_eq!(
        failing_report
            .requests
            .iter()
            .filter(|request| range_header(request).is_some_and(|range| range != "bytes=0-0"))
            .count(),
        2
    );
    assert_eq!(
        healthy_report
            .requests
            .iter()
            .filter(|request| range_header(request).is_some_and(|range| range != "bytes=0-0"))
            .count(),
        4
    );
    Ok(())
}

#[tokio::test]
async fn multi_source_conflicting_probe_total_keeps_ordered_single_source_path()
-> Result<(), Box<dyn Error>> {
    let body = (0..4096).map(|byte| (byte % 241) as u8).collect::<Vec<_>>();
    let (invalid_url, invalid_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::WrongTotal, 7, false).await?;
    let (unused_url, unused_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 1, false).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("conflicting-source.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&invalid_url, EngineKind::DirectHttp)?,
        FileName::new("conflicting-source.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[invalid_url, unused_url]).await?;
    attach_expected_sha256(&destination, &body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(repository, segmented_config(1))?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let completed = engine.download(id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    let invalid_report = invalid_server.await?;
    let backup_report = unused_server.await?;
    assert_eq!(range_header(&backup_report.requests[0]), Some("bytes=0-0"));
    assert_eq!(
        invalid_report
            .requests
            .iter()
            .filter(|request| range_header(request).is_some_and(|range| range != "bytes=0-0"))
            .count(),
        4
    );
    Ok(())
}

#[tokio::test]
async fn multi_source_same_size_different_bytes_never_publishes_bad_hash()
-> Result<(), Box<dyn Error>> {
    let expected_body = (0..4096).map(|byte| (byte % 239) as u8).collect::<Vec<_>>();
    let first_body = expected_body
        .iter()
        .map(|byte| byte.wrapping_add(1))
        .collect::<Vec<_>>();
    let second_body = expected_body
        .iter()
        .map(|byte| byte.wrapping_add(2))
        .collect::<Vec<_>>();
    let (first_url, first_server) =
        start_segment_server(first_body, 4, SegmentServerMode::Valid, 8, true).await?;
    let (second_url, second_server) =
        start_segment_server(second_body, 4, SegmentServerMode::Valid, 8, true).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("bad-cross-source.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("bad-cross-source.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[first_url, second_url]).await?;
    attach_expected_sha256(&destination, &expected_body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(repository, segmented_config(1))?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let result = engine.download(id, cancellation).await;

    assert!(matches!(
        result,
        Err(DownloadEngineError::Sha256Mismatch { .. })
    ));
    assert!(!tokio::fs::try_exists(&destination).await?);
    first_server.abort();
    second_server.abort();
    let _ = first_server.await;
    let _ = second_server.await;
    Ok(())
}

#[tokio::test]
async fn multi_source_range_retry_exhaustion_preserves_destination() -> Result<(), Box<dyn Error>> {
    let body = vec![63_u8; 4096];
    let expected_body = vec![19_u8; 4096];
    let (first_url, first_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::FailAfterProbe, 6, false).await?;
    let (second_url, second_server) =
        start_segment_server(body, 4, SegmentServerMode::FailAfterProbe, 6, false).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("range-exhaustion.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("range-exhaustion.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[first_url, second_url]).await?;
    attach_expected_sha256(&destination, &expected_body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(repository, segmented_config(1))?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let result = engine.download(id, cancellation).await;

    assert!(result.is_err());
    assert!(!tokio::fs::try_exists(&destination).await?);
    let first_report = first_server.await?;
    let second_report = second_server.await?;
    assert!(
        first_report
            .requests
            .iter()
            .any(|request| range_header(request) == Some("bytes=0-0"))
    );
    assert!(
        second_report
            .requests
            .iter()
            .any(|request| range_header(request) == Some("bytes=0-0"))
    );
    Ok(())
}

#[tokio::test]
async fn multi_source_cross_range_destination_collision_is_not_overwritten()
-> Result<(), Box<dyn Error>> {
    let body = b"expected data".to_vec();
    let (first_url, first_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 1, false).await?;
    let (second_url, second_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 1, false).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("cross-collision.bin");
    let original = b"existing user file";
    tokio::fs::write(&destination, original).await?;
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("cross-collision.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[first_url, second_url]).await?;
    attach_expected_sha256(&destination, &body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(repository, segmented_config(1))?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    assert!(matches!(
        engine.download(id, cancellation).await,
        Err(DownloadEngineError::DestinationExists)
    ));
    assert_eq!(tokio::fs::read(destination).await?, original);
    first_server.abort();
    second_server.abort();
    let _ = first_server.await;
    let _ = second_server.await;
    Ok(())
}

#[tokio::test]
async fn multi_source_cancelled_ranges_resume_with_the_same_digest_contract()
-> Result<(), Box<dyn Error>> {
    let body = vec![23_u8; 1024 * 1024];
    let (first_url, first_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 12, true).await?;
    let (second_url, second_server) =
        start_segment_server(body.clone(), 4, SegmentServerMode::Valid, 12, true).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("cross-source-resume.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("cross-source-resume.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[first_url, second_url]).await?;
    attach_expected_sha256(&destination, &body).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(
        repository.clone(),
        HttpEngineConfig {
            progress_time_interval: Duration::from_millis(1),
            progress_bytes_interval: 1,
            ..segmented_config(1)
        },
    )?;
    let (cancel_sender, cancellation) = watch::channel(false);
    let download = tokio::spawn(async move { engine.download(id, cancellation).await });
    let mut observed_progress = false;
    for _ in 0..200 {
        let current = database.downloads().get(id).await?;
        if current.is_some_and(|item| {
            item.progress().downloaded_bytes() > 0 && item.status() == DownloadStatus::Downloading
        }) {
            observed_progress = true;
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }
    assert!(
        observed_progress,
        "expected ranged bytes before cancellation"
    );
    cancel_sender.send(true)?;
    assert!(matches!(
        download.await?,
        Err(DownloadEngineError::Cancelled)
    ));

    let (_resume_cancel, resume_cancellation) = watch::channel(false);
    let cancelled_item = database
        .downloads()
        .get(id)
        .await?
        .ok_or("cancelled download disappeared")?;
    assert_eq!(cancelled_item.status(), DownloadStatus::Cancelled);
    let mut entries = tokio::fs::read_dir(dir.path()).await?;
    let mut partial_segments = 0;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_string_lossy().contains(".segments-") {
            partial_segments += 1;
        }
    }
    assert!(partial_segments > 0, "expected resumable range files");
    let engine = DirectHttpEngine::with_config(
        repository,
        HttpEngineConfig {
            progress_time_interval: Duration::from_millis(1),
            progress_bytes_interval: 1,
            ..segmented_config(1)
        },
    )?;
    let resumed = engine.download(id, resume_cancellation).await?;
    assert_eq!(resumed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(resumed.id(), id);
    assert_ne!(resumed.updated_at(), resumed.created_at());
    drop(resumed);
    let reports = [first_server.await?, second_server.await?];
    let planned_starts = [0, 262_144, 524_288, 786_432];
    assert!(
        reports
            .iter()
            .flat_map(|report| &report.requests)
            .filter_map(|request| range_header(request))
            .filter(|range| *range != "bytes=0-0")
            .filter_map(|range| {
                range
                    .strip_prefix("bytes=")
                    .and_then(|value| value.split_once('-'))
                    .and_then(|(start, _)| start.parse::<usize>().ok())
            })
            .any(|start| !planned_starts.contains(&start)),
        "expected the resumed request to continue within a previously started segment"
    );
    Ok(())
}

#[tokio::test]
async fn multi_source_sha256_rejects_same_size_bytes_with_matching_etag()
-> Result<(), Box<dyn Error>> {
    let expected_body = b"trusted expected object".to_vec();
    let incorrect_body = b"untrusted other object!".to_vec();
    assert_eq!(expected_body.len(), incorrect_body.len());
    let expected_sha256 = hex::encode(sha2::Sha256::digest(&expected_body));
    let headers = [("ETag", "\"same-validator\"")];
    let (incorrect_url, incorrect_server) =
        start_scripted_server(vec![scripted_response(200, &headers, &incorrect_body)]).await?;
    let (correct_url, correct_server) =
        start_scripted_server(vec![scripted_response(200, &headers, &expected_body)]).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("multi-checksum.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&incorrect_url, EngineKind::DirectHttp)?,
        FileName::new("multi-checksum.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[incorrect_url, correct_url]).await?;
    let mut checksum_path = destination.as_os_str().to_os_string();
    checksum_path.push(".fetchx.sha256");
    tokio::fs::write(PathBuf::from(checksum_path), expected_sha256).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(
        repository,
        HttpEngineConfig {
            minimum_segmented_file_size: u64::MAX,
            ..HttpEngineConfig::default()
        },
    )?;
    let (_cancel_tx, cancellation) = watch::channel(false);

    let completed = engine.download(id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, expected_body);
    assert_eq!(incorrect_server.await?.len(), 1);
    assert_eq!(correct_server.await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn multi_source_inconsistent_or_ignored_ranges_falls_back_to_one_source()
-> Result<(), Box<dyn Error>> {
    for mode in [
        SegmentServerMode::WrongTotal,
        SegmentServerMode::IgnoreRange,
    ] {
        let body = (0..4096).map(|byte| (byte % 239) as u8).collect::<Vec<_>>();
        let (url, server) = start_segment_server(body.clone(), 4, mode, 6, false).await?;
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let dir = tempfile::tempdir()?;
        let destination = dir.path().join("multi-range-fallback.bin");
        let item = DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(&url, EngineKind::DirectHttp)?,
            FileName::new("multi-range-fallback.bin")?,
            destination.clone(),
            Timestamp::from_unix_millis(1),
        );
        let id = item.id();
        attach_http_multi_source_set(
            &destination,
            &[url, "http://127.0.0.1:1/unused-backup".to_owned()],
        )
        .await?;
        repository.create(&item).await?;
        let engine = DirectHttpEngine::with_config(
            repository,
            HttpEngineConfig {
                segment_count: 4,
                minimum_segmented_file_size: 1,
                ..HttpEngineConfig::default()
            },
        )?;
        let (_cancel_tx, cancellation) = watch::channel(false);

        let completed = engine.download(id, cancellation).await?;

        assert_eq!(completed.status(), DownloadStatus::Completed);
        assert_eq!(tokio::fs::read(destination).await?, body);
        let report = server.await?;
        assert_eq!(report.requests.len(), 6);
        assert_eq!(
            report
                .requests
                .iter()
                .filter(|request| range_header(request).is_some())
                .count(),
            4
        );
    }
    Ok(())
}

#[tokio::test]
async fn multi_source_cancellation_keeps_source_bound_partial_for_resume()
-> Result<(), Box<dyn Error>> {
    let body = vec![23_u8; 1024 * 1024];
    let (url, first_chunk_received, continue_server, server) =
        start_cancel_resume_server(body.clone()).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("multi-resume.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::DirectHttp)?,
        FileName::new("multi-resume.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(
        &destination,
        &[url, "https://backup.example/file.bin".to_owned()],
    )
    .await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::with_config(
        repository.clone(),
        HttpEngineConfig {
            minimum_segmented_file_size: u64::MAX,
            ..HttpEngineConfig::default()
        },
    )?;
    let (cancel_sender, cancellation) = watch::channel(false);
    let engine_task = tokio::spawn(async move { engine.download(id, cancellation).await });
    first_chunk_received.await?;
    for _ in 0..100 {
        if repository
            .get(id)
            .await?
            .is_some_and(|item| item.progress().downloaded_bytes() > 0)
        {
            break;
        }
        sleep(Duration::from_millis(2)).await;
    }
    cancel_sender.send(true)?;
    assert!(matches!(
        engine_task.await?,
        Err(DownloadEngineError::Cancelled)
    ));
    let partial = PathBuf::from(format!("{}.part", destination.display()));
    assert!(tokio::fs::metadata(&partial).await?.len() > 0);
    let mut sources_path = destination.as_os_str().to_os_string();
    sources_path.push(".fetchx.sources");
    assert!(PathBuf::from(sources_path).exists());

    let _ = continue_server.send(());
    let resumed = DirectHttpEngine::with_config(
        repository,
        HttpEngineConfig {
            minimum_segmented_file_size: u64::MAX,
            ..HttpEngineConfig::default()
        },
    )?;
    let (_cancel_tx, cancellation) = watch::channel(false);
    let completed = resumed.download(id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, body);
    let requests = server.await?;
    assert_eq!(requests.len(), 2);
    assert!(range_header(&requests[1]).is_some());
    Ok(())
}

#[tokio::test]
async fn multi_source_retry_exhaustion_preserves_metadata_and_destination_collision()
-> Result<(), Box<dyn Error>> {
    let (first_url, first_server) =
        start_scripted_server(vec![scripted_response(404, &[], b"not found")]).await?;
    let (second_url, second_server) =
        start_scripted_server(vec![scripted_response(404, &[], b"not found")]).await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("multi-exhausted.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("multi-exhausted.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    attach_http_multi_source_set(&destination, &[first_url.clone(), second_url.clone()]).await?;
    repository.create(&item).await?;
    let engine = DirectHttpEngine::new(repository.clone())?;
    let (_cancel_tx, cancellation) = watch::channel(false);
    assert!(matches!(
        engine.download(id, cancellation).await,
        Err(DownloadEngineError::HttpStatus(404))
    ));
    assert_eq!(first_server.await?.len(), 1);
    assert_eq!(second_server.await?.len(), 1);
    assert!(!destination.exists());
    let mut sources_path = destination.as_os_str().to_os_string();
    sources_path.push(".fetchx.sources");
    assert!(PathBuf::from(sources_path).exists());

    let collision = dir.path().join("multi-collision.bin");
    tokio::fs::write(&collision, b"preserve me").await?;
    let collision_item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&first_url, EngineKind::DirectHttp)?,
        FileName::new("multi-collision.bin")?,
        collision.clone(),
        Timestamp::from_unix_millis(2),
    );
    let collision_id = collision_item.id();
    attach_http_multi_source_set(&collision, &[first_url, second_url]).await?;
    repository.create(&collision_item).await?;
    let (_cancel_tx, cancellation) = watch::channel(false);
    assert!(matches!(
        engine.download(collision_id, cancellation).await,
        Err(DownloadEngineError::DestinationExists)
    ));
    assert_eq!(tokio::fs::read(collision).await?, b"preserve me");
    Ok(())
}

/// Normal FTP download: start in-process server, download file, verify content.
#[tokio::test]
async fn ftp_normal_download_via_in_process_server() -> Result<(), Box<dyn Error>> {
    use crate::FtpEngine;

    let content = b"FTP regression test content".to_vec();
    let (addr, _server) = start_ftp_server(content.clone()).await?;

    let database = Database::connect("sqlite::memory:").await?;
    let repo = database.downloads();
    let engine = FtpEngine::new(repo.clone());

    let dir = tempfile::tempdir()?;
    let dest = dir.path().join("ftp-normal.bin");
    let url = format!("ftp://{addr}/file.bin");

    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftp)?,
        FileName::new("ftp-normal.bin")?,
        dest.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repo.create(&item).await?;

    let (_tx, rx) = watch::channel(false);
    let completed = crate::DownloadEngine::download(&engine, id, rx).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(&dest).await?, content);
    Ok(())
}

/// FTP resume via REST: place a partial file, start server that responds to REST,
/// verify the completed file contains the full content.
#[tokio::test]
async fn ftp_resume_rest_appends_remaining_bytes() -> Result<(), Box<dyn Error>> {
    use crate::FtpEngine;

    let full_content: Vec<u8> = (0..256_u32).map(|i| (i % 200) as u8).collect();
    let partial_size = 64_usize;

    // Start a server with the full content; FtpEngine will issue REST <offset>.
    let (addr, _server) = start_ftp_server(full_content.clone()).await?;

    let database = Database::connect("sqlite::memory:").await?;
    let repo = database.downloads();
    let engine = FtpEngine::new(repo.clone());

    let dir = tempfile::tempdir()?;
    let dest = dir.path().join("ftp-resume.bin");
    let tmp = dir.path().join("ftp-resume.bin.part");
    // Write partial data to simulate a previous interrupted download.
    tokio::fs::write(&tmp, &full_content[..partial_size]).await?;

    let url = format!("ftp://{addr}/file.bin");
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftp)?,
        FileName::new("ftp-resume.bin")?,
        dest.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repo.create(&item).await?;

    let (_tx, rx) = watch::channel(false);
    let completed = crate::DownloadEngine::download(&engine, id, rx).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(&dest).await?, full_content);
    Ok(())
}

/// FTP cancel: signal cancellation immediately and verify `Cancelled` is returned.
#[tokio::test]
async fn ftp_cancellation_returns_cancelled_error() -> Result<(), Box<dyn Error>> {
    use crate::{FtpEngine, ManagedDownloadControl, ManagedDownloadEngine};

    // Use a large content so the transfer has time to be cancelled.
    let content = vec![0u8; 512 * 1024];
    let (addr, _server) = start_ftp_server(content).await?;

    let database = Database::connect("sqlite::memory:").await?;
    let repo = database.downloads();
    let engine = FtpEngine::new(repo.clone());

    let dir = tempfile::tempdir()?;
    let dest = dir.path().join("ftp-cancel.bin");
    let url = format!("ftp://{addr}/file.bin");

    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftp)?,
        FileName::new("ftp-cancel.bin")?,
        dest.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repo.create(&item).await?;

    // Cancel immediately (before starting).
    let (cancel_tx, cancellation) = watch::channel(true);
    let (_pause_tx, pause) = watch::channel(false);
    let (_limit_tx, connection_limit) = watch::channel(1_usize);
    let control = ManagedDownloadControl {
        cancellation,
        pause,
        connection_limit,
        resource_demand: None,
        bandwidth: BandwidthControl::default(),
    };
    let result = engine.download_managed(id, control).await;
    drop(cancel_tx);

    assert!(
        matches!(result, Err(DownloadEngineError::Cancelled)),
        "expected Cancelled, got: {result:?}"
    );
    assert!(!dest.exists());
    Ok(())
}

#[tokio::test]
async fn ftps_explicit_tls_authenticates_and_verifies_transfer() -> Result<(), Box<dyn Error>> {
    use crate::{FtpEngine, ftp::FtpEngineConfig};

    let content = b"hermetic explicit FTPS transfer".to_vec();
    let server = start_ftps_server(
        content.clone(),
        None,
        "fetchx",
        "test-secret",
        Duration::ZERO,
        Duration::ZERO,
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-success.bin");
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-success.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;

    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            username: Some("fetchx".to_owned()),
            password: Some("test-secret".to_owned()),
            additional_root_certificates: vec![server.certificate],
            ..FtpEngineConfig::default()
        },
    );
    let (_cancel_tx, cancellation) = watch::channel(false);
    let completed = crate::DownloadEngine::download(&engine, id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, content);
    server.task.await?;
    Ok(())
}

#[tokio::test]
async fn ftps_rejects_untrusted_certificate_without_retry_or_publication()
-> Result<(), Box<dyn Error>> {
    use crate::{FtpEngine, ftp::FtpEngineConfig};

    let server = start_ftps_server(
        b"must remain private".to_vec(),
        None,
        "fetchx",
        "secret",
        Duration::ZERO,
        Duration::ZERO,
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-untrusted.bin");
    let temporary_path = dir.path().join("ftps-untrusted.bin.part");
    tokio::fs::write(&destination, b"preserve destination").await?;
    tokio::fs::write(&temporary_path, b"preserve partial").await?;
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-untrusted.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;
    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            tls_handshake_timeout: Duration::from_secs(2),
            ..FtpEngineConfig::default()
        },
    );

    let (_cancel_tx, cancellation) = watch::channel(false);
    let result = crate::DownloadEngine::download(&engine, id, cancellation).await;

    assert!(
        matches!(result, Err(DownloadEngineError::FtpTlsError(_))),
        "expected typed FTPS TLS failure, got {result:?}"
    );
    assert_eq!(tokio::fs::read(destination).await?, b"preserve destination");
    assert_eq!(tokio::fs::read(temporary_path).await?, b"preserve partial");
    server.task.await?;
    Ok(())
}

#[tokio::test]
async fn ftps_tls_handshake_timeout_fails_without_plaintext_fallback() -> Result<(), Box<dyn Error>>
{
    use crate::{FtpEngine, ftp::FtpEngineConfig};

    let server = start_ftps_server(
        b"not published".to_vec(),
        None,
        "fetchx",
        "secret",
        Duration::from_millis(300),
        Duration::ZERO,
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-handshake-timeout.bin");
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-handshake-timeout.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;
    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            tls_handshake_timeout: Duration::from_millis(75),
            additional_root_certificates: vec![server.certificate],
            ..FtpEngineConfig::default()
        },
    );

    let (_cancel_tx, cancellation) = watch::channel(false);
    let result = crate::DownloadEngine::download(&engine, id, cancellation).await;

    assert!(
        matches!(result, Err(DownloadEngineError::FtpTlsError(_))),
        "expected typed FTPS TLS timeout, got {result:?}"
    );
    assert!(!destination.exists());
    server.task.await?;
    Ok(())
}

#[tokio::test]
async fn ftps_rejects_invalid_credentials() -> Result<(), Box<dyn Error>> {
    use crate::{FtpEngine, ftp::FtpEngineConfig};

    let server = start_ftps_server(
        b"never transferred".to_vec(),
        None,
        "fetchx",
        "right",
        Duration::ZERO,
        Duration::ZERO,
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-invalid-auth.bin");
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-invalid-auth.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;
    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            username: Some("fetchx".to_owned()),
            password: Some("wrong".to_owned()),
            additional_root_certificates: vec![server.certificate],
            ..FtpEngineConfig::default()
        },
    );

    let (_cancel_tx, cancellation) = watch::channel(false);
    let result = crate::DownloadEngine::download(&engine, id, cancellation).await;

    assert!(matches!(
        result,
        Err(DownloadEngineError::FtpCredentialRequired)
    ));
    assert!(!destination.exists());
    server.task.await?;
    Ok(())
}

#[tokio::test]
async fn ftps_cancellation_preserves_existing_destination_and_partial() -> Result<(), Box<dyn Error>>
{
    use crate::{FtpEngine, ManagedDownloadControl, ManagedDownloadEngine, ftp::FtpEngineConfig};

    let content = b"existing bytes followed by a delayed FTPS transfer".to_vec();
    let server = start_ftps_server(
        content.clone(),
        None,
        "fetchx",
        "secret",
        Duration::ZERO,
        Duration::from_secs(1),
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-cancel.bin");
    let temporary_path = dir.path().join("ftps-cancel.bin.part");
    tokio::fs::write(&destination, b"preserve destination").await?;
    tokio::fs::write(&temporary_path, &content[..12]).await?;
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-cancel.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;
    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            username: Some("fetchx".to_owned()),
            password: Some("secret".to_owned()),
            read_timeout: Duration::from_millis(150),
            tls_handshake_timeout: Duration::from_millis(150),
            additional_root_certificates: vec![server.certificate],
            ..FtpEngineConfig::default()
        },
    );
    let (cancel_tx, cancellation) = watch::channel(false);
    let (_pause_tx, pause) = watch::channel(false);
    let (_limit_tx, connection_limit) = watch::channel(1_usize);
    let control = ManagedDownloadControl {
        cancellation,
        pause,
        connection_limit,
        resource_demand: None,
        bandwidth: BandwidthControl::default(),
    };
    let download = tokio::spawn(async move { engine.download_managed(id, control).await });
    server.data_ready.await?;
    cancel_tx.send(true)?;

    assert!(matches!(
        download.await?,
        Err(DownloadEngineError::Cancelled)
    ));
    assert_eq!(tokio::fs::read(destination).await?, b"preserve destination");
    assert_eq!(tokio::fs::read(temporary_path).await?, &content[..12]);
    server.task.await?;
    Ok(())
}

#[tokio::test]
async fn ftps_resume_uses_rest_and_preserves_partial_until_complete() -> Result<(), Box<dyn Error>>
{
    use crate::{FtpEngine, ftp::FtpEngineConfig};

    let content = b"resume FTPS data without rewriting the prefix".to_vec();
    let prefix_size = 12;
    let server = start_ftps_server(
        content.clone(),
        None,
        "fetchx",
        "secret",
        Duration::ZERO,
        Duration::ZERO,
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-resume.bin");
    let temporary_path = dir.path().join("ftps-resume.bin.part");
    tokio::fs::write(&temporary_path, &content[..prefix_size]).await?;
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-resume.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;
    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            username: Some("fetchx".to_owned()),
            password: Some("secret".to_owned()),
            additional_root_certificates: vec![server.certificate],
            ..FtpEngineConfig::default()
        },
    );

    let (_cancel_tx, cancellation) = watch::channel(false);
    let completed = crate::DownloadEngine::download(&engine, id, cancellation).await?;

    assert_eq!(completed.status(), DownloadStatus::Completed);
    assert_eq!(tokio::fs::read(destination).await?, content);
    assert!(!temporary_path.exists());
    server.task.await?;
    Ok(())
}

#[tokio::test]
async fn ftps_size_mismatch_preserves_destination_and_partial_data() -> Result<(), Box<dyn Error>> {
    use crate::{FtpEngine, ftp::FtpEngineConfig};

    let actual_content = b"changed remote content".to_vec();
    let prefix = b"old prefix";
    let server = start_ftps_server(
        actual_content.clone(),
        Some((actual_content.len() + 8) as u64),
        "fetchx",
        "secret",
        Duration::ZERO,
        Duration::ZERO,
    )
    .await?;
    let database = Database::connect("sqlite::memory:").await?;
    let repository = database.downloads();
    let dir = tempfile::tempdir()?;
    let destination = dir.path().join("ftps-mismatch.bin");
    let temporary_path = dir.path().join("ftps-mismatch.bin.part");
    tokio::fs::write(&destination, b"existing destination").await?;
    tokio::fs::write(&temporary_path, prefix).await?;
    let url = format!("ftps://{}/file.bin", server.address);
    let item = DownloadItem::new(
        DownloadId::new(),
        DownloadSource::new(&url, EngineKind::Ftps)?,
        FileName::new("ftps-mismatch.bin")?,
        destination.clone(),
        Timestamp::from_unix_millis(1),
    );
    let id = item.id();
    repository.create(&item).await?;
    let engine = FtpEngine::with_config(
        repository,
        FtpEngineConfig {
            username: Some("fetchx".to_owned()),
            password: Some("secret".to_owned()),
            additional_root_certificates: vec![server.certificate],
            ..FtpEngineConfig::default()
        },
    );

    let (_cancel_tx, cancellation) = watch::channel(false);
    let result = crate::DownloadEngine::download(&engine, id, cancellation).await;

    assert!(matches!(
        result,
        Err(DownloadEngineError::FtpIntegrityError { .. })
    ));
    assert_eq!(tokio::fs::read(destination).await?, b"existing destination");
    let mut expected_partial = prefix.to_vec();
    expected_partial.extend_from_slice(&actual_content[prefix.len()..]);
    assert_eq!(tokio::fs::read(temporary_path).await?, expected_partial);
    server.task.await?;
    Ok(())
}

/// FTP path traversal: the engine should reject URLs with `..` components
/// before attempting a connection.
#[tokio::test]
async fn ftp_path_traversal_rejected_before_connect() {
    use crate::ftp::parse_ftp_url;
    assert!(matches!(
        parse_ftp_url("ftp://host/pub/../../../etc/passwd"),
        Err(DownloadEngineError::FtpPathTraversal)
    ));
    assert!(matches!(
        parse_ftp_url("ftp://host/safe/../other"),
        Err(DownloadEngineError::FtpPathTraversal)
    ));
}

// ── Security / failure regression tests ──────────────────────────────────────

/// Connection refused: downloading from a port that actively refuses connections
/// must return a `DownloadEngineError` within a bounded time.
#[tokio::test]
async fn http_connection_refused_returns_error_quickly() -> Result<(), Box<dyn Error>> {
    // Use a very short request timeout so the refused-connection error surfaces fast.
    let config = HttpEngineConfig {
        minimum_segmented_file_size: u64::MAX, // force single stream
        ..HttpEngineConfig::default()
    };
    let (database, engine, directory) = setup_engine(config).await?;
    let dest = directory.destination("refused.bin");
    let item = make_item("http://127.0.0.1:1/file", &dest)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_tx, rx) = watch::channel(false);

    let result = tokio::time::timeout(Duration::from_secs(15), engine.download(id, rx)).await;

    assert!(
        result.is_ok(),
        "download did not complete within 15 s timeout"
    );
    assert!(
        result.unwrap().is_err(),
        "expected an error for refused connection"
    );
    Ok(())
}

/// HTTP 404: the engine must surface `DownloadEngineError::HttpStatus(404)`.
/// (This is already partially covered by `http_failures_persist_failed_state_and_safe_error`
/// but an explicit assertion ensures the exact variant is preserved.)
#[tokio::test]
async fn http_404_surfaces_correct_error_variant() -> Result<(), Box<dyn Error>> {
    let (url, server) =
        start_scripted_server(vec![scripted_response(404, &[], b"Not Found")]).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let dest = directory.destination("http-404.bin");
    let item = make_item(&url, &dest)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_tx, rx) = watch::channel(false);

    let result = engine.download(id, rx).await;
    server.await?;

    assert!(
        matches!(result, Err(DownloadEngineError::HttpStatus(404))),
        "expected HttpStatus(404), got: {result:?}"
    );
    assert!(!dest.exists());
    Ok(())
}

/// HTTP 500: the engine must surface `DownloadEngineError::HttpStatus(500)`.
/// 500 is transient, so provide enough responses for retry attempts.
#[tokio::test]
async fn http_500_surfaces_correct_error_variant() -> Result<(), Box<dyn Error>> {
    // 500 is transient — engine will retry 4 times with Retry-After: 0
    let responses = vec![
        scripted_response(500, &[("Retry-After", "0")], b"Server Error"),
        scripted_response(500, &[("Retry-After", "0")], b"Server Error"),
        scripted_response(500, &[("Retry-After", "0")], b"Server Error"),
        scripted_response(500, &[("Retry-After", "0")], b"Server Error"),
    ];
    let (url, server) = start_scripted_server(responses).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let dest = directory.destination("http-500.bin");
    let item = make_item(&url, &dest)?;
    let id = item.id();
    database.downloads().create(&item).await?;
    let (_tx, rx) = watch::channel(false);

    let result = engine.download(id, rx).await;
    server.await?;

    assert!(
        matches!(result, Err(DownloadEngineError::HttpStatus(500))),
        "expected HttpStatus(500), got: {result:?}"
    );
    assert!(!dest.exists());
    Ok(())
}

/// Credential leak guard: verify that ftp.rs and sftp.rs do not log passwords
/// or credential values via `log::` or `tracing` macros at fields that contain
/// "password", "pass", or "credential". We check the source text in-process.
///
/// This is a static analysis test — it reads the source files as strings and
/// searches for dangerous patterns. It does not require running the engines.
#[test]
fn credential_fields_are_not_logged_in_ftp_or_sftp() {
    // Patterns that would indicate a password field being logged:
    // e.g.  info!("login {} {}", user, password)
    //       warn!(password = %pass, ...)
    let sources = [include_str!("ftp.rs"), include_str!("sftp.rs")];
    for src in &sources {
        let lower = src.to_ascii_lowercase();
        // Find every occurrence of a tracing/log macro call.
        for line in lower.lines() {
            let has_macro = line.contains("info!(")
                || line.contains("warn!(")
                || line.contains("debug!(")
                || line.contains("error!(")
                || line.contains("trace!(");
            if has_macro {
                assert!(
                    !line.contains("password") && !line.contains("passwd"),
                    "Potential credential leak found in source line: {line}"
                );
            }
        }
    }
}

// ── yt-dlp binary-detection test ────────────────────────────────────────────
//
// This test attempts to detect the external yt-dlp binary on PATH. If absent it
// returns early (skip). It does not validate behavior beyond version output.
// NOT VALIDATED — results depend on the CI/developer environment.

/// Detect yt-dlp binary; skip gracefully if absent.
/// NOT VALIDATED — binary presence depends on environment.
#[tokio::test]
async fn ytdlp_binary_detection_skips_when_absent() {
    use std::process::Stdio;
    let Ok(output) = tokio::process::Command::new("yt-dlp")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let version_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        !version_output.trim().is_empty(),
        "yt-dlp --version returned empty output"
    );
}

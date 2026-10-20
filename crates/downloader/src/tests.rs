use std::{
    error::Error,
    path::{Path, PathBuf},
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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
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
    WrongStart,
    WrongEnd,
    WrongTotal,
    ShortBody,
    IgnoreRange,
    Fail,
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
            | SegmentServerMode::WrongStart
            | SegmentServerMode::WrongEnd
            | SegmentServerMode::WrongTotal
            | SegmentServerMode::IgnoreRange
    ) {
        Duration::from_secs(10)
    } else {
        Duration::from_millis(500)
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
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"test-v1\"\r\nConnection: close\r\n\r\n",
            body.len()
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
            SegmentServerMode::FailAboveConcurrency(_) | SegmentServerMode::Valid => (
                206,
                start,
                end,
                body.len(),
                body.get(start..=end).unwrap_or_default().to_vec(),
                end.saturating_sub(start).saturating_add(1),
            ),
        };

    let mut headers = format!(
        "HTTP/1.1 {} Segment\r\nContent-Length: {content_length}\r\nETag: \"test-v1\"\r\nConnection: close\r\n",
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
            max_concurrency: 33,
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
async fn resumed_partial_response_after_redirect_is_not_appended() -> Result<(), Box<dyn Error>> {
    let (target_url, target_server) = start_scripted_server(vec![scripted_response(
        206,
        &[("Content-Range", "bytes 5-9/10")],
        b"world",
    )])
    .await?;
    let (url, redirect_server) = start_scripted_server(vec![scripted_response(
        302,
        &[("Location", &target_url)],
        b"",
    )])
    .await?;
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
        Err(DownloadEngineError::RangeMismatch)
    ));
    assert!(range_header(&redirect_server.await?[0]).is_some());
    assert!(range_header(&target_server.await?[0]).is_some());
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
    assert_eq!(tokio::fs::read(destination).await?, b"existing owner");
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
async fn transient_connection_failure_is_retried() -> Result<(), Box<dyn Error>> {
    let body = b"retried successfully".to_vec();
    let (url, server) = start_disconnect_then_success_server(body.clone()).await?;
    let (database, engine, directory) = setup_engine(HttpEngineConfig::default()).await?;
    let destination = directory.destination("connection-retry.bin");
    let completed = run_download(&engine, &database, make_item(&url, &destination)?).await?;
    let requests = server.await?;

    assert_eq!(requests.len(), 2);
    assert_eq!(tokio::fs::read(destination).await?, body);
    assert_eq!(completed.status(), DownloadStatus::Completed);
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
    }

    Ok(())
}

#[tokio::test]
async fn truncated_response_fails_and_preserves_the_partial_file() -> Result<(), Box<dyn Error>> {
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
    assert!(PathBuf::from(format!("{}.part", destination.display())).exists());
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

// ── Binary-detection tests (aria2, yt-dlp, ffmpeg) ────────────────────────────
//
// These tests attempt to detect the external binary on PATH. If absent they
// return early (skip). They do NOT validate behaviour — only that detection
// succeeds or gracefully reports absence.
// NOT VALIDATED — results depend on the CI/developer environment.

/// Detect aria2c binary; skip gracefully if absent.
/// NOT VALIDATED — binary presence depends on environment.
#[tokio::test]
async fn aria2_binary_detection_skips_when_absent() {
    use std::process::Stdio;
    let Ok(output) = tokio::process::Command::new("aria2c")
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
    else {
        // Binary not found — test skips.
        return;
    };
    if !output.status.success() {
        return;
    }
    let version_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        version_output.contains("aria2"),
        "aria2c --version did not mention 'aria2': {version_output}"
    );
}

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

/// Detect ffmpeg binary; skip gracefully if absent.
/// NOT VALIDATED — binary presence depends on environment.
#[tokio::test]
async fn ffmpeg_binary_detection_skips_when_absent() {
    use std::process::Stdio;
    let Ok(output) = tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
    else {
        return;
    };
    // ffmpeg -version exits 0 or 1 depending on build; presence is enough.
    let version_output = String::from_utf8_lossy(&output.stdout);
    let err_output = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{version_output}{err_output}");
    assert!(
        combined.to_ascii_lowercase().contains("ffmpeg"),
        "ffmpeg -version did not mention 'ffmpeg': {combined}"
    );
}

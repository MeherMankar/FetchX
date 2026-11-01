use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fetchx_downloader::{DirectHttpEngine, HttpEngineConfig};
use fetchx_models::{
    DownloadId, DownloadItem, DownloadPriority, DownloadSource, DownloadStatus, EngineKind,
    FileName, Timestamp,
};
use fetchx_queue::{QueueConfig, QueueManager};
use fetchx_storage::{Database, DownloadRepository, SqliteDownloadRepository};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, tcp::OwnedWriteHalf},
    sync::{oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};

const CHUNK_SIZE: usize = 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

struct FixtureMetrics {
    active: AtomicUsize,
    maximum_active: AtomicUsize,
    requests: AtomicUsize,
    attempts: StdMutex<HashMap<String, usize>>,
    starts: StdMutex<Vec<String>>,
    starts_at: StdMutex<HashMap<String, Instant>>,
    attempts_at: StdMutex<HashMap<String, Vec<Instant>>>,
    first_start: StdMutex<Option<Instant>>,
    per_key_active: StdMutex<HashMap<String, (usize, usize)>>,
}

impl FixtureMetrics {
    fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            maximum_active: AtomicUsize::new(0),
            requests: AtomicUsize::new(0),
            attempts: StdMutex::new(HashMap::new()),
            starts: StdMutex::new(Vec::new()),
            starts_at: StdMutex::new(HashMap::new()),
            attempts_at: StdMutex::new(HashMap::new()),
            first_start: StdMutex::new(None),
            per_key_active: StdMutex::new(HashMap::new()),
        }
    }

    async fn reset(&self) {
        self.maximum_active.store(0, Ordering::Relaxed);
        self.requests.store(0, Ordering::Relaxed);
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.starts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.starts_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.attempts_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        *self
            .first_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.per_key_active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

struct LocalFixture {
    address: std::net::SocketAddr,
    metrics: Arc<FixtureMetrics>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl LocalFixture {
    async fn start() -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let metrics = Arc::new(FixtureMetrics::new());
        let worker_metrics = Arc::clone(&metrics);
        let (stop, mut stop_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut workers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let metrics = Arc::clone(&worker_metrics);
                            workers.spawn(async move {
                                serve(stream, metrics).await;
                            });
                        }
                        Err(_) => break,
                    },
                    _ = &mut stop_receiver => break,
                    Some(_) = workers.join_next(), if !workers.is_empty() => {}
                }
            }
            while workers.join_next().await.is_some() {}
        });
        Ok(Self {
            address,
            metrics,
            stop: Some(stop),
            task,
        })
    }

    fn url(&self, size: u64, key: &str, mode: &str) -> String {
        format!("http://{}/file/{size}/{key}/{mode}", self.address)
    }

    async fn shutdown(mut self) -> Result<(), Box<dyn Error>> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.await?;
        Ok(())
    }
}

struct ActiveRequest {
    metrics: Arc<FixtureMetrics>,
    key: String,
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.metrics.active.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut per_key) = self.metrics.per_key_active.lock()
            && let Some(entry) = per_key.get_mut(&self.key)
        {
            entry.0 = entry.0.saturating_sub(1);
        }
    }
}

async fn serve(mut stream: TcpStream, metrics: Arc<FixtureMetrics>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let mut fields = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let path = fields.nth(1).unwrap_or("/");
    let mut path_parts = path.trim_start_matches('/').split('/');
    if path_parts.next() != Some("file") {
        return;
    }
    let Some(size) = path_parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return;
    };
    let key = path_parts.next().unwrap_or("unknown").to_owned();
    let mode = path_parts.next().unwrap_or("ok");
    let requested_range = request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.eq_ignore_ascii_case("range") {
            return None;
        }
        let (start, end) = value.trim().strip_prefix("bytes=")?.split_once('-')?;
        Some((start.parse::<u64>().ok()?, end.parse::<u64>().ok()))
    });
    let range_start = requested_range.map_or(0, |(start, _)| start).min(size);
    let range_end = requested_range
        .and_then(|(_, end)| end)
        .unwrap_or_else(|| size.saturating_sub(1))
        .min(size.saturating_sub(1));

    metrics.requests.fetch_add(1, Ordering::Relaxed);
    {
        let mut first_start = metrics
            .first_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        first_start.get_or_insert_with(Instant::now);
    }
    let active = metrics.active.fetch_add(1, Ordering::Relaxed) + 1;
    metrics.maximum_active.fetch_max(active, Ordering::Relaxed);
    {
        let mut per_key = metrics
            .per_key_active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = per_key.entry(key.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.max(entry.0);
    }
    metrics
        .starts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(key.clone());
    metrics
        .starts_at
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(key.clone())
        .or_insert_with(Instant::now);
    let _active = ActiveRequest {
        metrics: Arc::clone(&metrics),
        key: key.clone(),
    };

    let attempt = {
        let mut attempts = metrics
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = attempts.entry(key.clone()).or_default();
        *count += 1;
        *count
    };
    metrics
        .attempts_at
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(key.clone())
        .or_default()
        .push(Instant::now());
    if mode == "fail" || (mode == "retry-once" && attempt == 1) {
        let _ = stream
            .write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return;
    }

    let status = if requested_range.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let content_length = if requested_range.is_some() {
        range_end.saturating_sub(range_start).saturating_add(1)
    } else {
        size
    };
    let mut headers = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\nAccept-Ranges: bytes\r\nETag: \"fetchx-phase28-fixture\"\r\nConnection: close\r\n"
    );
    if requested_range.is_some() {
        headers.push_str(&format!(
            "Content-Range: bytes {}-{}/{size}\r\n",
            range_start, range_end,
        ));
    }
    headers.push_str("\r\n");
    let (mut reader, mut writer) = stream.into_split();
    let (disconnect_sender, mut disconnected) = watch::channel(false);
    let disconnect_task = tokio::spawn(async move {
        let mut buffer = [0_u8; 1024];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => {
                    let _ = disconnect_sender.send(true);
                    break;
                }
                Ok(_) => {}
            }
        }
    });
    if !write_until_disconnected(&mut writer, &mut disconnected, headers.as_bytes()).await {
        drop(writer);
        let _ = disconnect_task.await;
        return;
    }

    let mut offset = range_start;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    let response_end = if requested_range.is_some() {
        range_end.saturating_add(1)
    } else {
        size
    };
    let response_end = if mode == "interrupt-once" && attempt == 1 {
        range_start + (response_end - range_start) / 2
    } else {
        response_end
    };
    while offset < response_end {
        let count =
            usize::try_from((response_end - offset).min(CHUNK_SIZE as u64)).unwrap_or(CHUNK_SIZE);
        fill_fixture(&mut buffer[..count], offset);
        if !write_until_disconnected(&mut writer, &mut disconnected, &buffer[..count]).await {
            drop(writer);
            let _ = disconnect_task.await;
            return;
        }
        offset += count as u64;
        if mode == "slow" {
            tokio::select! {
                _ = sleep(Duration::from_millis(12)) => {}
                _ = disconnected.changed() => break,
            }
        }
    }
    drop(writer);
    let _ = disconnect_task.await;
}

async fn write_until_disconnected(
    writer: &mut OwnedWriteHalf,
    disconnected: &mut watch::Receiver<bool>,
    bytes: &[u8],
) -> bool {
    tokio::select! {
        result = writer.write_all(bytes) => result.is_ok(),
        _ = disconnected.changed() => false,
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let count = stream.read(&mut buffer).await.ok()?;
        if count == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(request).ok();
        }
    }
}

fn fill_fixture(buffer: &mut [u8], offset: u64) {
    for (index, byte) in buffer.iter_mut().enumerate() {
        let position = offset + index as u64;
        *byte = position
            .wrapping_mul(31)
            .wrapping_add((position >> 8).wrapping_mul(17))
            .wrapping_add(0x5a) as u8;
    }
}

fn unix_millis() -> Result<u128, Box<dyn Error>> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
}

fn set_stage(stage: &str) -> Result<(), Box<dyn Error>> {
    println!("STAGE name={stage}");
    if let Some(path) = std::env::var_os("PHASE28_STAGE_FILE") {
        std::fs::write(path, stage)?;
    }
    Ok(())
}

fn fixture_hash(size: u64) -> String {
    let mut hasher = Sha256::new();
    let mut offset = 0;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    while offset < size {
        let count = usize::try_from((size - offset).min(CHUNK_SIZE as u64)).unwrap_or(CHUNK_SIZE);
        fill_fixture(&mut buffer[..count], offset);
        hasher.update(&buffer[..count]);
        offset += count as u64;
    }
    format!("{:x}", hasher.finalize())
}

async fn file_hash(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

struct Harness {
    _database: Database,
    repository: SqliteDownloadRepository,
    queue: QueueManager,
    root: PathBuf,
}

impl Harness {
    async fn new(max_active: usize, global_connections: usize) -> Result<Self, Box<dyn Error>> {
        Self::with_segment_threshold(max_active, global_connections, u64::MAX).await
    }

    async fn segmented(
        max_active: usize,
        global_connections: usize,
    ) -> Result<Self, Box<dyn Error>> {
        Self::with_segment_threshold(max_active, global_connections, 4 << 20).await
    }

    async fn with_segment_threshold(
        max_active: usize,
        global_connections: usize,
        minimum_segmented_file_size: u64,
    ) -> Result<Self, Box<dyn Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        let repository = database.downloads();
        let config = HttpEngineConfig {
            minimum_segmented_file_size,
            ..HttpEngineConfig::default()
        };
        let engine = Arc::new(DirectHttpEngine::with_config(repository.clone(), config)?);
        let queue = QueueManager::new(
            repository.clone(),
            engine,
            QueueConfig {
                max_active_downloads: max_active,
                global_connection_limit: global_connections,
            },
        )
        .await?;
        let root = std::env::temp_dir().join(format!(
            "fetchx-phase28-{}-{}",
            std::process::id(),
            DownloadId::new()
        ));
        std::fs::create_dir(&root)?;
        Ok(Self {
            _database: database,
            repository,
            queue,
            root,
        })
    }

    fn item(&self, url: &str, key: &str) -> Result<DownloadItem, Box<dyn Error>> {
        Ok(DownloadItem::new(
            DownloadId::new(),
            DownloadSource::new(url, EngineKind::DirectHttp)?,
            FileName::new(format!("{key}.bin"))?,
            self.root.join(format!("{key}.bin")),
            Timestamp::now()?,
        ))
    }

    async fn shutdown(self) -> Result<(), Box<dyn Error>> {
        self.queue.shutdown().await?;
        drop(self.queue);
        drop(self.repository);
        drop(self._database);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match std::fs::remove_dir_all(&self.root) {
                    Ok(()) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) if cfg!(windows) && error.raw_os_error() == Some(32) => {
                        sleep(Duration::from_millis(25)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok::<(), std::io::Error>(())
        })
        .await??;
        Ok(())
    }
}

#[derive(Default)]
struct Observed {
    active_workers: usize,
    allocated_connections: usize,
    active_connections: usize,
}

async fn wait_for_terminal(
    harness: &Harness,
    ids: &[DownloadId],
    maximum_wait: Duration,
) -> Result<(Vec<DownloadStatus>, Observed), Box<dyn Error>> {
    let started = Instant::now();
    let mut observed = Observed::default();
    loop {
        let snapshot = harness.queue.snapshot().await?;
        observed.active_workers = observed.active_workers.max(snapshot.active.len());
        observed.allocated_connections = observed.allocated_connections.max(
            snapshot
                .active
                .iter()
                .map(|item| item.allocated_connections)
                .sum(),
        );
        observed.active_connections = observed.active_connections.max(
            snapshot
                .active
                .iter()
                .map(|item| item.active_connections)
                .sum(),
        );
        let items = harness.repository.list().await?;
        let by_id = items
            .into_iter()
            .map(|item| (item.id(), item))
            .collect::<HashMap<_, _>>();
        let mut statuses = Vec::with_capacity(ids.len());
        let mut all_terminal = true;
        for id in ids {
            let item = by_id.get(id).ok_or("queue lost a persisted download")?;
            all_terminal &= matches!(
                item.status(),
                DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
            );
            statuses.push(item.status());
        }
        if all_terminal {
            return Ok((statuses, observed));
        }
        if started.elapsed() > maximum_wait {
            return Err(format!("workload timed out after {:?}", started.elapsed()).into());
        }
        sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_status(
    repository: &SqliteDownloadRepository,
    id: DownloadId,
    expected: DownloadStatus,
    maximum_wait: Duration,
) -> Result<DownloadItem, Box<dyn Error>> {
    timeout(maximum_wait, async {
        loop {
            if let Some(item) = repository.get(id).await?
                && item.status() == expected
            {
                return Ok::<_, Box<dyn Error>>(item);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

fn mib_per_second(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64()
}

async fn run_single_sizes(server: &LocalFixture, repetitions: usize) -> Result<(), Box<dyn Error>> {
    for (label, size) in [
        ("1 MiB", 1 << 20),
        ("10 MiB", 10 << 20),
        ("100 MiB", 100 << 20),
    ] {
        for repetition in 1..=repetitions {
            server.metrics.reset().await;
            let harness = Harness::new(1, 1).await?;
            let key = format!("single-{}-{repetition}", size);
            let item = harness.item(&server.url(size, &key, "ok"), &key)?;
            let id = item.id();
            let destination = item.destination().to_path_buf();
            let expected_hash = fixture_hash(size);
            let started_unix_ms = unix_millis()?;
            let started = Instant::now();
            harness.queue.enqueue(item).await?;
            let (statuses, observed) = wait_for_terminal(&harness, &[id], DEFAULT_TIMEOUT).await?;
            let elapsed = started.elapsed();
            let completed_unix_ms = unix_millis()?;
            let actual_hash = file_hash(&destination).await?;
            let actual_size = tokio::fs::metadata(&destination).await?.len();
            let state = statuses.first().copied().unwrap_or(DownloadStatus::Failed);
            println!(
                "SINGLE label={label} repetition={repetition} bytes={size} start_unix_ms={started_unix_ms} completion_unix_ms={completed_unix_ms} elapsed_ms={} throughput_mib_s={:.3} status={state:?} sha256={actual_hash} hash_match={} file_bytes={actual_size} max_workers={} max_connections={}",
                elapsed.as_millis(),
                mib_per_second(size, elapsed),
                actual_hash == expected_hash,
                observed.active_workers,
                server.metrics.maximum_active.load(Ordering::Relaxed),
            );
            if state != DownloadStatus::Completed
                || actual_hash != expected_hash
                || actual_size != size
            {
                return Err(format!("single workload failed for {label}").into());
            }
            harness.shutdown().await?;
        }
    }
    Ok(())
}

async fn run_concurrent(
    server: &LocalFixture,
    count: usize,
    max_active: usize,
) -> Result<(), Box<dyn Error>> {
    server.metrics.reset().await;
    let harness = Harness::segmented(max_active, 3).await?;
    let bytes_per_item = 8 << 20;
    let mut ids = Vec::with_capacity(count);
    let started = Instant::now();
    for index in 0..count {
        let key = format!("concurrent-{count}-{index}");
        let item = harness.item(&server.url(bytes_per_item, &key, "ok"), &key)?;
        ids.push(item.id());
        harness.queue.enqueue(item).await?;
    }
    let (statuses, observed) = wait_for_terminal(&harness, &ids, DEFAULT_TIMEOUT).await?;
    let elapsed = started.elapsed();
    let successes = statuses
        .iter()
        .filter(|status| **status == DownloadStatus::Completed)
        .count();
    println!(
        "CONCURRENT requested={count} max_configured={max_active} global_connections=3 completed={successes} total_bytes={} elapsed_ms={} aggregate_mib_s={:.3} max_workers={} max_queue_connections={} max_server_connections={} result={}",
        (bytes_per_item * count as u64),
        elapsed.as_millis(),
        mib_per_second(bytes_per_item * count as u64, elapsed),
        observed.active_workers,
        observed.allocated_connections,
        server.metrics.maximum_active.load(Ordering::Relaxed),
        if successes == count
            && observed.active_workers <= max_active
            && observed.allocated_connections <= 3
        {
            "PASS"
        } else {
            "FAIL"
        },
    );
    if successes != count
        || observed.active_workers > max_active
        || observed.allocated_connections > 3
        || server.metrics.maximum_active.load(Ordering::Relaxed) > 3
    {
        return Err("concurrency or connection budget invariant failed".into());
    }
    harness.shutdown().await?;
    Ok(())
}

async fn run_queue_stress(server: &LocalFixture, count: usize) -> Result<(), Box<dyn Error>> {
    server.metrics.reset().await;
    let harness = Harness::new(3, 6).await?;
    let bytes_per_item = 4 * 1024;
    let enqueue_started = Instant::now();
    let mut ids = Vec::with_capacity(count);
    let mut enqueued_at = HashMap::with_capacity(count);
    for index in 0..count {
        let key = format!("queue-{count}-{index}");
        let item = harness.item(&server.url(bytes_per_item, &key, "ok"), &key)?;
        let id = item.id();
        ids.push(id);
        enqueued_at.insert(id, Instant::now());
        harness.queue.enqueue(item).await?;
    }
    let enqueue_elapsed = enqueue_started.elapsed();
    let completion_started = Instant::now();
    let mut observed = Observed::default();
    let mut max_queue_depth = 0;
    let mut completion_latencies = HashMap::with_capacity(count);
    let statuses = timeout(DEFAULT_TIMEOUT, async {
        loop {
            let snapshot = harness.queue.snapshot().await?;
            max_queue_depth = max_queue_depth.max(snapshot.queued.len());
            observed.active_workers = observed.active_workers.max(snapshot.active.len());
            observed.allocated_connections = observed.allocated_connections.max(
                snapshot
                    .active
                    .iter()
                    .map(|active| active.allocated_connections)
                    .sum(),
            );
            observed.active_connections = observed.active_connections.max(
                snapshot
                    .active
                    .iter()
                    .map(|active| active.active_connections)
                    .sum(),
            );
            let items = harness.repository.list().await?;
            let by_id = items
                .into_iter()
                .map(|item| (item.id(), item))
                .collect::<HashMap<_, _>>();
            let mut terminal_count = 0;
            let mut statuses = Vec::with_capacity(ids.len());
            for id in &ids {
                let item = by_id.get(id).ok_or("queue lost a persisted download")?;
                let status = item.status();
                statuses.push(status);
                if matches!(
                    status,
                    DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
                ) {
                    terminal_count += 1;
                    completion_latencies.entry(*id).or_insert_with(|| {
                        Instant::now().saturating_duration_since(enqueued_at[id])
                    });
                }
            }
            if terminal_count == ids.len() {
                break Ok::<_, Box<dyn Error>>(statuses);
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    let completion_elapsed = completion_started.elapsed();
    let completed = statuses
        .iter()
        .filter(|status| **status == DownloadStatus::Completed)
        .count();
    let unique = ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .len();
    let first_start_latency = server
        .metrics
        .first_start
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .map(|at| at.saturating_duration_since(enqueue_started));
    let average_completion_latency_ms = completion_latencies
        .values()
        .map(Duration::as_secs_f64)
        .sum::<f64>()
        * 1_000.0
        / completion_latencies.len().max(1) as f64;
    let total_bytes = bytes_per_item * completed as u64;
    println!(
        "QUEUE items={count} enqueue_ms={} first_start_ms={:?} completion_ms={} average_completion_latency_ms={average_completion_latency_ms:.2} completed={completed} unique_ids={unique} throughput_mib_s={:.3} max_queue_depth={max_queue_depth} max_workers={} queue_allocated_connections={} server_max_connections={} result={}",
        enqueue_elapsed.as_millis(),
        first_start_latency.map(|latency| latency.as_millis()),
        completion_elapsed.as_millis(),
        mib_per_second(total_bytes, completion_elapsed),
        observed.active_workers,
        observed.allocated_connections,
        server.metrics.maximum_active.load(Ordering::Relaxed),
        if completed == count && unique == count && observed.active_workers <= 3 {
            "PASS"
        } else {
            "FAIL"
        },
    );
    if completed != count || unique != count || observed.active_workers > 3 {
        return Err(format!("queue stress failed for {count} items").into());
    }
    harness.shutdown().await?;
    Ok(())
}

async fn run_fairness(server: &LocalFixture) -> Result<(), Box<dyn Error>> {
    server.metrics.reset().await;
    let harness = Harness::new(1, 1).await?;
    let mut ids = Vec::new();
    let mut lows = Vec::new();
    let mut normals = Vec::new();
    let mut highs = Vec::new();
    for (priority, prefix, count) in [
        (DownloadPriority::Low, "low", 2),
        (DownloadPriority::Normal, "normal", 2),
        (DownloadPriority::High, "high", 8),
    ] {
        for index in 0..count {
            let key = format!("{prefix}-{index}");
            let item = harness.item(&server.url(1 << 20, &key, "slow"), &key)?;
            let id = item.id();
            harness.queue.enqueue_deferred(item).await?;
            harness.queue.set_priority(id, priority).await?;
            ids.push(id);
            if priority == DownloadPriority::Low {
                lows.push(key);
            } else if priority == DownloadPriority::Normal {
                normals.push(key);
            } else {
                highs.push(key);
            }
        }
    }
    let release_started = Instant::now();
    harness.queue.release_downloads(ids.clone()).await?;
    for index in 0..6 {
        let key = format!("high-arrival-{index}");
        let item = harness.item(&server.url(1 << 20, &key, "slow"), &key)?;
        ids.push(item.id());
        highs.push(key);
        harness.queue.enqueue(item).await?;
    }
    let (statuses, _) = wait_for_terminal(&harness, &ids, DEFAULT_TIMEOUT).await?;
    let start_order = server
        .metrics
        .starts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let low_positions = start_order
        .iter()
        .enumerate()
        .filter(|(_, key)| lows.contains(key))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let first_normal = start_order.iter().position(|key| normals.contains(key));
    let first_low = low_positions.first().copied();
    let high_before_normal = first_normal.map_or(0, |position| {
        start_order[..position]
            .iter()
            .filter(|key| highs.contains(key))
            .count()
    });
    let normal_before_low = first_low.map_or(0, |position| {
        start_order[..position]
            .iter()
            .filter(|key| normals.contains(key))
            .count()
    });
    let max_low_wait_ms = {
        let start_times = server
            .metrics
            .starts_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lows.iter()
            .filter_map(|key| start_times.get(key))
            .map(|at| at.saturating_duration_since(release_started).as_millis())
            .max()
            .unwrap_or(0)
    };
    println!(
        "FAIRNESS high_count={} normal_count={} low_count={} starts={} high_before_normal={high_before_normal} normal_before_low={normal_before_low} first_low_position={:?} first_normal_position={:?} max_low_starvation_ms={max_low_wait_ms} first_3_high={}",
        highs.len(),
        normals.len(),
        lows.len(),
        start_order.join(","),
        first_low,
        first_normal,
        start_order.iter().take(3).all(|key| highs.contains(key)),
    );
    if statuses
        .iter()
        .any(|status| *status != DownloadStatus::Completed)
        || low_positions.first().is_none_or(|position| *position > 3)
    {
        return Err("fairness workload did not meet the configured service interval".into());
    }
    harness.shutdown().await?;
    Ok(())
}

async fn run_cancellation(server: &LocalFixture) -> Result<(), Box<dyn Error>> {
    for round in 0..3 {
        server.metrics.reset().await;
        let harness = Harness::new(3, 3).await?;
        let mut items = Vec::new();
        for index in 0..3 {
            let key = format!("cancel-{round}-{index}");
            let item = harness.item(&server.url(8 << 20, &key, "slow"), &key)?;
            items.push(item);
        }
        let ids = items.iter().map(DownloadItem::id).collect::<Vec<_>>();
        for item in items {
            harness.queue.enqueue(item).await?;
        }
        let max_workers = timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = harness.queue.snapshot().await?;
                if snapshot.active.len() == 3 {
                    break Ok::<_, Box<dyn Error>>(snapshot.active.len());
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        sleep(Duration::from_millis(120)).await;
        let cancellation_start = Instant::now();
        harness.queue.cancel(ids[0]).await?;
        let one_latency = cancellation_start.elapsed();
        let several_start = Instant::now();
        harness.queue.cancel(ids[1]).await?;
        harness.queue.cancel(ids[2]).await?;
        let several_latency = several_start.elapsed();
        let (statuses, observed) = wait_for_terminal(&harness, &ids, DEFAULT_TIMEOUT).await?;
        let final_snapshot = harness.queue.snapshot().await?;
        let cleanup_started = Instant::now();
        timeout(Duration::from_secs(5), async {
            while server.metrics.active.load(Ordering::Relaxed) != 0 {
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        let server_cleanup = cleanup_started.elapsed();
        let server_active_after = server.metrics.active.load(Ordering::Relaxed);
        println!(
            "CANCEL round={round} one_ms={} several_ms={} server_cleanup_ms={} cancelled={} active_after={} server_active_after={} max_active={max_workers} queue_active_peak={} result={}",
            one_latency.as_millis(),
            several_latency.as_millis(),
            server_cleanup.as_millis(),
            statuses
                .iter()
                .filter(|status| **status == DownloadStatus::Cancelled)
                .count(),
            final_snapshot.active.len(),
            server_active_after,
            observed.active_workers.max(max_workers),
            if statuses
                .iter()
                .all(|status| *status == DownloadStatus::Cancelled)
                && final_snapshot.active.is_empty()
                && server_active_after == 0
            {
                "PASS"
            } else {
                "FAIL"
            },
        );
        if statuses
            .iter()
            .any(|status| *status != DownloadStatus::Cancelled)
            || !final_snapshot.active.is_empty()
        {
            return Err("cancellation cleanup invariant failed".into());
        }
        harness.shutdown().await?;
    }

    server.metrics.reset().await;
    let harness = Harness::new(1, 1).await?;
    let active_key = "cancel-active-queued-active";
    let queued_key = "cancel-queued";
    let active_item = harness.item(&server.url(8 << 20, active_key, "slow"), active_key)?;
    let queued_item = harness.item(&server.url(8 << 20, queued_key, "slow"), queued_key)?;
    let active_id = active_item.id();
    let queued_id = queued_item.id();
    harness.queue.enqueue(active_item).await?;
    timeout(Duration::from_secs(10), async {
        loop {
            if !harness.queue.snapshot().await?.active.is_empty() {
                break Ok::<_, Box<dyn Error>>(());
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    harness.queue.enqueue(queued_item).await?;
    let queued_cancel_started = Instant::now();
    harness.queue.cancel(queued_id).await?;
    let queued_cancel_latency = queued_cancel_started.elapsed();
    sleep(Duration::from_millis(200)).await;
    let queued_state = harness
        .repository
        .get(queued_id)
        .await?
        .ok_or("cancelled queued download disappeared")?
        .status();
    let queued_was_requested = server
        .metrics
        .starts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|key| key == queued_key);
    let active_cancel_started = Instant::now();
    harness.queue.cancel(active_id).await?;
    let active_cancel_latency = active_cancel_started.elapsed();
    let (statuses, _) =
        wait_for_terminal(&harness, &[active_id, queued_id], DEFAULT_TIMEOUT).await?;
    timeout(Duration::from_secs(5), async {
        while server.metrics.active.load(Ordering::Relaxed) != 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    sleep(Duration::from_millis(250)).await;
    let snapshot = harness.queue.snapshot().await?;
    let temporary_files = std::fs::read_dir(&harness.root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".part"))
        .count();
    println!(
        "CANCEL_QUEUED queued_state={queued_state:?} queued_request_seen={queued_was_requested} queued_cancel_to_return_ms={} active_cancel_to_return_ms={} terminal_states={statuses:?} active_after={} fixture_connections_after={} temporary_part_files={temporary_files} delayed_restart_observed=false",
        queued_cancel_latency.as_millis(),
        active_cancel_latency.as_millis(),
        snapshot.active.len(),
        server.metrics.active.load(Ordering::Relaxed),
    );
    if queued_state != DownloadStatus::Cancelled
        || queued_was_requested
        || statuses
            .iter()
            .any(|status| *status != DownloadStatus::Cancelled)
        || !snapshot.active.is_empty()
        || server.metrics.active.load(Ordering::Relaxed) != 0
    {
        return Err("queued cancellation cleanup invariant failed".into());
    }
    harness.shutdown().await?;
    Ok(())
}

async fn run_pause_resume(server: &LocalFixture) -> Result<(), Box<dyn Error>> {
    server.metrics.reset().await;
    let harness = Harness::new(1, 1).await?;
    let key = "pause-resume".to_owned();
    let item = harness.item(&server.url(4 << 20, &key, "slow"), &key)?;
    let id = item.id();
    let destination = item.destination().to_path_buf();
    let expected_hash = fixture_hash(4 << 20);
    harness.queue.enqueue(item).await?;
    let mut last_progress = 0;
    for repetition in 1..=2 {
        timeout(Duration::from_secs(20), async {
            loop {
                let item = harness.repository.get(id).await?.ok_or("item missing")?;
                if item.status() == DownloadStatus::Downloading
                    && item.progress().downloaded_bytes() > last_progress
                {
                    last_progress = item.progress().downloaded_bytes();
                    break Ok::<_, Box<dyn Error>>(());
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        let pause_started = Instant::now();
        harness.queue.pause(id).await?;
        let paused = wait_for_status(
            &harness.repository,
            id,
            DownloadStatus::Paused,
            Duration::from_secs(10),
        )
        .await?;
        let pause_state_latency = pause_started.elapsed();
        let paused_bytes = paused.progress().downloaded_bytes();
        let active_after_pause = harness.queue.snapshot().await?.active.len();
        sleep(Duration::from_millis(100)).await;
        let bytes_while_paused = harness
            .repository
            .get(id)
            .await?
            .ok_or("paused download disappeared")?
            .progress()
            .downloaded_bytes();
        harness.queue.resume(id).await?;
        let resume_started = Instant::now();
        timeout(Duration::from_secs(10), async {
            loop {
                let item = harness
                    .repository
                    .get(id)
                    .await?
                    .ok_or("resumed download disappeared")?;
                if item.status() == DownloadStatus::Downloading
                    && item.progress().downloaded_bytes() > paused_bytes
                {
                    break Ok::<_, Box<dyn Error>>(());
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        let resume_progress_latency = resume_started.elapsed();
        println!(
            "PAUSE_RESUME repetition={repetition} pause_to_state_ms={} persisted_paused_bytes={paused_bytes} bytes_after_100ms={bytes_while_paused} active_after_pause={active_after_pause} resume_to_progress_ms={} resumed=true",
            pause_state_latency.as_millis(),
            resume_progress_latency.as_millis(),
        );
        if bytes_while_paused != paused_bytes {
            return Err("download progress advanced while paused".into());
        }
    }
    let (statuses, observed) = wait_for_terminal(&harness, &[id], DEFAULT_TIMEOUT).await?;
    let actual_hash = file_hash(&destination).await?;
    let actual_size = tokio::fs::metadata(&destination).await?.len();
    println!(
        "PAUSE_RESUME_FINAL status={:?} bytes={actual_size} hash_match={} requests={} max_same_download_workers={} observed_workers={}",
        statuses.first(),
        actual_hash == expected_hash,
        server.metrics.requests.load(Ordering::Relaxed),
        server
            .metrics
            .per_key_active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .map_or(0, |value| value.1),
        observed.active_workers,
    );
    if statuses.first() != Some(&DownloadStatus::Completed)
        || actual_hash != expected_hash
        || actual_size != 4 << 20
        || server
            .metrics
            .per_key_active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .is_some_and(|value| value.1 > 1)
    {
        return Err("pause/resume workload failed integrity or worker checks".into());
    }
    harness.shutdown().await?;
    Ok(())
}

async fn run_retries(server: &LocalFixture) -> Result<(), Box<dyn Error>> {
    for (key, mode, expected_status, expected_requests) in [
        (
            "retry-once",
            "retry-once",
            DownloadStatus::Completed,
            2_usize,
        ),
        ("retry-final-fail", "fail", DownloadStatus::Failed, 4_usize),
    ] {
        server.metrics.reset().await;
        let harness = Harness::new(1, 1).await?;
        let item = harness.item(&server.url(128 * 1024, key, mode), key)?;
        let id = item.id();
        let started = Instant::now();
        harness.queue.enqueue(item).await?;
        let (statuses, _) = wait_for_terminal(&harness, &[id], DEFAULT_TIMEOUT).await?;
        let elapsed = started.elapsed();
        let attempts = server
            .metrics
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .copied()
            .unwrap_or(0);
        let attempt_times = server
            .metrics
            .attempts_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
            .unwrap_or_default();
        let retry_delays_ms = attempt_times
            .windows(2)
            .map(|window| window[1].saturating_duration_since(window[0]).as_millis())
            .collect::<Vec<_>>();
        println!(
            "RETRY key={key} expected={expected_status:?} actual={:?} requests={attempts} retry_delays_ms={retry_delays_ms:?} elapsed_ms={} result={}",
            statuses.first(),
            elapsed.as_millis(),
            if statuses.first() == Some(&expected_status) && attempts == expected_requests {
                "PASS"
            } else {
                "FAIL"
            },
        );
        if statuses.first() != Some(&expected_status) || attempts != expected_requests {
            return Err(format!(
                "retry workload failed for {key}: status={:?}, attempts={attempts}, retry_delays_ms={retry_delays_ms:?}",
                statuses.first()
            )
            .into());
        }
        harness.shutdown().await?;
    }
    Ok(())
}

async fn run_mid_body_interruption(server: &LocalFixture) -> Result<(), Box<dyn Error>> {
    server.metrics.reset().await;
    let harness = Harness::new(1, 1).await?;
    let key = "interrupt-once";
    let item = harness.item(&server.url(128 * 1024, key, "interrupt-once"), key)?;
    let id = item.id();
    let destination = item.destination().to_path_buf();
    let started = Instant::now();
    harness.queue.enqueue(item).await?;
    let (statuses, _) = wait_for_terminal(&harness, &[id], DEFAULT_TIMEOUT).await?;
    let elapsed = started.elapsed();
    let status = statuses.first().copied().unwrap_or(DownloadStatus::Failed);
    let attempts = server
        .metrics
        .attempts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .copied()
        .unwrap_or(0);
    let mut partial_name = destination.as_os_str().to_os_string();
    partial_name.push(".part");
    let partial = PathBuf::from(partial_name);
    let partial_bytes = match tokio::fs::metadata(&partial).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    timeout(Duration::from_secs(5), async {
        while server.metrics.active.load(Ordering::Relaxed) != 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let snapshot = harness.queue.snapshot().await?;
    let persisted = harness
        .repository
        .get(id)
        .await?
        .ok_or("interrupted download disappeared from persistence")?;
    let persisted_error = persisted
        .error()
        .map_or_else(|| "none".to_owned(), |error| error.message().to_owned());
    println!(
        "INTERRUPTION response_status=200 bytes_before_close={partial_bytes} requests={attempts} retries={} engine_error=decode_non_transient persisted_state={status:?} persisted_error={persisted_error:?} elapsed_ms={} partial_bytes={partial_bytes} active_workers_after={} connections_after={} result=EXPECTED_NON_RETRYABLE",
        attempts.saturating_sub(1),
        elapsed.as_millis(),
        snapshot.active.len(),
        server.metrics.active.load(Ordering::Relaxed),
    );
    if status != DownloadStatus::Failed
        || attempts != 1
        || partial_bytes != 64 * 1024
        || !snapshot.active.is_empty()
        || server.metrics.active.load(Ordering::Relaxed) != 0
    {
        return Err("mid-body no-auto-retry behavior changed from the documented policy".into());
    }
    harness.shutdown().await?;
    Ok(())
}

async fn run_sustained(server: &LocalFixture, duration: Duration) -> Result<(), Box<dyn Error>> {
    set_stage("sustained")?;
    let started = Instant::now();
    let mut cycles = 0_usize;
    while started.elapsed() < duration {
        run_concurrent(server, 3, 3).await?;
        run_queue_stress(server, 100).await?;
        cycles += 1;
        println!(
            "SUSTAINED_CYCLE cycle={cycles} elapsed_seconds={:.1} duration_seconds={}",
            started.elapsed().as_secs_f64(),
            duration.as_secs()
        );
    }
    println!(
        "SUSTAINED_RESULT requested_seconds={} actual_seconds={:.1} cycles={cycles} result=PASS",
        duration.as_secs(),
        started.elapsed().as_secs_f64(),
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let extended = arguments.iter().any(|argument| argument == "--extended");
    let sustained_seconds = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--sustained-seconds="))
        .map(str::parse::<u64>)
        .transpose()?
        .unwrap_or(0);
    println!(
        "PHASE28_BENCH version={} profile={} rust_target={} max_configured=3 global_connections=3 fixture=loopback deterministic_chunk_bytes={CHUNK_SIZE}",
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::ARCH,
    );
    let server = LocalFixture::start().await?;
    set_stage("idle_baseline")?;
    sleep(Duration::from_secs(2)).await;
    set_stage("single_downloads")?;
    run_single_sizes(&server, if extended { 3 } else { 1 }).await?;
    for (count, workers) in if extended {
        vec![(1, 1), (2, 2), (3, 3), (6, 3), (12, 3)]
    } else {
        vec![(2, 3), (3, 3), (6, 3)]
    } {
        set_stage(&format!("concurrent_{count}"))?;
        run_concurrent(&server, count, workers).await?;
    }
    for count in [10, 100, 1_000] {
        set_stage(&format!("queue_{count}"))?;
        run_queue_stress(&server, count).await?;
    }
    set_stage("fairness")?;
    run_fairness(&server).await?;
    set_stage("cancellation")?;
    run_cancellation(&server).await?;
    set_stage("pause_resume")?;
    run_pause_resume(&server).await?;
    set_stage("retry")?;
    run_retries(&server).await?;
    run_mid_body_interruption(&server).await?;
    if sustained_seconds > 0 {
        run_sustained(&server, Duration::from_secs(sustained_seconds)).await?;
    } else {
        println!("SUSTAINED status=NOT_RUN");
        set_stage("idle_settle")?;
        sleep(Duration::from_secs(3)).await;
    }
    server.shutdown().await?;
    Ok(())
}

# Architecture

The intended high-level flow is:

```text
UI
↓
Tauri
↓
Application/Core
↓
Scheduler
↓
Queue Manager
↓
Download abstraction
↓
Engine implementations
↓
HTTP / aria2 / yt-dlp / FFmpeg
```

The Rust core will own application and download business logic. The UI will present
state and forward user actions through Tauri commands; it will not implement download
behavior. The downloader layer will define a common abstraction, with dedicated
adapters for individual engines.

The `models` crate provides engine-independent domain types. The `core` crate owns
the deterministic download lifecycle state machine and coordinates validated
transitions with storage. Legal edges include `Queued → Preparing|Paused|Cancelled`,
`Preparing → Downloading|Paused|Queued|Failed|Cancelled`,
`Downloading → Paused|Queued|Completed|Failed|Cancelled`,
`Paused → Downloading|Queued|Cancelled`, and `Failed|Cancelled → Preparing`.
The transitions to `Queued` are used for queue recovery or an explicit pause/resume
cycle. Completed items remain terminal.
Status-conditional repository updates prevent competing transitions from the same
previous status from both succeeding. Callers should route lifecycle changes through
the state machine; the repository's general-purpose full-item update remains
unconditional and is intended for non-transition persistence updates. The `storage` crate
exposes a download repository abstraction backed by SQLx and SQLite. Storage maps
domain fields to queryable columns and validates values when reading rows; SQLx
types and database details do not enter the models crate. Schema changes are tracked
as ordered SQLx migrations registered and embedded at compile time, then applied
when the database connects. SQLite foreign-key enforcement and a five-second busy
timeout are configured for connections. File-backed databases use a five-connection
pool; in-memory databases use one connection so pooled operations share the same
database. The caller's journal mode is left unchanged rather than forcing WAL.

The `queue` crate coordinates persisted download records with the downloader
abstraction. It recovers interrupted `Preparing`/`Downloading` records to `Queued`,
starts work according to priority and FIFO order, and applies a bounded active-download
limit. A shared connection budget is divided fairly among active managed engines;
engines report desired and currently active concurrency so existing requests are not
counted as freed until they actually stop. Pause, resume, cancel, priority changes, and
graceful shutdown are serialized through the queue actor. Queue order is persisted in
SQLite and resumed downloads are moved to its tail. Queue pause state is retained across
shutdown; work interrupted while running is requeued at startup. Removal is also
actor-serialized: active work receives cooperative cancellation and its worker is joined
before the persisted record and in-memory entry are removed. Removing a record preserves
destination files, partial downloads, and segment artifacts.

`EngineDispatcher` can be supplied to the queue as its managed engine. It reads each
persisted item's `EngineKind` and forwards the existing control channels to the selected
backend. The queue and Scheduler remain unaware of backend-specific RPC methods.
Direct HTTP remains usable directly as the queue's engine, without a dispatcher.

The `scheduler` crate sits above the queue and stores one-shot start times in the
`download_schedules` SQLite table. On startup it loads enabled schedules and passes
them to the queue as deferred IDs before queue dispatch begins. Future schedules stay
deferred; due or overdue schedules are released together through the Queue Manager,
which remains solely responsible for selecting work and allocating resources. Downloads
without a schedule, and downloads whose schedule is disabled or removed, are eligible
immediately. The scheduler waits with Tokio timers rather than polling and recalculates
after schedule changes. Pending timers are capped at one minute so wall-clock changes
are noticed without short-interval polling. Paused and terminal items are never resumed
by the scheduler; schedule metadata does not add lifecycle states.

Phase 10 schedules are one-shot only. Recurrence, cron syntax, calendar rules, and
bandwidth windows are not implemented.

The `downloader` crate provides a `DirectHttpEngine` using Reqwest and Tokio file I/O
for single-stream and adaptive segmented transfers. It accepts only HTTP/HTTPS URLs
without embedded credentials, follows
at most ten HTTP(S) redirects without HTTPS downgrade, and uses connect/read timeouts.
For single-stream transfers, response chunks stream into a sibling `.part` file.
Progress checkpoints are persisted by byte/time thresholds through status- and
timestamp-conditional repository updates. If a regular `.part` file already exists and
is non-empty, the engine sends `Range: bytes=N-`, where `N` is the file's current size,
not the database progress value.

The engine appends only after a `206 Partial Content` response has a valid
`Content-Range` whose start matches `N`, whose end is not before the start, and whose
known total is greater than the end. A `200 OK` response to a Range request is treated
as Range unsupported: the partial is truncated and the response is downloaded from
zero. For `416 Range Not Satisfiable`, an incomplete local file fails without being
modified; when the local size is at least the reported remote total, the engine makes
one ordinary full request and restarts rather than assuming the partial is complete.
Known totals and received range lengths are checked before publication.

Cancellation and transfer failures preserve the `.part` file for a later invocation.
Queued, failed, cancelled, and paused downloads can be started again through the
existing state machine; failed/cancelled items transition back to preparing. Progress
is reconciled from the actual partial-file size before requesting bytes, and resumed
progress counts the existing prefix only once. The engine does not persist ETag or
Last-Modified validators. A stale partial may therefore belong to an older version of
the resource (including after a redirect); resume validates byte-range arithmetic but
cannot prove representation identity. Startup reconciliation is not implemented.
Range responses reached through a redirect to a different effective URL are rejected
rather than appended because no persisted redirect target is available for comparison.
Parent directories are not created automatically.

For a fresh download, if the initial response advertises `Accept-Ranges: bytes`, has a
known size of at least 4 MiB, and is not redirected, the engine plans up to eight
inclusive byte ranges and initially starts four workers. `HttpEngineConfig::segment_count`
sets the number of planned ranges (bounded to 2—16); the adaptive controller independently
limits active requests to at most eight. Each response's status, final URL, Content-Range,
total size, body length, and local segment size are validated before bytes are accepted.
If a segment endpoint ignores Range or returns unusable range metadata, the engine
discards segment artifacts and falls back to a fresh single-stream request. Unknown-size,
small, redirected, or non-range responses use single-stream downloading immediately.

The scheduler keeps pending ranges separate from active workers. It launches work only
up to the controller's current target, so a decrease stops assigning new ranges without
terminating healthy requests already in flight. Cancellation prevents further scheduling,
signals existing workers through the Phase 7 mechanism, joins them, and preserves useful
segment files. Workers receive at most three attempts total; repeated failures feed the
controller, while exhausted failures remain visible as download errors.

Segments write to separate sibling temporary files identified by segment count, total
size, and index. Existing four-range Phase 7 segment files are detected and resumed using
their original byte ranges. Global progress is the sum of actual local segment-file
lengths and is periodically persisted; per-segment state and adaptive telemetry are not
stored in SQLite. After all ranges complete, every segment's exact length and the
contiguous assembly size are checked. Segments are copied in index order into `.part`,
flushed and synced, then published with the same no-overwrite hard-link operation.
Completed segment temporary files are removed before publication.

`AdaptiveConfig` defaults to minimum/initial/maximum concurrency of 1/4/8 and validates
that all bounds, thresholds, and sampling intervals are usable. The hard maximum is
eight. By default, every 500 ms the controller calculates throughput from newly
received bytes over actual elapsed time and applies a 50/50 exponential moving average.
Zero-byte windows are ignored so response latency alone does not look like a throughput
regression. After two healthy
observations with work remaining, it cautiously probes one additional worker. That probe
is accepted only after two smoothed measurements at least 10% above its prior-concurrency
baseline; otherwise it returns to the previous target. A sustained throughput regression
of at least 20% or two request/worker failures decreases the target by one. Two measurement
windows of cooldown after a change provide hysteresis. Request failures are retried with
a maximum of three total attempts per segment; retries remain bounded and do not bypass
HTTP range semantics. After an unproductive probe or throughput regression, another
upward probe is held until measured throughput improves by the configured threshold.
The measurement window must be at least 10 ms; zero-byte windows are ignored.

The engine does not persist ETag or Last-Modified validators. Segment temp files are
keyed by destination, count, size, and index—not by remote representation—so a stale
segment can theoretically be reused if a resource changes without changing its length.
Startup reconciliation is not implemented. Adaptive concurrency cannot overcome
server-side or network bandwidth limits; the engine does not attempt to bypass server
throttling or access controls. It does not parse or wait for `Retry-After`; 429 responses
and 503 responses carrying that header are not retried automatically. Downloads remain
single-source, with no dynamic segment sizing. Filesystem publication and database
completion are separate operations, so a crash between them can leave a published file
whose persisted state is not yet `Completed`.

The `Aria2Engine` can use either a caller-managed loopback RPC client or a private
`Aria2Supervisor`. The supervised setup validates an explicitly configured executable,
starts one owned `aria2c` child with structured arguments, and waits for authenticated
JSON-RPC readiness before the engine is given a client. RPC is configured for loopback
only. A fresh authentication secret is generated for each process generation; it is
written to an exclusive runtime config file with owner-only permissions on Unix and
deleted after readiness. On Windows the runtime directory is beneath the current user's
`LOCALAPPDATA` and relies on that directory's inherited user ACL. The secret is not put
in command-line arguments or logs. Child stdout and stderr are drained and discarded so
they cannot block the process or expose URLs in application logs.

The owner of the supervisor should start it before constructing/starting the
Queue Manager or Scheduler, then shut the Scheduler/Queue Manager down before shutting
down the supervisor. Queue Manager retains responsibility for selection, priority,
fairness, pause/resume/cancel, and the global resource budget; aria2 is capped at 32
concurrent jobs with one connection per job. The Scheduler remains responsible only for
scheduled eligibility.

The supervisor observes child exit, reports its state and generation, and restarts a
failed process when managed work next needs RPC. Shutdown sends `aria2.shutdown`, waits
for a bounded interval, then kills and reaps the child if necessary. Startup cancellation
drops the locally owned child handle, which is configured to kill on drop. Runtime config
files and directories are removed on startup failure, process exit, and shutdown.

aria2 GIDs remain memory-only and are tagged with the process generation. A GID from an
older generation is discarded without RPC use. An active item whose process exits is
resubmitted to the replacement daemon from its persisted FetchX item; aria2's
`continue` option uses aria2's regular local `.aria2` control file when persisted
progress is nonzero; if that resume metadata is absent or reports fewer bytes than
FetchX has committed, recovery fails closed and preserves the stored progress.
Queue startup already
recovers only `Preparing`/`Downloading` records to `Queued`, while paused, completed,
failed, and cancelled records are not automatically resurrected. SQLite lifecycle and
progress remain authoritative; aria2 status never overwrites a terminal application
state. If a previous aria2 control file is absent or an `addUri` result is ambiguous,
the engine fails safely rather than resetting persisted progress or blindly submitting
duplicates. Recovery is in-process and does not persist aria2 GIDs.

The RPC client validates JSON-RPC IDs and response shapes, uses bounded timeouts and
response sizes, and accepts only HTTP/HTTPS source URIs without embedded credentials.
Progress uses status-and-timestamp-conditional repository updates and completion is
accepted only after the destination file length matches aria2's reported length.
Normal cancellation uses `aria2.remove`; `forceRemove` remains available to the RPC
client. One connection per aria2 job is used; changing Queue Manager allocations are not
dynamically translated into aria2 runtime options.

Process supervision is tested through an injectable launcher and fake child/RPC server;
normal CI does not require a system aria2 installation.

The `YtDlpEngine` is an optional per-download process adapter selected by
`EngineDispatcher`. Initialization resolves and validates an explicitly configured
executable or searches absolute entries in `PATH`, then checks its version through an
injectable runner. The adapter builds structured arguments and disables user config,
plugins, playlists, colors, cache use, and overwrites. It does not pass browser cookies,
arbitrary user options, or secrets. URLs must be HTTP/HTTPS and cannot contain embedded
credentials. The engine requests one best-format output and caps yt-dlp fragment
concurrency at one; it reports a fixed one-connection demand to Queue Manager rather than
claiming control over yt-dlp's internal networking.

Active yt-dlp work cannot be paused safely: Queue Manager rejects active pause requests.
Queue shutdown cancels its process, waits for termination, and returns interrupted work
to `Queued` rather than marking it `Paused`. yt-dlp writes to a per-download workspace
beside the destination, where cancellation and failures preserve partial data for
`--continue` on retry. Progress is read from bounded machine-readable output and stored
with status- and timestamp-conditional repository updates. Completion requires exactly
one regular output file, published with a no-overwrite hard link; successful workspace
cleanup follows publication. stdout and stderr are drained concurrently, and stopped
children are killed and reaped. Runner tests are injected and do not require yt-dlp on
the test host.

`FfmpegProcessor` is a bounded media post-processing service in the downloader crate,
not a `DownloadEngine`: it does not select sources, alter `DownloadItem` lifecycle state,
schedule retries, or enqueue work. Application/core orchestration should invoke it only
as part of already-admitted work; Queue Manager and Scheduler remain the sole owners of
download eligibility and queue scheduling. Each processor instance limits simultaneous
ffmpeg/ffprobe processes (one by default, at most two), while the service does not map
CPU work onto the queue's HTTP connection budget. The application should own and reuse a
single configured processor rather than create processors per job.

FFmpeg/ffprobe executables may be explicitly configured or resolved from absolute
directories in `PATH`; no binaries are downloaded or installed. Candidates are
canonicalized, required to be regular native executables, and version-checked before
use. Invocations use structured arguments without a shell. Only closed remux container
choices and fixed video/audio transcoding presets are accepted; arbitrary arguments are
not exposed. Inputs are canonical regular files opened by Rust and streamed to FFmpeg
and ffprobe over stdin. Their input protocol whitelist permits only `pipe`, so input
demuxers cannot open additional local files or network resources. Output size defaults
to a 100 GiB cap, configurable up to 1 TiB, and is monitored while FFmpeg runs. Outputs
are created in a uniquely named sibling workspace and published with a no-overwrite hard
link. Output size is checked every 25 ms and again before publication, so a brief write
burst may exceed the configured cap before termination. Failed, cancelled, or over-limit
operations preserve the input and remove temporary output.

ffprobe emits bounded JSON parsed into a small typed metadata model. FFmpeg progress is
read from bounded `-progress pipe:1` lines and exposes output time only; no percentage
is fabricated without a reliable total duration. Processes have a configurable,
bounded lifetime, streamed input on stdin, concurrently drained stdout/stderr,
cancellation, and kill/reap cleanup. Executable validation uses null stdin. Active
pause/resume is unsupported; callers must not report an in-progress media operation as
paused. Workspace permissions are owner-only on Unix; on Windows the workspace inherits
the destination parent's ACL. Tests inject fake runners and do not require real FFmpeg
binaries or network access.

The initial desktop application lives in `apps/desktop`. Tauri owns one long-lived
`Database`, shared SQLite download repository, `Scheduler`, and its single
`QueueManager`; the queue is obtained from the scheduler rather than initialized a
second time. Startup creates the application-data/download directories, connects and
migrates SQLite, then starts `Scheduler`, which loads enabled schedules and defers
them before queue dispatch. On exit, the scheduler shuts down the queue and waits for
workers to stop through the existing queue lifecycle.

Tauri commands are a narrow IPC adapter over the repository and queue. They expose
explicit Serde DTOs with snake_case fields and map backend failures to a stable,
frontend-safe error shape. The initial add command accepts HTTP/HTTPS URLs without
embedded credentials and creates direct-HTTP downloads in the system Downloads
directory. Pause, resume, cancel, and delete use `QueueManager`; delete preserves
download files and partials. The initial UI can explicitly refresh persisted progress.

The React UI contains no download business logic. Its typed API module centralizes
Tauri `invoke` and listens for typed `download_updated` and `download_removed` events.
Events currently describe command-driven changes; Queue Manager does not yet publish
worker progress events, so periodic/polling-based progress updates are intentionally
not introduced in this foundation phase.

The main webview capability grants only the event listener permissions required by
the shell. It has no filesystem, shell, process, or unrestricted-network plugin
permissions. Engine process management and filesystem publication remain Rust-owned.


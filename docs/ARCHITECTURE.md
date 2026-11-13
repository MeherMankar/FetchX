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
HTTP / aria2 / yt-dlp / native media
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

Diagnostics use shared typed models and a bounded in-memory recorder. CoreServices
assembles live queue, engine, and read-only storage health into a JSON snapshot; the
CLI's `diagnostics` command checks an existing database without creating it or applying
migrations. The Android bridge exposes the CoreServices snapshot to Kotlin. Event names,
correlation IDs, and error codes are constrained; snapshots do not contain source URLs,
credentials, database paths, or raw error text. RPC request telemetry uses internal
random request IDs, aggregate outcome counters, and elapsed-time log fields, without
adding a remote diagnostics RPC endpoint. Queue snapshots report current/average
download speed, database snapshots report health-check latency, and torrent upload
totals are nullable when an active engine cannot provide them.

Storage maintenance opens existing databases without creating files or applying
migrations for read-only health checks. Checks report SQLite integrity, foreign-key
violations, required tables and indexes, migration history, and row counts. Before
applying pending migrations to an existing database, storage creates and validates a
SQLite snapshot; unsupported future schemas and inconsistent migration histories are
refused. The CLI exposes `fetchx db check|repair|backup|export|import`. `repair` is
deliberately non-destructive: it reports when a healthy database needs no repair and
refuses automatic repair when diagnostics are unhealthy. Backups are consistent
SQLite snapshots and are validated before success is reported.

Versioned JSON exports contain download records, schedules, and torrent metadata, but
exclude `app_config` so engine credentials and remote-authentication state remain local.
Exports also refuse source URLs, magnet trackers, or torrent metadata that may contain
credentials. Imports validate the complete document, constrain absolute download and
torrent paths (including existing symlink resolution) to the configured downloads root,
normalize interrupted work to queued,
create a validated recovery backup, and replace records in a transaction. Other FetchX
processes must be stopped during import; the Android application has no CLI maintenance
surface and continues to use its app-private database path and shared migration code.

The `remote-protocol` crate defines the transport-independent FetchX JSON-RPC 2.0
envelopes, stable `fetchx.rpc.v1` discovery metadata, request identifiers, supported
download commands, and event notification names. It bounds and validates incoming
JSON and method parameters before server dispatch.

The `remote-server` crate provides the standalone Axum HTTP/JSON-RPC and WebSocket
transport. It initializes the same `CoreServices` graph as the CLI and desktop,
dispatches supported download operations through the repository and queue, and streams
the existing bounded `DownloadEventBus` with a bounded per-client queue. The default
listener is loopback-only. Download responses intentionally omit source URLs,
destination paths, and raw engine diagnostics. Operations without a shared CoreServices
implementation are omitted from discovery or return an explicit unsupported-operation
error.

The server's authentication boundary verifies an HTTP bearer token before JSON-RPC
parsing and dispatch, and verifies WebSocket credentials before event subscription.
Tokens are generated from the OS cryptographic RNG; only a SHA-256 verifier is stored
in the existing SQLite configuration table. The CLI explicitly initializes or rotates
credentials and displays the plaintext token only for that operation. `/health` is
public; all RPC methods, including discovery, and the WebSocket event endpoint require
authentication. Role authorization is enforced server-side for each RPC and event
subscription. TLS is opt-in and fail-closed; browser CORS and WebSocket Origin checks
use exact configured origins. Browser WebSocket clients obtain a short-lived,
single-use event ticket over authenticated JSON-RPC because browser WebSocket APIs
cannot set an Authorization header. Tickets are bounded, event-scoped, and invalidated
by credential rotation.

The Phase 21F React/TypeScript remote web client is a protocol client, not another
application service. It accesses only `/health`, `system.discover`, and advertised
JSON-RPC operations; it does not connect to SQLite, CoreServices, the filesystem, or
engine internals. Views for operations absent from discovery explicitly report that
they are unavailable rather than simulating server state.

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
for single-stream and adaptive segmented transfers. It downloads an explicitly supplied
URL; it does not crawl pages or discover candidate URLs. It accepts only HTTP/HTTPS
URLs without embedded credentials, follows at most ten HTTP(S) redirects without
HTTPS downgrade, and uses connect/read timeouts. For single-stream transfers, response
chunks stream into a sibling `.part` file. Progress checkpoints are persisted by
byte/time thresholds through status- and timestamp-conditional repository updates.

Resume metadata is stored beside the partial as `.part.meta`, including the original
URL and a response validator (a strong ETag when available, otherwise Last-Modified).
An existing non-empty `.part` is resumed only when that metadata matches the requested
URL and includes a validator; the engine then sends `Range: bytes=N-` and `If-Range`,
where `N` is the partial file's actual size, not the database progress value. Without
matching resume metadata, the engine starts a fresh transfer rather than appending
unverified bytes.

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
progress counts the existing prefix only once. A resumed response is appended only
when its range starts at the local size, its validator still matches, and its effective
URL has not changed through a redirect. A `200 OK` to a range request restarts from
zero; an incomplete `416 Range Not Satisfiable` fails without modifying the partial.
The sidecar also records finalization state and expected size so a later invocation can
recover a complete temporary file after a crash during publication. Startup-wide
reconciliation is not implemented. Parent directories are not created automatically.

For a fresh download, if the initial response advertises `Accept-Ranges: bytes`, has a
known size at least `minimum_segmented_file_size` (4 MiB by default), and is not
redirected, the engine plans inclusive byte ranges and initially starts four workers.
`HttpEngineConfig::segment_count` sets the number of planned ranges (2—32; eight by
default); the adaptive controller independently limits active requests according to
its configured maximum (eight by default, at most 32). Each response's status, final
URL, Content-Range, total size, body length, and local segment size are validated before
bytes are accepted.
If a segment endpoint ignores Range or returns unusable range metadata, the engine
discards segment artifacts and falls back to a fresh single-stream request. Unknown-size,
small, redirected, or non-range responses use single-stream downloading immediately.

The scheduler keeps pending ranges separate from active workers. It launches work only
up to the controller's current target, so a decrease stops assigning new ranges without
terminating healthy requests already in flight. Cancellation prevents further scheduling,
signals existing workers through the Phase 7 mechanism, joins them, and preserves useful
segment files. Workers receive at most four attempts total; repeated failures feed the
controller, while exhausted failures remain visible as download errors.

Segments write to separate sibling temporary files identified by segment count, total
size, and index. Existing segment files are resumed using their original byte ranges
only when the saved URL and response validator match. Global progress is the sum of
actual local segment-file lengths and is periodically persisted; per-segment state and
adaptive telemetry are not stored in SQLite. After all ranges complete, every segment's
exact length and the contiguous assembly size are checked. Segments are copied in index
order into `.part`, flushed and synced, then published with the same no-overwrite
hard-link operation. Completed segment temporary files are removed before publication.

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

Resume and segment reuse rely on the server's strong ETag or Last-Modified validator;
they cannot prove representation identity if a server reuses a validator for changed
content. Startup-wide reconciliation is not implemented. Transient connection errors
and HTTP 408, 425, 429, and 5xx responses are retried within a four-attempt, 30-second
budget; a valid `Retry-After` value is honored when it fits that budget. Adaptive
concurrency cannot overcome server-side or network bandwidth limits, and the engine
does not attempt to bypass server throttling or access controls. Downloads remain
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
credentials. The engine requests one best-format output and allows yt-dlp to download up
to sixteen HLS/DASH fragments concurrently. This can improve throughput when the source
supports parallel fragment requests, but cannot exceed source or network bandwidth limits.
It reports a fixed one-connection demand to Queue Manager rather than claiming control
over yt-dlp's internal networking.

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

`FfmpegProcessor` is a bounded native media post-processing service in the downloader crate,
not a `DownloadEngine`: it does not select sources, alter `DownloadItem` lifecycle state,
schedule retries, or enqueue work. Application/core orchestration should invoke it only
as part of already-admitted work; Queue Manager and Scheduler remain the sole owners of
download eligibility and queue scheduling. Each processor instance bounds concurrent
native work, while the service does not map CPU work onto the queue's HTTP connection
budget. The application should own and reuse a single configured processor rather than
create processors per job.

The processor does not discover, install, or launch FFmpeg/ffprobe executables. Supported
operations use FetchX's native MP4, OxideAV, and Symphonia implementations; unsupported
formats and operations return an explicit native-unsupported error. It enforces
cancellation and output-size bounds, and publishes output without overwriting an
existing destination. Broad video transcoding and full FFmpeg feature parity are not
implemented.

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

## Plugin trust and authorization (Phase 23A)

`plugin-api` owns the shared versioned manifest, typed permissions/capabilities,
signature metadata, trust context, and fail-closed evaluator. `plugin-manager`
requires explicit trust and matching per-capability approval before process start,
and checks the plugin's handshake identity, version, and reported capability subset.
Trust approvals are currently in-memory. The Android bridge exposes the same
evaluator, but Android has no plugin execution adapter and therefore supports no
plugin capabilities yet. Neither platform receives OS-level plugin sandboxing from
this policy model; filesystem/network/process isolation is deferred to Phase 23B.
Signature fields are structural metadata only; cryptographic verification is
deferred to Phase 23C.

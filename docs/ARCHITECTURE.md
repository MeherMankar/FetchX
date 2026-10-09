# Architecture

The intended high-level flow is:

```text
UI
↓
Tauri
↓
Application/Core
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
transitions with storage. Legal edges are `Queued → Preparing|Cancelled`,
`Preparing → Downloading|Failed|Cancelled`, `Downloading → Paused|Completed|Failed|Cancelled`,
and `Paused → Downloading|Cancelled`; `Failed|Cancelled → Preparing` is allowed
for an explicit retry. Completed items remain terminal.
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
sets the number of planned ranges (bounded to 2–16); the adaptive controller independently
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

Queue workers, application services, UI, and non-HTTP engine adapters are not
implemented yet.

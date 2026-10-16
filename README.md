# MyDownloader

MyDownloader is the foundation of a cross-platform download manager. The long-term
goal is to provide a desktop application and CLI, with a Rust core that can support
download queues, resumability, external engines, and other download-manager features.

## Development status

This repository currently contains the Rust workspace foundation, engine-independent
domain models, SQLite persistence, a deterministic download lifecycle state machine,
and a direct HTTP/HTTPS downloader with safe Range-based resume and adaptive segmented
transfers. It also includes a persistent download queue with priority/FIFO scheduling,
pause/resume/cancel/removal controls, startup recovery, and a global connection budget
(defaults: three active downloads and 16 connections, bounded to 32 and 64). The first
Tauri 2 desktop foundation is in place with a minimal React + TypeScript shell. It
currently exposes direct HTTP/HTTPS downloads, queue controls, and explicit refresh;
live progress notifications and engine selection are not yet wired to the desktop.
The downloader crate includes an optional FFmpeg/ffprobe media post-processing
service for metadata inspection, remuxing, and a small set of transcoding presets. It
requires externally installed executables configured by path or discovered on `PATH`;
it never installs binaries. The processor bounds concurrent child processes, supports
cancellation but not active pause/resume, streams selected input files over stdin with
only the `pipe` input protocol enabled, caps output at 100 GiB by default (configurable
up to 1 TiB), and does not own download lifecycle or scheduling. Fake runners keep normal
tests independent of FFmpeg. The optional yt-dlp
engine is configured as an external process adapter and uses fake runners in tests; no
installed yt-dlp is required for CI. The optional aria2 engine can use the JSON-RPC
client through the engine dispatcher and a private `aria2c` process owned by
the Rust supervisor. Process supervision requires an explicitly configured executable
path; no shell or PATH-based executable discovery is used. One-shot scheduled start
times are persisted in SQLite and coordinated by an event-driven Scheduler above the
Queue Manager.

Android's embedded yt-dlp runtime also supports selectable media formats. Adaptive
video/audio selections use a bundled, statically linked FFmpeg 8.1.2 executable
(LGPL configuration, without GPL-only external libraries), built for the app's
supported ABIs with `scripts/build-android-ffmpeg.sh`.

The supervised aria2 RPC listener is restricted to loopback and requires a
runtime-generated authentication secret. The secret is supplied through a short-lived
private runtime config file, not process arguments, logs, or persistent application
storage. Child output is drained without logging. GIDs remain memory-only and are scoped
to a process generation; after a crash, active jobs are resubmitted from MyDownloader's
persisted state, while completed and cancelled records are never resurrected. Shutdown
is graceful first, then bounded and forced if required. Each aria2 job uses one
connection; Queue Manager remains authoritative for queueing and global resources.
Supervision tests use injected fake processes and do not require aria2 on the test host.

Resume uses the actual `<destination>.part` length for a single `Range` request and
appends only after validating a `206` response and its `Content-Range`. A `200`
response restarts from zero; cancellation and transfer failures preserve partial data.
Queue startup recovery requeues interrupted downloads, but ETag/Last-Modified validators
and validation of partial files against a remote representation are not implemented,
so a partial may belong to an older resource version.

The Scheduler supports immediate, future, and overdue one-shot starts, with persistent
enable/disable state. It wakes on schedule changes or the next due time and defers
future downloads without using queue slots or connection budget. Once eligible, the
Queue Manager applies priority, FIFO, fairness, and resource limits. Recurring schedules,
calendar rules, and cron syntax are not implemented. Wall-clock changes are rechecked
at least once per minute while a distant schedule is pending.

For fresh responses that advertise byte ranges and provide a known size of at least
4 MiB, the engine plans up to eight ranges and starts with four workers. A bounded
adaptive controller may probe up to eight concurrent requests when measured throughput
supports doing so; it backs off for sustained regression or repeated worker errors.
Smaller, unknown-size, or non-range responses use the existing single-stream path.
If range requests are ignored or return unusable metadata, segment artifacts are
discarded and the download restarts through the single-stream path. Segments are
validated and assembled in order; cancelled/failed segment files can be resumed on a
later invocation, including the prior four-range Phase 7 layout. Segment metadata and
adaptive telemetry are not persisted in SQLite.

By default, adaptive throughput is calculated from actual bytes received over 500 ms
measurement windows and smoothed with a 50/50 exponential moving average. The controller
requires at least a 10% improvement in two observations to accept each one-worker probe, uses a
20% sustained-regression threshold, and waits through two cooldown windows after a
decision. Rejected probes are not repeated until measured throughput at the restored
target improves by at least the configured threshold. Repeated failures trigger a
one-worker reduction; failed segments receive at
most three attempts in total. Zero-byte windows are ignored so response latency alone
does not trigger a throughput backoff. The minimum, initial, and maximum concurrency default to
1, 4, and 8 and can be changed through `HttpEngineConfig::adaptive`, with eight as the
hard maximum.

Adaptive concurrency cannot overcome server-side or network bandwidth limits. The
engine respects range responses and does not attempt to bypass server throttling or
access controls. It does not parse or wait for `Retry-After`; 429 responses and 503
responses carrying that header are not retried automatically.

## Technology stack

- Rust (stable toolchain, edition 2024)
- Tokio for asynchronous operations
- Reqwest for streaming HTTP/HTTPS requests
- Serde and Serde JSON for serialization
- SQLx with SQLite for download persistence
- Tauri 2 for the desktop adapter and React + TypeScript for presentation

The dependency versions and shared configuration are centralized in the root
`Cargo.toml`. Crate manifests opt into dependencies as the corresponding functionality
is implemented.

## Workspace

- `crates/models` — shared domain and data structures
- `crates/core` — application logic and persisted download state transitions
- `crates/storage` — SQLx/SQLite persistence for download records
- `crates/downloader` — direct HTTP/HTTPS engine with resume/adaptive segmentation,
  optional aria2 and yt-dlp engine adapters, FFmpeg/ffprobe media processing, and engine
  dispatcher
- `crates/queue` — persistent queue scheduling, lifecycle control, and global resource allocation
- `crates/scheduler` — persistent one-shot start times and event-driven eligibility handoff
- `apps/desktop/src-tauri` — Tauri commands, shared application services, and lifecycle
- `apps/desktop/ui` — minimal React desktop shell and typed IPC client
- `migrations` — embedded, ordered SQLite schema migrations
- `tests` — workspace-level integration tests
- `docs` — architecture, roadmap, and AI contribution rules

## Build

Install the stable Rust toolchain with `rustup`, then run from the repository root:

```sh
cargo check --workspace
```

## Tests

Run the crate smoke tests and any future workspace tests with:

```sh
cargo test --workspace
```

The storage crate accepts a caller-provided SQLite URL and applies embedded
migrations during connection, for example `sqlite://mydownloader.db`. In-memory
SQLite databases are also supported for tests. The repository API is documented
through the `DownloadRepository` trait; database setup is provided by
`Database::connect` for SQLite URLs and `Database::connect_path` for filesystem
paths.

Build the initial desktop frontend from the repository root with:

```sh
npm --prefix apps/desktop/ui run build
```

The Tauri application uses the system Downloads directory for files and its
application-data directory for SQLite. Its webview has no filesystem, shell,
process, or unrestricted network permissions. The initial capability permits only
the event-listener APIs needed by the UI; download and process operations remain
behind Rust commands.

Before submitting changes, also run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
[docs/ROADMAP.md](docs/ROADMAP.md), and [docs/AI_RULES.md](docs/AI_RULES.md).

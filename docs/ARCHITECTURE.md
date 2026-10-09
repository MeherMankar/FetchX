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
and `Paused → Downloading|Cancelled`; terminal states have no outgoing transitions.
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

The `downloader` crate currently provides a single-stream `DirectHttpEngine` using
Reqwest and Tokio file I/O. It accepts only HTTP/HTTPS URLs without embedded
credentials, follows at most ten HTTP(S) redirects without HTTPS downgrade, and uses
connect/read timeouts. Response chunks stream into a sibling `.part` file; progress
checkpoints are persisted by byte/time thresholds through status-conditional
repository updates. After content-length verification and file sync, a same-directory
hard link publishes the file without replacing an existing destination. Parent
directories are not created automatically. Cancellation uses a Tokio watch receiver,
cleans up the partial file, and transitions through the state machine.

This first engine does not support Range requests, resume, retries, segmented
downloads, speed calculations, or multi-connection transfers. Atomic file publication
and database state changes are separate operations; a process crash between them can
leave a published file whose persisted state is not yet `Completed`.

Queue workers, application services, UI, and non-HTTP engine adapters are not
implemented yet.

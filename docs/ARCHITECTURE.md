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

The `downloader` crate provides a single-stream `DirectHttpEngine` using Reqwest and
Tokio file I/O. It accepts only HTTP/HTTPS URLs without embedded credentials, follows
at most ten HTTP(S) redirects without HTTPS downgrade, and uses connect/read timeouts.
Response chunks stream into a sibling `.part` file; progress checkpoints are persisted
by byte/time thresholds through status- and timestamp-conditional repository updates.
If a regular
`.part` file already exists and is non-empty, the engine sends `Range: bytes=N-`, where
`N` is the file's current size, not the database progress value.

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

Downloads remain single-stream: segmentation, parallel ranges, retries, and speed
calculations are not implemented. After content validation and file sync, a
same-directory hard link publishes the file without replacing an existing destination.
Filesystem publication and database completion are separate operations, so a crash in
between can leave a published file whose persisted state is not yet `Completed`.

Queue workers, application services, UI, and non-HTTP engine adapters are not
implemented yet.

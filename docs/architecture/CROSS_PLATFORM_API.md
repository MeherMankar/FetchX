# FetchX Cross-Platform API

## API principle

There should be one Rust domain contract and one host-neutral operation vocabulary. Current transports may differ, but they must map to the same models, validation, errors, and lifecycle semantics.

```text
Rust domain API / CoreServices
          |
          +-- UniFFI: Android
          +-- Tauri commands + events: desktop
          +-- JSON-RPC 2.0: CLI/server/remote and browser clients
          +-- Optional C ABI for future native hosts
```

`CoreServices` is the current composition root. It already bundles storage, queue, scheduler, engines, configuration, event bus, diagnostics, and built-in URL utilities.

## Stable operation groups

| Operation group | Required behavior |
|---|---|
| Create download | URL/file source, filename, destination, priority, engine, headers, optional checksum |
| Batch admission | Per-input URL/options, optional host-selected absolute destination, independent validation/admission results, shared duplicate/collision handling |
| Add media/torrent | yt-dlp format selection or torrent metadata and file selection |
| Control | Pause, resume, cancel, remove, retry |
| Priority and queue | Change priority, list queue, inspect queue snapshot |
| Concurrency and bandwidth | Read/update global/per-download limits and connection budget |
| Scheduling | Create, replace, enable, disable, and remove a one-shot schedule |
| Categories/naming | Link extraction, URL normalization, filename/package suggestion, duplicate detection, category match |
| History and metadata | List/read download history, schedules, torrent metadata, media formats |
| Capabilities | Discover protocol/engine, runtime, platform, and adapter support |
| Events | Subscribe to typed create/update/remove lifecycle and retry/engine/queue notifications |
| Errors | Explicit stable error family and code, not raw transport text |

## Core domain types

The stable Rust types should include:

- `DownloadId`, `DownloadItem`, `DownloadSource`, `DownloadStatus`, `DownloadPriority`;
- `EngineKind`, capability and availability values;
- schedule, queue snapshot, progress, and category types;
- torrent metadata/selection/telemetry types;
- media format types;
- typed configuration and diagnostics.

UniFFI and Tauri should expose DTOs derived from these types. Kotlin and TypeScript should remain transport models, not independent business definitions.

## Error contract

Every transport should map errors to a stable family:

- invalid request or configuration;
- validation failure;
- not found;
- conflict or invalid state transition;
- unavailable or disabled capability;
- storage failure;
- engine/runtime failure;
- cancellation;
- internal fault.

Host-facing messages should be context-rich in the UI, while machine-readable codes and correlated IDs remain stable. Source URLs, credentials, filesystem paths, and raw engine diagnostics must not leak from remote APIs.

## Android UniFFI

UniFFI is appropriate for Android because it is already the generated Kotlin/FFI boundary.

The `DownloadService` contract exposes download, torrent, media, schedule, built-in, diagnostics, browser-RPC, and Android batch-admission operations. `enqueue_download_batch_with_destinations` is additive: each request carries a URL, filename, absolute destination, engine/media options, and optional SHA-256; each result returns the input URL, optional accepted ID, and optional typed error. Android keeps package selection in each destination path. Kotlin owns Android public-storage staging and publication; Rust owns admission validation, duplicate handling, collision resolution, and queue lifecycle. Rust writes any checksum sidecar before enqueueing. Existing batch operations and desktop behavior are unchanged.

Recommended convergence:

1. Keep operation names and DTO field mappings contract-tested.
2. Generate or verify Kotlin DTO mappings instead of hand-maintaining parallel semantic models.
3. Remove production link/sanitization fallbacks when the Rust bridge is present.
4. Add capability/status, queue-limit, bandwidth-limit, category/history, and richer typed lifecycle operations to the same interface.
5. Map Android priority names to stable Rust priority values without silently dropping `CRITICAL` if it is introduced in shared models.

## Desktop Tauri

Tauri is the desktop host boundary. Commands are acceptable thin adapters around `CoreServices`. Each command should:

- deserialize transport DTOs;
- perform only OS/native-path adaptation;
- call the Rust core;
- map typed errors to a stable IPC error;
- emit core events without re-implementing lifecycle.

Desktop React should treat Tauri events and refresh responses as projections of core state.

## JSON-RPC

`fetchx.rpc.v1` discovery should continue as the remote compatibility surface. Discovery is transport metadata: capability matrices and operation availability belong to the core capability registry and must be included honestly in advertised methods.

Remote policy should restrict destructive operations and never expose SQLite or arbitrary local paths.

## Events

The canonical event stream should include, while avoiding duplicate channels:

- created, started, progress, paused, resumed, completed, failed, cancelled;
- retry started and attempt count;
- queue changed and slot/connection-budget changed;
- schedule changed;
- engine availability or runtime state changed;
- capability configuration changed.

Event projection:

```text
Core DownloadEventBus
    |-- Android callback refresh listener
    |-- Desktop Tauri event bridge
    |-- Remote WebSocket broadcaster
    `-- CLI polling or command result
```

Hosts may coalesce high-frequency progress updates. Core state remains the source of truth and UI snapshots must remain recoverable after missed or lagged events.

## Storage/API compatibility

Add fields and operations without changing existing SQLite migration semantics. Transport DTO versions may evolve independently only when the Rust core maintains backward-compatible serialization. Physical database files are not required to be shared across devices, but logical schema versions and migrations must remain compatible.

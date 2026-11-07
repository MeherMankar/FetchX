# Gopeed Feature Analysis

## Repository and license

- Reference repository: `GopeedLab/gopeed`.
- Reviewed revision: `8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b`.
- Repository license: GNU GPL-3.0.
- No Gopeed source was copied into FetchX. The Apache-2.0 FetchX implementation is an independent design based on documented behavior.

## Selected capability

FetchX selected a compact runtime-status projection inspired by Gopeed's separation between durable task metadata and frequently changing runtime state.

FetchX already exposed progress in full download records. The selected improvement adds one shared-core projection containing only:

- Download ID.
- Lifecycle status.
- Downloaded bytes.
- Optional total bytes.
- Optional speed.
- Optional estimated remaining time.
- Percentage in basis points.
- Updated timestamp.

This avoids repeatedly transferring URLs, destinations, filenames, priorities, engine configuration, and errors from Android, desktop, CLI/remote clients when UI polling only needs progress state.

## Shared implementation

- Domain model: `crates/models/src/runtime.rs`
- Core API: `CoreServices::download_runtime_statuses`
- Android UniFFI: `DownloadRuntimeStatusDto` and `list_runtime_statuses`
- Desktop Tauri: `list_runtime_statuses`
- Desktop TypeScript: `listRuntimeStatuses`
- Remote JSON-RPC: `download.runtimeStatus`

No durable storage or migration is required because runtime status is derived from current `DownloadItem` state.

## Target integration

| Target | Integration status |
|---|---|
| Android | Shared UniFFI method backed by canonical Rust projection |
| Desktop Windows/Linux/macOS | Shared Tauri command and TypeScript API backed by canonical Rust projection |
| CLI | Existing list output remains metadata-compatible; core projection is available for future compact polling |
| Remote server/UI | Added JSON-RPC `download.runtimeStatus`; UI protocol typing may consume it incrementally |
| Browser extension | Same remote RPC boundary where exposed by the host |

## Validation

- Models, core services, Android bridge: 84 unit/integration tests passed; 4 live extraction tests remained intentionally ignored.
- Desktop Tauri Rust integration: `cargo check -p fetchx-desktop` passed.
- Remote protocol/server and desktop Rust integration: `cargo check -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-desktop` passed.

## Deferred work

The next Gopeed-derived feature should be persistent post-download extraction progress, but only after confirming it does not duplicate FetchX's existing archive handling and media extraction orchestration.

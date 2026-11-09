# FetchX Shared-Core Architecture

## Assessment scope

This document describes the architecture evidenced by the FetchX repository. It is an assessment and compatibility plan, not a migration implementation.

## Applications present in this workspace

| Application or adapter | Evidence | Targets/platform role |
|---|---|---|
| Android app | `apps/android/app`, modular Android libraries and Gradle settings | Android, API 26+, ARM64 and x86_64 builds; downloads, foreground service, browser integration |
| Desktop app | `apps/desktop/src-tauri`, Tauri 2 and React | One Tauri shell packaged for Windows, Linux, and macOS; targets are declared in `bundle.targets` |
| CLI | `crates/cli`, binary `fetchx` | Desktop/headless Rust hosts |
| Remote server | `crates/remote-server` | Platform-independent HTTP/WebSocket JSON-RPC host |
| Remote web UI | `apps/remote-ui` | Browser client for the remote JSON-RPC server |
| Browser extension | `apps/browser/extension` | Chromium and Firefox extension builds |
| Native browser host | `apps/browser/native-host` | Native host runtime used for browser integration |

Separate macOS, Windows, and Linux application trees do not exist. Tauri supplies those three desktop targets from the same desktop adapter and UI.

## Current architecture

```text
Android UI / desktop React / CLI / remote clients
                     |
          platform API adapters
                     |
          CoreServices (Rust)
       |       |        |        |
    models  storage   queue  scheduler
                     |
              engine dispatcher
      |       |       |       |       |
     HTTP  native   yt-dlp  HLS   FTP/SFTP
           torrent   FFmpeg metalink aria2
```

The Rust workspace is the implementation center:

- `crates/models`: engine-independent persisted/domain models and validation.
- `crates/core`: lifecycle state-machine transitions.
- `crates/storage`: SQLite repositories, migrations, backup, import/export, and maintenance.
- `crates/downloader`: direct HTTP segmentation, retries, bandwidth control, FTP, SFTP, HLS, Metalink, yt-dlp, FFmpeg, aria2, native torrent, and process supervision.
- `crates/queue`: persisted priority/FIFO queue, resource budgeting, recovery, cancellation, and events.
- `crates/scheduler`: persistent one-shot scheduling over the queue.
- `crates/core-services`: platform-independent composition root used by desktop, Android, CLI, and remote hosts.
- `crates/android-bridge`: UniFFI mobile API, DTOs, generated-binding boundary, and Android runtime adapters.
- `crates/remote-protocol` / `crates/remote-server`: transport-independent JSON-RPC and authenticated HTTP/WebSocket host.
- `crates/browser-protocol`, browser native host, extension, and Android browser modules: transport/UI integrations.
- `crates/plugin-api`, plugin manager: plugin manifest and policy support.

## Canonical lifecycle

1. A host validates platform-independent request fields and resolves a destination.
2. The host creates a domain item through `CoreServices` or repository operations.
3. Storage persists the item and schedules or queue metadata.
4. `Scheduler` marks future work deferred and `QueueManager` enforces priority, FIFO, concurrency, and connection budgets.
5. `EngineDispatcher` maps an engine kind to the managed engine implementation.
6. The engine reports progress and terminal state through queue lifecycle and events.
7. Storage and the bounded `DownloadEventBus` publish state changes.
8. Hosts translate shared events to Tauri, Android UniFFI callbacks, CLI output, or JSON-RPC.

The lifecycle is already canonical in Rust. Host adapters should not mutate status or duplicate queue/resource decisions.

## Persistence boundary

SQLite schema compatibility is centralized in `crates/storage/database.rs` and `migrations/`. Twelve embedded migrations are currently registered. Android, desktop, CLI, and remote hosts all use the same migration implementation, although each host supplies a separate physical database path.

Logical schema ownership:

- downloads and queue order;
- schedules;
- engine configuration;
- yt-dlp video/audio selection;
- torrent metadata and selection;
- native torrent and FTP/SFTP/Metalink support.

Hosts must provide app-private SQLite storage paths, but must consume the same repository contracts and migrations. Database import/export behavior already prevents app-config credentials and sensitive source metadata from crossing host trust boundaries.

## Current host integration

### Android

`CoreServices` is constructed by `crates/android-bridge/src/service.rs`. UniFFI DTOs expose a subset of core operations to Kotlin. The Kotlin bridge owns Android storage publication, foreground execution, notifications, network-policy integration, and media runtime adapter behavior. The download UI uses a Kotlin repository abstraction backed by the Rust service.

Android contains known platform-independent fallback code (URL extraction, filename sanitization, and filename/package suggestions). Where a native bridge is unavailable, these fallbacks are primarily tests/recovery behavior, not a second runtime, but they should be removed or reduced for production convergence.

### Desktop

The desktop Rust adapter constructs the same `CoreServices` and forwards its bounded event bus to Tauri events. React maps Tauri DTOs to UI state. Desktop also owns OS paths, native dialogs, helper/binary installation, and browser IPC. Host-side `add_download` routing contains request validation and destination construction that should remain thin; URL validation should increasingly be centralized in Rust.

### CLI and remote server

CLI and remote server both construct `CoreServices`, which is already the preferred shared composition boundary. The remote server exposes JSON-RPC over authenticated HTTP and WebSocket and should be considered the protocol host for remote clients, not a second downloader implementation.

### Browser integrations

The browser extension/native host and Android browser module are platform adapters around shared protocols and core download operations. Browser-specific extraction, permissions, history, or tab storage is not download-engine business logic.

## Identified duplication and boundary debt

- Android and desktop maintain language-specific DTO/domain copies. That is necessary for generated/static bindings today, but mappings and enum values should be generated or contract-tested from Rust.
- Android has a local fallback URL parser and filename sanitization beside shared Rust built-ins.
- Android has a Kotlin `DownloadRepository` interface and desktop has separate Rust/Tauri DTOs instead of one generated host-neutral operation contract.
- Desktop contains request validation and destination construction, acceptable at the platform adapter boundary only if filename, URL, package, and engine rules are not duplicated.
- Android foreground execution and desktop background lifecycle remain platform-specific by design, but queue and transfer state must stay in Rust.
- Desktop helper installation is host-specific and may supply an executable to the shared engine runtime, not implement the engine itself.
- Aria2 remains represented as a persisted engine option even where normal desktop/Android UX now defaults to direct HTTP or native torrent. Capability reporting must distinguish implementation availability from persisted compatibility.

## Target principle

> Platform-specific code supplies storage paths, permissions, lifecycle, UI, notifications, native dialogs, and executable runtimes. Platform-independent business logic is implemented once in Rust crates below `core-services`.

No application should parse transfer state, decide queue eligibility, validate engine availability, allocate segments, perform retries, persist download lifecycle transitions, or implement torrent/media orchestration independently.

## aria2 migration status

The shared Rust core still supports aria2 through its supervisor and JSON-RPC adapter. The native `librqbit` engine remains additive, and this migration has not removed aria2 executables, RPC, configuration, helper management, or persisted task metadata. CoreServices now has explicit torrent-backend admission paths; legacy paths preserve their aria2-first behavior. Confirmed local `.torrent` and magnet indexes are passed to the native engine only after shared-core validation. Authenticated remote JSON-RPC exposes capability-gated magnet metadata inspection and explicit selection confirmation; the read operation requires download-read permission and selection requires download-control permission. The operation remains outside the restricted browser/native-host allowlist. Full aria2 restart/configuration parity remains open. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for the coverage table, source/license review, tests, and remaining phases.

# FetchX Roadmap

> **Core principle:** FetchX is one download platform with multiple clients.
> The Rust core is the single source of truth for all download business logic.
> Windows (.exe) and Android (.apk) are client/presentation/platform layers
> that share the same core -- they do NOT have separate download implementations.

---

## Architecture Overview

```
                       FETCHX SHARED CORE

                              |

           +------------------+------------------+

           |                                     |

    Windows Application                  Android Application

          .exe                                  .apk

           |                                     |

       Tauri UI                         Android UI / GeckoView

           |                                     |

           +----------------+-----------------+

                             |

                       Rust Core

                             |

       +---------------------+---------------------+

       |                     |                     |

   Downloader              Queue               Scheduler

       |                     |                     |

   Segmenter               Retry             State Machine

       |                     |                     |

   Engines               Storage           Resource Policy

       |                     |                     |

       +---------------------+---------------------+

                             |

                       Shared Services

```

---

## Platform Coverage Key

Each phase documents coverage using these categories:

- **SHARED CORE** -- implemented once in Rust; consumed by all clients
- **WINDOWS ADAPTER** -- Windows-only platform glue (Tauri, Win32, etc.)
- **ANDROID ADAPTER** -- Android-only platform glue (Bridge, UniFFI, etc.)
- **UI/UX** -- separate presentation per platform, same shared business logic
- **SERVER/REMOTE** -- platform-independent server; remote clients use same protocol
- **PARTIAL** -- currently works on one platform; cross-platform alignment required

---

## Phase Dependency Graph

```
Phases 1-20O (Completed)
        |
20P CoreServices Consolidation
        |
20Q Download Reliability + Turbo Acceleration
        |
20R Bandwidth / Resource Policies
        |
20S BitTorrent / Magnet
        |
21A Remote Protocol
        |
21B Headless Server
        |
21C Authentication
        |
21D Authorization
        |
21E Remote Security
        |
21F / 21G / 21H / 21I (Parallel)
        |
22 Browser Integration
        |
23 Plugin Security
        |
24 Storage / Migration
        |
25 Observability
        |
26 Production Packaging
        |
27 E2E Testing
        |
28 Advanced Networking
        |
29 Download Intelligence
        |
30 Cloud / Device Sync
```

---

## Cross-Platform Coverage Matrix

| Phase | Shared Rust Core | Windows .exe | Android .apk | Notes |
|-------|-----------------|-------------|-------------|-------|
| 1-13 | YES | YES | YES (via Bridge) | Platform adapters for runtime only |
| 14A-16C | YES | YES | PARTIAL | Desktop UI done; Android consumes same core |
| 17 | YES | YES | Android adapter planned (Phase 21I) | Browser protocol shared |
| 18 | YES | YES | Future (21H remote CLI) | CLI uses CoreServices |
| 19 | YES | YES | PARTIAL | Android plugin host planned |
| 20A | N/A | N/A | YES | Android foundation only |
| 20B-20C | YES | N/A | YES | Bridge to shared Rust core |
| 20D | Shared core | PARTIAL | YES | Windows download UI cross-platform alignment required |
| 20E-20L | Shared core | N/A | YES | Browser phases Android-first |
| 20M | YES (research) | YES | YES | VPN/Tor research applies to both |
| 20N-20O | Shared core | PARTIAL | YES | Storage/notification platform-specific; business logic shared |
| 20P | YES | YES | YES | CoreServices unification |
| 20Q | YES | YES | YES | Turbo engine in shared Rust |
| 20R | YES | YES | YES | Policy in shared Rust |
| 20S | YES | YES | YES | aria2 engine already shared |
| 21A-21E | YES | YES | YES | Protocol and server are platform-independent |
| 21F-21I | YES | Web/CLI | Web/Android | UI differs; protocol same |
| 22A-22B | YES | YES | YES | Browser integration uses shared queue |
| 23A-23D | YES | YES | YES | Plugin security enforced equally |
| 24-27 | YES | YES | YES | Both platforms must pass E2E |
| 28-30 | YES | YES | YES | All networking/intelligence in shared Rust |

---

## Completed Phases

### 1. Phase 1 -- Project Foundation (completed)

**Platform Coverage:** SHARED CORE
Rust workspace, Cargo.toml, project structure, CI baseline.

---

### 2. Phase 2 -- Domain Models (completed)

**Platform Coverage:** SHARED CORE
`crates/models` -- DownloadItem, DownloadStatus, DownloadProgress, DownloadPriority,
DownloadId, DownloadSource, FileName, EngineKind, Timestamp, DownloadError.
Consumed identically by Windows, Android (via Bridge DTOs), and CLI.

---

### 3. Phase 3 -- SQLite Persistence (completed)

**Platform Coverage:** SHARED CORE
`crates/storage` -- Database, SqliteDownloadRepository, SqliteConfigRepository.
SQLite file path differs per platform (provided by platform adapter); the
persistence logic itself is fully shared.

---

### 4. Phase 4 -- Download State Machine (completed)

**Platform Coverage:** SHARED CORE
`crates/core` -- DownloadStateMachine, validate_transition. All transition
validation is shared. Platform layers never implement their own state machines.

---

### 5. Phase 5 -- Direct HTTP Downloader (completed)

**Platform Coverage:** SHARED CORE
`crates/downloader` -- DirectHttpEngine, segmented HTTP using reqwest/rustls-tls.
Windows and Android both consume this engine through EngineDispatcher.
- Windows adapter: runtime path configuration via Tauri.
- Android adapter: runtime path via BridgeInitializer/CoreServices.

---

### 6. Phase 6 -- Resume and Range Support (completed)

**Platform Coverage:** SHARED CORE
Range request headers, partial-file resume, Content-Range handling. Shared across all platforms.

---

### 7. Phase 7 -- Segment Manager (completed)

**Platform Coverage:** SHARED CORE
Multi-segment coordination, positional writes, progress tracking. Shared.

---

### 8. Phase 8 -- Adaptive Acceleration (completed)

**Platform Coverage:** SHARED CORE
Dynamic connection count adjustment based on network feedback. Shared.

---

### 9. Phase 9 -- Queue Manager (completed)

**Platform Coverage:** SHARED CORE
`crates/queue` -- QueueManager, QueueConfig, DownloadEventBus, EventRepository.
All clients (Windows, Android, CLI) consume the same queue. Platform
layers call enqueue/pause/resume/cancel through CoreServices.

---

### 10. Phase 10 -- Persistent One-Shot Scheduler (completed)

**Platform Coverage:** SHARED CORE
`crates/scheduler` -- Scheduler, ScheduleError. Shared. Platform layers
provide wall-clock time; scheduling logic is platform-independent.

---

### 11A. Phase 11A -- aria2 JSON-RPC Client and Engine Adapter (completed)

**Platform Coverage:** SHARED CORE
`crates/downloader` -- Aria2Client, Aria2Engine. Shared across platforms.

---

### 11B. Phase 11B -- aria2 Process Supervision and Restart Recovery (completed)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER
Shared: Aria2Supervisor, retry/restart logic.
Windows adapter: aria2c.exe path resolution.
Android adapter: aria2 binary not yet bundled -- Android uses aria2 only if
an external binary is available (planned: Phase 26B dependency bundling).

---

### 12. Phase 12 -- yt-dlp Integration (completed)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER
Shared: YtDlpEngine, process invocation, output parsing.
Windows adapter: yt-dlp.exe path.
Android adapter: yt-dlp not yet bundled for Android -- planned Phase 26B.

---

### 13. Phase 13 -- FFmpeg Integration (completed)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER
Shared: FFmpeg pipeline, post-processing.
Windows adapter: ffmpeg.exe.
Android adapter: FFmpeg not yet bundled -- planned Phase 26B.

---

### 14A. Phase 14A -- Tauri 2 Desktop Foundation (completed)

**Platform Coverage:** WINDOWS ADAPTER + SHARED CORE
Tauri 2 shell, React renderer bootstrap. Windows-specific host. The download
business logic it calls is shared Rust Core.

---

### 14B. Phase 14B -- Live Download Events/Progress (completed)

**Platform Coverage:** SHARED CORE (event bus) + WINDOWS UI
DownloadEventBus (broadcast channel) is shared. Tauri event forwarding is
Windows-adapter. Android consumes the same bus via UniFFI Bridge (Phase 20B).

---

### 14C. Phase 14C -- Priority and Scheduling Controls (completed)

**Platform Coverage:** SHARED CORE + WINDOWS UI
Queue priority and scheduling logic in shared Rust. Windows UI surfaces them.
Android UI exposes same operations through DownloadsViewModel (Phase 20D).

---

### 15A. Phase 15A -- Full React Desktop UI (completed)

**Platform Coverage:** WINDOWS UI
React/Tauri desktop interface. Business logic remains in shared Rust Core.

---

### 15B. Phase 15B -- UI Polish (completed)

**Platform Coverage:** WINDOWS UI
Queue snapshot, bulk actions, drag-and-drop, keyboard shortcuts, toasts.

---

### 16A. Phase 16A -- Engine Configuration Persistence/UI (completed)

**Platform Coverage:** SHARED CORE (ConfigService) + WINDOWS UI
ConfigService (`crates/core-services`) persists engine config in shared SQLite.
Windows UI surfaces settings. Android exposes engine config via Bridge (future
Phase 20P alignment item).

---

### 16B. Phase 16B -- Runtime Engine Activation (completed)

**Platform Coverage:** SHARED CORE + WINDOWS ADAPTER
EngineRuntime, EngineDispatcher in `crates/core-services`. Shared.
Supervisor startup uses OS-specific paths (Windows adapter).
Android: same CoreServices consumed via Bridge.

---

### 16C. Phase 16C -- Add Download Engine Selection (completed)

**Platform Coverage:** SHARED CORE + UI (Windows + Android)
Engine selection validation in shared Rust. Both Windows (React dialog) and
Android (AddDownloadDialog) expose engine selection.

---

### 17. Phase 17 -- Browser Extension / Native Messaging (completed)

**Platform Coverage:** SHARED CORE (browser-protocol) + WINDOWS ADAPTER
`crates/browser-protocol` -- shared message types and loopback IPC logic.
`apps/browser/native-host` -- Windows-side native messaging host.
`apps/browser/extension` -- MV3 extension (Chrome/Firefox).
Android integration: planned Phase 21I (browser extension connects to Android FetchX).
**Cross-platform alignment required:** Android FetchX does not yet accept
connections from browser extensions.

---

### 18. Phase 18 -- CLI (completed)

**Platform Coverage:** SHARED CORE + CLI ADAPTER
`crates/cli` -- thin dispatcher using `crates/core-services`. No download logic
in the CLI itself.

Key limitation: desktop Tauri app has not yet been migrated to `crates/core-services`.
Desktop and CLI share design but retain some duplicated service initialization.
**Cross-platform alignment required:** Phase 20P resolves this.

Commands: `version`, `engines`, `config`, `add`, `list`, `status`, `pause`,
`resume`, `cancel`, `remove`, `retry`, `priority`, `schedule`, `monitor`, `plugins`.

---

### 19. Phase 19 -- Plugin Architecture (completed)

**Platform Coverage:** SHARED CORE (plugin-api, plugin-manager) + WINDOWS ADAPTER
Out-of-process plugin model; length-prefixed JSON IPC over stdin/stdout.
`crates/plugin-api` -- manifest schema, IPC framing, typed messages.
`crates/plugin-manager` -- PluginManager: discovery, validation, lifecycle.
Android plugin host: planned (requires platform-specific process management).
**Cross-platform alignment required:** Android FetchX does not yet support plugin processes.

Known limitations:
1. Built-in engines (DirectHttp, aria2, yt-dlp) NOT converted to plugins.
2. Plugin enable/disable state not persisted in SQLite.
3. No filesystem/network sandboxing in this phase.
4. Plugin hot-reload and restart-on-crash not implemented.

---

### 20A. Phase 20A -- Android Foundation (completed)

**Platform Coverage:** ANDROID ADAPTER
GeckoView selected as browser engine (full WebExtension API, ETP, session isolation).
Android module structure: `:app`, `:core`, `:downloads`.
minSdk 26, targetSdk 35, compileSdk 35. Kotlin 2.0.21, AGP 8.7.3, Compose BOM 2024.12.01.
Shared Rust Core: unchanged in this phase.

Security: cleartext HTTP blocked; INTERNET, FOREGROUND_SERVICE,
FOREGROUND_SERVICE_DATA_SYNC, POST_NOTIFICATIONS declared.

---

### 20B. Phase 20B -- Rust to Android Bridge (completed)

**Platform Coverage:** ANDROID ADAPTER -> SHARED CORE
`crates/android-bridge` -- UniFFI 0.29, JNA backend.
Wraps `crates/core-services` (CoreServices) behind a self-managed Tokio runtime.
DTO layer: DownloadInfoDto, DownloadProgressDto, DownloadStatusDto, DownloadPriorityDto,
DownloadEventDto -- SQLx types never cross the FFI boundary.
MobileApiError: Validation, NotFound, UnavailableEngine, InvalidStateTransition,
Storage, Runtime.
Event delivery: DownloadEventBus broadcast -> UniFFI callback -> Kotlin SharedFlow.
14 unit tests; all pass. APK ~10 MB.

---

### 20C. Phase 20C -- Android Rust Native Build / Runtime Integration (completed)

**Platform Coverage:** ANDROID ADAPTER
NDK r27 (27.2.12479018), cargo-ndk 4.1.2.
arm64-v8a + x86_64 `.so` files cross-compiled and packaged.
`libfetchx_android_bridge.so` confirmed in APK.
`libjnidispatch.so` pre-loaded via `System.loadLibrary("jnidispatch")`.
Bridge initialization confirmed on physical device:
`I MyDownloaderBridge: Bridge initialized successfully`.

---

### 20D. Phase 20D -- Android Download Manager UI (completed)

**Platform Coverage:** ANDROID UI -> SHARED CORE
DownloadsViewModel uses shared DownloadRepository (backed by Bridge -> Rust Core).
No download business logic duplicated in Kotlin.
6-tab UI: All/Active/Queued/Completed/Failed/Cancelled.
AddDownloadDialog: URL input, filename, priority (LOW/NORMAL/HIGH).
sanitizeFailureReason() prevents internal Rust error tokens reaching users.
Tests: 14 ViewModel tests + 20 utility tests; all pass.

**Cross-platform alignment note:** Windows React UI has equivalent download
management. Both call the same Rust Core through their respective adapters.

---

### 20E. Phase 20E -- GeckoView Browser Foundation (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
GeckoView 137.0.20250325171416 (arm64-v8a).
GeckoRuntimeHolder: process-level singleton (created exactly once per process).
BrowserViewModel: owns GeckoSession across configuration changes.
BrowserScreen: URL bar, back/forward/reload, loading indicator, error surface.
8 unit tests for normalizeUrl().

---

### 20F. Phase 20F -- Multi-tab Browser Sessions (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
TabState, TabStore (pure Kotlin), TabManager (GeckoSession lifecycle).
One GeckoSession per tab; minimum-one-tab policy enforced.
22 JVM unit tests (NormalizeUrlTest + TabStoreTest).

---

### 20G. Phase 20G -- History / Navigation State (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
HistoryEntry, HistoryStore (pure Kotlin, no Android imports).
Per-tab history; recorded on `onPageStop(success=true)` only.
about:blank excluded; consecutive duplicates skipped; max 200 entries per tab.
36 JVM unit tests; all pass.

---

### 20H. Phase 20H -- Bookmarks (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
Bookmark, BookmarkStore, BookmarkController -- all pure Kotlin, JVM-testable.
Global (not per-tab); in-memory only; non-http(s) URLs rejected.
Clearing history does NOT clear bookmarks.
81 JVM unit tests total; all pass.

---

### 20I. Phase 20I -- Tracker Blocking (completed)

**Platform Coverage:** ANDROID ADAPTER (browser) -> SHARED CORE (categories)
GeckoView native ContentBlocking configured for ad, analytics, social categories.
TrackerBlockingController: per-tab counts, max 20 hostnames retained.
On-device confirmed: isBlocking=true, category 2 (ad) + category 4 (analytics).
17 controller JVM tests.

---

### 20J. Phase 20J -- Privacy / Fingerprinting Controls (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
GeckoView runtime privacy settings: fingerprinting protection, cryptomining
protection, third-party cookie isolation, HTTPS-only mode.
PrivacyController (pure Kotlin, JVM-testable); wired to GeckoRuntimeSettings.

---

### 20K. Phase 20K -- Extension Framework (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
WebExtensionController-backed extension management.
ExtensionManager, ExtensionStore, GeckoExtensionGateway, ExtensionPackageImporter.
.xpi sideloading supported; permission-requiring extensions blocked pending Phase 20L/23.

---

### 20L. Phase 20L -- Extension Permissions (completed)

**Platform Coverage:** ANDROID ADAPTER (browser)
Extension permission review model; requested permissions surfaced in UI.
Extensions with unsupported permissions blocked from installation.

---

### 20M. Phase 20M -- VPN/Tor Integration Research (completed)

**Platform Coverage:** SHARED RESEARCH (applies to both Windows and Android)
Research-only phase; no implementation added.
Document: `docs/ANDROID_VPN_TOR_RESEARCH.md` (20 sections).
GeckoView 137 checked: no public Java-level proxy API.
Recommended architecture: Option C -- Per-component SOCKS5 proxy.
Finding applies to both Android (GeckoView + reqwest) and Windows (reqwest + aria2).

---

### 20N. Phase 20N -- Android Storage / Notifications (completed)

**Platform Coverage:** ANDROID ADAPTER + SHARED CORE (storage business logic)
StorageManager: default app-specific dir + SAF custom dir with persisted URI permission.
Filename sanitization: path traversal, null bytes, reserved chars, length cap, `..` removal.
DownloadNotificationManager: 3 channels (active/LOW, complete/DEFAULT, failed/DEFAULT).
DownloadForegroundService: observes `observeDownloads()` from Bridge; no duplicate logic.
14 JVM unit tests (7 sanitize + 7 formatting); all pass.
**Cross-platform alignment note:** Windows stores downloads in user-configured path
via Tauri; filename sanitization should be aligned in Phase 20P/20Q.

---

### 20O. Phase 20O -- Android Security / Privacy Audit and Remediation (completed)

**Platform Coverage:** ANDROID ADAPTER + SHARED CORE
Security audit conducted; findings remediated.
No DRM/CAPTCHA/auth bypass introduced.
All security boundaries preserved per AI_RULES.md.

---

## Planned Phases

---

### Phase 20P -- CoreServices Consolidation (CRITICAL)

**Goal:** Make `crates/core-services` the single service bootstrap/graph shared by
Windows, Android, CLI, and future headless server.

**Platform Coverage:** SHARED CORE (full); Windows adapter, Android adapter, CLI adapter

**Architecture:**
```
Windows .exe  -+
CLI            -+-> CoreServices --> FetchX Core (crates/core)
Android .apk  -+
Server        -+
```

**Tasks:**
- Migrate desktop Tauri initialization to `crates/core-services` (currently partial)
- Integrate Android Bridge with shared CoreServices (already done; verify completeness)
- Eliminate duplicated service construction in desktop AppState
- Unify: storage init, queue init, scheduler init, engine init, event bus, config,
  shutdown lifecycle, error handling, dependency injection
- Ensure CLI and desktop use the same service graph (not just the same design)

**Windows requirements:** Desktop AppState migrated; no duplicate service init code.
**Android requirements:** BridgeInitializer verified to use exactly the same CoreServices.
**CLI requirements:** Already uses CoreServices; verify no divergence after desktop migration.

**Tests:** startup, shutdown, duplicate-init, concurrent-init, persistence recovery,
engine-startup failure, graceful engine shutdown.

**Exit criteria:** Desktop and CLI use identical CoreServices/service graph.
No duplicated service initialization in any client.

**Dependencies:** Phases 1-20O completed.

---

### Phase 20Q -- Download Reliability and Turbo Acceleration (CRITICAL)

**Goal:** Harden download reliability and extend the existing Segment Manager with
configurable turbo/multi-connection profiles. ALL logic in shared Rust Core.

**Platform Coverage:** SHARED CORE; UI/UX differs per platform

**Critical rule:** The Turbo engine MUST live in `crates/downloader`. Windows and Android
consume the same implementation through their respective platform adapters.
Do NOT implement separate Android and Windows Turbo downloaders.

**20Q.1 -- HTTP Validators (Shared Core)**
Persist per-download: ETag, Last-Modified, Content-Length, Accept-Ranges, Content-Range.
Implement: If-Range, conditional requests.
Goal: prevent corrupted resume when remote resource changes.

**20Q.2 -- Advanced Retry Policy (Shared Core)**
RetryPolicy supporting: exponential backoff, jitter, Retry-After, HTTP status
classification (429 -> Retry-After; 5xx -> backoff; 404 -> no-retry; 401 -> no-retry),
network error classification (DNS failure, reset, TLS), max attempts, max elapsed time.

**20Q.3 -- Crash Consistency (Shared Core + Both Platforms)**
Test recovery from: process termination, app kill (Android + Windows), network loss,
disk-full, aria2/yt-dlp/FFmpeg crash, database interruption.
Verify: SQLite state, partial files, segment state, queue state, engine state.

**20Q.4 -- Atomic Finalization (Shared Core)**
`partial -> verify -> finalize -> atomic rename -> completed`

**20Q.5 -- Turbo Segmentation (Shared Core -- extends existing Segment Manager)**
Do NOT create a second downloader. Extend `crates/downloader`.
Support: dynamic segments, multiple connections, adaptive connection count,
segment reassignment, fast/slow/stalled worker detection, connection reuse, positional writes.

Configurable profiles:
- Balanced: 2-8 connections
- Turbo: 4-16 connections
- Extreme: 8-32 connections (never blindly use maximum)

**20Q.6 -- Dynamic Segment Sizing (Shared Core)**
Fast workers receive larger segments. Slow workers do not hold oversized segments.

**20Q.7 -- Adaptive Acceleration (Shared Core)**
Monitor: aggregate speed, per-worker speed, network quality, server throttling,
retry frequency, stalls, connection efficiency. Automatically adjust concurrency.

**20Q.8 -- Resource Policies (Shared Core)**
Global + per-download: connection limits, bandwidth limits, active-download limits.
Profiles: Unlimited, Normal, Turbo, Limited, Night, Custom.

**20Q.9 -- Capability Detection (Shared Core)**
Detect: Range support, Content-Length, Content-Range, validators, redirects.
Automatically choose: single stream OR segmented stream.

**20Q.10 -- Fault Injection Infrastructure (Shared Core)**
Reusable test infrastructure: NetworkFault, DiskFault, EngineFault, ProcessFault, DatabaseFault.

**Windows requirements:** Windows UI exposes turbo profile selection; uses shared engine.
**Android requirements:** Android UI exposes turbo profile selection; same Rust engine via Bridge.

**Exit criteria:** Downloads survive realistic interruptions without corruption.
Turbo profiles available and testable on both Windows and Android.

---

### Phase 20R -- Bandwidth and Resource Policies (HIGH)

**Platform Coverage:** SHARED CORE; UI/UX per platform

Introduce ResourcePolicy in shared Rust covering:
global connections, per-download connections, global bandwidth, per-download bandwidth,
engine bandwidth, CPU/memory limits where applicable, active-download limits.

Speed profiles: Unlimited, Normal, Limited, Night, Custom.

**Windows requirements:** Windows UI exposes policy controls; calls shared Rust policy.
**Android requirements:** Android UI exposes policy controls; calls same shared policy via Bridge.

---

### Phase 20S -- BitTorrent / Magnet Foundation (HIGH)

**Platform Coverage:** SHARED CORE (engine abstraction); UI/UX per platform

Use existing aria2 integration initially. Do NOT implement a custom BitTorrent engine.

Support via EngineDispatcher abstraction:
- .torrent files, magnet URIs, torrent metadata, file selection
- trackers, peer statistics, seed state, upload/download statistics

**Windows requirements:** Windows UI surfaces torrent/magnet add dialog; same queue.
**Android requirements:** Android UI (AddDownloadDialog) surfaces torrent/magnet input; same queue via Bridge.

---

### Phase 21A -- FetchX Remote Protocol (CRITICAL)

**Platform Coverage:** SHARED CORE (protocol definition); all clients consume same protocol

JSON-RPC 2.0 logical model. Namespaces: `download.*`, `queue.*`, `scheduler.*`,
`engine.*`, `plugin.*`, `config.*`, `system.*`.

Key commands: `download.create`, `download.get`, `download.list`, `download.pause`,
`download.resume`, `download.cancel`, `download.retry`, `download.remove`,
`download.setPriority`, `download.setEngine`, `download.setSpeedLimit`.

Events: `download.created`, `download.updated`, `download.progress`, `download.paused`,
`download.resumed`, `download.completed`, `download.failed`, `queue.updated`,
`engine.started`, `engine.stopped`, `engine.failed`, `plugin.enabled`, `plugin.disabled`.

Protocol versioning: `fetchx.rpc.v1`. Every connection discovers protocol version,
server version, supported capabilities.

**Windows:** Windows .exe acts as a local server and remote client.
**Android:** Android .apk acts as a remote client (Phase 21I wires browser to it).
**Shared:** Protocol definition and validation in shared Rust.

---

### Phase 21B -- Headless Server (CRITICAL)

**Platform Coverage:** SERVER/REMOTE (platform-independent) -> SHARED CORE

Standalone server using CoreServices -> FetchX Core. No Tauri dependency.
HTTP/RPC + WebSocket transport.

Supports: persistent downloads, queue, scheduler, engines, plugins, storage,
configuration, remote events.

**Windows:** Server can run as Windows service or standalone process.
**Android:** Android FetchX acts as a local server for Phase 21I browser integration.
**Shared:** CoreServices graph identical to desktop and CLI.

---

### Phase 21C -- Authentication and Device Pairing (CRITICAL)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER (secure storage differs)

Implement: device identity, pairing, device code, access tokens, refresh tokens,
sessions, token rotation, token revocation, device list, device removal, session expiration.

Secure local/LAN pairing flow. No unauthenticated remote download API.

**Windows:** Token storage in Windows Credential Manager or encrypted file.
**Android:** Token storage in Android Keystore.

---

### Phase 21D -- Authorization (CRITICAL)

**Platform Coverage:** SHARED CORE

Permissions: `downloads.read`, `downloads.create`, `downloads.modify`,
`downloads.delete`, `queue.modify`, `settings.read`, `settings.write`,
`plugins.read`, `plugins.manage`, `system.read`.

Roles or equivalent authorization policy. Both platforms enforce identical policy.

---

### Phase 21E -- Remote Security (CRITICAL)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER

Address: SSRF, path traversal, unsafe URL schemes, remote filesystem abuse,
request-size limits, rate limiting, WebSocket authentication, origin validation,
replay protection, token expiration, token revocation, download-directory restrictions,
DNS rebinding considerations, remote command authorization.

**Windows + Android:** Both must enforce equivalent security policy.
Only the platform networking adapter differs.

---

### Phase 21F -- Remote Web UI (HIGH)

**Platform Coverage:** UI/UX (web client) -> SHARED CORE via Remote Protocol

Web client areas: Dashboard, Downloads, Queue, Engines, Scheduler, Settings,
Devices, Plugins, System/Health.

Must use same remote protocol as CLI and all other clients. No separate API.

---

### Phase 21G -- Docker / NAS Deployment (HIGH)

**Platform Coverage:** SERVER/REMOTE

Official FetchX server container. Security baseline: non-root, read-only root
filesystem where practical, no-new-privileges, restricted volumes, healthcheck,
configurable resource limits.

Targets: NAS, home server, VPS, Docker host.

---

### Phase 21H -- Remote CLI (HIGH)

**Platform Coverage:** SHARED CORE + CLI ADAPTER

Extend existing CLI to use the same remote protocol.

Examples: `fetchx remote connect`, `fetchx remote status`, `fetchx remote downloads`,
`fetchx remote add`, `fetchx remote pause`, `fetchx remote resume`, `fetchx remote cancel`.

No duplicate business logic in CLI.

---

### Phase 21I -- Browser Remote Integration (HIGH)

**Platform Coverage:** SHARED CORE (protocol) + WINDOWS ADAPTER + ANDROID ADAPTER

Allow browser extension and native-messaging integration to target:
- Local FetchX (Windows .exe or Android .apk)
- Remote FetchX server

Users select a FetchX server/device. Must use authenticated remote protocol.

**Windows:** Native messaging host updated to support remote routing.
**Android:** Android FetchX accepts connections from browser extensions.

---

### Phase 22A -- Browser Download Interception (HIGH)

**Platform Coverage:** SHARED CORE (download handling) + BOTH PLATFORM ADAPTERS

Flow: `Browser -> extension -> FetchX -> URL analysis -> engine selection -> queue`

Download request MUST enter the shared Rust downloader queue on both platforms.

Support: direct HTTP/HTTPS downloads, filename detection, MIME type, referrer
(when explicitly available/authorized), multiple simultaneous downloads.

Do NOT bypass: DRM, CAPTCHA, authentication, access controls, anti-bot protections.

**Windows:** Via native messaging host -> CoreServices.
**Android:** Via Android FetchX local service -> Bridge -> CoreServices.

---

### Phase 22B -- Firefox Extension (HIGH)

**Platform Coverage:** SHARED CORE (browser protocol) + EXTENSION ADAPTER

Complete Firefox support alongside existing Chrome/Edge support.
Shared browser-protocol crate; only extension manifest format differs.

---

### Phase 23A -- Plugin Security (HIGH)

**Platform Coverage:** SHARED CORE + BOTH PLATFORM ADAPTERS

Upgrade plugin architecture with:
PluginManifest, PluginPermission, PluginCapability, PluginTrust, PluginSignature, PluginVersion.

Permission categories: network, filesystem, downloads, metadata, process, engine, settings.

No silent dangerous permission grants. Both Windows and Android enforce equivalent policy.

---

### Phase 23B -- Plugin Sandbox Hardening (HIGH)

**Platform Coverage:** SHARED CORE + BOTH PLATFORM ADAPTERS

Retain out-of-process architecture. Add where technically applicable:
CPU/memory limits, IPC message-size limits, execution timeouts, crash isolation,
controlled restart policy, filesystem restrictions, network restrictions.

Both platforms must implement equivalent sandboxing.

---

### Phase 23C -- Plugin Signing (HIGH)

**Platform Coverage:** SHARED CORE

Real cryptographic verification: publisher identity, package hash, signature,
trusted publisher model, revocation.

Do NOT implement fake "verified" states without actual cryptographic verification.

---

### Phase 23D -- Plugin Registry (MEDIUM)

**Platform Coverage:** SHARED CORE + UI/UX per platform

Registry contents: plugin name, version, publisher, permissions, signature,
compatibility, release notes, download information.

Not a prerequisite for the core plugin runtime.

---

### Phase 24 -- Production Storage and Migration (MEDIUM)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER (filesystem paths differ)

Add: database backup, migration validation, schema version diagnostics,
repair/check mode, export, import, safe migration failure handling.

CLI commands: `fetchx db check`, `fetchx db repair`, `fetchx db backup`,
`fetchx db export`, `fetchx db import`.

Both Windows and Android must safely handle persisted state migrations.
Only the filesystem adapter differs (Windows path vs Android app-storage path).

---

### Phase 25 -- Observability and Diagnostics (MEDIUM)

**Platform Coverage:** SHARED CORE + UI/UX per platform

Shared metrics: `downloads_total`, `downloads_active`, `downloads_failed`,
`bytes_downloaded`, `bytes_uploaded`, `average_speed`, `retry_count`,
`engine_failures`, `queue_depth`, `database_latency`, `rpc_requests`, `plugin_crashes`.

Diagnostics command produces a safe diagnostic bundle.
NEVER include: passwords, tokens, cookies, session strings, API keys, credentials.

Both Android and Windows must expose useful diagnostics.

---

### Phase 26A -- Production Packaging (CRITICAL)

**Platform Coverage:** WINDOWS ADAPTER + ANDROID ADAPTER (separate packaging)

**Windows:** installer, signing, Tauri bundle, uninstaller, start-menu integration,
file associations, protocol associations.

**Linux:** AppImage, .deb, .rpm.

**macOS:** .app, DMG, code signing, notarization.

**Android:** APK signing, release configuration, minification, Play/distribution readiness.

Shared business logic is identical despite platform packaging differences.

---

### Phase 26B -- Dependency Bundling (HIGH)

**Platform Coverage:** PLATFORM ADAPTER (platform-specific binaries)

Production-manage: aria2, yt-dlp, FFmpeg per platform.
Requirements: platform-specific binaries, version detection, integrity validation,
controlled updates, fallback/error handling.

**Windows:** Bundle aria2c.exe, yt-dlp.exe, ffmpeg.exe.
**Android:** Bundle ARM64 binaries; evaluate runtime extraction approach.

Do not depend on arbitrary PATH installations for normal consumer usage.

---

### Phase 26C -- Auto Update (HIGH)

**Platform Coverage:** PLATFORM ADAPTER (update mechanism differs)

Channels: Stable, Beta, Nightly.
Flow: `check -> download -> verify signature/integrity -> install -> restart`.

**Windows:** Tauri updater with signed artifacts.
**Android:** In-app APK download and verification.

Never execute unverified update artifacts on either platform.

---

### Phase 27 -- Production End-to-End Testing (HIGH)

**Platform Coverage:** BOTH PLATFORMS -- must pass independently

Test BOTH Windows .exe AND Android .apk:
```
Browser -> FetchX -> Queue -> Engine -> Download -> Persistence -> Restart -> Recovery
```

Test scenarios: network interruption, disk failure, engine failure, retry,
segmentation, pause, resume, cancellation, application restart, crash recovery.

Both platforms must have full E2E coverage. A pass on Windows does not substitute
for Android test coverage.

---

### Phase 28 -- Advanced Networking (MEDIUM)

**Platform Coverage:** SHARED CORE (proxy logic) + PLATFORM ADAPTER (OS networking APIs)

Potential features: HTTP proxy, HTTPS proxy, SOCKS5, per-download proxy,
per-engine proxy, proxy profiles, IPv4/IPv6 policies, DNS policy,
connection pooling, Tor routing integration.

Reuse Phase 20M research. The proxy/routing logic lives in shared Rust.
Only the OS-specific socket/networking adapter differs between Windows and Android.

---

### Phase 29 -- Download Intelligence (MEDIUM)

**Platform Coverage:** SHARED CORE

URL classifier -> engine selection.
Examples:
- HTTP/HTTPS -> Direct HTTP
- .torrent / magnet -> aria2
- Supported media page -> yt-dlp
- Post-processing required -> FFmpeg

Add: network-quality estimation, health-aware retry, adaptive engine selection,
improved adaptive concurrency.

Do NOT implement: DRM bypass, CAPTCHA bypass, access-control bypass, anti-bot circumvention.

Both Windows and Android consume the same classifier logic.

---

### Phase 30 -- Cloud / Device Sync (FUTURE)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER (secure storage)

Optional. FetchX must remain fully functional without a cloud account.

Potential: download history sync, settings sync, device sync, queue sync,
browser sync, bookmark sync.

Both Windows and Android participate as sync clients using the same protocol.

---

## Architectural Principles

1. Rust remains the source of truth for all business logic.
2. React (Windows) and Compose/GeckoView (Android) are presentation layers only.
3. Desktop, CLI, headless server, and all clients share `crates/core-services`.
4. Remote clients use the same FetchX protocol -- no duplicated APIs.
5. Engine implementations remain behind the EngineDispatcher abstraction.
6. Persistence must go through `crates/storage`.
7. No duplicated queue/scheduler/download logic in any client.
8. No shell-string command execution; structured process arguments only.
9. New functionality requires tests.
10. Security-sensitive features require explicit threat modelling.
11. Do not silently grant dangerous plugin permissions.
12. Do not implement fake cryptographic verification.
13. No DRM/CAPTCHA/access-control bypass.
14. No credential/session-cookie theft.
15. No anti-bot circumvention.
16. No automatic Git commits/pushes.
17. Update .gitignore after completion of every phase.

---

## Motrix-Inspired Lessons

FetchX should learn from Motrix's strengths:
- Host-neutral Rust core
- Engine adapter abstraction
- MDXP-style remote protocol
- WebSocket event model
- Headless server mode
- Docker/NAS deployment
- Browser integration
- Plugin isolation

**However, do NOT copy Motrix blindly.** FetchX retains:
- Rust core with Tokio
- SQLx/SQLite persistence
- Existing queue/scheduler architecture
- Multi-engine architecture (DirectHttp, aria2, yt-dlp, FFmpeg)
- Existing Android/GeckoView architecture
- Existing security boundaries

The objective is architectural improvement, not Motrix duplication.

---

## Phase Priority Summary

| Priority | Phases |
|----------|--------|
| CRITICAL | 20P, 20Q, 21A, 21B, 21C, 21D, 21E, 26A |
| HIGH | 20R, 20S, 21F, 21G, 21H, 21I, 22A, 22B, 23A, 23B, 23C, 26B, 26C, 27 |
| MEDIUM | 23D, 24, 25, 28, 29 |
| FUTURE | 30 |

---

## Cross-Platform Alignment Required

The following completed phases currently have gaps in cross-platform coverage
that should be addressed in Phase 20P or dedicated alignment tasks:

| Phase | Gap | Resolution |
|-------|-----|-----------|
| 11B | aria2 not bundled for Android | Phase 26B |
| 12 | yt-dlp not bundled for Android | Phase 26B |
| 13 | FFmpeg not bundled for Android | Phase 26B |
| 14A-16C | Desktop not yet migrated to shared CoreServices | Phase 20P |
| 17 | Android does not accept browser-extension connections | Phase 21I |
| 19 | Android does not yet support plugin processes | Phase 20P+ |
| 20D | Windows download UI not aligned with Android UI patterns | Phase 20P |
| 20N | Windows filename sanitization not formally aligned | Phase 20Q |

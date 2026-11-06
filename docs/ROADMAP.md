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
Phases 1-20P (Completed)
        |
        +-------------------------------------------+
        |                                           |
20P CoreServices Consolidation          20B-A Browser Policy Core
        |                                           |
20Q Download Reliability + Turbo     20B-B FetchX Shields
        |                                  /       \
20R Bandwidth / Resource Policies  20B-C Content  20B-D Advanced Tracking
        |                          Filtering           |
20S BitTorrent / Magnet                 \             /
        |
20T Download Reliability Hardening
        |                          20B-E Fingerprinting Protection
21A Remote Protocol                          |
        |                          20B-F Browser Security Hardening
21B Headless Server                          |
        |                          20B-G Privacy/Security Regression Tests
21C Authentication                           |
        |                          20B-H Shields & Site Controls UI
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
26A Production Packaging
26D Release Automation (workflow implementation complete)
        |
27 E2E Testing
        |
28 Advanced Networking
        |
29 Download Intelligence
        |
30 Production Reliability / Failure Recovery
        |
31 Cloud / Device Sync
```

---

## Cross-Platform Coverage Matrix

| Phase | Shared Rust Core | Windows .exe | Android .apk | Notes |
|-------|-----------------|-------------|-------------|-------|
| 1-13 | YES | YES | YES (via Bridge) | Platform adapters for runtime only |
| 14A-16C | YES | YES | PARTIAL | Desktop UI done; Android consumes same core |
| 17 | YES | YES | In-app GeckoView adapter (Phase 21I) | External browser-to-Android connection not implemented |
| 18 | YES | YES | Future (21H remote CLI) | CLI uses CoreServices |
| 19 | YES | YES | PARTIAL | Android plugin host planned |
| 20A | N/A | N/A | YES | Android foundation only |
| 20B-20C | YES | N/A | YES | Bridge to shared Rust core |
| 20D | Shared core | PARTIAL | YES | Windows download UI cross-platform alignment required |
| 20E-20L | Shared core | N/A | YES | Browser phases Android-first |
| 20M | YES (research) | YES | YES | VPN/Tor research applies to both |
| 20N-20O | Shared core | PARTIAL | YES | Storage/notification platform-specific; business logic shared |
| 20B-A | N/A | N/A | YES | Browser Policy Core: Android browser layer only |
| 20B-B | N/A | N/A | YES | FetchX Shields: Android browser layer only |
| 20B-C | N/A | N/A | YES | Content Filtering: Android browser + optional WebExtension |
| 20B-D | N/A | N/A | YES | Advanced Tracking Protection: Android browser layer |
| 20B-E | N/A | N/A | YES | Fingerprinting Protection: Android browser layer |
| 20B-F | N/A | N/A | YES | Browser Security Hardening: Android browser layer |
| 20B-G | N/A | N/A | YES | Privacy/Security Tests: Android instrumentation |
| 20B-H | N/A | N/A | YES | Shields UI: Android Compose browser UI |
| 20Q | YES | YES | YES | Turbo engine in shared Rust |
| 20R | YES | YES | YES | Policy in shared Rust |
| 20S | YES | YES | YES | aria2 engine already shared |
| 21A-21E | YES | YES | YES | Protocol and server are platform-independent |
| 21F-21I | YES | Web/CLI | Web/Android | UI differs; protocol same |
| 22A-22B | YES | YES | YES | Browser integration uses shared queue |
| 23A | YES | YES | Evaluator only | Shared policy; Android plugin execution unavailable |
| 23B-23D | Planned | Planned | Planned | Sandbox, cryptographic signing, and registry work |
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
Phase 21I provides a built-in GeckoView extension for the FetchX Android app's
own browser through an in-process bridge. External Chrome/Firefox extensions do
not connect to Android FetchX over a network listener.

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

## Brave-Inspired Browser Privacy & Security Research Reference

FetchX has studied Brave's privacy/security architecture as a reference model.
Brave concepts -- Shields, filter lists, fingerprinting defenses, URL parameter
stripping, referrer policies, security hardening -- are incorporated as inspiration
for FetchX's own browser policy and controls architecture.

**What this means:**
- Brave is REFERENCE / INSPIRATION, not a dependency.
- FetchX will NOT include Brave Core.
- FetchX will NOT become a Chromium fork.
- FetchX will NOT copy Chromium-specific internals.
- GeckoView remains the browser engine.
- Only capabilities actually supported by GeckoView are implemented.
- Unsupported GeckoView capabilities are classified as LIMITED or ENGINE-LEVEL,
  not worked around unsafely.

**Engineering Rule (GeckoView Capability Audit):**
> Every Brave-inspired feature must first undergo a GeckoView capability audit.
> If GeckoView does not expose the required capability, the roadmap must classify
> it as LIMITED, UNSUPPORTED, WEBEXTENSION-DEPENDENT, or ENGINE-LEVEL rather than
> introducing an unsafe workaround.

---

## Browser Privacy / Security Track (Phases 20B-A through 20B-H)

These phases form a dedicated browser privacy and security track, numbered with
the `20B-` prefix to distinguish them from the download-manager track (20P, 20Q, ...)
while preserving all historical phase numbering and all existing planned phases.

**Track dependency graph:**
```
GeckoView research / capability audit
            |
      20B-A Browser Policy Core
            |
      20B-B FetchX Shields
            |
  +---------+---------+
  |                   |
20B-C Content Filtering  20B-D Advanced Tracking Protection
  |                   |
  +----+--------------+
       |
  20B-E Advanced Fingerprinting Protection
       |
  20B-F Browser Security Hardening
       |
  20B-G Privacy & Security Regression Testing
       |
  20B-H Shields & Site Controls UI
```

The browser privacy track is intentionally independent of the download-manager
track (20P-20S, 21A-21I, etc.). Download phases do not depend on browser privacy
phases unless explicitly noted.

---

### Phase 20B-A -- Browser Policy Core (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer); browser-specific only

**Goal:** Establish a single, centralized `FetchXBrowserPolicy` object that owns all
browser-level privacy and security decisions. Eliminates scattered policy checks across
`GeckoRuntimeHolder`, `PrivacyController`, `TabManager`, and `BrowserViewModel`.

**Architecture:**
```
FetchXBrowserPolicy
        |
        +-- PrivacyPolicy
        |       adBlocking, trackerBlocking, cookiePolicy, fingerprintPolicy
        |
        +-- SecurityPolicy
        |       httpsOnly, mixedContent, dangerousDownloads, certErrors
        |
        +-- NetworkPolicy
        |       referrerPolicy, trackingParamStripping, dnsPolicy
        |
        +-- SitePolicy
        |       per-site overrides, exceptions, trust level
        |
        +-- PermissionPolicy
        |       camera, microphone, location, notifications, popups, WebRTC
        |
        +-- TrackingPolicy
        |       ETP level, first-party isolation, storage isolation
        |
        +-- FingerprintPolicy
        |       canvas, WebGL, audio, fonts, screen, UA, hardware, platform
        |
        +-- CookiePolicy
        |       third-party, isolation, partitioning, SameSite enforcement
        |
        +-- HTTPSPolicy
        |       httpsOnly, upgradeInsecure, certOverride rules
                |
                v
        GeckoView Adapter
        (maps FetchXBrowserPolicy domains -> GeckoRuntimeSettings,
         GeckoSessionSettings, ContentBlocking, WebExtensionController)
                |
                v
           GeckoView
```

**Requirements:**
- Centralize all privacy/security decisions in one typed structure
- Support global defaults and per-site policy overrides
- Support per-tab policy state where GeckoView allows it
- Preserve all existing Phase 20I/20J/20K/20L behavior -- no regressions
- The policy object must be serializable/persistable for user settings
- GeckoView adapter translates policy to actual GeckoView API calls

**Dependencies:** Phases 20I, 20J, 20K, 20L completed.

**Tests:** Policy construction, policy override (global vs per-site), GeckoView
adapter correctness, persistence round-trip, no-regression against existing phases.

**Exit criteria:** All existing privacy/security controls route through
`FetchXBrowserPolicy`. No scattered `GeckoRuntimeSettings` calls outside the adapter.

---

### Phase 20B-B -- FetchX Shields (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer)

**Goal:** Implement a Brave-inspired Shields system with per-site controls and
protection statistics, using GeckoView-native capabilities where available and
WebExtension-based filtering where not.

**Shield modes:**
- `Off` -- no protections active for this site
- `Standard` -- balanced protection; may be set as global default
- `Strict` -- aggressive protection; may break some sites; user-overridable per-site

**Shield controls (subject to GeckoView capability audit -- see classification below):**

| Control | GeckoView Classification |
|---------|------------------------|
| Ad blocking | NATIVE_GECKO (ContentBlocking.AntiTracking.AD) |
| Tracker blocking | NATIVE_GECKO (ContentBlocking.AntiTracking.*) |
| First-party tracking | NATIVE_GECKO (ContentBlocking + cookie partitioning) |
| Fingerprinting protection | NATIVE_GECKO (GeckoRuntimeSettings.setFingerprintingProtection) |
| Third-party cookie controls | NATIVE_GECKO (ContentBlocking.CookieBehavior) |
| HTTPS enforcement | NATIVE_GECKO (GeckoRuntimeSettings.HTTPS_ONLY) |
| JavaScript policy | LIMITED (no per-site JS toggle in current GeckoView public API) |
| Referrer policy | WEBEXTENSION or LIMITED |
| Tracking URL parameter filtering | WEBEXTENSION |
| Cryptomining protection | NATIVE_GECKO (ContentBlocking.AntiTracking.CRYPTOMINING) |
| Notification abuse protection | CONFIGURABLE (PermissionDelegate) |
| Popup blocking | CONFIGURABLE (GeckoSession popup policy) |

**Per-site features:**
- Per-origin Shield mode override (stored in SitePolicy)
- Temporary exception (session-only, not persisted)
- Permanent exception (persisted)
- Shield statistics: trackers blocked, ads blocked, requests blocked
- HTTPS upgrade status
- Site trust level

**Safe defaults:** Standard mode for all sites. Users may set per-site exceptions
or change the global default. Aggressive mode must be user-initiated.

**Compatibility warning:** Strict mode may break site functionality. FetchX must
surface clear user messaging when strict blocking is likely responsible for breakage.

**Dependencies:** Phase 20B-A (Browser Policy Core).

**Tests:** Shield mode activation, per-site override, statistics counting, exception
persistence, GeckoView ContentBlocking integration, standard vs strict behavior.

**Exit criteria:** Functional Shield system on device. Statistics visible in UI.
Per-site overrides work and persist across sessions.

---

### Phase 20B-C -- Content Filtering and Filter Lists (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer) + potential WEBEXTENSION layer

**Goal:** Establish a sustainable content filtering system with managed filter lists.
The implementation path MUST follow a GeckoView capability audit before committing
to a specific engine approach.

**GeckoView Capability Audit (required first):**
Classify each implementation approach:
- `NATIVE_GECKO` -- GeckoView ContentBlocking handles this natively
- `WEBEXTENSION` -- requires a bundled WebExtension (e.g., uBlock Origin subset)
- `FETCHX_IMPLEMENTABLE` -- FetchX can implement in Kotlin/Rust without engine changes
- `ENGINE_LEVEL` -- requires changes to GeckoView itself; not possible in FetchX
- `LIMITED` -- partial support with caveats

| Capability | Classification | Notes |
|------------|----------------|-------|
| Basic ad blocking | NATIVE_GECKO | Via ETP category flags |
| Full ABP/uBlock filter syntax | ENGINE_LEVEL / WEBEXTENSION | Cannot implement full adblock syntax without engine or extension |
| Network request interception (full) | WEBEXTENSION | WebExtension webRequest API |
| Element hiding / cosmetic filtering | WEBEXTENSION | Content script injection |
| First-party URL rewriting | LIMITED | Partial via NavigationDelegate |
| DNS-based blocking | ENGINE_LEVEL | Not exposed by GeckoView |

**Architecture (Brave-inspired, GeckoView-adapted):**
```
FilterListManager
        |
        +-- FilterListIndex (list catalogue)
        +-- FilterListUpdater (background updates with integrity check)
        +-- RuleParser (Adblock Plus / uBO syntax subset)
        +-- RuleMatcher (network request matching)
        +-- CosmeticEngine (element hiding, content script)
        +-- FilterStatistics (match counts per list and per site)
```

**Filter list candidates:**
- EasyList (ads)
- EasyPrivacy (tracking)
- Peter Lowe's Ad and tracking server list
- Regional lists (opt-in)
- FetchX-maintained curated list
- User custom rules

**Filter list management requirements:**
- Versioned lists with SHA-256 integrity check
- Automatic background updates (configurable interval)
- Rollback on failed/malformed update
- Malformed-rule isolation (one bad rule must not break the whole list)
- User enable/disable per list
- Performance limit: filter matching must not measurably delay page load
- Memory limit: total filter set must fit within Android background memory budget

**Important:** Do NOT assume full uBlock-compatible behavior is achievable without
a WebExtension. Phase 20B-C includes the audit that determines which filtering path
is used. Implementation choices may include:
1. Native GeckoView ContentBlocking (simplest; limited syntax support)
2. Bundled uBlock Origin WebExtension (most capable; extension overhead)
3. Hybrid: native for network blocking + extension for cosmetic filtering

**Dependencies:** Phase 20B-B (FetchX Shields).

**Exit criteria:** At least one filter list (EasyList or EasyPrivacy) operational.
Filter-list update mechanism tested. Per-list statistics tracked.

---

### Phase 20B-D -- Advanced Tracking Protection (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer)

**Goal:** Extend beyond basic ETP to cover advanced tracking vectors.

**Targets (subject to GeckoView capability audit):**

| Protection | Classification | Notes |
|------------|----------------|-------|
| URL tracking parameter stripping | WEBEXTENSION or FETCHX_IMPLEMENTABLE | Strip utm_*, fbclid, gclid, etc. from URLs before navigation |
| Referrer policy enforcement | LIMITED / WEBEXTENSION | GeckoView does not expose per-site referrer policy in public API |
| Redirect tracking protection | WEBEXTENSION | Detect and unwrap tracking redirects |
| Third-party cookie blocking | NATIVE_GECKO | ACCEPT_NON_TRACKERS or ACCEPT_FIRST_PARTY |
| Storage partitioning / dFPI | NATIVE_GECKO | GeckoRuntimeSettings cookie behavior |
| Cross-site tracking via localStorage | ENGINE_LEVEL | First-party isolation partially available |
| Login state tracking (link decoration) | WEBEXTENSION / LIMITED | |
| Cryptomining protection | NATIVE_GECKO | ContentBlocking.AntiTracking.CRYPTOMINING |
| Notification permission abuse | CONFIGURABLE | Aggressive default-deny policy for notification permission |
| Global Privacy Control header | FETCHX_IMPLEMENTABLE | Inject GPC header via NavigationDelegate |
| Anti-tracking URL parameters | FETCHX_IMPLEMENTABLE | Strip known tracking params from all navigated URLs |

**URL Tracking Parameter Stripping (high priority):**
- Strip known tracking parameters (utm_source, utm_medium, utm_campaign, fbclid,
  gclid, msclkid, mc_eid, yclid, etc.) from URLs before loading
- Maintain a curated, updatable tracking-parameter list
- Do not strip parameters that may affect site functionality
- Log stripped parameters in Shield statistics

**Global Privacy Control:**
- Inject `Sec-GPC: 1` header on all requests
- Make this a user-configurable option (default: on in Strict mode)

**Dependencies:** Phase 20B-B (FetchX Shields), Phase 20B-C (Content Filtering).

**Exit criteria:** Tracking parameter stripping operational and tested.
GPC header injection tested. Per-feature capability matrix documented.

---

### Phase 20B-E -- Advanced Fingerprinting Protection (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer)

**Goal:** Reduce browser fingerprint entropy without making FetchX more uniquely
identifiable through naive spoofing.

**Core principle:**
> Prioritize reducing entropy and maintaining consistent privacy-preserving behavior.
> Do NOT blindly randomize fingerprint values.
> Do NOT make FetchX more unique through naive spoofing.
> Consistent values across sessions are often more private than random-per-session values.

**Fingerprinting surface capability matrix:**

| Surface | Classification | FetchX Approach |
|---------|----------------|-----------------|
| Canvas fingerprinting | NATIVE_GECKO | GeckoView noise injection (ContentBlocking.AntiTracking.FINGERPRINTING) |
| WebGL fingerprinting | NATIVE_GECKO | Same as Canvas via GeckoView |
| WebAudio fingerprinting | NATIVE_GECKO | GeckoView fingerprinting protection |
| Font enumeration | ENGINE_LEVEL | GeckoView exposes a generic font set; cannot fully restrict |
| Screen dimensions / devicePixelRatio | LIMITED | Cannot reliably override without engine support |
| Timezone | LIMITED | Android system timezone; cannot spoof without engine support |
| Language / Accept-Language | FETCHX_IMPLEMENTABLE | Override Accept-Language header to reduce entropy |
| Hardware concurrency | LIMITED | GeckoView may report navigator.hardwareConcurrency; no override exposed |
| Device memory | LIMITED | navigator.deviceMemory; no override in current GeckoView public API |
| User-Agent | NATIVE_GECKO (already set) | Set to Firefox/GeckoView version; do not expose GeckoView token |
| Client Hints (UA-CH) | LIMITED | GeckoView may respond to CH requests; no direct policy API exposed |
| WebRTC IP leakage | CONFIGURABLE | Restrict WebRTC via permission policy; mDNS where available |
| Battery API | ENGINE_LEVEL | Not normally exposed in Firefox/GeckoView |
| Sensor APIs | ENGINE_LEVEL | Not normally exposed in Firefox |
| Storage identifiers (localStorage, IndexedDB) | NATIVE_GECKO | dFPI / storage partitioning |
| Connection type / NetworkInformation | LIMITED | GeckoView exposes this; no public override |

**User-Agent strategy:**
- Maintain a standardized Firefox/GeckoView User-Agent string matching the installed GeckoView version
- Do not expose `GeckoView/` embedder token to sites
- Update UA string when GeckoView is upgraded
- Do not randomize UA per-request (increases uniqueness)

**GeckoView fingerprinting protection (NATIVE_GECKO):**
- Use `GeckoRuntimeSettings.setFingerprintingProtection(true)` for canvas/WebGL/audio noise injection
- Verify GeckoView applies noise consistently and does not increase uniqueness

**WebRTC IP leakage (CONFIGURABLE):**
- Default: restrict WebRTC peer-to-peer IP discovery (mDNS-only or disable non-proxied UDP)
- Policy: configurable in SecurityPolicy / FetchXBrowserPolicy

**ENGINE_LEVEL limitations (must document, not hack around):**
All ENGINE_LEVEL surfaces are documented in the UI as limitations. FetchX will not
introduce unsafe workarounds (e.g., JavaScript injection to override navigator properties)
that could be detected and used to fingerprint FetchX users specifically.

**Dependencies:** Phase 20B-A (Browser Policy Core), Phase 20B-B (FetchX Shields).

**Exit criteria:** Canvas/WebGL/audio noise injection tested via automated test page.
WebRTC IP restriction tested. ENGINE_LEVEL limitations documented in UI.

---

### Phase 20B-F -- Browser Security Hardening (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer)

**Goal:** Systematic browser security hardening. Builds on Phase 20O (Android
Security Audit) with browser-specific security controls.

**Process isolation note:**
> FetchX will NOT reimplement Chromium's multi-process isolation.
> FetchX relies on GeckoView's supported process and sandbox architecture.
> This phase audits and verifies GeckoView's process model rather than replacing it.

**Security hardening areas:**

**HTTPS and mixed content:**
- HTTPS-only mode (already in Phase 20J; verify and extend)
- Mixed-content blocking: block active mixed content; warn on passive
- Certificate error handling: no silent override; user must confirm
- HSTS awareness (GeckoView handles this natively)
- Dangerous download warnings (flag downloads triggered from HTTP pages)

**Permission controls (via GeckoSession.PermissionDelegate):**
- Camera: default deny; prompt on request; per-site persist
- Microphone: default deny; prompt; per-site persist
- Location: default deny; prompt; per-site persist
- Notifications: aggressive default deny; prompt; per-site persist
- Persistent storage: default deny; prompt
- Autoplay: configurable (default: block audio autoplay)

**Popup and redirect policy:**
- Block unrequested popups (GeckoSession.onPopupBlock or equivalent)
- Allow user to review blocked popups
- Detect and warn on suspicious redirect chains

**WebRTC security:**
- Restrict non-proxied WebRTC UDP (mDNS-only or disable)
- Prevent local IP leakage via WebRTC

**Extension security:**
- Preserve Phase 20K/20L extension permission model
- No silent permission escalation
- Extension installation must require user confirmation
- Extension origin permissions audited before installation

**Navigation security:**
- External URI scheme handling: only well-known safe schemes (mailto:, tel:) auto-handled
- intent: / market: schemes: prompt user before delegating to external app
- file:// restriction: local file access restricted to explicitly opened files
- javascript: scheme: blocked in navigation
- data: URI navigation: restricted (navigating to data: URIs disallowed)

**Download security:**
- Flag downloads from HTTP (non-HTTPS) origins
- Warn on executable file types (.apk, .exe, .sh, etc.)
- Download destination safety (no path traversal -- already in Phase 20N)

**GeckoView process model audit:**
- Document GeckoView's content process and GPU process architecture
- Verify FetchX does not run privileged operations in the content process
- Audit IPC boundaries between browser UI, GeckoView content, and Rust Core
- Document trust boundaries: browser -> Rust Core -> download system

**Dependencies:** Phase 20O (Android Security/Privacy Audit), Phase 20B-A, 20B-B.

**Exit criteria:** Permission policy tested on device. HTTPS-only tested.
Mixed-content blocking confirmed. External URI handling audited.
GeckoView process model documented.

---

### Phase 20B-G -- Privacy and Security Regression Testing (HIGH)

**Platform Coverage:** ANDROID ADAPTER (browser layer)

**Goal:** Automated and manual regression test suite ensuring all privacy/security
features work correctly and do not regress across GeckoView upgrades.

**Test categories:**

**Tracker blocking tests:**
- ETP category activation (AD, ANALYTIC, SOCIAL, CRYPTOMINING, FINGERPRINTING)
- Per-site tracker exception
- Shield mode change (Standard -> Strict -> Off)
- Statistics increment on block

**Filter list tests:**
- Rule parser: valid rules parsed; malformed rules isolated
- Rule matcher: network request match/no-match
- Filter list update: successful fetch, checksum verify, apply
- Filter list rollback on bad update
- Performance: filter matching latency under 5ms for typical page

**Cookie isolation tests:**
- Third-party cookie blocking: verify no cross-site cookies
- Storage partitioning: verify localStorage isolated per first-party
- Session cookie behavior in normal vs incognito

**Fingerprinting tests:**
- Canvas noise injection verified via test page
- WebGL noise injection verified
- WebAudio fingerprint variation verified
- UA string does not expose GeckoView embedder token
- WebRTC local IP not leaked (mDNS-only verified)

**HTTPS tests:**
- HTTPS-only mode: HTTP navigation redirected or blocked
- Mixed content: active mixed content blocked
- Cert error handling: cert error shown; no silent override

**Permission tests:**
- Camera request: prompt shown; deny by default
- Microphone request: prompt shown; deny by default
- Location request: prompt shown; deny by default
- Notification request: prompt shown; aggressive-deny default

**WebRTC tests:**
- Local IP not leaked via WebRTC in restricted mode

**Extension security tests:**
- Extension with dangerous permissions blocked
- Extension permission prompt displayed correctly
- Extension cannot access origins outside its declared scope

**Navigation security tests:**
- javascript: URI blocked in navigation
- data: URI navigation blocked
- External intent URI requires user confirmation
- file:// navigation restricted

**Per-site policy tests:**
- Site override persists across sessions
- Site exception removes protection correctly
- Temporary exception expires at session end

**Test infrastructure:**
- JVM unit tests for pure Kotlin policy logic
- Android instrumentation tests for GeckoView integration
- Mock HTTP server for network-level tests
- Browser test pages (privacy check pages) for on-device verification
- Regression fixture set: URL list + expected block/allow decisions

**Dependencies:** All 20B-A through 20B-F phases.

**Exit criteria:** All test categories pass. Regression fixture set established.
GeckoView upgrade process includes running this test suite.

---

### Phase 20B-H -- Shields and Site Controls UI (MEDIUM)

**Platform Coverage:** ANDROID UI (browser layer)

**Goal:** Expose FetchX Shields and all browser privacy/security controls through
a coherent, FetchX-native UI. Does NOT copy Brave's exact UI design.

**Shield panel (address bar -> Shield icon -> panel):**
```
FetchX Shield Panel
+---------------------------------------+
| [Shield icon]  Site: example.com       |
|                Shield: Standard [v]    |
+---------------------------------------+
| Trackers blocked:       12             |
| Ads blocked:             8             |
| Requests blocked:       20             |
| Fingerprinting:     Protected          |
| Cookies:              Isolated         |
| HTTPS:                    On           |
+---------------------------------------+
| [Per-site exceptions]  [Details]       |
+---------------------------------------+
```

**Global privacy/security settings screen:**
- Shield mode default (Off / Standard / Strict)
- Tracker blocking category controls
- Fingerprinting protection toggle
- Cryptomining protection toggle
- Cookie policy selector
- HTTPS-only toggle
- JavaScript policy (global; per-site in site panel)
- Referrer policy (if implemented)
- Tracking URL parameter stripping toggle
- GPC (Global Privacy Control) toggle
- Filter list management

**Site-specific panel:**
- Override Shield mode for this site
- Allow/block cookies for this site
- Allow/block JavaScript for this site
- View blocked items for this site
- Remove site exception

**Permission status:**
- Show per-site permissions granted (camera, microphone, location, notifications)
- Revoke per-site permission from UI

**Design principles:**
- FetchX design language; do not copy Brave branding
- Statistics shown only if meaningful (not zero-state noise)
- Advanced controls behind a secondary "Details" or "Advanced" panel
- Aggressive mode changes require user confirmation
- Clear messaging when Strict mode may cause site breakage

**Dependencies:** Phase 20B-B through 20B-F; Phase 20B-G (testing).

**Exit criteria:** Shield panel displays live statistics. Per-site controls work.
Global settings correctly propagate to GeckoView. UI tested on device.

---

---

### Phase 20P -- CoreServices Consolidation (CRITICAL)

**Status: IMPLEMENTATION COMPLETE.** Windows, Android, and CLI now use the
shared `CoreServices` graph. Rust tests, Windows desktop/CLI builds, Android
native bridge builds, and Android debug/release APK builds pass. A device
startup smoke test remains unverified when no Android device is connected.

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
- [x] Migrate desktop Tauri initialization to `crates/core-services`
- [x] Integrate Android Bridge with shared CoreServices
- [x] Remove duplicate production service construction from desktop AppState
- [x] Unify storage, queue, scheduler, engines, event bus, configuration,
  shutdown lifecycle, error handling, dependency injection
- [x] Ensure CLI and desktop use the same service graph

**Windows requirements:** Met; desktop AppState retains the CoreServices owner and
only exposes cloned service handles to Tauri commands.
**Android requirements:** Met; BridgeInitializer owns the Rust bridge, whose
DownloadService constructs CoreServices.
**CLI requirements:** Met; CLI bootstrap uses CoreServices with host-specific paths.

**Tests:** Shared graph startup/shutdown and repeated/concurrent shutdown,
concurrent independent startups, persisted state recovery, engine startup failures,
desktop bootstrap, CLI bootstrap, and Android bridge service initialization/shutdown.

**Exit criteria:** Desktop and CLI use identical CoreServices/service graph.
No duplicated service initialization in any client.

**Device validation:** Android arm64/x86_64 Rust bridge and debug/release APKs
build successfully. Device launch validation is pending because no Android
device was connected during this phase.

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

### Phase 20T -- Download Reliability Hardening (HIGH)

**Status: IMPLEMENTATION COMPLETE; AUTOMATED VALIDATION PASS; LIVE-ENGINE VALIDATION PARTIAL.**

Hardened HLS progress checkpoint persistence and yt-dlp output finalization recovery.
Progress persistence now reports concurrent lifecycle changes rather than silently
discarding them, and yt-dlp can reconcile a validated output after a crash between
publishing the file and persisting `Completed`. CLI, desktop, and Android remain
adapters over shared Rust/CoreServices; this phase does not add a headless server
or remote-fetch implementation.

**Validation:**
- [x] Workspace Rust check, serial workspace tests, and strict Clippy
- [x] Desktop UI build and Windows desktop/CLI Rust builds
- [x] Android debug unit tests, APK assembly, install/launch, UniFFI bridge initialization, and process-restart recovery on a connected arm64 device
- [x] Deterministic HTTP, HLS, segmented/Turbo, aria2/torrent, yt-dlp, and FFmpeg regression tests
- [ ] Validate live aria2/BitTorrent workflows with a real daemon
- [ ] Exercise the live torrent lifecycle on-device (aria2 daemon unavailable)
- [ ] Validate Tauri packaging and real external-engine workflows

This phase remains **PARTIAL** until the unavailable live aria2 workflow and
Tauri packaging checks can be run. Android bridge startup, Downloads UI, and
process restart were verified on-device; live torrent validation is blocked by
the unavailable aria2 daemon. The available automated tests do not demonstrate
real daemon-restart recovery or race-proof filesystem confinement.

---

### Phase 20W.4 -- Android Download Lifecycle Validation

**Status: PARTIAL; Direct HTTP lifecycle live-validated on the Android emulator.**

On-device validation used a local range-capable, throttled HTTP fixture:
- A segmented 64 MiB Direct HTTP download was paused at 39,059,456 /
  67,108,864 bytes. Its `paused` state and segment files survived an app
  force-stop/relaunch. Resuming completed at 67,108,864 / 67,108,864 bytes;
  the destination file was 67,108,864 bytes and the `completed` state survived
  another relaunch.
- A separate Direct HTTP transfer was cancelled at 65,863,680 /
  67,108,864 bytes. Its `cancelled` state remained persisted and segmented
  partial data was retained; no completed destination was published.
- A separate 32 MiB Direct HTTP fixture completed successfully.

Previously recorded live YouTube Auto and manual-format downloads also remain
valid evidence; manual selection is not repeated here. Automated Rust workspace
format/check/test/Clippy validation and the Android browser, bridge, downloads,
and app unit-test tasks plus debug APK assembly passed.

This phase does not claim live aria2/BitTorrent daemon lifecycle coverage.
yt-dlp active pause remains explicitly unsupported; its supported completion
path is covered by the existing live YouTube evidence and automated tests.

The Phase 20W.4.2 physical-device native-torrent lifecycle remains
**BLOCKED / NOT VALIDATED**. The connected V2135 device could not receive a ping
reply from the host Wi-Fi address (`10.74.55.222`), so no physical-device torrent
completion, seeding, Stop Seeding, queue-release, persistence, or file-integrity
claim is made. No FetchX networking changes were made to bypass this environment
limitation.

---

### Phase 21A -- FetchX Remote Protocol (CRITICAL)

**Status: PROTOCOL CONTRACT IMPLEMENTED; AUTOMATED VALIDATION PASS.**

**Platform Coverage:** SHARED CORE (protocol definition); all clients consume same protocol

The new `fetchx-remote-protocol` crate defines bounded JSON-RPC 2.0 request and
response envelopes, validates request IDs, protocol version, supported methods,
and typed command parameters, and advertises the `fetchx.rpc.v1` protocol version
and implemented capabilities through `system.discover`. It defines event
notification names for download, queue, engine, and plugin lifecycle events.

Key commands: `download.create`, `download.get`, `download.list`, `download.pause`,
`download.resume`, `download.cancel`, `download.retry`, `download.remove`,
`download.setPriority`, `download.setEngine`, `download.setSpeedLimit`.

Events: `download.created`, `download.updated`, `download.progress`, `download.paused`,
`download.resumed`, `download.completed`, `download.failed`, `queue.updated`,
`engine.started`, `engine.stopped`, `engine.failed`, `plugin.enabled`, `plugin.disabled`.

Protocol versioning: `fetchx.rpc.v1`. Every connection discovers protocol version,
server version, supported capabilities.

This phase provides the shared wire contract and validation only. It does not add
network transport, command dispatch, authentication, or authorization; these remain
in Phases 21B, 21C, and 21D.

**Windows:** Windows .exe acts as a local server and remote client.
**Android:** Phase 21I's browser integration is an in-app GeckoView adapter to
CoreServices, not a remote browser client.
**Shared:** Protocol definition and validation in shared Rust.

---

### Phase 21B -- Headless Server (CRITICAL)

**Status: IMPLEMENTED; LOOPBACK INTEGRATION VALIDATION PASS.**

**Platform Coverage:** SERVER/REMOTE (platform-independent) -> SHARED CORE

Standalone server using CoreServices -> FetchX Core. No Tauri dependency.
HTTP/JSON-RPC + WebSocket event transport. `fetchx server` defaults to
`127.0.0.1:39800`; request bodies, active RPC/WebSocket sessions, dispatch time,
WebSocket frames, and per-client event queues are bounded. Graceful shutdown closes
event sessions and drains active handlers before shutting down CoreServices.

Implemented RPC operations: discovery, HTTP(S)/magnet create, get/list, pause, resume,
cancel, remove, and priority. Remote DTOs omit source URLs and filesystem paths.
Event streaming uses the existing CoreServices download event bus. Retry, engine
selection for existing jobs, per-download speed limits, scheduler, plugin, and config
operations are not advertised unless a future shared service API can support them.
Authorization, TLS, origin validation, SSRF protections, and LAN/public deployment
remain out of scope; bind only to loopback.

**Windows:** Server can run as Windows service or standalone process.
**Android:** Phase 21I uses the in-app GeckoView bridge; Android does not start a
local network server for browser-extension requests.
**Shared:** CoreServices graph identical to desktop and CLI.

---

### Phase 21C -- Authentication and Device Pairing (CRITICAL)

**Status: TOKEN AUTHENTICATION IMPLEMENTED; LOCAL HTTP/WEBSOCKET INTEGRATION TESTED.**

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER (secure storage differs)

Implemented for the headless server: explicit CLI initialization and rotation of a
256-bit bearer token, SHA-256 verifier persisted through SQLite app configuration,
central authenticated HTTP/JSON-RPC dispatch, authenticated WebSocket event access,
and fail-closed restart behavior. This is a local token-authentication baseline, not a
pairing/session system.

Device identity, pairing codes, refresh tokens, device/session management, OS-specific
secret stores, and LAN exposure remain unimplemented. Authentication alone does not
make LAN/public deployment safe.

**Windows:** Verifier is in the existing local SQLite configuration database.
**Android:** No Android authentication client/server integration in this phase.

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

**Implementation status: PARTIAL.** The React/TypeScript remote client uses
`fetchx.rpc.v1` for Dashboard, Downloads, Queue, and System/Health, including
authenticated RPC and the WebSocket event stream. Engine management/status,
Scheduler, Settings, Devices, and Plugins remain explicitly unavailable because
the current server discovery contract does not expose those operations. The web
client does not simulate or duplicate those features. A follow-up protocol/core
scope is required before those views can become functional.

---

### Phase 21G -- Docker / NAS Deployment (HIGH)

**Platform Coverage:** SERVER/REMOTE

Official FetchX server container. Security baseline: non-root, read-only root
filesystem where practical, no-new-privileges, restricted volumes, healthcheck,
configurable resource limits.

Targets: NAS, home server, VPS, Docker host.

**Implementation status: PARTIAL.** A multi-stage container image and hardened
Compose deployment are documented in [docs/DOCKER_DEPLOYMENT.md](DOCKER_DEPLOYMENT.md).
The image build and container runtime have not been validated because the Docker
Engine is unavailable in the current environment.

---

### Phase 21H -- Remote CLI (HIGH)

**Platform Coverage:** SHARED CORE + CLI ADAPTER

Extend existing CLI to use the same remote protocol.

Examples: `fetchx remote connect`, `fetchx remote status`, `fetchx remote downloads`,
`fetchx remote add`, `fetchx remote pause`, `fetchx remote resume`, `fetchx remote cancel`.

No duplicate business logic in CLI.

---

### Phase 21I -- Browser Remote Integration (HIGH)

**Platform Coverage:** SHARED CORE (protocol) + WINDOWS ADAPTER + ANDROID IN-APP ADAPTER

On Windows, the browser extension targets either the local desktop through
authenticated loopback IPC or a manually configured remote FetchX server through
the authenticated remote protocol. Remote target selection applies to the
Windows extension/native host.

**Windows:** Native messaging host supports local and remote routing.
**Android:** The app's built-in GeckoView extension dispatches browser RPC
through the Android bridge to CoreServices. This is same-device, in-app browser
integration; it does not expose a LAN/network RPC listener. External
Chrome/Firefox-to-Android connections, Android remote-target selection, pairing,
device discovery, and refresh tokens are not implemented in Phase 21I.
Phase 22A's broader browser download interception and queue-routing behavior is
also outside Phase 21I.

**Status: COMPLETE.** The Windows extension/native-host local and authenticated
remote routing uses the shared `fetchx.rpc.v1` contract. The Android in-app
GeckoView extension action, popup, and native-message path were live validated
through the Android bridge and CoreServices: `system.discover`, `download.list`,
and `download.create` returned through the real popup. An IANA `robots.txt`
download completed at 24 bytes with SHA-256
`e5c4b84484ee4216e9373be99380320c25dd94805f99f0a805846f087636553f`;
its file and database record persisted after force-stop and relaunch. The
Android test tasks passed (317 tests, no failures, errors, or skips), and the
targeted Rust Android bridge RPC test passed. The emulator's observed System UI
ANR was separate from FetchX; no FetchX crash or ANR was observed. No Git
history changes were made during validation.

---

### Phase 22A -- Browser Download Interception (HIGH)

**Platform Coverage:** SHARED CORE (download handling) + BOTH PLATFORM ADAPTERS

Flow: `Browser -> extension -> FetchX -> URL analysis -> engine selection -> queue`

Download request MUST enter the shared Rust downloader queue on both platforms.

Support: direct HTTP/HTTPS downloads, filename detection, and multiple
simultaneous downloads. MIME is determined from FetchX's own HTTP response;
browser MIME values are not forwarded. Referrer forwarding is excluded until an
explicit user-consent policy exists.

Do NOT bypass: DRM, CAPTCHA, authentication, access controls, anti-bot protections.

**Windows:** Via native messaging host -> CoreServices.
**Android:** Via Android FetchX local service -> Bridge -> CoreServices.

**Status: COMPLETE; IMPLEMENTED, TESTED, AND LIVE VALIDATED ON WINDOWS AND ANDROID.**
Windows interception pauses before RPC submission, cancels only after a
correlated acceptance, resumes after an explicit rejection, and preserves a
paused item when the result is ambiguous. Android queues GeckoView external
responses for confirmation and routes accepted HTTP(S) downloads through the
bridge. Both paths use the shared `download.create` operation and sanitized
suggested filenames; browser-specific adapters reject non-HTTP(S) URLs.
Validation passed for the extension tests (14), TypeScript check, Chromium and
Firefox builds, workspace Rust tests (660 passed, 0 failed, 7 ignored), and
Android browser/bridge/app unit-test tasks (317 passed, 0 failures, errors, or
skips). Strict workspace Clippy is not clean due to two unrelated baseline
lints in `crates/models/src/progress.rs` and
`crates/remote-server/src/auth.rs`; no Phase 22A-specific warning remains. The
Android GeckoView external-response confirmation path was live validated on
`Pixel_10_Pro`: accepting the `httpbin.org/response-headers` response created a
completed `direct_http` record with 136/136 bytes. The destination file was
136 bytes with SHA-256
`7a13f3e34ecf508e1c5ed12e26a37c4546b34cbfef7ed0f5d307e8640bcb0549`.
The completed record and file remained present after force-stop and relaunch.
Windows live validation used Chrome 156 and an isolated profile. A successful
handoff canceled the browser item only after FetchX acceptance and completed a
174/174-byte `direct_http` download with SHA-256
`d42cbf89145094b020e1804e58ef80b769038f13937251193179062c50a15426`; the file
matched the fixture and the database record/file persisted after desktop
relaunch. An explicit remote-target rejection resumed Chrome's item, which
completed at 168 bytes without creating a FetchX record. The test exposed
Chrome's delayed filename reporting; the extension now waits for the resolved
filename before submitting `download.create`.

The browser-provided MIME value is not forwarded; Direct HTTP reads
`Content-Type` from its own response, while Android uses response MIME only to
derive a suggested filename. Referrer forwarding remains intentionally
excluded until an explicit user-consent policy exists. Cookies, credentials,
and other request headers are not forwarded. GeckoView cancellation cannot
resume an external response.

---

### Phase 22B -- Firefox Extension (HIGH)

**Platform Coverage:** SHARED CORE (browser protocol) + EXTENSION ADAPTER

Complete Firefox support alongside existing Chrome/Edge support.
Shared browser-protocol crate; only extension manifest format differs.

**Status: COMPLETE — implemented, tested, and live validated on Windows with
Firefox 159.** The Firefox Manifest V2 adapter uses the shared interception
and RPC logic, with a separate native-host manifest and Firefox extension-ID
allowlist. A real accepted download completed at 215/215 bytes, matched its
expected SHA-256, and persisted in both the filesystem and database after
force-stop/relaunch. Explicit FetchX rejection and unavailable-native-host
fallbacks were live-tested with a 2 MiB range-capable fixture; the rejected
request created no FetchX record. Extension tests (26), TypeScript checking,
Chromium/Firefox builds, Rust formatting/check, and workspace tests (660 passed,
0 failed, 7 ignored) passed. Strict Clippy remains blocked by a pre-existing
`collapsible_if` warning in `crates/models/src/progress.rs`.

Known limitation: Firefox may not resume a paused download when the origin
does not support HTTP range requests; the extension reports the failed browser
recovery instead of silently claiming success.

---

### Phase 23A -- Plugin Security (HIGH)

**Platform Coverage:** SHARED CORE + WINDOWS MANAGER + ANDROID POLICY BRIDGE

**Status:** COMPLETE for the Phase 23A security-policy scope. The shared typed
manifest, fail-closed evaluator, Windows manager enforcement, and Android
UniFFI policy bridge are implemented and validated. The Android emulator ran the
built-in security instrumentation successfully: 4 of 4 assertions passed, covering
unknown/untrusted/revoked trust, missing permission or capability approval,
unsupported runtime capability, stale fingerprint, malformed manifest, and
undeclared permission handling. The Android bridge remains deny-closed because no
plugin runtime or privileged capability adapters exist; it does not claim a
positive Android plugin grant path. A user-facing approval workflow remains
future work, outside the Phase 23A runtime contract.

Upgrade plugin architecture with:
PluginManifest, PluginPermission, PluginCapability, PluginTrust, PluginSignature, PluginVersion.

Permission categories: network, filesystem, downloads, metadata, process, engine, settings.

No silent dangerous permission grants. Cryptographic signature verification is
deferred to Phase 23C; OS sandboxing is deferred to Phase 23B.

---

### Phase 23B -- Plugin Sandbox Hardening (HIGH)

**Platform Coverage:** SHARED CORE + BOTH PLATFORM ADAPTERS

**Status:** IMPLEMENTED(SCOPE-SPECIFIC) for the runtime controls that exist in
FetchX's current host model. The manager remains out-of-process and enforceable
via the Phase 23A policy gate, and it now tracks bounded IPC, startup/shutdown
timeouts, and crash-loop shutdowns. Actual OS sandboxing is not claimed where a
platform lacks a real privileged execution adapter.

Current enforced controls:
- out-of-process launch with host-owned lifecycle management;
- IPC payload size bounds;
- startup/shutdown timeout enforcement;
- crash-loop detection and supervisor disablement;
- deny-closed behavior on Android unless a real plugin runtime exists.

Controls that require a true host adapter remain platform-limited and must be
reported as `NOT ENFORCED` or `NOT AVAILABLE` when they cannot be proven.

---

### Phase 23C -- Plugin Signing (HIGH)

**Platform Coverage:** SHARED CORE

**Status: COMPLETE (scope-specific) — shared signing and manager enforcement
implemented and tested.** Ed25519 signatures use a versioned canonical
manifest payload bound to the current installed plugin-directory tree hash,
publisher key fingerprint, plugin version, permissions, and capabilities.
Explicit local publisher trust and publisher/plugin revocation are enforced
before manager startup; Phase 23A approval and runtime-support checks remain
mandatory. The plugin API and manager test suites passed (66 tests total).

Scope limitations: FetchX has no distributable plugin archive, so this signs the
installed directory artifact model rather than a future archive format. Trust
and revocation are in-memory for the manager lifetime; persistent trust,
authenticated key rotation, and remote revocation distribution remain future
work. Android exposes shared policy/signing code but has no plugin execution
adapter. Windows manager tests prove signature verification and tamper denial,
but no positive Windows plugin startup/live validation is available because the
host currently supports no plugin capabilities.

Do NOT implement fake "verified" states without actual cryptographic verification.

---

### Phase 23D -- Plugin Registry (MEDIUM)

**Platform Coverage:** SHARED CORE

**Status: SHARED REGISTRY MODEL IMPLEMENTED; marketplace scope remains
deferred.** The shared plugin API defines strict untrusted catalog metadata,
compatibility and download validation, a provider abstraction with a local
provider, and computed assessment against Phase 23C signatures and local
publisher trust. Registry contents include plugin name/version, publisher,
permissions/capabilities, signature metadata, compatibility, release notes,
and per-target download information.

The registry does not fetch remote catalogs or artifacts, install plugins,
grant Phase 23A approvals, bypass Phase 23B runtime checks, or provide
marketplace UI. Its artifact-admission result is not installation permission;
the artifact hash is supplied by the caller and is not measured by this API.
These behaviors require separately scoped implementation and validation.

Not a prerequisite for the core plugin runtime.

---

### Phase 24 -- Production Storage and Migration (MEDIUM)

**Platform Coverage:** SHARED CORE + PLATFORM ADAPTER (filesystem paths differ)

**Status: STORAGE MAINTENANCE AND CLI IMPLEMENTED; ISOLATED ANDROID MIGRATION AND
PERSISTENCE VALIDATED; WINDOWS RUNTIME VALIDATION REMAINS OPEN.** Storage provides
read-only integrity/schema diagnostics, validates
migration history, snapshots existing databases before applying migrations, and
refuses unsupported future or inconsistent schemas. SQLite snapshot backups are
validated before success. A migration failure retains the recovery snapshot and
original database.

CLI commands: `fetchx db check`, `fetchx db repair`, `fetchx db backup`,
`fetchx db export`, and `fetchx db import`. Repair is intentionally conservative and
does not attempt destructive corruption recovery. Versioned exports omit local
configuration/secrets and refuse likely credential-bearing URLs or private tracker
data. Import validates paths under an allowed downloads root, takes a recovery backup,
resolves existing symlinks to prevent path escape, requeues interrupted work, and
transactionally replaces download, schedule, and
torrent metadata. Other FetchX processes must be stopped before import.

Storage unit tests cover diagnostics, migration snapshots/failure, backup validation,
export/import round trips, interrupted-work recovery, and rejected inputs. On a
Pixel_10_Pro Android 17 emulator, an isolated `dev.fetchx.app.phase24test` installation
migrated a schema-v10 app-private fixture to v11, preserved its paused download, passed
SQLite integrity validation, and retained a valid pre-migration snapshot. After an
OS-level force-stop, a second app instrumentation run relaunched the application and
revalidated the migrated record, schema, and integrity. The existing
`dev.fetchx.app.debug` installation/database was not modified. This validates the
Android migration and process-restart persistence path only; Windows runtime migration
and broader platform persistence checks remain separate release validation. The Android
application has no CLI maintenance surface; database maintenance commands remain CLI-only.

---

### Phase 25 -- Observability and Diagnostics (MEDIUM)

**Platform Coverage:** SHARED CORE + UI/UX per platform

**Status: SHARED DIAGNOSTICS, CLI, AND ANDROID BRIDGE IMPLEMENTED; PHYSICAL-DEVICE
INSTRUMENTATION VALIDATED; BROADER RUNTIME VALIDATION OPEN.** On the V2135 Android 13
device, the Phase 25 diagnostics instrumentation test passed (1 test, 0 failures),
verifying bridge snapshot availability, healthy storage status, and omission of the
private database path. The test used an isolated application ID to avoid replacing the
existing debug installation. This does not validate broader download, RPC, plugin, or
storage-maintenance runtime behavior. Shared snapshots include queue counts/throughput,
download lifecycle and queue counters, storage health/migration/integrity results, RPC
outcomes, plugin lifecycle counters, and bounded recent events. Queue state provides
total/active counts, downloaded bytes, current and average speed; upload bytes are
included when active torrent telemetry is available. Storage reports health-check latency.
`fetchx diagnostics --json` performs a read-only database inspection; CoreServices
snapshots are exposed through the Android bridge. RPC request IDs are generated
internally and no diagnostic RPC method is published.

Snapshots and events omit passwords, tokens, cookies, session strings, API keys,
credentials, source URLs, database paths, request payloads, and raw error strings.
Metrics and history are process-local. Runtime validation and any metrics not currently
backed by an observable source remain open; do not treat source tests as live evidence.

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

### Phase 26D -- Automated Release Builds (MEDIUM)

**Status: IMPLEMENTATION COMPLETE; GitHub-hosted workflow execution pending.**
The release workflow is configured to build Android APKs (arm64-v8a and x86_64),
Windows installers (NSIS `.exe` and `.msi`), Linux packages (`.deb` and AppImage),
macOS `.dmg`, and CLI binaries for Windows, Linux, and macOS. It attaches
SHA-256 checksums and creates a draft GitHub Release from either a version tag
or a manual workflow dispatch.

**Tasks:**
- [x] Build platform release artifacts with GitHub Actions
- [x] Support tag and manual version triggers
- [x] Generate checksums and create a draft release for review
- [x] Document optional Android and Apple signing configuration
- [ ] Run the workflow on GitHub and verify uploaded release artifacts

Android APK signing and Apple signing/notarization are optional repository-secret
configurations. Without Android signing secrets, the generated APK is unsigned;
Windows installers are currently unsigned. Linux RPM packaging and automated
version updates across manifests are not included. This phase completes the
release-workflow implementation, not the broader production packaging scope in
Phase 26A.

See [docs/RELEASING.md](RELEASING.md) for release triggers and signing setup.

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

#### Phase 28 performance evidence

**Status: COMPLETE — SCOPE-SPECIFIC / ENVIRONMENT-LIMITED** for desktop
performance and resource validation of existing shared-core behavior. The
deterministic HTTP benchmark covered payloads from 1 to 100 MiB, concurrency
through 12 requested downloads, queue scale through 1,000 items, fairness,
cancellation, pause/resume, bounded retries, mid-body interruption semantics,
and available CPU/memory/disk measurements. Detailed methodology, results,
and limitations are documented in
[PHASE28_BENCHMARK_REPORT.md](./PHASE28_BENCHMARK_REPORT.md).

Mid-body body-stream decode errors are intentionally non-transient under the
current retry policy: no automatic retry occurs, the partial file and Failed
state are preserved, and workers/connections are released. Safe manual
range-based resume is a separate behavior.

Limitations: Android performance was not validated because no device/emulator
was available; real torrent performance was not validated because no
reproducible peer/seeder environment was available; the sustained cycle stage
ran 583.3 seconds rather than the requested 600 seconds; memory-leak absence
and exact per-process physical disk I/O are not proven; strict workspace
Clippy remains blocked by a pre-existing unrelated `clippy::collapsible_if`
in `crates/models/src/progress.rs:21`. This performance evidence does not
implement or validate the proxy, SOCKS5, DNS-policy, or Tor-routing features
listed above.

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

### Phase 30 -- Production Reliability, Resilience & Failure Recovery (HIGH)

**Platform Coverage:** SHARED CORE; platform-specific runtime validation

Harden existing download lifecycle behavior under process interruption,
shutdown, network and filesystem failures, persistence errors, and queue
failures. Reuse the existing state machine, retry policy, SQLite storage,
queue recovery, diagnostics, and engine abstractions; add deterministic
integration and subprocess tests where safe and practical.

Validate interrupted-download recovery, retryable network failures and
exhaustion, persistence consistency, filesystem error reporting, queue
resource cleanup, legal state transitions, and idempotent lifecycle operations.
Report Android and remote/RPC validation separately; do not claim runtime
coverage without execution evidence. Do not duplicate retry mechanisms or
change the intentional non-retryable classification of mid-body decode errors
without evidence.

This phase is reliability hardening of existing capabilities. It does not
implement Download Intelligence, introduce unrelated features, or imply that
all platform/crash scenarios are live validated.

**Current status: PARTIAL — DEFERRED VERIFICATION.** Retry diagnostics,
injected filesystem-failure cases, missing-partial recovery, retry
cancellation/pause behavior, and a subprocess crash-recovery harness pass.
The downloader library suite passed
(270 passed, 3 ignored), including killing and recovering an isolated
downloader test process; this does not validate recovery of the FetchX Windows
application. The queue suite passed (35 tests), including a queue-drain test
after injected StorageFull errors; the storage suite passed (35 tests).
Changed-crate Clippy passed with the known unrelated warning allowed.
The original queue regression injects `StorageFull` engine errors across
multiple serialized items and verifies failed items persist, later downloads
complete, and the queue drains; the separate opt-in regression below covers
the actual downloader filesystem path.
An opt-in `phase30-queue-fault-injection` regression now routes a controlled
`StorageFull` failure through the real QueueManager, EngineDispatcher, and
DirectHttpEngine body-write path. It verifies the failed item remains
unpublished, the queue releases its single active slot, and a subsequent
queued item completes with the expected payload hash.
The core state-machine suite also verifies that a read-only SQLite database
rejects a lifecycle persistence update while preserving the prior persisted
state; this is deterministic storage-failure coverage, not live application
fault injection.
Workspace check and tests now pass under the reduced-memory Cargo profile;
strict workspace Clippy remains blocked by the pre-existing
`clippy::collapsible_if` warning in `crates/models/src/progress.rs:21`.
An isolated Android `phase30Test` package built, installed, and launched with a
separate UID and private database. This validates isolated startup only, not a
bridge-backed download. Android active-download recovery, live Android bridge
download execution, Windows app process recovery, remote download completion,
real OS permission/disk-full behavior, and queue cleanup after an actual
filesystem write failure remain unvalidated. See
[`PHASE30_RELIABILITY_REPORT.md`](./PHASE30_RELIABILITY_REPORT.md).

The Android browser verification follow-up is deferred until a safe,
adequately resourced target is available. Remaining checks are current-HEAD
test execution; Compose tab-grid runtime testing; GeckoView title/location
callback integration; TabManager and BrowserViewModel identity verification;
live process-restart restoration; investigation of the historical search
discrepancy; and explanation of snapshot generations 64–66. Historical test
reports are not evidence for the current HEAD, and this work does not claim
Phase 30 completion.

---

### Phase 31 -- Cloud / Device Sync (FUTURE)

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
18. **GeckoView capability audit required.** Every Brave-inspired browser privacy/security
    feature must be classified (NATIVE_GECKO / CONFIGURABLE / FETCHX_IMPLEMENTABLE /
    WEBEXTENSION / LIMITED / ENGINE_LEVEL) before implementation begins.
    Unsupported capabilities must be documented as limitations, not worked around unsafely.
19. **Browser privacy logic stays in the browser layer.** Browser-specific privacy/security
    controls live in the Android browser module. They must not duplicate or replace Rust Core
    download logic.
20. **Brave is reference, not dependency.** FetchX draws inspiration from Brave's Shields
    architecture and privacy concepts. FetchX will not include Brave Core, Chromium, or any
    Brave-specific code. GeckoView remains the browser engine.

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
| HIGH | 20R, 20S, 20B-A, 20B-B, 20B-C, 20B-D, 20B-E, 20B-F, 20B-G, 21F, 21G, 21H, 21I, 22A, 22B, 23A, 23B, 23C, 26B, 26C, 27, 30 |
| MEDIUM | 20B-H, 23D, 24, 25, 28, 29 |
| FUTURE | 31 |

---

## Cross-Platform Alignment Required

The following completed phases still have gaps in cross-platform coverage that
require dedicated alignment tasks:

| Phase | Gap | Resolution |
|-------|-----|-----------|
| 11B | aria2 not bundled for Android | Phase 26B |
| 12 | yt-dlp not bundled for Android | Phase 26B |
| 13 | FFmpeg not bundled for Android | Phase 26B |
| 17 | External browser extensions cannot connect to Android FetchX; in-app GeckoView bridge only | Not implemented in Phase 21I |
| 19 | Android does not yet support plugin processes | Phase 20P+ |
| 20D | Windows download UI not aligned with Android UI patterns | Phase 20P |
| 20N | Windows filename sanitization not formally aligned | Phase 20Q |

# Android Architecture

> **Phase 20 foundation document.** Describes the layered architecture of the
> MyDownloader Android application as established in Phase 20A.

---

## Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Jetpack Compose UI                           │
│  DownloadsScreen  BrowserScreen  SettingsScreen  HistoryScreen  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                 ViewModel / Use-case Layer                       │
│  DownloadsViewModel  BrowserViewModel  (Kotlin coroutines)      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│               Repository Interfaces (:core module)              │
│  DownloadRepository  (defined in :core, implemented in :bridge) │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│             Rust Bridge — UniFFI generated bindings             │
│  (Phase 20B: crates/mobile-api → Kotlin bindings in :core)     │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│              crates/core-services (existing Rust)               │
│  Database  EventRepository  ConfigService  EngineRuntime        │
│  EngineDispatcher  QueueManager  Scheduler  PluginManager       │
└─────────────────────────────────────────────────────────────────┘
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
         ┌──────────────────┐  ┌──────────────────┐
         │  crates/queue    │  │ crates/downloader │
         │  crates/scheduler│  │  DirectHttp       │
         │  crates/storage  │  │  aria2            │
         │  crates/models   │  │  yt-dlp           │
         └──────────────────┘  │  FFmpeg           │
                                └──────────────────┘
```

**Principle:** Kotlin/Compose owns Android presentation and platform integration.
Rust owns downloader business logic. The Android application must NOT reimplement
the Rust downloader.

---

## Module Breakdown

### :app (apps/android/app)

**Responsibility:** Application entry point, DI wiring, Activity, NavHost.

- `MyDownloaderApp` — `Application` subclass, initialises singletons.
  In Phase 20B this will load the UniFFI native library and initialise
  the `CoreServices` adapter.
- `MainActivity` — single `ComponentActivity`, calls `enableEdgeToEdge()`,
  sets Compose content with `AppNavigation`.
- `AppNavigation` — root `NavHost`, registers all top-level routes.
- `Screen` — sealed class for all navigation destinations.
- `MyDownloaderTheme` — Material 3 theme with dynamic colour (API 31+).

**Depends on:** `:core`, `:downloads`
**Will depend on (later phases):** `:browser` (20F), `:privacy` (20J), `:extensions` (20K)

### :core (apps/android/core)

**Responsibility:** Domain models, service interfaces, no Android framework APIs in
the main source set.

- `DownloadItem` — the canonical view of a download as seen by the UI layer.
- `DownloadState` — sealed class mirroring the Rust `DownloadState` enum.
- `DownloadEvent` — sealed class for engine events (Started, Progress, Paused, etc.).
- `DownloadPriority` — enum mirroring the Rust `Priority` enum.
- `DownloadRepository` — the interface the UI layer uses; implemented in Phase 20B.
- `NetworkRoute` — sealed class for routing (Direct, SystemVpn, SocksProxy, Tor).

**Depends on:** kotlinx-coroutines, lifecycle-viewmodel (for `StateFlow` support)
**Does NOT depend on:** any other module in this project.

### :downloads (apps/android/downloads)

**Responsibility:** Download manager UI — screens, ViewModels, row composables.

- `DownloadsScreen` — four-tab screen (Active, Queued, Completed, Failed).
- `DownloadItemRow` — composable row showing progress, speed, ETA, actions.
- `DownloadsViewModel` — stub in Phase 20A; wired to `DownloadRepository` in 20B.

**Depends on:** `:core`, Compose, Material 3, lifecycle-viewmodel-compose

### :browser (Phase 20F)

**Responsibility:** GeckoView browser shell — address bar, tabs, navigation.

**Depends on:** `:core`, GeckoView AAR (arm64, armeabi-v7a, x86_64)

### :privacy (Phase 20J)

**Responsibility:** Enhanced Tracking Protection configuration, content blocking
settings UI, per-site exception management.

**Depends on:** `:core`, `:browser`

### :extensions (Phase 20K)

**Responsibility:** WebExtension install/manage UI backed by
`GeckoView.WebExtensionController`.

**Depends on:** `:core`, `:browser`

---

## Rust Bridge Strategy

### Why UniFFI

UniFFI (Mozilla's Uniform Foreign Function Interface) was chosen over raw JNI for
these reasons:

1. **Safe generated bindings** — UniFFI generates Kotlin bindings from a `.udl`
   interface definition. No manual JNI is written by the developer.
2. **Type-safe** — complex Rust types (enums, structs, errors) are represented as
   idiomatic Kotlin types without manual marshalling.
3. **Maintainable** — internal Rust refactors do not break the Android binding as
   long as the `.udl` interface is stable.
4. **Production tested** — UniFFI is used by Mozilla in Firefox for Android to bridge
   Rust application-services to Kotlin.
5. **No unsafe** — the generated Kotlin code and the UniFFI scaffold are safe;
   `#![forbid(unsafe_code)]` is maintained in the mobile API crate.

### Bridge crate location

`crates/mobile-api` (Phase 20B):
- UDL file: `crates/mobile-api/src/mobile_api.udl`
- Generated Kotlin: placed in `:core/src/main/kotlin/dev/mydownloader/core/bridge/`
  as part of the Gradle build.
- The UDL defines `DownloadService`, `DownloadInfo`, `DownloadEvent`, `DownloadPriority`.
- The generated bindings implement the `DownloadRepository` interface defined in `:core`.

### Why crates/mobile-api is separate from crates/core-services

`crates/core-services` already exists and is shared by the desktop and CLI. The mobile
API crate wraps it rather than duplicating it. This keeps the mobile API stable
(only the UDL surface needs to be stable) while allowing `crates/core-services`
internals to evolve.

---

## Data Flow: Add Download

```
User taps "Add Download" in DownloadsScreen
    │
    ▼
DownloadsViewModel.addDownload(url, fileName, destination, priority)
    │
    ▼
DownloadRepository.addDownload(...)          [interface in :core]
    │
    ▼
RustDownloadRepository.addDownload(...)      [UniFFI impl, Phase 20B]
    │
    ▼
UniFFI-generated Kotlin: DownloadService.addDownload(...)
    │ (crosses JNI boundary)
    ▼
crates/mobile-api: MobileApi::add_download(...)
    │
    ▼
crates/core-services: QueueManager::add(...)
    │
    ▼
crates/queue: item enqueued, engine dispatcher assigns engine
    │
    ▼
Selected engine (DirectHttp / aria2 / yt-dlp) begins downloading
```

---

## Event Flow: Progress Update

```
Rust engine: fires DownloadEvent::Progress { id, bytes, total, speed, eta }
    │
    ▼
crates/mobile-api: UniFFI callback interface (Phase 20B)
    │ (crosses JNI boundary via generated callback)
    ▼
Kotlin: RustDownloadRepository emits to SharedFlow<DownloadEvent>
    │
    ▼
DownloadsViewModel collects from SharedFlow
    │
    ▼
_downloads StateFlow updated
    │
    ▼
Compose recomposition: DownloadItemRow re-renders progress bar, speed, ETA
```

The engine emits events; the Android layer never polls. This matches how the desktop
and CLI consume events from the existing `DownloadEvent` broadcast bus.

---

## Multi-Profile Design

| Profile type | Storage | History | Cookies | Session lifetime |
|---|---|---|---|---|
| Regular | Persistent (getExternalFilesDir) | Persistent SQLite | Persistent | Survives app restart |
| Private | In-memory / encrypted ephemeral | Not persisted | Not persisted | Wiped on GeckoSession close |
| Per-site isolation (Phase 20H+) | GeckoView session storage context | Per-context | Per-context | Per GeckoSession |

Private browsing uses `GeckoSessionSettings.usePrivateMode(true)`. A private tab's
GeckoSession is created fresh; when closed, its storage context is discarded.

---

## Background Execution Strategy

| Scenario | Mechanism | Phase |
|---|---|---|
| Active download (user-initiated, ongoing) | `ForegroundService` with `FOREGROUND_SERVICE_TYPE_DATA_SYNC` | 20E |
| Scheduled download (start at future time) | `WorkManager` `PeriodicWorkRequest` / `OneTimeWorkRequest` | 20E |
| Paused download (no CPU needed) | No service; state persisted in SQLite via Rust storage | 20E |

The foreground service binds to the Rust bridge; the Rust engine continues downloading
in its own thread pool. The Android service is responsible for lifecycle, the
notification, and graceful shutdown — not for download logic.

---

## Storage Strategy

| Use case | API | Permission required |
|---|---|---|
| App-internal downloads folder | `getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)` | None (app-scoped) |
| User-chosen directory | SAF `ActivityResultContracts.OpenDocumentTree` | None (SAF grant) |
| Media files (photos, video) | `MediaStore` insert | None (app-submitted to MediaStore) |
| Full filesystem access | NOT USED | Would require `MANAGE_EXTERNAL_STORAGE` — too broad |

`READ_EXTERNAL_STORAGE` and `WRITE_EXTERNAL_STORAGE` are not requested. The
`MANAGE_EXTERNAL_STORAGE` permission is explicitly not used (Google Play policy
restriction and unnecessary for the use case).

---

## NetworkRoute Abstraction

The `NetworkRoute` sealed class (in `:core`) flows through the system as follows:

```
User selects network route (UI setting)
    │
    ▼
DownloadRepository.addDownload(..., route: NetworkRoute)
    │
    ▼
UniFFI bridge: serialised to a Rust enum
    │
    ▼
crates/mobile-api: selects reqwest Client configured for the route
    │   Direct → default reqwest Client
    │   SystemVpn → same Client (VPN is device-level, transparent)
    │   SocksProxy(host, port) → reqwest with SOCKS5 proxy
    │   Tor → reqwest with SOCKS5 proxy to 127.0.0.1:9050 (Orbot)
    ▼
Download proceeds via selected network path
```

GeckoView browser sessions also accept a network route via
`GeckoRuntimeSettings.proxySettings` (Phase 20M).

---

## Phase Roadmap

| Sub-phase | Module(s) added/changed | Key deliverable |
|---|---|---|
| 20A | :app, :core, :downloads | Foundation, models, interfaces, download UI scaffold |
| 20B | crates/mobile-api, :core bridge impl | UniFFI bridge, real DownloadRepository |
| 20C | :app, :core | CoreServices Android adapter, engine runtime wiring |
| 20D | :downloads | Full download list UI (progress, speed, ETA, actions) |
| 20E | :app service | ForegroundService, WorkManager, notifications |
| 20F | :browser (new) | GeckoView shell, address bar, tabs, navigation |
| 20G | :browser | Download interception, handoff to DownloadRepository |
| 20H | :browser | Private tabs, per-session GeckoSession profiles |
| 20I | :browser | History, bookmarks (SQLite via Rust storage) |
| 20J | :privacy (new) | ETP configuration, tracker blocking, fingerprinting resistance |
| 20K | :extensions (new) | WebExtensionController, extension install |
| 20L | :extensions | Extension permission model |
| 20M | :core, :browser | Tor/VPN research and integration |
| 20N | :app, :downloads | Scoped storage, MediaStore, notification channels |
| 20O | All | Security/privacy audit |
| 20P | :app | Release build, signing, ProGuard, AAB |

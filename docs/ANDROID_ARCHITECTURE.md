# Android Architecture

> **Phase 20 foundation document.** Describes the layered architecture of the
> FetchX Android application as established in Phase 20A.

---

## Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Jetpack Compose UI                           │
│  DownloadsScreen  BrowserScreen  SettingsScreen  HistoryScreen  │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                 ViewModel / Use-case Layer                       │
│  DownloadsViewModel  BrowserViewModel  (Kotlin coroutines)      │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│               Repository Interfaces (:core module)              │
│  DownloadRepository  (defined in :core, implemented in :bridge) │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│             Rust Bridge — UniFFI generated bindings             │
│  (Phase 20B: crates/mobile-api → Kotlin bindings in :core)     │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│              crates/core-services (existing Rust)               │
│  Database  EventRepository  ConfigService  EngineRuntime        │
│  EngineDispatcher  QueueManager  Scheduler  PluginManager       │
└──────────────────────────────────────────────────────────────────┘
                              │
                    ┌──────────┬─────────┐
                    ▼                   ▼
         ┌─────────────────┐  ┌─────────────────┐
         │  crates/queue    │  │ crates/downloader │
         │  crates/scheduler│  │  DirectHttp       │
         │  crates/storage  │  │  aria2            │
         │  crates/models   │  │  yt-dlp           │
         └──────────────────┘  │ Native media      │
                                └──────────────────┘
```

**Principle:** Kotlin/Compose owns Android presentation and platform integration.
Rust owns downloader business logic. The Android application must NOT reimplement
the Rust downloader.

---

## Module Breakdown

### :app (apps/android/app)

**Responsibility:** Application entry point, DI wiring, Activity, NavHost.

- `FetchXApp` — `Application` subclass, initialises singletons.
  In Phase 20B this will load the UniFFI native library and initialise
  the `CoreServices` adapter.
- `MainActivity` — single `ComponentActivity`, calls `enableEdgeToEdge()`,
  sets Compose content with `AppNavigation`.
- `AppNavigation` — root `NavHost`, registers all top-level routes.
- `Screen` — sealed class for all navigation destinations.
- `FetchXTheme` — Material 3 theme with dynamic colour (API 31+).

**Depends on:** `:core`, `:downloads`
**Will depend on (later phases):** `:browser` (20F–20K)

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

Phase 20A: Scaffold screens with four tabs (Active/Queued/Completed/Failed).

Phase 20D: Full production UI. See _UI Layer (Phase 20D)_ section below.

**Depends on:** `:core`, Compose, Material 3, lifecycle-viewmodel-compose

### :browser (Phases 20E–20J)

**Responsibility:** GeckoView browser shell — address bar, tabs, navigation,
history, bookmarks, and native tracker protection.

Tracker blocking is configured once on the process-wide GeckoRuntime with its
built-in ad, analytics, and social tracking categories. GeckoView 137 also
requires `GeckoSessionSettings.useTrackingProtection`, which defaults to false;
`TabManager` enables it on each session before opening it. Each session forwards
`ContentBlocking.Delegate.onContentBlocked` events to a shared pure-Kotlin
`TrackerBlockingController`. It keeps bounded per-tab counts and host-only
details in memory; it does not implement a parallel URL rule matcher or persist
browsing data. The global in-memory toggle updates runtime categories for
subsequent requests without recreating sessions.

A V2135 Android 13 device test on public HTTPS `edition.cnn.com` observed native
blocked callbacks in logcat for GeckoView ad and analytics categories. Automated
tests cover controller filtering and state. Full physical multi-tab isolation,
tab cleanup, and the final-tab invariant remain unverified.

**Depends on:** `:core`, GeckoView AAR (arm64, armeabi-v7a, x86_64)

Phase 20J adds a `PrivacyController` and browser-wide controls in this module for
GeckoView fingerprinting protection, cryptomining blocking, third-party cookie
isolation, and HTTPS-Only mode. The controller combines those settings with
tracker blocking and applies them to the shared runtime, so existing and future
tabs use the same configuration. Settings are in-memory, not per-site, and are
not persisted.

Private browsing, per-site exceptions, and a separate `:privacy` Android module
are not implemented by Phase 20J.

### Browser extensions (Phase 20K)

**Responsibility:** Extension domain models, in-memory registry, runtime-scoped
lifecycle manager, GeckoView adapter, and the browser's minimal install/manage UI.
These components currently live in `:browser`; Phase 20K does not add a separate
Gradle module.

The manager uses the process-wide `GeckoRuntime.webExtensionController`, loads
GeckoView's installed-extension list at startup, and owns enable/disable/uninstall
operations. It outlives individual tabs and BrowserViewModel instances. Local
`.xpi` installation is staged in bounded app-private cache; GeckoView verifies
the package signature. Until Phase 20L, extensions requesting API or host
permissions are denied rather than granted automatically.

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
- Generated Kotlin: placed in `:core/src/main/kotlin/dev/fetchx/core/bridge/`
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

## Video Quality Selection

The browser asks the embedded yt-dlp runtime for available formats before adding a
media download. The selected format ID is persisted with the queued source. Adaptive
video-only formats are paired with the best available audio track; Android supplies
the FetchX native media compatibility shim to yt-dlp for supported merge-required
selections. Unsupported post-processing fails explicitly; no external FFmpeg
executable is bundled.

Format discovery also returns yt-dlp's video title and extractor/platform metadata.
The Android browser uses these with the selected quality label to name downloads
`<title> - <quality> - <platform>.<extension>`, for example
`Video title - 720p - XHamster.mp4`. When yt-dlp metadata is unavailable, the
browser falls back to the page title, host name, and best-quality label.

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

## Browser Session and Storage Scope

The browser currently has one normal process-wide `GeckoRuntime` shared by all
tabs. FetchX does not create private sessions or per-site GeckoView contexts.
Third-party cookie isolation is enabled through GeckoView's
`ACCEPT_FIRST_PARTY_AND_ISOLATE_OTHERS` cookie behavior; this is not a private
browsing profile.

Navigation history is in-memory, per tab, and capped at 200 entries. Bookmarks
are in-memory and shared across tabs. Neither is stored in SQLite by the browser
module. Closing a tab closes its GeckoSession and removes that tab's history and
tracker statistics; it does not claim to erase shared runtime/profile data.

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

The read-only **Storage usage** screen scans FetchX's app-specific downloads,
browser profile/cache, runtime workspaces, SQLite database/WAL/SHM, and remaining
private app data. Browser extensions are included in browser-profile usage.
Persisted SAF document trees are outside app-owned paths and are reported as not
measured; the screen never deletes downloads, profile data, cache, or workspaces.

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

## Native Build Pipeline (Phase 20C)

### Android NDK

- **NDK version:** r27.2.12479018
- **NDK path:** `D:\Android\Sdk\ndk\27.2.12479018`
- Installed via the command-line SDK tools (`sdkmanager`). Android Studio is not used.

### Rust Android Targets

The following Rust cross-compilation targets are installed in the project toolchain:

| Target triple | Android ABI | Status |
|---|---|---|
| `aarch64-linux-android` | `arm64-v8a` | Installed |
| `x86_64-linux-android` | `x86_64` | Installed |

`armeabi-v7a` is not targeted — the project's minimum SDK is 26 (Android 8.0), and
all modern physical devices support arm64-v8a.

### cargo-ndk

**Version:** 4.1.2

`cargo-ndk` wraps the NDK toolchain so that a regular `cargo build` command
cross-compiles for Android ABIs without manual linker flags.

Build command (one ABI at a time, release):

```
cargo ndk -t arm64-v8a -o apps/android/bridge/src/main/jniLibs build --release -p fetchx-android-bridge
```

Both ABIs in one pass:

```
cargo ndk -t arm64-v8a -t x86_64 -o apps/android/bridge/src/main/jniLibs build --release -p fetchx-android-bridge
```

### `.cargo/config.toml` Linker Configuration

`.cargo/config.toml` at the workspace root configures the NDK linkers for each
Android target so plain `cargo build --target aarch64-linux-android` also works:

| Target | Linker |
|---|---|
| `aarch64-linux-android` | `D:\Android\Sdk\ndk\27.2.12479018\toolchains\llvm\prebuilt\windows-x86_64\bin\aarch64-linux-android33-clang.cmd` |
| `x86_64-linux-android` | `D:\Android\Sdk\ndk\27.2.12479018\toolchains\llvm\prebuilt\windows-x86_64\bin\x86_64-linux-android33-clang.cmd` |

### Native Library

- **Crate:** `crates/android-bridge` (renamed/aliased `fetchx-android-bridge`)
- **crate-type:** `["cdylib", "lib"]` — `cdylib` produces the `.so` for Android;
  `lib` keeps the rlib for host-side `cargo test`.
- **Generated filename:** `libfetchx_android_bridge.so`

### UniFFI Kotlin Bindings

The `:bridge:generateUniFfiKotlinBindings` Gradle task generates the Kotlin
bindings from `crates/android-bridge/uniffi/android_bridge.udl` with the
workspace-locked UniFFI 0.29 bindgen. It runs before the bridge module's Android
build and writes the output under `apps/android/bridge/build/generated/`; Gradle
compiles that output as part of the bridge source set. Generated Kotlin bindings
are not checked into `src/main/kotlin`, preventing stale copies from being
compiled alongside one another.

The Rust native library is still built separately with `cargo ndk`. When the UDL
changes, regenerate/build both the Kotlin bindings and native library from the
same checkout before packaging the APK so their UniFFI API checksums agree.

### jniLibs Packaging

The `.so` files are placed in the `:bridge` module's `jniLibs` source directory
so that Gradle packages them into the APK automatically via `sourceSets`:

```
apps/android/bridge/src/main/jniLibs/
┌── arm64-v8a/
│   └─── libfetchx_android_bridge.so   ← generated by cargo-ndk
└─── x86_64/
    └─── libfetchx_android_bridge.so   ← generated by cargo-ndk
```

`apps/android/bridge/build.gradle.kts` wires this via:

```kotlin
android {
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}
```

### Git Policy for .so Files

- `.so` files are listed in `.gitignore` — compiled binaries are not committed.
- `.gitkeep` placeholder files exist in each ABI subdirectory so that the
  directory structure is preserved in git and Gradle does not emit warnings
  about missing `jniLibs` source sets.

### One-Shot Build Script

`scripts/build-android-native.ps1` automates the Rust cross-compilation step:

```powershell
.\scripts\build-android-native.ps1
```

The default is an optimized release build for each configured ABI. This avoids
packaging Rust debug information in Android APKs. Use `-Debug` only when native
debug symbols are needed; `-Release` is retained as an explicit equivalent to
the default. Both builds copy the resulting `.so` files into
`apps/android/bridge/src/main/jniLibs/<abi>/`.

After running this script, a normal `./gradlew assembleDebug` (or `assembleRelease`)
produces an APK that contains the Rust native libraries.

### UniFFI Build Validation History

Phase 20I stabilization found that a project rename had left two committed
generated Kotlin files in the bridge source tree. Both declared the same
`dev.fetchx.bridge.uniffi` types, and one contained older API checksums. The
bridge now generates a single binding from the current UDL during the Gradle
build. The native `.so` must still be rebuilt with `cargo ndk` from the same
checkout; an older binary can fail UniFFI's runtime checksum validation.

---

## Phase Roadmap

| Sub-phase | Module(s) added/changed | Key deliverable |
|---|---|---|
| 20A | :app, :core, :downloads | Foundation, models, interfaces, download UI scaffold |
| 20B | crates/mobile-api, :core bridge impl | UniFFI bridge, real DownloadRepository |
| 20C | :app, :core | CoreServices Android adapter, engine runtime wiring |
| 20D | :downloads | Full download manager UI (progress, speed, ETA, actions) |
| 20E | :app service | ForegroundService, WorkManager, notifications |
| 20F | :browser (new) | GeckoView shell, address bar, tabs, navigation |
| 20G | :browser | Download interception, handoff to DownloadRepository |
| 20H | :browser | Private browsing is deferred; no private sessions are implemented |
| 20I | :browser | In-memory per-tab history, in-memory bookmarks, tracker blocking |
| 20J | :browser | Runtime-wide fingerprinting, cryptomining, cookie-isolation, and HTTPS-only controls |
| 20K | :browser | Runtime-scoped WebExtension manager, local package install and lifecycle UI |
| 20L | :browser | Permission review UI and explicit permission decisions |
| 20M | :core, :browser | Tor/VPN research and integration |
| 20N | :app, :downloads | Scoped storage, MediaStore, notification channels |
| 20O | All | Security/privacy audit |
| 20P | :app | Release build, signing, ProGuard, AAB |

---

## UI Layer (Phase 20D)

Phase 20D replaced the placeholder download screen with a production-quality
download manager UI. All download state originates from the Rust core via the
existing DownloadRepository → BridgeDownloadRepository → UniFFI → Rust chain.
No business logic lives in composables.

### DownloadsUiState

```kotlin
data class DownloadsUiState(
    val isLoading: Boolean = false,
    val error: String? = null,         // user-readable; never a stack trace
    val bridgeAvailable: Boolean = true
)
```

Surface as `StateFlow<DownloadsUiState>` from `DownloadsViewModel`.

### DownloadsViewModel — filtered StateFlows

Each tab has its own `StateFlow<List<DownloadItem>>` derived from the source
`downloads` StateFlow using `map { }.stateIn(WhileSubscribed(5_000))`:

| StateFlow          | Filter predicate                     |
|--------------------|--------------------------------------|
| `allDownloads`     | All items, sorted newest-first        |
| `activeDownloads`  | `state is DownloadState.Active`       |
| `queuedDownloads`  | `state is DownloadState.Queued`       |
| `completedDownloads` | `state is DownloadState.Completed` |
| `failedDownloads`  | `state is DownloadState.Failed`       |
| `cancelledDownloads` | `state is DownloadState.Cancelled` |

### DownloadsScreen layout

```
Scaffold
┌── TopAppBar ("Downloads")
┌── FAB (Add Download)
┌── SnackbarHost
└─── Column
    ┌── [Bridge-unavailable banner if bridgeAvailable == false]
    ┌── TabRow (All | Active | Queued | Completed | Failed | Cancelled)
    └─── Content
        ┌── Empty state (All tab): icon + "No downloads yet" + Button("Add Download")
        ┌── Empty state (other tabs): Text("No <tab> downloads")
        └─── LazyColumn (key = item.id)
            └─── DownloadItemRow (per item)
```

### AddDownloadDialog fields

| Field            | Type         | Notes                                              |
|------------------|--------------|----------------------------------------------------|
| URL              | OutlinedTextField | Client-side: blank → error; no http/https → error |
| File name        | OutlinedTextField | Auto-derived from URL path segment; user-editable  |
| Destination      | Text (read-only) | `"$downloadsDir/$fileName"`                        |
| Priority         | FilterChip row | LOW / NORMAL / HIGH (CRITICAL never shown)         |

### Rule: CRITICAL priority not exposed in UI

`DownloadPriority.CRITICAL` is a Rust internal value (maps to `High` at the bridge
level). The `AddDownloadDialog` only shows LOW / NORMAL / HIGH. The
`PriorityBadge` in `DownloadItemRow` displays CRITICAL as "HIGH".

### Lifecycle and recomposition rules

- `collectAsStateWithLifecycle` used everywhere — not `collectAsState`.
- `LazyColumn` item keys are always `item.id` — stable identity prevents
  unnecessary item recomposition on list updates.
- `rememberSaveable` used for dialog state (`showAddDialog`, URL field, etc.)
  so configuration changes do not dismiss a half-filled dialog.
- No expensive sorting or filtering inside composable bodies — all filtering
  happens in the ViewModel's derived StateFlows.
- Snackbar errors shown via `LaunchedEffect(uiState.error)` — one snackbar
  per error event, cleared via `viewModel.clearError()` after display.

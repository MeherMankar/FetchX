# Roadmap

1. Phase 1 — Project foundation (completed)
2. Phase 2 — Domain models (completed)
3. Phase 3 — SQLite persistence (completed)
4. Phase 4 — Download state machine (completed)
5. Phase 5 — Direct HTTP downloader (completed)
6. Phase 6 — Resume and Range support (completed)
7. Phase 7 — Segment manager (completed)
8. Phase 8 — Adaptive acceleration (completed)
9. Phase 9 — Queue manager (completed)
10. Phase 10 — Persistent one-shot scheduler (completed)
11. Phase 11A — aria2 JSON-RPC client and engine adapter (completed)
12. Phase 11B — aria2 process supervision and restart recovery (completed)
13. Phase 12 — yt-dlp integration (completed)
14. Phase 13 — FFmpeg integration (completed)
15. Phase 14A — Tauri 2 desktop foundation with minimal React shell (completed)
16. Phase 14B — Live download events/progress (completed)
17. Phase 14C — Priority + scheduling controls (completed)
18. Phase 15A — Full React desktop UI (completed)
19. Phase 15B — UI polish: queue snapshot, bulk actions, drag-and-drop, keyboard shortcuts, confirmations, toasts, retry (completed)
20. Phase 16A — Engine configuration UI: persistent aria2 + yt-dlp settings, secret masking, diagnostic commands (completed)
21. Phase 16B — Runtime engine activation: EngineRuntime, EngineDispatcher integration, optional engine startup, graceful shutdown (completed)
22. Phase 16C — Engine selection in Add Download: per-download engine choice, backend availability validation, retry preserves engine (completed)
23. Phase 17 — Browser extension: MV3 Chrome/Firefox extension, native messaging host, context menu, popup, resource scanner, auto-intercept, loopback IPC server (completed)
24. Phase 18 — CLI (completed)

    Architecture: The CLI does NOT implement a second downloader. All
    download logic, queue management, scheduling, engine selection, and
    persistence are delegated to the existing Rust core through a new
    shared `crates/core-services` crate that wires the same services
    used by the desktop application.

    New crates:
    - `crates/core-services` — reusable service bootstrap (Database,
      EventRepository, ConfigService, EngineRuntime, Scheduler,
      QueueManager) extracted so the CLI does not duplicate the desktop's
      service wiring.
    - `crates/cli` — thin command dispatcher using clap; delegates
      entirely to core-services and existing queue/scheduler/repository.

    Commands implemented:
    `version`, `engines`, `config`, `add`, `list`, `status`,
    `pause`, `resume`, `cancel`, `remove`, `retry`, `priority`,
    `schedule`, `monitor`

    Features:
    - All commands support `--json` for machine-readable output
    - Documented exit codes (0 success, 1 error, 2 invalid args,
      3 not found, 4 invalid state, 5 engine unavailable, 6 config error)
    - Engine selection reuses existing availability validation — an
      unavailable engine is rejected, never silently downgraded
    - `monitor` subscribes to the existing DownloadEvent broadcast bus
      (no aggressive polling)
    - Secret redaction: aria2 RPC secret never appears in any output
    - Structured process invocation — no shell command construction
    - Cross-platform path resolution (Windows/Linux/macOS)
    - Ctrl+C handled cleanly via tokio::signal

    Known limitations:
    1. `retry` creates a new download ID and preserves the original
       partial files, consistent with the existing desktop retry behavior.
    2. `monitor` uses ANSI clear-screen sequences; non-ANSI terminals
       may not clear the previous display correctly.
    3. `schedule --at` accepts local wall-clock time only; no timezone
       selection is currently supported.
    4. The desktop `AppState` has not yet been migrated to use
       `crates/core-services`. The desktop and CLI therefore share
       design but still contain some duplicated service initialization
       code. Consolidation is a future task.

25. Phase 19 — Plugin architecture (completed)

    Architecture: Out-of-process plugin model. Plugins run as external
    child processes communicating with the host via a length-prefixed JSON
    protocol over stdin/stdout. No in-process dynamic loading.

    New crates:
    - `crates/plugin-api` — shared protocol types: versioned manifest
      schema, IPC framing (4-byte LE length + JSON, max 64 KiB), typed
      host request / plugin response messages, manifest validation.
    - `crates/plugin-manager` — `PluginManager` service: directory
      discovery, manifest validation, registry, process lifecycle
      (start/handshake/stop), bounded timeouts, failure isolation.

    Security boundary:
    - No secrets forwarded to plugin processes.
    - Structured process invocation only (no shell strings).
    - Executable paths must be absolute with no path traversal.
    - IPC messages bounded at 64 KiB before allocation.
    - `#![forbid(unsafe_code)]` in both new crates.
    - `deny_unknown_fields` on manifest and protocol types.

    Capabilities defined: engine, post_processor, url_resolver, metadata.

    CLI extension: `mydownloader plugins [list|info|status]` with --json.

    Known limitations:
    1. Built-in engines (DirectHttp, aria2, yt-dlp) are NOT converted to
       plugins. The `engine` capability defines the interface for future
       external engine plugins only.
    2. Plugin enable/disable state is not yet persisted in SQLite.
    3. No plugin sandboxing (filesystem/network isolation) in this phase.
    4. Plugin hot-reload and restart-on-crash are not implemented.

26. Phase 20A — Android foundation (completed)

    Browser engine decision: After evaluating Android System WebView (Chromium),
    GeckoView (Mozilla), Crosswalk (EOL), and a custom Chromium build, GeckoView
    was selected. Justification: GeckoView is the only embeddable Android engine
    with a real WebExtension API (via WebExtensionController), built-in Enhanced
    Tracking Protection, per-session profile isolation, and native private browsing.
    The ~37 MB arm64 APK size cost is accepted. Chromium WebView was rejected due
    to no extension API, no privacy controls, and no profile isolation.
    Full comparison: docs/ANDROID_BROWSER.md

    SDK versions:
    - minSdk = 26 (Android 8.0 Oreo — NotificationChannel, reliable JobScheduler)
    - targetSdk = 35 (Android 15 — Google Play requirement)
    - compileSdk = 35

    Android module structure created at apps/android/:
    - :app — MainActivity, MyDownloaderApp, AppNavigation, MyDownloaderTheme,
      Screen sealed class. Single-Activity architecture with Compose NavHost.
    - :core — Domain models: DownloadItem, DownloadState, DownloadEvent,
      DownloadPriority, DownloadRepository (interface), NetworkRoute.
      No Android framework APIs in the main source set. This is the stable
      interface the UI layer uses; never depends on the Rust bridge directly.
    - :downloads — DownloadsScreen (four tabs: Active/Queued/Completed/Failed),
      DownloadItemRow (progress, speed, ETA, actions), DownloadsViewModel (stub;
      wired to real DownloadRepository in Phase 20B).

    Gradle tooling:
    - Kotlin 2.0.21, AGP 8.7.3, Compose BOM 2024.12.01
    - Version catalog: apps/android/gradle/libs.versions.toml
    - GeckoView version pinned in version catalog comment (not added as
      dependency until Phase 20F browser module)

    Architecture: Kotlin/Compose owns Android presentation and platform
    integration. Rust owns downloader business logic. The Android application
    does NOT reimplement the Rust downloader. The DownloadRepository interface
    in :core will be implemented by a UniFFI-backed class in Phase 20B.

    Security configuration:
    - network_security_config.xml: cleartext HTTP blocked by default;
      localhost exempt for development tooling only.
    - Permissions declared: INTERNET, FOREGROUND_SERVICE,
      FOREGROUND_SERVICE_DATA_SYNC, POST_NOTIFICATIONS.

    New documentation:
    - docs/ANDROID_BROWSER.md — browser engine comparison and GeckoView decision
    - docs/ANDROID_ARCHITECTURE.md — layered architecture, data flow, event flow,
      module breakdown, storage strategy, background execution strategy
    - docs/ANDROID_PRIVACY.md — threat model, ETP, cookie isolation, private
      browsing, fingerprinting resistance, WebRTC controls, HTTPS-first
    - docs/ANDROID_EXTENSIONS.md — GeckoView WebExtensionController, available
      WebExtension APIs, permission model, security isolation, phase plan

    Rust workspace: unchanged. No Rust crates added or modified in Phase 20A.

    Next phase: Phase 20B — Rust ↔ Android bridge via UniFFI.
    Deliverables: crates/mobile-api crate, mobile_api.udl, generated Kotlin
    bindings, real DownloadRepository implementation, UniFFI callback model
    for DownloadEvent propagation.
27. Phase 20B — Rust ↔ Android bridge (completed)

    Bridge technology: UniFFI 0.29 with JNA backend for Android.
    Justification: UniFFI generates type-safe Kotlin bindings from a UDL
    interface file, eliminating handwritten JNI glue. The tokio feature on
    the uniffi dependency enables push-based async callback machinery for
    the event listener. The crate produces both a cdylib (for Android .so)
    and a lib target (for host cargo test).

    New Rust crate — crates/android-bridge:
    - DownloadService: wraps CoreServices behind a self-managed Tokio
      runtime. All async Rust work runs on the bridge's own runtime; the
      Android UI thread is never blocked.
    - Public API (14 methods): addDownload, listDownloads, getDownload,
      pauseDownload, resumeDownload, cancelDownload, removeDownload,
      retryDownload, setPriority, scheduleDownload, addEventListener,
      shutdown — plus CoreConfig constructor.
    - DTO layer: DownloadInfoDto, DownloadProgressDto, DownloadStatusDto,
      DownloadPriorityDto, DownloadEventDto. SQLx / internal Rust types
      never cross the FFI boundary.
    - MobileApiError enum: Validation, NotFound, UnavailableEngine,
      InvalidStateTransition, Storage, Runtime.
    - Event delivery: subscribes to the existing DownloadEventBus
      (broadcast channel) and forwards events to a Kotlin callback via
      UniFFI's callback interface — no polling.
    - Shutdown: idempotent; cleanly stops CoreServices and drops the
      Tokio runtime (joining all spawned tasks).
    - 14 unit tests: init/shutdown, double-shutdown, add/list/get,
      invalid URL, not-found errors, priority roundtrip, event delivery,
      retry state validation, concurrent calls.

    New Android module — :bridge:
    - Depends on :core (for the DownloadRepository interface) and JNA.
    - android_bridge.kt: UniFFI-generated Kotlin bindings, committed to
      source. Generated via uniffi-bindgen from the UDL source.
    - BridgeDownloadRepository: implements DownloadRepository from :core.
      Maps DownloadInfoDto ↔ DownloadItem, drives reactive StateFlow and
      SharedFlow from Rust events. All bridge calls dispatched to
      Dispatchers.IO; UI thread never blocked.
    - BridgeInitializer: singleton lifecycle manager. Double-checked
      locking; idempotent shutdown. Called from Application.onCreate().
    - 14 JVM unit tests: all DTO mapping combinations, CRITICAL→HIGH
      priority mapping, sentinel value handling, error message defaults.

    Wiring:
    - :app and :downloads both depend on :bridge.
    - MyDownloaderApp.onCreate() initializes the bridge.
    - DownloadsViewModel now accepts a real DownloadRepository and
      exposes reactive StateFlow + action methods (pause/resume/cancel/
      remove/retry/setPriority).
    - ViewModels never depend on JNI/UniFFI directly — only on the
      DownloadRepository interface from :core.

    Architecture summary:
      Compose → ViewModel → DownloadRepository → BridgeDownloadRepository
      → UniFFI DownloadService → Rust CoreServices → Queue/Scheduler/
      Storage/EngineDispatcher → DirectHTTP/aria2/yt-dlp

    Known limitations:
    1. Native .so files are not cross-compiled yet (no cargo-ndk / Android
       NDK build pipeline on the development machine). The APK compiles and
       all Kotlin bridge code is correct, but bridge calls will produce
       UnsatisfiedLinkError at runtime until the NDK pipeline is set up.
       jniLibs/ directories contain .gitkeep placeholders.
       → NDK pipeline limitation resolved in Phase 20C (see below).
    2. DownloadPriority.CRITICAL has no Rust equivalent; mapped to High.
    3. Engine status not yet surfaced via the DownloadRepository interface.
    4. ProcessLifecycleOwner shutdown not yet wired (Application.onTerminate
       is emulator-only); bridge lifetime is coextensive with the process.

    Validation:
    - cargo fmt / check / clippy: all pass, 0 warnings on new code.
    - cargo test --workspace: 316 tests pass (1 pre-existing desktop
      failure unrelated to Phase 20B).
    - ./gradlew assembleDebug: BUILD SUCCESSFUL (122 tasks).
    - ./gradlew :bridge:test: all JVM unit tests pass.
    - APK: apps/android/app/build/outputs/apk/debug/app-debug.apk (~10 MB).
28. Phase 20C — Android Rust Native Build & Runtime Integration (partial)

    Goal: package Rust native libraries into the Android APK so the UniFFI
    bridge can execute on a real physical device.

    Environment setup:
    - Android NDK r27 (27.2.12479018) installed at D:\Android\Sdk\ndk\27.2.12479018
      via command-line sdkmanager. Android Studio not used.
    - Rust Android targets installed: aarch64-linux-android, x86_64-linux-android.
    - cargo-ndk 4.1.2 installed; used to cross-compile the bridge crate.
    - .cargo/config.toml updated with NDK linker paths for both targets.

    Native build:
    - crates/android-bridge built as cdylib (+ lib for host tests).
    - arm64-v8a library: libmydownloader_android_bridge.so (~12.6 MB, unstripped debug)
    - x86_64 library: libmydownloader_android_bridge.so (~12.3 MB, unstripped debug)
    - Build script: scripts/build-android-native.ps1 -Release
    - Both .so files packaged into APK via apps/android/bridge/src/main/jniLibs/.
    - .so files excluded from git (.gitignore); .gitkeep placeholders preserve dirs.

    APK:
    - apps/android/app/build/outputs/apk/debug/app-debug.apk (~34 MB with .so)
    - APK confirmed to contain arm64-v8a and x86_64 native libraries.
    - :bridge:test (JVM unit tests): all pass.

    Physical device test result: FAILED (partial)
    - Device connected and APK installed successfully. App launched.
    - Two runtime failures observed:

      ISSUE 1 (bridge, primary): UnsatisfiedLinkError for JNA's own
      libjnidispatch.so (com/sun/jna/android-aarch64/libjnidispatch.so not found
      in resource path). The generated android_bridge.kt uses a JNA-based UniFFI
      scaffold; JNA requires its Android AAR variant (jna:X.Y.Z:@aar) to supply
      libjnidispatch.so. Either switch to the JNA Android AAR dependency, or
      regenerate the Kotlin bindings with the uniffi-bindgen version that matches
      uniffi 0.29 (which should not require JNA at all).

      ISSUE 2 (UI, secondary): FATAL EXCEPTION — DownloadsViewModel cannot be
      instantiated without a factory (NoSuchMethodException for the no-arg
      constructor). DownloadsViewModel requires a ViewModelProvider.Factory or
      dependency injection. Unrelated to the native build.

    Rust native libraries are now in the APK. Runtime bridge activation
    is blocked by the JNA configuration issue above. Resolved in next phase.

29. Phase 20D — Android Download Manager UI (completed)

    Implemented a production-quality download manager UI on top of the Phase 20B/20C
    Rust-UniFFI bridge. All state is derived from the existing DownloadRepository;
    no business logic was added to the UI layer.

    DownloadsViewModel changes:
    - Added DownloadsUiState(isLoading, error, bridgeAvailable) surfaced as StateFlow.
    - Replaced four computed get() filtered lists with six derived StateFlow<List<DownloadItem>>:
      allDownloads (sorted newest-first), activeDownloads, queuedDownloads,
      completedDownloads, failedDownloads, cancelledDownloads — all using
      SharingStarted.WhileSubscribed(5_000) to avoid spurious upstream subscriptions.
    - Added addDownload(url, fileName, destinationPath, priority): maps repository
      exceptions to user-readable strings — no stack traces or internal messages exposed.
    - Added clearError() for Snackbar dismissal.
    - Factory extended with bridgeAvailable: Boolean parameter.

    AddDownloadDialog (new):
    - AlertDialog with URL field, file name field (auto-derived from URL path segment),
      read-only destination hint, and priority chips (LOW/NORMAL/HIGH — CRITICAL not shown).
    - Client-side format validation: blank URL, missing http/https scheme.
    - rememberSaveable for all dialog state (survives configuration changes).
    - No URL values are logged.

    DownloadsScreen changes:
    - 6-tab TabRow: All, Active, Queued, Completed, Failed, Cancelled.
    - FAB (Add Download) → opens AddDownloadDialog.
    - Snackbar for ViewModel errors via SnackbarHostState + LaunchedEffect.
    - Bridge-unavailable banner (errorContainer) when uiState.bridgeAvailable == false.
    - Rich empty state on "All" tab: icon + "No downloads yet" + "Add Download" button.
    - Simple empty messages on other tabs.
    - All StateFlow collection uses collectAsStateWithLifecycle.
    - LazyColumn item keys = item.id.
    - No business logic in composables.

    DownloadItemRow changes:
    - Priority badge (Surface chip) beside engine badge: LOW/NORMAL/HIGH colour-coded.
    - Active state: percentage + downloaded/total size line below progress bar.
    - Completed state: destination filename hint shown beneath status line.
    - formatBytes/formatSpeed/formatEta promoted to internal visibility.

    AppNavigation changes:
    - bridgeAvailable flag passed to DownloadsViewModel.Factory.
    - downloadsDir resolved from LocalContext (getExternalFilesDir) and passed
      to DownloadsScreen — ViewModel stays Context-free.

    Tests added (DownloadsViewModelTest — 14 tests):
    - emptyStateWhenNoDl, downloadAppearsInList, addDownloadCallsRepository,
      pauseCallsRepository, resumeCallsRepository, cancelCallsRepository,
      retryCallsRepository, removeCallsRepository, filteringActiveReturnsOnlyActive,
      filteringCompletedReturnsOnlyCompleted, filteringCancelledReturnsOnlyCancelled,
      bridgeUnavailableSetsBridgeAvailableFalse, urlValidationEmptyShowsError,
      clearErrorResetsError.
    - FakeDownloadRepository: full DownloadRepository test double with MutableStateFlow
      and per-method call tracking.

    Validation:
    - cargo fmt --check: pass. cargo check --workspace: pass.
    - ./gradlew :downloads:test: 14/14 tests pass. BUILD SUCCESSFUL.
    - ./gradlew assembleDebug: BUILD SUCCESSFUL.
    - APK: apps/android/app/build/outputs/apk/debug/app-debug.apk.
    - Device test: no device connected during this phase.

    Review findings addressed (iteration 2):
    - lifecycle-runtime-compose added as an explicit direct dependency in
      downloads/build.gradle.kts and libs.versions.toml so collectAsStateWithLifecycle
      no longer relies solely on transitive exposure from lifecycle-viewmodel-compose.
    - mapAddDownloadError no longer interpolates e.message for IllegalArgumentException;
      the raw bridge/Rust message is never surfaced in the user-visible error string.
    - sanitizeFailureReason() 80-char passthrough tightened: strings containing colons
      or snake_case tokens (structural internal-origin signals) fall through to the
      generic fallback instead of being shown verbatim.

    Known limitations:
    1. The Phase 20C JNA/libjnidispatch UnsatisfiedLinkError remains unresolved at
       runtime. The UI falls back to NoOpDownloadRepository (empty list + bridge-
       unavailable banner displayed).
    2. No background download service yet (Phase 20E).
    3. Open-file action for completed downloads is not implemented — no filesystem
       behaviour was invented; only Remove is offered.

30. Phase 20E — Background downloads
30. Phase 20E — WebView browser shell
31. Phase 20F — Download interception
32. Phase 20G — Tabs / private tabs
33. Phase 20H — History / bookmarks
34. Phase 20I — Tracker blocking
35. Phase 20J — Privacy / fingerprinting controls
36. Phase 20K — Extension framework
37. Phase 20L — Extension permissions
38. Phase 20M — VPN / Tor integration research
39. Phase 20N — Android storage / notifications
40. Phase 20O — Security / privacy audit
41. Phase 20P — Android release build
42. Phase 21 — Remote/headless mode
43. Phase 22 — Production packaging and releases

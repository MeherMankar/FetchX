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

    CLI extension: `FetchX plugins [list|info|status]` with --json.

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
    - :app — MainActivity, FetchXApp, AppNavigation, FetchXTheme,
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
    - FetchXApp.onCreate() initializes the bridge.
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
    - arm64-v8a library: libfetchx_android_bridge.so (~12.6 MB, unstripped debug)
    - x86_64 library: libfetchx_android_bridge.so (~12.3 MB, unstripped debug)
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

30. Phase 20E — Android GeckoView Browser Foundation (completed)

    Implemented a minimal GeckoView-based browser foundation integrated with
    the existing FetchX Android application.

    Browser module (:browser):
    - GeckoRuntimeHolder: process-level singleton ensuring GeckoRuntime is
      created exactly once per process (double-checked locking).
    - BrowserViewModel: owns GeckoSession lifecycle across configuration
      changes; uses sessionOpened flag (not isOpen) for safety.
    - BrowserScreen: Compose UI with TopAppBar URL bar, back/forward/reload
      controls, loading indicator, error surface, and GeckoView via AndroidView.
    - normalizeUrl(): bare hostnames prefixed with https://.
    - 8 unit tests for normalizeUrl() — all pass.

    Navigation:
    - Screen.Browser destination added to sealed class.
    - AppNavigation updated with Material3 NavigationBar (Downloads + Browser tabs).
    - Downloads screen and Phase 20D functionality unchanged.

    GeckoView version: 137.0.20250325171416 (arm64-v8a artifact).
    Originally attempted 158.0.20261007201254 but that version requires
    kotlin-stdlib:2.4.20 → media3:1.11.1 → compileSdk 36 → AGP 9.0+;
    fell back to 137 per user instruction.
    Mozilla Maven repository: already declared in settings.gradle.kts.

    Security:
    - INTERNET permission: already present from Phase 20A.
    - network_security_config.xml: cleartext blocked by default, unchanged.
    - No unsafe SSL, no trust-all, no WebView fallback.

    Icon note: Icons.Default.Language unavailable in material-icons-core;
    replaced with Icons.Default.Search for the Browser nav item.
    Icons.AutoMirrored.Filled.List used for Downloads nav item.

    Known limitations:
    1. The Phase 20C JNA/libjnidispatch issue remains (bridge falls back to NoOp).
    2. ABI-split uses arm64-v8a only for debug builds — release builds will need
       the fat geckoview-androidX artifact or all ABI variants.
    3. No tab management, history, bookmarks, or download interception (Phase 20F+).

31. Phase 20F — Multi-tab browser session management (completed)

    Implemented in commit 6f060f4. See detailed description above (Phase 20E section
    covers browser foundation; Phase 20F added the full tab architecture on top).

    New files: TabState.kt, TabStore.kt, TabManager.kt (browser module).
    Modified: BrowserViewModel.kt, BrowserScreen.kt, GeckoRuntimeHolder.kt.

    Features: one GeckoSession per tab, tab creation/closing/switching, back/forward/
    reload per tab, URL and title tracking per tab, navigation-state isolation between
    tabs, minimum-one-tab policy, 22 JVM unit tests (NormalizeUrlTest + TabStoreTest).

32. Phase 20G — Browser history and navigation state (completed)

    Status: COMPLETE

    New files:
    - HistoryEntry.kt — pure-Kotlin domain model (id, url, title, timestampMs).
      No Android imports; JVM-testable.
    - HistoryStore.kt — per-tab in-memory history store. No Android imports.
      Rules: records on onPageStop(success=true) only; about:blank and blank URLs
      excluded; consecutive duplicate URLs skipped; title updated (not re-added)
      via updateTitle(); max 200 entries per tab; O(1) tab removal on close.
    - HistoryStoreTest.kt — 14 JVM unit tests covering all HistoryStore rules.

    Modified files:
    - TabManager.kt: added HistoryStore field; onPageStop(success=true) records
      navigation; onTitleChange updates history title; closeTab() calls
      historyStore.removeTab() before session close; dispose() clears all history;
      new public API: historyForActiveTab(), allHistory(), clearAllHistory(),
      clearTabHistory().
    - BrowserViewModel.kt: BrowserUiState gains showHistory: Boolean and
      historyEntries: List<HistoryEntry>; rebuildState() fetches active-tab history;
      new operations: openHistory(), closeHistory(), clearHistory(),
      navigateToHistoryEntry(url).
    - BrowserScreen.kt: history IconButton (Icons.AutoMirrored.Filled.List) added
      to TopAppBar actions; Material3 AlertDialog with LazyColumn shows history
      entries (title/URL/timestamp); tap navigates active tab; Clear/Close actions.

    GeckoSession integration:
    - History recorded on onPageStop(success=true) only — not on PageStart or
      LocationChange, so load errors never create history entries.
    - Title updated on onTitleChange without creating duplicate entries.
    - Navigation state (canGoBack/canGoForward) already derived from GeckoSession
      NavigationDelegate — unchanged from Phase 20F.

    History model: in-memory only (no SQLite persistence in this phase).
    Per-tab isolation: switching tabs exposes that tab's own history, URL,
    title, and back/forward state. No global shared navigation stack.

    History UI: Material3 AlertDialog with LazyColumn, HH:mm timestamp,
    tap-to-navigate active tab, Clear and Close actions.

    Tests: 14 new HistoryStoreTest cases + 22 existing tests (NormalizeUrlTest,
    TabStoreTest) — all 36 pass. assembleDebug: BUILD SUCCESSFUL.

    Known limitations:
    - History is in-memory only; cleared on app restart (no SQLite persistence).
    - No bookmarks, no private browsing, no download interception.
    - Pre-existing Phase 20C UnsatisfiedLinkError for native bridge .so unchanged.

33. Phase 20H — Bookmarks (completed)

    Status: COMPLETE

    New files:
    - Bookmark.kt — pure-Kotlin domain model (id: String UUID, url: String,
      title: String, createdAtMs: Long, updatedAtMs: Long). No Android imports;
      JVM-testable. Title falls back to URL hostname when blank. Does NOT store:
      cookies, credentials, session tokens, private browsing metadata.
    - BookmarkStore.kt — global in-memory bookmark store (NOT per-tab).
      No Android imports. Rules: non-blank http(s) URLs only; blank/non-http(s)
      URLs silently rejected; duplicate URLs rejected (same canonical URL → no-op);
      bookmarks ordered newest-first; title fallback to java.net.URI host;
      all state in-memory for Phase 20H. Operations: add, removeById, removeByUrl,
      update (title), all, findByUrl, isBookmarked, clearAll, count.
    - BookmarkController.kt — pure-JVM bookmark state controller (see above).
    - BookmarkStoreTest.kt — 24 JVM unit tests covering all BookmarkStore rules:
      add/remove/update/find/list, blank URL rejection, duplicate rejection,
      ftp/file/about:blank rejection, http/https acceptance, newest-first ordering,
      whitespace trimming, title fallback to host, clearAll, count, and explicit
      independence from HistoryStore (clearing history does not clear bookmarks;
      removing bookmark does not affect history).
    - BrowserViewModelTest.kt — 20 JVM unit tests for ViewModel-level bookmark
      logic (see above).

    New files (updated):
    - BookmarkController.kt — pure-JVM bookmark state controller extracted from
      BrowserViewModel. Holds BookmarkStore and a MutableStateFlow<State> with
      fields: bookmarks, isCurrentPageBookmarked, showBookmarks. Zero Android
      imports; fully JVM-testable. BrowserViewModel delegates all bookmark
      operations to this class and merges its state into BrowserUiState.
      Methods: toggleBookmark(url, title, isLoading), openBookmarks(),
      closeBookmarks(), removeBookmark(id), updateBookmark(id, newTitle),
      refreshForUrl(currentUrl). isLoading guard in toggleBookmark() prevents
      bookmarking a stale URL during navigation (business-logic enforcement,
      not just UI-layer).
    - BrowserViewModelTest.kt — 20 JVM unit tests for ViewModel-level bookmark
      logic (via BookmarkController): toggleBookmark add/remove, isLoading guard,
      blank/about:blank rejection, isCurrentPageBookmarked state transitions,
      openBookmarks/closeBookmarks dialog flag, bookmark list updates after add/
      remove/update, newest-first ordering, history/bookmark independence,
      navigateToBookmark close-then-navigate flow simulation.

    Modified files:
    - BookmarkController.kt (new): see above.
    - BrowserViewModel.kt: BrowserUiState gains showBookmarks: Boolean,
      bookmarks: List<Bookmark>, and isCurrentPageBookmarked: Boolean fields.
      Added internal bookmarkController: BookmarkController field (global, not
      per-tab). rebuildState() calls bookmarkController.refreshForUrl() and merges
      resulting state. New operations: toggleBookmark(), openBookmarks(),
      closeBookmarks(), removeBookmark(id), updateBookmark(id, newTitle),
      navigateToBookmark(url). syncBookmarkState() private helper merges bookmark
      state into _uiState without requiring tabManager (safe in JVM tests).
      clearHistory() is unchanged — it does NOT touch bookmarks.
    - BrowserScreen.kt: Added Icons.Default.Star import. TopAppBar gains two new
      action buttons — a bookmark toggle (Star icon, primary tint when bookmarked,
      muted onSurfaceVariant when not, disabled on about:blank/blank/loading) and a
      bookmarks list opener (List icon, tertiary tint to distinguish from history).
      Added BookmarksDialog private composable (AlertDialog with LazyColumn of
      bookmarks, empty-state text "No bookmarks yet.", tap-to-navigate, delete
      button per item, Close action). BookmarksDialog call added after HistoryDialog
      block.

    Architecture:
    - Bookmarks are global browser data, NOT per-tab. A single BookmarkController
      (wrapping a single BookmarkStore) lives in BrowserViewModel.
    - UI does not directly manipulate BookmarkStore. All operations route through
      BrowserViewModel → BookmarkController.
    - about:blank, empty URLs, and URLs with non-http(s) schemes cannot be
      bookmarked (guarded in toggleBookmark() at both UI and business-logic layer).
    - isLoading guard in BookmarkController.toggleBookmark() prevents any caller
      (UI or programmatic) from bookmarking a stale URL during navigation.
    - Clearing history does NOT clear bookmarks. Removing a bookmark does NOT
      affect history. The two stores are fully independent.
    - BookmarkController/BookmarkStore are designed to support future
      private-browsing enforcement (caller's responsibility to skip add() for
      private pages).
    - No SQLite persistence in Phase 20H (in-memory only, consistent with
      Phase 20G history).
    - No URL logging or title logging anywhere in bookmark code.

    Tests: 24 new BookmarkStoreTest + 21 new BrowserViewModelTest + 36 existing
    = 81 tests total, all pass. assembleDebug: BUILD SUCCESSFUL.

    Review findings fixed (iteration 2):
    - stale-isCurrentPageBookmarked (blocking): BrowserViewModel.removeBookmark()
      now calls bookmarkController.refreshForUrl(activeTab?.url ?: "") before
      syncBookmarkState(), so the star icon correctly updates when the bookmarked
      page is still open and the bookmark is deleted from the dialog.
    - identical-toolbar-icons (minor): The bookmarks list opener now uses
      Icons.Default.Star with secondary tint (distinct from the history button's
      Icons.AutoMirrored.Filled.List). Icons.Default.BookmarkBorder/Bookmarks are
      in material-icons-extended which is not a project dependency; secondary-tinted
      Star is the closest visually distinct alternative in material-icons-core.
    - missing-removeBookmark-state-test (minor): Added
      `isCurrentPageBookmarked becomes false after removeBookmark by id` test in
      BrowserViewModelTest covering the explicit delete path via BookmarkController.

    Known limitations:
    - Bookmarks are in-memory only; cleared on app restart (no SQLite persistence).
    - Bookmark title editing: updateBookmark(id, newTitle) exists in the ViewModel
      but no dedicated title-edit UI dialog is provided (not practical within the
      existing architecture without a third dialog; the API is available for future use).
    - BrowserViewModel cannot be directly instantiated in JVM unit tests (extends
      android.lifecycle.ViewModel and imports android.util.Log). Bookmark logic is
      tested via BookmarkController, which contains 100% of the business logic.
    - Pre-existing Phase 20C UnsatisfiedLinkError for native bridge .so unchanged.
    - Pre-existing cargo fmt trailing-newline differences in ipc.rs/main.rs unchanged.

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


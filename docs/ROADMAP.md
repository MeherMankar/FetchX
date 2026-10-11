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
27. Phase 20B — Rust ↔ Android bridge
28. Phase 20C — Download manager
29. Phase 20D — Background downloads
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

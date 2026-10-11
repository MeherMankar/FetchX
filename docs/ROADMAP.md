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

26. Phase 20 — Android
27. Phase 21 — Remote/headless mode
28. Phase 22 — Production packaging and releases

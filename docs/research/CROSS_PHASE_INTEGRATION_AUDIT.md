# Cross-Phase Shared-Core Integration Audit

## Scope and worktree baseline

This audit verifies the completed work through the JDownloader 2 phase and checks whether shared-core capabilities are actually consumed by each supported host. It does not add another repository-inspired feature.

- Before Prompt 13 edits, `master` was at `573c30b` (`feat: implement per-source package grouping for batch downloads in CoreServices`), matching `origin/master`; `git status --short` was empty. The phase work was committed through that revision.
- The Android duplicate-warning integration, this audit, and Prompt 14's Android batch-admission implementation/documentation are uncommitted. No commit or push was made.
- The Phase 0 architecture, [cross-platform API](../architecture/CROSS_PLATFORM_API.md), [platform adapters](../architecture/PLATFORM_ADAPTERS.md), [capability matrix](../architecture/CAPABILITY_MATRIX.md), roadmap, and all prior research reports were reviewed. Those phase reports remain the detailed record of source revisions, licenses, and phase-specific test results.

## Verified phase claims

- **Persisted runtime status and duplicate lookup:** Rust `CoreServices` projects persisted download state and resolves normalized URL matches. Duplicate lookup is advisory; it does not block creation. The Android service already exposed `find_duplicate_download`, but Kotlin had no consumer. This audit wires that method to the Android Add dialog as a confirmation warning. Users can still choose **Add anyway**.
- **Queue lifecycle and ordering:** queue ordering remains priority-first, with moves restricted to queued records of the same priority and stored using the existing `queue_order` data. Queue dispatch, persistence, and interrupted-work recovery remain Rust-owned. Android exposes ordering methods and UI controls; browser/native-host mutation is restricted by design.
- **Android restart policy:** the foreground service returns `START_STICKY` only when the bridge is available, allowing the system to recreate it and reconnect to persisted Rust state. The policy has host-side unit coverage, not a device process-kill/restart test.
- **aria2 remote diagnostics:** the desktop settings probe validates an HTTPS remote endpoint (or IP loopback HTTP), calls `getVersion`, then reads `getGlobalStat`. Statistics failure does not invalidate a successful version probe. Endpoint credentials are transient and are not returned or persisted. The probe is not an Android UniFFI operation and is not exposed as a CLI, FetchX remote, or browser operation.
- **yt-dlp audio:** MP3/M4A selection is typed in the Rust model, persisted through SQLite migration 12, and represented by export version 2 with compatibility for the earlier version-1/schema-11 export shape. Android, desktop, and local CLI consume the feature; remote JSON-RPC and the browser extension/native host do not expose media-specific selection.
- **Batch admission and package grouping:** `CoreServices::enqueue_download_batch` and `enqueue_download_batch_by_source` own URL validation, within-batch duplicate handling, per-input outcomes, filename collision handling, and source grouping. Desktop Tauri uses these operations. Android's bulk Add path now uses the additive `enqueue_download_batch_with_destinations` API through UniFFI; Android keeps its selected package directory in each destination path, and per-source grouping remains desktop-only. Android media options and optional SHA-256 checksums travel with each input. Core writes checksum sidecars before queue admission, so dispatch cannot race ahead of checksum setup.
- **XDM direct MPD behavior:** the shared downloader dispatcher recognizes HTTP(S) `.mpd` paths and routes them to yt-dlp. The queue checks pause support against the effective engine route; for unpausable yt-dlp work it uses cancel-and-requeue behavior where required. Query-string text alone does not trigger MPD routing. This is shared-core behavior, but the prior XDM report's synthetic local-manifest check is not a live site download or a complete media transfer.
- **Motrix-inspired remote engine status:** `engine.runtimeStatus` is behind the authenticated remote dispatch and engine-read permission, is included in protocol discovery, and returns a sanitized status projection. The remote UI refreshes this status on connection and on its periodic refresh. The browser RPC allowlist deliberately excludes the engine-status method.

## Capability-by-target matrix

Legend: **Consumed** means there is a concrete UI/command caller; **exposed** means an adapter or protocol method exists without a consumer; **unsupported by design** means the operation is intentionally restricted; **unverified** means host/device/live-runtime behavior was not demonstrated. Runtime proof is distinguished from compilation and unit tests.

| Capability | Canonical Rust/API | Android | Desktop | CLI | Remote server/UI | Browser/native host |
|---|---|---|---|---|---|---|
| Download runtime status | `CoreServices::download_runtime_statuses` projects `id`, `status`, `progress`, and `updated_at` from each persisted `DownloadItem` | Status/progress are consumed through the existing list/event flow; the separate UniFFI projection has no repository/UI caller | Status/progress/update time are consumed through the existing list/event flow; the separate Tauri/TypeScript wrapper has no UI caller | No separate runtime-status command; standard list/monitor commands consume persisted download state | Status/progress/update time are consumed through `download.list`; the separate RPC/helper is not called, so older servers need no new capability | Read-only RPC method is allowlisted, but no extension caller was found |
| Persisted duplicate lookup | `CoreServices::find_duplicate_download`; normalized persisted-source query | **Consumed** by the Add dialog warning; explicit Add anyway preserves intentional duplicates | Tauri/TypeScript API exposed; no UI caller found | Local and remote duplicate commands available | Authenticated `download.findDuplicate` available; no remote UI caller found | Browser RPC allowlist permits lookup; no extension caller found |
| Queue ordering and dispatch | Rust queue actor, priority-first dispatch, persisted `queue_order` | UniFFI methods and queued-row controls consumed; restart policy is Android-specific | Tauri commands and queue controls consumed | Local and remote queue move commands available | Authenticated operations and remote UI controls consumed | Queue mutations unsupported by the read-only browser method allowlist |
| Android sticky service recovery | Rust state recovery plus Android foreground-service policy | Implemented; host-side policy tests passed; process recreation on a real device is unverified | Not applicable; Rust persisted recovery applies | Not applicable; Rust persisted recovery applies | Not applicable | Not applicable |
| Remote aria2 preflight and global statistics | Shared `Aria2Client` JSON-RPC methods and validated response type | Client source exists, but no UniFFI operation or UI consumer | Settings probe consumes version and global-stat results | No aria2 endpoint probe command | No FetchX remote method or remote UI | Unsupported/not exposed |
| yt-dlp MP3/M4A selection | Typed `YtDlpAudioFormat`, downloader lifecycle, migration 12/export v2 | Browser picker and UniFFI path consume it; no real media/device run | Tauri Add UI consumes it; build/test results are in Seal report | Local `--audio-format` consumes it | No media-specific JSON-RPC operation | No media-specific browser/native-host operation |
| Multi-URL batch admission | `enqueue_download_batch`; additive `enqueue_download_batch_with_destinations` | Bulk Add UI consumes UniFFI; per-item outcomes, selected destination/media options, public-storage staging, and partial success are wired; device/runtime validation unverified | Tauri Add UI consumes the shared batch operation | No batch command | No batch RPC/UI | Popup offers batch selection but submits individual download requests; no shared-core batch call |
| Per-source package grouping | `enqueue_download_batch_by_source` | Not integrated | Desktop batch UI consumes grouping; explicit package override remains supported | No batch adapter | No batch RPC/UI | No shared-core grouping call |
| XDM direct MPD route and effective-engine pause/cancel | `EngineDispatcher` plus queue lifecycle | Shared routing path; depends on Android yt-dlp runtime; no real MPD device transfer | Shared routing path; no site download validation | Shared routing path; no live MPD transfer validation | Shared routing path for accepted download requests; no live remote MPD transfer | Shared core path for submitted URLs; extension build issue remains unrelated |
| Motrix-inspired engine runtime status | `CoreServices::engine_runtime_statuses`, authenticated remote RPC, role authorization and safe projection | No engine-runtime-status operation or UI | Local engine status consumed by settings; remote RPC remains a distinct path | No dedicated engine-runtime-status command found | `engine.runtimeStatus` is advertised and consumed by the remote UI, including periodic refresh | Excluded from the restricted browser RPC allowlist |

This matrix describes code paths and known consumers. A shared Rust API or passing host compilation is not treated as proof of device, desktop-runtime, network, or live-engine execution.

## Selected integration gap and implementation

### Android persisted duplicate warning

**Why it matters:** the shared core could identify an already-persisted URL match, and UniFFI exposed it, but the Android Add flow did not give users that information.

**Acceptance:** before an HTTP(S) URL is submitted (including extracted bulk links), query the shared-core duplicate lookup; on matches, show a warning and require a second explicit **Add anyway** action; on lookup failure, surface a generic retryable message and do not proceed; do not log URLs or alter Rust duplicate policy.

**Implementation:** `DuplicateDownloadRepository` is an optional Android core-side capability; `BridgeDownloadRepository` delegates to `DownloadService.findDuplicateDownload`; `DownloadsViewModel` exposes the repository operation; and `AddDownloadDialog` performs the check and warning. Kotlin owns only interaction; Rust still owns URL normalization and persisted lookup. No Rust API, UniFFI contract, persistence, migration, or dependency changed.

**Validation:** Android ViewModel tests cover successful delegation, a no-match result, and lookup-error propagation. The dialog's confirmation interaction was compiled but not exercised in a Compose UI test. The `:downloads` module unit-test task compiled the UniFFI bridge and passed. Android app Kotlin compilation passed when the environment's failing Python-provisioning task was skipped; this is compilation, not a full app package or runtime test.

### Android shared-core batch admission (Prompt 14)

**Why it matters:** the Android bulk-link path previously made independent per-item calls, so it could not use Rust's batch URL validation, within-batch duplicate handling, collision avoidance, and per-input results without losing Android's explicit destination and storage behavior.

**Contract:** the additive `BatchDownloadDestinationRequest` carries URL, filename, absolute destination, engine, optional yt-dlp format/audio options, and optional SHA-256. UniFFI exposes one request/result DTO per input. The request deliberately has no separate package-name field: Android preserves its selected package by encoding it in each destination path. The original CoreServices batch APIs and desktop callers are unchanged.

**Ownership and behavior:** Kotlin retains Android destination selection and public-storage staging/finalization, converts engine/media options, and maps per-input outcomes. Rust validates the URL, engine, media options, checksum and destination; resolves collisions; admits valid items independently; and returns errors without rejecting unrelated valid inputs. For checksummed batch items, Rust creates the established `.fetchx.sha256` sidecar before enqueueing, preventing a fast queue worker from starting before checksum setup. The previous post-enqueue Android bridge write was removed. Cancellation does not roll back items already accepted by the queue.

**Validation:** `cargo test -p fetchx-core-services --lib` passed (31 tests), including explicit destinations, partial success, duplicate URLs, collision resolution, and checksum sidecar creation. `cargo test -p fetchx-android-bridge --lib` passed (25 tests), including UniFFI service result/destination/checksum coverage. `Set-Location apps\android; .\gradlew.bat :bridge:testDebugUnitTest` passed, and `Set-Location apps\android; .\gradlew.bat :downloads:testDebugUnitTest` passed. These are host unit tests and Kotlin compilation, not a device or live-download test. Public-storage publication remains covered by the existing Android storage tests, not by a real-device batch run.

**Remaining boundary:** bulk admission is consumed by Android UI and desktop UI only. CLI, remote JSON-RPC/UI, and browser/native-host still lack batch operations; browser bulk selection continues to submit individual requests. Android media engine availability and public-storage publication for an admitted batch remain dependent on platform runtime and device permissions.

### Persisted download runtime-status audit (Prompt 15)

**Verified contract:** `DownloadRuntimeStatus` contains only `id`, `status`, `progress`, and `updated_at`, and `From<&DownloadItem>` copies those fields without additional derivation. `CoreServices::download_runtime_statuses()` obtains the same persisted records as the ordinary download-list APIs and projects that subset; it does not report independent engine-observed or fresher state.

**Existing consumers:** Android's `listDownloads()` and Rust event snapshots map the ordinary `DownloadInfoDto` status/progress into `DownloadItem`, and `DownloadItemRow` renders lifecycle/progress details. Android's DTO also carries `updated_at`, but the Kotlin UI model currently omits that timestamp; if the UI needs it later, it can be mapped from the existing list response rather than fetched separately. Desktop's `listDownloads()` plus download-change events populate `DownloadRow`, which renders status, progress, speed/ETA, and `updated_at`. The remote UI refreshes `download.list` on connection, events, and its existing periodic refresh; the table renders status and progress, while the dashboard sorts recent downloads by `updated_at`. CLI list/monitor commands also read persisted `DownloadItem` status. The runtime-specific UniFFI/Tauri/RPC methods and client wrappers remain exposed but have no direct UI callers.

**Decision and compatibility:** no additional UI integration or polling was added. The regular download-list record already contains every runtime-status field; fetching and merging the projection would duplicate state, add a second read path, and risk mismatches without supplying missing status information. Existing list/event refresh and error behavior remains unchanged; no separate missing/stale runtime-status state is introduced. In particular, remote UI does not require `download.runtimeStatus`, preserving compatibility with older servers. The additive runtime-status APIs remain available to downstream clients that need the compact projection. No public contract, migration, or dependency changed.

**Validation:** `cargo test -p fetchx-models projection_preserves_every_lifecycle_status` passed (1 test); `cargo test -p fetchx-remote-server runtime_status_rpc_shape_includes_shared_progress_fields` passed (1 test); `cargo test -p fetchx-desktop` passed (60 tests); `Set-Location apps\android; .\gradlew.bat :downloads:testDebugUnitTest` passed; `Set-Location apps\desktop\ui; npm run build` passed; `Set-Location apps\remote-ui; npm test` passed (5 tests); and `Set-Location apps\remote-ui; npm run build` passed. No UI/adapter code changed, so no UI-specific test was added; no device/runtime behavior is claimed.

## Unresolved limitations and phase claims

- **Ridm:** the supplied GitLab project still returns inaccessible/not-found results, its API returned 404, and the page was blocked by sign-in/access challenge. No revision, license, source, tests, or documentation could be verified. See [Ridm research](RIDM_FEATURE_ANALYSIS.md).
- **JDownloader 2:** the official SVN trunk HTTPS endpoint returned HTTP 401 and the SVN client was unavailable. The reviewed `mirror/jdownloader` snapshot (`f274b29897aad2e0ff0d4fd148e42aadf8b622fa`, SVN r49881 dated 2024-09-27) is non-authoritative and has no root license file; the current authoritative revision and exact license remain unknown. See [JDownloader research](JDOWNLOADER2_FEATURE_ANALYSIS.md).
- **Android tooling:** the earlier Seal validation attempt crashed in the Gradle wrapper JVM while `net.dll` enumerated network interfaces (`NetworkInterface.getAll`); that historical failure remains accurately documented. During this audit, the focused `:downloads:testDebugUnitTest` task passed. A normal `:app:compileDebugKotlin` attempt failed earlier at Chaquopy's configured `yt-dlp==2026.8.19` installation with pip `OSError: [Errno 22] Invalid argument`; rerunning with `-x :app:installDebugPythonRequirements` compiled app Kotlin successfully. No device/emulator, force-stop, process-restart, reboot, or live-download validation was performed.
- **Browser extension:** the XDM report's build failure remains `src/background/media-interception.ts(103,10): TS6133: 'classify' is declared but its value is never read`. The extension was not changed or rebuilt in this audit. Its tests passed in the prior Motrix validation, but that does not make the reported build pass.
- **Desktop Clippy:** the two prior warnings remain in untouched desktop files: `apps/desktop/src-tauri/src/app.rs:297` (`collapsible_if`) and `apps/desktop/src-tauri/src/commands.rs:1032` (`too_many_arguments`). The prior phases recorded them; this audit's Kotlin change does not address them.
- **Runtime limits:** no live aria2 daemon, external endpoint, authenticated media site, remote multi-device session, physical Android device, or Windows/Linux/macOS desktop runtime was used. Reports' mocked RPC, synthetic MPD, Rust tests, and host builds are not represented as those runtime validations.

## Validation performed for this audit

| Command | Result |
|---|---|
| `cargo test -p fetchx-core-services --lib` | Passed: 31 unit tests, including explicit-destination batch cases |
| `cargo test -p fetchx-android-bridge --lib` | Passed: 25 unit tests, including batch UniFFI mapping and sidecar behavior |
| `cargo test -p fetchx-core-services --test duplicate_download` | Passed: 1 persisted normalized-URL lookup test |
| `Set-Location apps\android; .\gradlew.bat :downloads:testDebugUnitTest --tests 'dev.fetchx.downloads.ui.DownloadsViewModelTest'` | Passed: 22 ViewModel tests; UniFFI generation and bridge/download module Kotlin compilation succeeded |
| `Set-Location apps\android; .\gradlew.bat :bridge:testDebugUnitTest` | Passed: Android bridge host unit tests and Kotlin compilation |
| `Set-Location apps\android; .\gradlew.bat :downloads:testDebugUnitTest` | Passed: Android downloads host unit tests and Kotlin compilation |
| `Set-Location apps\android; .\gradlew.bat :app:compileDebugKotlin -x :app:installDebugPythonRequirements` | Passed Kotlin compilation; Python provisioning was explicitly skipped |
| `Set-Location apps\android; .\gradlew.bat :app:compileDebugKotlin` | Failed before Kotlin compilation: Chaquopy pip installation of `yt-dlp==2026.8.19` returned `OSError: [Errno 22] Invalid argument` |
| `cargo test -p fetchx-remote-server --lib runtime_status -j 1` | Passed: 1 sanitized runtime-status response-shape test |
| `cargo test -p fetchx-remote-server --lib authorization -j 1` | Passed: 4 authorization and role-permission tests |
| `cargo test -p fetchx-queue --lib -j 1` | Passed: 42 queue ordering, dispatch, recovery, and lifecycle tests |
| `cargo test -p fetchx-downloader is_dash_url_matches_manifest_paths_without_matching_query_values -j 1` | Passed: direct DASH path routing test |
| `cargo clippy -p fetchx-downloader -p fetchx-desktop --all-targets -j 1 -- -D warnings` | Failed only on the two known desktop warnings: `app.rs:297` `collapsible_if` and `commands.rs:1032` `too_many_arguments`; no Rust files were changed in this audit |
| `cargo clippy -p fetchx-core-services -p fetchx-android-bridge --all-targets -j 1 -- -D warnings` | Passed after collapsing the new destination/audio-extension conditional; no warnings in the changed crates |
| `cargo fmt --all` | Passed |
| `cargo fmt --all -- --check` | Passed after Prompt 14 changes |
| `git diff --check` | Passed after Prompt 14 documentation updates |

Earlier cross-phase test/build evidence remains in the linked research reports, including queue/recovery, remote authorization/status, aria2 statistics, storage migration/export compatibility, and desktop/remote UI checks. This audit did not rerun every unrelated suite.

## Remaining work, in priority order

1. Validate Android batch media routing, checksum verification, public-storage staging/publication, and process recovery on a supported device/emulator.
2. Keep the compact persisted download-runtime-status API available to downstream clients; existing app surfaces already consume the same status through their ordinary list/event flows.
3. Add batch operations to CLI/remote/browser only if product requirements justify those transport surfaces; browser bulk selection still submits individual calls.
4. Resolve the pre-existing browser `TS6133` issue separately and keep the two desktop Clippy warnings tracked.
5. Continue the documented Gopeed `resolve_task` investigation; do not infer missing JDownloader or Ridm behavior from inaccessible sources.

## Files changed in Prompt 13

- `apps/android/core/src/main/kotlin/dev/mydownloader/core/download/DownloadRepository.kt`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/BridgeDownloadRepository.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/DownloadsViewModel.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/DownloadsScreen.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/AddDownloadDialog.kt`
- `apps/android/downloads/src/test/kotlin/dev/mydownloader/downloads/ui/DownloadsViewModelTest.kt`
- `docs/research/CROSS_PHASE_INTEGRATION_AUDIT.md`
- `docs/architecture/CAPABILITY_MATRIX.md`
- `docs/roadmap/SHARED_CORE_ROADMAP.md`

## Files changed in Prompt 14

- `crates/core-services/src/lib.rs`
- `crates/android-bridge/src/dto.rs`
- `crates/android-bridge/src/lib.rs`
- `crates/android-bridge/src/service.rs`
- `crates/android-bridge/uniffi/android_bridge.udl`
- `apps/android/core/src/main/kotlin/dev/mydownloader/core/download/DownloadRepository.kt`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/BridgeDownloadRepository.kt`
- `apps/android/bridge/src/test/kotlin/dev/mydownloader/bridge/MappingTest.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/AddDownloadDialog.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/DownloadsViewModel.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/DownloadsScreen.kt`
- `apps/android/downloads/src/test/kotlin/dev/mydownloader/downloads/ui/DownloadsViewModelTest.kt`
- `docs/architecture/CROSS_PLATFORM_API.md`
- `docs/architecture/CAPABILITY_MATRIX.md`
- `docs/roadmap/SHARED_CORE_ROADMAP.md`

## Files changed in Prompt 15

- `docs/research/CROSS_PHASE_INTEGRATION_AUDIT.md`
- `docs/architecture/CAPABILITY_MATRIX.md`
- `docs/roadmap/SHARED_CORE_ROADMAP.md`

## Prompt 17 — staged aria2/native torrent migration

- The pinned aria2 review and feature-parity table are in [`ARIA2_NATIVE_RUST_MIGRATION.md`](ARIA2_NATIVE_RUST_MIGRATION.md). aria2 remains the compatibility backend; this work does not claim a full replacement.
- Shared queue, priority, storage, and protocol implementations remain Rust-owned. Explicit backend-selection APIs now prevent a requested native torrent engine from being silently replaced by aria2. Legacy callers preserve their previous aria2-first fallback.
- Confirmed local `.torrent` selections are passed to librqbit. Magnet selection parity remains outstanding. The metadata repository lookup now filters by download ID, with a multi-row regression test.
- Desktop, CLI, and remote explicit selection are integrated; Android's existing native fallback is unchanged. No UniFFI or transport DTO change, schema migration, new dependency, aria2 setting removal, or existing-download rewrite was made.
- `cargo fmt --all`; shared-core, storage, CLI, remote-server, Android bridge, and desktop Tauri tests passed. Shared-core/adapter Clippy passed; desktop Clippy passed with two existing warnings in `app.rs` and the batch command in `commands.rs`. These are host-side tests; no live swarm, aria2 daemon, Android device/emulator, packaged desktop application, or live transfer was tested.
- The next bounded step is native magnet metadata and file selection, with restart/recovery acceptance tests before changing defaults or removing aria2.

See the migration report for the aria2 revision/license, reviewed source files, exact per-capability status, adapter results, and remaining phases: [`ARIA2_NATIVE_RUST_MIGRATION.md`](ARIA2_NATIVE_RUST_MIGRATION.md).

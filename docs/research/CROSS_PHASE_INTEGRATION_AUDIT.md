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
- **aria2 remote diagnostics:** the desktop settings probe validates an HTTPS remote endpoint (or IP loopback HTTP), calls `getVersion`, then reads `getGlobalStat`. Statistics failure does not invalidate a successful version probe. Its endpoint credentials are transient and are not returned or persisted. Android has a separate Kotlin settings screen that probes a separately configured endpoint and stores that profile in app preferences; it is not an Android UniFFI operation or a FetchX download route. Neither probe is exposed as CLI, FetchX remote, or browser RPC.
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

## Prompt 18 — native torrent validation

- Added a controlled local-peer test using synthetic multi-file metainfo. It verifies selected-file-only payload, completion progress, and transition to seeding without trackers, DHT, local discovery, or a public swarm.
- CoreServices local inspection returns metadata without queue admission; duplicate admissions of the same torrent receive distinct generated default destinations.
- Native execution reparses local `.torrent` bytes and rejects stale info hashes, unconfirmed/empty selections, and indexes outside the current file list.
- Existing lifecycle tests cover pause then cancellation against a non-seeding fixture and stopping a completed seed without deleting its payload. The loopback transfer does not test pause/resume, retry, or cancellation while a peer transfer is active; torrent-specific restart recovery and magnet metadata selection remain open.
- No global default change was made. Desktop local torrent admission explicitly selects native, while CLI and legacy CoreServices/remote routes retain aria2-first behavior. aria2 functionality, settings, and existing records remain intact.

See [`ARIA2_NATIVE_RUST_MIGRATION.md`](ARIA2_NATIVE_RUST_MIGRATION.md) for exact commands, runtime scope, and remaining acceptance criteria.

## Prompt 19 — native magnet metadata and lifecycle

- Shared Rust now acquires magnet metadata with librqbit, parses its validated info bytes, persists the file list, and waits for an explicit non-empty confirmed selection before selected-file transfer. It validates the saved info hash/indexes at execution and removes the metadata-only torrent before selection confirmation. Metadata acquisition observes pause/cancel and reports cleanup errors.
- Android can inspect and confirm a selection when existing routing chooses native torrent (native fallback; no explicit engine chooser). Desktop exposes Tauri metadata and selection calls and recognizes `native_torrent` jobs in the detail selection flow. CLI supports metadata inspection and selection after explicit `--engine native-torrent` admission. Authenticated remote metadata and selection operations are documented and validated separately in Prompt 20 below.
- The controlled fixture is synthetic, non-private, trackerless, DHT-disabled, discovery-disabled, and loopback-only. It verifies magnet metadata/info hash, explicit selection and selected-only transfer, partial progress, pause, new engine/session continuation, a second active transfer's seeder interruption, cancellation and managed-handle cleanup, peer restoration, and retry to completion. The test does not prove exact piece reuse before resumed peer data or process/device restart recovery. It also does not exercise a real adapter runtime, destination collision, or a retry policy for arbitrary network errors. Private-torrent `ut_metadata` exchange is unsupported in this test setup.
- No Android device/emulator, packaged desktop, public swarm, or aria2 daemon was tested. Global defaults, aria2 settings, packaging, RPC, and persisted records remain unchanged. The native migration remains partial.

Exact test/build results and limitations are tracked in [`ARIA2_NATIVE_RUST_MIGRATION.md`](ARIA2_NATIVE_RUST_MIGRATION.md).

## Prompt 20 — remote native magnet file selection

- Added `download.torrentMetadata` and `download.selectTorrentFiles` to remote JSON-RPC parsing, discovery, dispatch, and authorization. Metadata reads require `DownloadsRead`; confirming selection requires `DownloadsControl`. Dispatch delegates validation/persistence to CoreServices, including pending-state, non-empty/unique/in-range indexes, and expected info-hash checks.
- Metadata response is a sanitized projection of native magnet state and indexed file paths/sizes; it does not expose magnet URI, trackers, or destination path. Missing downloads, invalid parameters, and stale/premature selection map to typed sanitized errors.
- The calls are intentionally excluded from the restricted browser/native-host method allowlist. The remote UI only calls operations advertised by discovery, polls for metadata readiness, displays files and acquisition/confirmation/stale/failure/unavailable states, and allows explicit confirmation. The add form's optional engine choice defaults to automatic/server default, preserving prior routing; older servers display unavailable selection rather than receiving unknown calls.
- Test coverage added for protocol method/request contracts, discovery and role authorization, dispatch response/error/persistence behavior, browser-boundary exclusion, UI state transitions, and capability fallback. `cargo test -j 1 -p fetchx-remote-protocol -p fetchx-core-services -p fetchx-remote-server` passed (CoreServices 35 unit + 2 integration, protocol 11, server 21 unit + 3 integration, plus CoreServices doctest); `cargo test -j 1 -p fetchx-downloader native_magnet -- --nocapture` passed 3 tests. Remote UI `npm test` passed 9 tests and `npm run build` passed TypeScript/Vite. `cargo fmt --all -- --check`, targeted Clippy, and `git diff --check` passed. No live remote network/device run was performed.
- Global defaults, aria2 RPC/settings/packaging, and persisted schemas/records remain unchanged. The native migration remains partial.

## Prompt 21 — native torrent default

The target-specific decision is recorded in [`ARIA2_NATIVE_RUST_MIGRATION.md`](ARIA2_NATIVE_RUST_MIGRATION.md). New CoreServices magnet and local `.torrent` admissions with no engine now route to native Rust; they fail with native-engine-unavailable rather than silently falling back to aria2. Explicit Aria2 remains supported through the explicit CoreServices API.

- Android uses native torrent by default and always configures the librqbit session directory in app-private storage. There is no Android aria2 runtime or engine selector.
- Desktop's default Add choice is native via the shared default path; users can explicitly choose aria2 when its runtime is available. The runtime status DTO now includes native torrent availability.
- CLI defaults to native and preserves `--engine aria2`. Non-torrent engine auto-detection/defaults are unchanged.
- The current remote UI selects native when discovery advertises both magnet metadata and file-selection RPCs, and aria2 remains an explicit option. On older servers it leaves engine unspecified; older clients that omit engine retain the server's aria2-first compatibility route. Browser/native-host remains HTTP(S)-only and cannot submit torrents.
- Existing `DownloadItem.source.engine` values drive retry/recovery as before; no persisted download, aria2 GID, aria2 configuration, helper, RPC, or package was rewritten or removed. New/missing engine config defaults enable native; existing explicit disabled settings remain unchanged.
- Regression coverage includes default native magnet/file admission, magnet pending/unconfirmed state, explicit aria2 non-substitution when unavailable, CLI engine resolution, Android bridge default, old remote capability fallback, and unchanged HTTP defaults. Prompt 19 controlled-loopback evidence still covers selected-only transfer, pause/recovery/interruption/retry/cancellation. Exact Prompt 21 commands/results are recorded in the migration report.
- Validation passed: the serial Rust package test command covering models, downloader, storage, queue, CoreServices, CLI, remote protocol/server, Android bridge, and desktop Tauri; remote UI tests (10) and production build; desktop UI production build; Android `:downloads:test`; formatting; and `git diff --check`. Targeted Clippy passed with two existing desktop warnings (`collapsible_if` in `app.rs` and `too_many_arguments` in `commands.rs`). The test with dispatch paused asserts the magnet remains `Queued` with pending metadata and no confirmed selection; this is the expected pre-dispatch state.
- Validation is host-side only; no Android device, packaged desktop, real aria2 daemon, public swarm, or live old/new remote-server interoperability run was performed.

## Prompt 22 — remaining aria2 dependency audit

The worktree was clean at Prompt 21's commit `4a08ab9` before this documentation-only update. The Rust workspace, Android/desktop/CLI/remote/browser sources, build/package configuration, README, and migration architecture records were searched and traced. There is no new non-torrent path that defaults to aria2, so no download behavior was changed.

| Dependency class | Current evidence and status |
|---|---|
| Active execution | `EngineDispatcher` routes persisted `EngineKind::Aria2` records through `Aria2Engine`/`Aria2Client`. This is necessary for explicit aria2 jobs and compatible persisted records; it is not the regular new-download default. |
| Existing task recovery | Download `engine_kind` remains persisted, while aria2 torrent tasks retain `aria2_gid` in torrent metadata/export. The aria2 engine reconciles GIDs/status through RPC. No automatic migration to another engine occurs. Android currently passes no local aria2 executable; any legacy Android row persisted as aria2 remains stored but cannot run locally unless the runtime is supplied. |
| Explicit fallback | CLI accepts `--engine aria2`; desktop and current remote UI offer aria2 for torrent admissions; CoreServices exposes explicit engine selection. Current remote UI chooses native when both selection RPCs are advertised. Older remote clients that omit engine retain the aria2-first magnet route when aria2 is available. Remote HTTP creation only supports `direct_http`. |
| Process/config/package | Optional local daemon startup and control use `Aria2Supervisor`, persisted `EngineConfig.aria2`, and `Aria2Client`. Desktop offers a Windows-only per-user helper installation (not bundled into app resources); Android disables aria2 extraction and uses native torrent. Settings and helpers remain required for the fallback. |
| Diagnostics only | CLI engine status, desktop local/remote probe, Android remote endpoint probe, and remote `engine.runtimeStatus` report reachability/availability or global counts. The remote FetchX server is not a general aria2 RPC proxy; browser/native-host has no aria2 RPC route. |
| Shared controls/options | FetchX QueueManager owns admission, priority and shared resource/concurrency policy. ResourcePolicy retains an aria2 bandwidth setting. Per-engine transfer and restart semantics remain distinct; full parity for aria2 upload/global options and native torrent crash recovery is not established. |
| Storage and secrets | Stable `aria2` engine serialization and torrent GID fields preserve old records. Desktop configuration DTOs and CLI output expose only whether a secret is configured; download exports contain task/torrent metadata (including GID where applicable), not engine configuration or RPC secrets. Android's separate remote aria2 profile is preserved in app preferences and is not part of download export. |
| Dead/duplicate code | No execution, recovery, settings, helper, or diagnostic aria2 implementation was proven dead. Android and desktop endpoint probes are separate host settings features, not unused duplicate download engines. |

New-download defaults are Rust-native by protocol: HTTP/HTTPS uses direct HTTP (with existing HLS/DASH/yt-dlp resolution), FTP/FTPS and SFTP use their Rust engines, Metalink uses the shared Rust orchestrator, and new torrent/magnet admissions use native Rust. CLI may explicitly select aria2; remote HTTP rejects it; current desktop/remote torrent fallbacks remain explicit. Browser capture is HTTP(S)-only. Platform-specific protocol availability remains limited to each adapter's exposed inputs.

No behavior migration is justified in Prompt 22. Redirecting explicit jobs or persisted aria2 rows would remove a supported fallback; replacing persisted torrent recovery would change engine identity without proven equivalent GID reconciliation or native process-restart recovery. Exact dependency and protocol tables, plus the next deprecation decision, are in [`ARIA2_NATIVE_RUST_MIGRATION.md`](ARIA2_NATIVE_RUST_MIGRATION.md). Source inspection and `git diff --check` are the validation boundary; no tests/builds were run for this documentation-only update.

Before any aria2 removal, require verified native process/device recovery and partial-piece reuse, live persisted-aria2 reconciliation on supported targets, live current/older remote compatibility, a migration/rollback policy for explicit users, and an Android policy for legacy aria2 rows. Preserve RPC credentials/settings and helper availability until those gates pass.

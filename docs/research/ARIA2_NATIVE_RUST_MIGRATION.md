# aria2 to Native Rust Migration

## Prompt 34 — external aria2 runtime removed

This is the current end state; earlier prompt sections below are historical phase snapshots and do not describe the current runtime.

- New downloads use FetchX's shared Rust engines. `aria2` is not a selectable runtime, fallback, subprocess, RPC client, supervisor, packaged helper, or settings UI. Explicit legacy engine requests are rejected; torrent requests do not fall back to another engine.
- Startup and queue recovery leave persisted aria2 jobs untouched and do not dispatch them. Unfinished legacy rows are displayed as unavailable because the external engine was removed. Resume, retry, pause, cancel, queue/scheduling changes are not offered for those rows; users may still inspect or independently remove them.
- Desktop and Android retain the user-confirmed HTTP(S) and torrent restart flows. These create separate native jobs through CoreServices, with fresh source/hash confirmation and collision-safe destinations. Old rows, GIDs, metadata, destinations, output, partial bytes, and `.aria2` sidecars are not reused or rewritten.
- The legacy `EngineKind::Aria2`, persisted config fields, GID/metadata columns, and parsers remain only to deserialize and display historical data and to support the explicit restart APIs. Compatibility code does not contact or launch aria2. No schema migration was needed.
- Remote UI only sends magnet creation to servers advertising the native metadata/selection capabilities; older servers are shown an unavailable message rather than receiving an engine-omitted request that could route through a legacy fallback.
- Native coverage is not claimed to be full aria2 parity: HTTP(S), native BitTorrent, and Metalink have tested subsets; local FTPS explicit-TLS behavior and SFTP wire behavior have bounded hermetic evidence, but broader server interoperability and aria2 parity remain unverified; DASH uses yt-dlp rather than a native DASH parser. No native rejection triggers aria2 fallback.
- Windows validation: the full `cargo test --workspace` suite passed, including CoreServices (43), downloader (241 passed, 3 ignored), queue (42), remote-server (22 including the no-fallback regression test), CLI (26), and Android bridge (25). Android bridge/downloads unit tests and app Kotlin compilation passed; desktop and remote UI builds/tests passed; desktop Tauri `cargo check`, Rust formatting, and `git diff --check` passed. The first workspace run caught a stale remote integration expectation of nine engine-status entries; removing aria2 made the correct count eight, so the assertion now also verifies aria2 is absent. Unmodified strict Clippy identified two unrelated existing desktop warnings (`collapsible_if` in `app.rs` and `too_many_arguments` in `commands.rs`); the rerun passed with only those two lint classes allowed and all other warnings denied. No Android device, live aria2 daemon, public tracker/seeder, or live external protocol server was tested.
- The downloaded ignored Android `aria2c-arm64-v8a` asset was removed. No aria2 source or binary is included in release packaging.

## Historical scope and evidence through Prompt 33

This section records the migration history through Prompt 33; the current end state is documented above.

- aria2 source was pinned to [`9e7273583f83e881e3ec067b523ba88724088d2f`](https://github.com/aria2/aria2/commit/9e7273583f83e881e3ec067b523ba88724088d2f) (2026-06-25).
- The repository's `COPYING` contains GPL version 2 text; reviewed source headers state GPL-2.0-or-later and include an OpenSSL linking exception. No aria2 source was copied.
- Reviewed the [aria2 manual](https://aria2.github.io/manual/en/html/), including RPC behavior, and source paths `doc/manual-src/en/aria2c.rst`, `src/DownloadEngine.cc`, `src/RequestGroup.cc`, `src/RequestGroupMan.cc`, and `src/RpcMethodImpl.cc`.
- Reviewed focused upstream tests: `test/SegmentManTest.cc`, `test/RequestGroupTest.cc`, `test/RequestGroupManTest.cc`, `test/HttpResponseTest.cc`, `test/FtpConnectionTest.cc`, `test/Metalink2RequestGroupTest.cc`, and `test/BtPostDownloadHandlerTest.cc`.
- Inspected FetchX's aria2 engine/supervisor, engine dispatcher, native torrent engine, storage and queue integration, configuration, desktop helper UI, CLI, remote creation path, Android fallback, and torrent metadata persistence.

The upstream code was used only as a behavioral reference. No aria2 implementation code was copied.

## Feature parity for behavior FetchX relies on

“Covered” means an existing FetchX implementation owns the behavior. “Partial” means some behavior exists but does not yet satisfy all aria2-backed workflows. “Missing” means the native path does not currently provide it. “Not used” means FetchX does not rely on that aria2 capability for its current workflows.

| aria2 behavior or operation | FetchX Rust status | Evidence and limits |
|---|---|---|
| HTTP/HTTPS transfer and segmented ranges | Covered | `DirectHttpEngine` owns range negotiation, workers, retries, and validator-safe segment reuse. |
| FTP/FTPS and SFTP | Covered | Shared Rust `FtpEngine` and `SftpEngine`; adapter availability remains configuration/platform dependent. |
| HLS and DASH/media extraction | Covered | Rust HLS engine and dispatcher; DASH is routed to the configured yt-dlp runtime, not a native DASH parser. |
| Metalink | Covered | Shared `MetalinkEngine` resolves mirrors/checksums using FetchX models and storage. |
| Magnet and local `.torrent` transfers | Partial | `librqbit` is integrated into the shared dispatcher. Local `.torrent` selection and native magnet metadata acquisition, persisted explicit selection, and selected-file transfer are implemented. The controlled lifecycle is tested, but full platform/device parity and crash recovery are not established. |
| Local and magnet file selection | Covered for the migrated paths | Persisted confirmed indexes are revalidated against current metainfo (info hash and file bounds) before being supplied to librqbit `only_files`; unconfirmed, empty, stale, and out-of-range selections are rejected. Magnet metadata-only sessions are removed after acquisition, before selection is awaited and persisted. |
| HTTP resume, retries, and remote-change safety | Covered for direct HTTP | FetchX reuses its existing validator-safe Rust implementation. Do not infer identical restart/resume semantics for every protocol. |
| Torrent resume/recovery and seeding | Partial | Existing FetchX persisted state and native lifecycle are retained; librqbit session persistence is disabled and aria2-specific recovery behavior is not replaced wholesale. |
| Shared priority, queue ordering, admission limits | Covered | The Rust queue/scheduler remains authoritative; no competing engine queue was introduced. |
| Per-task/global rate and connection controls | Partial | Native torrents receive per-download rate and peer limits; full aria2 option parity, including upload/global controls, is not established. |
| Pause, resume, cancellation, and shutdown | Partial | Native lifecycle controls and session cleanup are covered by existing tests. Full cross-restart compatibility for aria2-managed torrents remains outstanding. |
| Progress, speed, ETA, and lifecycle reporting | Partial | Native progress is mapped through FetchX state; all aria2 diagnostic fields and reporting cadence are not claimed equivalent. |
| aria2 JSON-RPC task/status/control operations | Not replaced | FetchX still consumes its local aria2 RPC engine for existing aria2 tasks and compatibility workflows. |
| aria2 version/global-stat diagnostics and process supervision | Not replaced | Supervisor, helper installation, RPC telemetry, and settings remain in place. |
| aria2-specific configuration and saved task records | Not replaced | Existing persisted settings and downloads remain compatible; no destructive migration or default change was made. |
| Metalink/checksum options beyond current FetchX APIs, arbitrary file allocation policy, and unrelated aria2 RPC methods | Not used as a native migration target | No current FetchX workflow requiring exact parity for these aria2-specific options was established in this phase. |

## FetchX implementation in this phase

This is a bounded Phase B slice rather than a backend replacement:

- Added explicit CoreServices APIs for magnet and local torrent admission. They validate that the selected engine is aria2 or native torrent and that it is available. Existing admission methods remain compatibility wrappers with their prior aria2-first behavior.
- Routed explicit backend selection through desktop add/retry, CLI `--engine native-torrent`, and remote magnet creation. CLI's default remains aria2; remote requests without an engine retain aria2-first selection and native fallback.
- Connected the native torrent engine to the existing persisted torrent metadata repository. For confirmed local `.torrent` selection, FetchX converts the stored zero-based indexes and supplies them as librqbit `only_files`.
- Rejects an empty confirmed local selection: librqbit 9.0.1's `src/session.rs` passes an empty set to the selected-piece filter in `src/torrent_state/initializing.rs`, which selects no files and leaves a task with no payload to complete.
- Re-parses local `.torrent` bytes at execution time and checks the current info hash and file-index bounds before applying a saved selection. This prevents a replaced torrent file from reusing a stale selection against different metadata.
- Fixed `get_torrent_metadata(id)` to filter by `download_id`; the prior unfiltered query could return another download's selection. Added a multi-row regression test.
- Fixed Windows path classification so verbatim drive paths such as `\\?\C:\...` are not mistaken for UNC paths; actual UNC paths remain rejected.
- Kept aria2 enabled as the compatibility engine. No executable, RPC client, supervisor, settings, packaging, database fields, or existing downloads were removed or rewritten.

The implementation reuses `crates/downloader` (`DirectHttpEngine`, `Aria2Engine`, and `NativeTorrentEngine`), `crates/core-services`, `crates/queue`, `crates/storage`, `crates/models`, and the existing `EngineDispatcher`. It does not add another queue, dependency, or schema migration.

## Adapter and compatibility status

| Surface | Status |
|---|---|
| Android | Native torrent is the default and is enabled with an app-private librqbit session directory. Magnet selection and local `.torrent` selection are supported. aria2 is not packaged and there is no Android engine chooser, so explicit aria2 is not a supported Android fallback. No device run was performed. |
| Desktop (Windows/macOS/Linux) | Native torrent is the CoreServices default and is exposed by Tauri/UI for magnet metadata selection and local `.torrent` selection. The add UI offers aria2 as an explicit fallback when its runtime is available. |
| CLI | Native torrent is the default for magnet and local `.torrent` additions; `--engine aria2` remains an explicit fallback. Torrent inspection/file-selection commands remain available. |
| Remote server/UI | The current remote UI selects native torrent by default only when discovery advertises both metadata and selection RPCs; aria2 remains selectable. On older servers it leaves the engine unspecified and preserves legacy server routing. Older clients that omit `engine` retain the server's aria2-first compatibility route. |
| Browser/native host | Torrent admission is not supported: browser capture accepts HTTP(S) only and local browser RPC validation rejects magnets. HTTP behavior is unchanged; torrent default is not applicable. |
| Existing data/settings | Persisted downloads retain their stored `EngineKind` and are dispatched through that engine; no task rewrite or aria2 GID migration occurs. aria2 configuration, helper packaging, and RPC remain. New/missing engine configurations enable native torrent by default; an existing explicit `native_torrent.enabled = false` is preserved. |

Prompt 21 changes the default only for new torrent admissions. CoreServices' no-engine magnet and local `.torrent` APIs now choose native Rust and return `UnavailableEngine(NativeTorrent)` rather than silently switching to aria2 when native is disabled/unavailable. Explicit `*_with_engine(..., Aria2, ...)` calls remain authoritative. HTTP, FTP/FTPS, SFTP, HLS/DASH, yt-dlp, Metalink, and other routing rules are unchanged.

### Prompt 21 target compatibility decision

| Target | Previous default for magnet / `.torrent` | Current default | Native selection available | Explicit aria2 | Existing records/settings |
|---|---|---|---|---|---|
| CoreServices | aria2 first, native fallback | Native for new admissions without an engine | Magnet waits for metadata and an explicit valid selection; local `.torrent` indexes are persisted and revalidated | Yes, explicit engine API | Each task's persisted engine is unchanged; settings are not rewritten |
| Android | Native only when aria2 was absent; local torrent path used native | Native | Yes, Android bridge/UI exposes magnet and local file selection | No product-level chooser; aria2 is not bundled because Android execution is restricted | Persisted engine and user data unchanged; Android uses private session storage |
| Desktop | Native was hardcoded for torrent creation | Native default via CoreServices; user can explicitly choose aria2 | Yes, Tauri and DetailPanel | Yes, add-form fallback when available | Stored engine choices/retry paths remain unchanged; aria2 settings/helper manager retained |
| CLI | aria2 | Native | Yes, torrent inspection/selection | Yes, `--engine aria2` | Stored downloads unchanged |
| Remote UI/server | Automatic server routing (aria2 first, native fallback) | Current UI selects native when both selection RPC capabilities are advertised; older servers keep legacy routing | Yes, capability-gated metadata and confirmation | Yes, UI selection | Older clients omitting `engine` retain the legacy server route; stored records/settings unchanged |
| Browser/native host | Not applicable; HTTP(S)-only | Not applicable | No torrent intake by design | Not applicable | HTTP browser capture unchanged |

An existing persisted `native_torrent.enabled = false` remains disabled. In that case the shared default does not silently select aria2: admission fails clearly unless the caller explicitly selects an available aria2 runtime. Aria2 defaults, credentials, helpers, and settings are not modified.

## Prompt 18 — native torrent runtime evidence

The deterministic fixture is generated in the test from two synthetic 16 KiB files and a private multi-file `.torrent`. A local librqbit session verifies and seeds those files. The downloader connects to the seeder's loopback peer address through a private test seam; DHT, trackers, and local service discovery are disabled. No public swarm, external content, or aria2 daemon is involved.

End-to-end assertions confirm that local torrent metainfo can be inspected without queue admission, a confirmed index is persisted and applied, only the selected 16 KiB file is written, downloaded-byte progress reaches the selected file size, and the download transitions to `Seeding`. A CoreServices regression check enqueues the same local torrent twice and verifies that the generated default destinations differ.

The validation exposed stale-selection risk when `.torrent` bytes change after selection. The engine now reparses the current file and rejects a different info hash, empty or unconfirmed selections, and indexes outside the current file list. Focused tests cover these rejections and the controlled-peer transfer.

The acceptance scope is still incomplete: the peer transfer does not exercise pause/resume mid-transfer, retry, cancellation during a live transfer, or app shutdown/restart recovery. Existing lifecycle tests cover pause then cancellation against an unavailable tracker and stopping a completed seed without deleting its payload; CoreServices separately tests generic persisted-queue recovery. Magnet metadata acquisition and magnet file selection are not exercised by this fixture. Explicit user-supplied destination collisions are not covered; only unique generated default destinations were verified.

No default change was made. Defaults remain adapter-specific: desktop torrent admission explicitly selects native torrent, while the CLI and legacy CoreServices/remote paths preserve aria2-first behavior (with the existing native fallback where configured). This evidence is not sufficient to promote native torrent globally or remove aria2.

## Prompt 19 — native magnet metadata, selection, and controlled lifecycle

The shared Rust lifecycle admits a native magnet without selecting every file, acquires metainfo through librqbit, parses librqbit's validated `info_bytes` with FetchX's existing parser, verifies the magnet info hash, and persists the resulting file list in the existing torrent metadata row. The temporary metadata-only torrent is removed before the engine waits for user confirmation. Transfer starts only after `selection_confirmed` is persisted with a non-empty index list; execution rechecks the info hash and index bounds before passing selected indexes to librqbit. Cancellation/pause also interrupt metadata admission/initialization, and cleanup errors are surfaced. Tests verify pre-admission cancellation leaves no torrent in the session and inject a post-handle metadata failure to verify the cleanup finalizer removes the temporary torrent and preserves the original error.

The controlled integration test uses a synthetic, non-private multi-file torrent: an unselected 16 KiB file and a selected 4 MiB file. A local librqbit seeder is addressed directly by loopback IP; there are no trackers, DHT, local discovery, public peers, or aria2 daemon. It verifies magnet metadata acquisition and info hash, waits for an explicit persisted selection, completes only the selected payload, pauses after partial progress, creates a new engine/session for the same persisted job and destination, and then completes with at least the previous persisted progress. The test also removes the sole seeder during a second active transfer, cancels after the controlled interruption, verifies the managed torrent handle was removed, restores the local seeder, retries the same persisted download, and verifies selected-file completion. An unselected destination may be absent or an empty placeholder; it must not contain payload bytes.

The restart test is an engine/session recreation with the same destination, not a process-kill/device restart test; it does not assert exact piece reuse before new peer data arrives. Peer interruption/retry is covered by controlled seeder removal and restoration, not by an actual network failure or arbitrary retry policy. Destination collision behavior, Android process recovery, adapter runtime consistency, and long-term seeding lifecycle are not covered by this fixture. Synthetic private torrents are not used for magnet metadata exchange because this librqbit test setup rejects their `ut_metadata` requests; the non-private fixture is still private to the test because only a loopback peer is supplied and discovery is disabled.

Adapter support is deliberately bounded: Android can inspect/poll and confirm a selection for native magnets when the existing routing chooses the native engine (no Android backend chooser); desktop exposes Tauri metadata and selection calls and displays selection for `native_torrent`; CLI can inspect/select after explicit native admission; the remote server/UI now expose authenticated metadata and selection calls, as detailed in Prompt 20 below. No global default, aria2 setting, packaging, or persisted-download behavior changed.

## Prompt 20 — remote native magnet metadata and selection

The authenticated remote JSON-RPC contract adds:

- `download.torrentMetadata` — read-only metadata projection for a native magnet. It returns the download ID/state, info hash, name, indexed file paths/sizes, and persisted selection state. It excludes the magnet URI, tracker URLs, and destination path.
- `download.selectTorrentFiles` — explicit selection confirmation using download ID, displayed info hash, and file indexes. It routes through CoreServices, which revalidates the pending native-magnet state, info hash, and non-empty unique in-range indexes before persisting the confirmed selection. It does not start a separate remote transfer manager.

Authorization maps metadata reads to `DownloadsRead` and selection mutation to `DownloadsControl` (operator/admin). Errors are typed and sanitized: missing downloads are not-found, malformed/empty input is invalid-params, and premature/stale confirmation is conflict. Discovery advertises both calls. They are intentionally not added to the restricted browser/native-host method allowlist. The remote Downloads detail surface polls while metadata is pending, lists selectable files when ready, confirms selected indexes, displays acquisition/confirmation/stale/failure/unavailable states, and checks server-advertised capabilities before calling either operation. Older servers continue to work; unsupported selection is shown as unavailable rather than calling an unknown method. The magnet-add engine selector is optional and defaults to automatic/server default, preserving prior routing.

This makes remote selection available alongside Android's native fallback, desktop Tauri/UI, and CLI's explicit native admission. It does not alter global engine defaults, aria2 compatibility, stored download records, or schema. Automated RPC/CoreServices/UI tests and a UI production build are the validation boundary; no live remote server, device, or remote network runtime was tested. Exact post-change results are listed under Prompt 20 validation below.

## Tests and validation

Completed checks for this validation slice:

- `cargo fmt --all` — passed.
- `cargo test -j 1 -p fetchx-downloader -p fetchx-storage -p fetchx-core-services -p fetchx-queue -p fetchx -p fetchx-remote-server -p fetchx-android-bridge` — passed. This includes the controlled-loopback transfer and stale-selection regression, CLI engine-default test, local metadata inspection and destination uniqueness checks, Android bridge tests, remote-server tests, storage tests, and associated doctests.
- `cargo test -j 1 --manifest-path apps\desktop\src-tauri\Cargo.toml` — passed: 60 desktop Tauri tests.
- `cargo clippy -j 1 --all-targets -p fetchx-downloader -p fetchx-storage -p fetchx-core-services -p fetchx-queue -p fetchx -p fetchx-remote-server -p fetchx-android-bridge` — passed.
- `cargo clippy -j 1 --all-targets --manifest-path apps\desktop\src-tauri\Cargo.toml` — passed with two existing warnings: a collapsible `if` in `app.rs` and the existing batch command's argument count in `commands.rs`.

These are compilation and automated test results, including a real transfer between two local peer sessions. No public/live swarm, aria2 daemon, Android device/emulator, packaged desktop application, or external network transfer was exercised. Kotlin/TypeScript UI checks were not needed because this slice changes no Android or desktop UI source.

### Prompt 19 validation

- `cargo test -j 1 -p fetchx-downloader` — passed: 293 unit tests passed, 3 ignored; both native-torrent lifecycle integration tests passed.
- `cargo test -j 1 -p fetchx-downloader native_magnet -- --nocapture` — passed: all 3 magnet tests (acquisition cleanup before/after a handle, and controlled-peer metadata/selection/lifecycle).
- `cargo test -j 1 -p fetchx-core-services torrent -- --nocapture` — passed: 6 tests.
- `cargo test -j 1 -p fetchx-downloader -p fetchx-core-services -p fetchx-storage -p fetchx-queue -p fetchx -p fetchx-remote-server -p fetchx-android-bridge` — passed before the final test-only cleanup helper refactor; the final downloader and CoreServices results are separately listed above.
- `cargo test -j 1 --manifest-path apps\desktop\src-tauri\Cargo.toml` — passed: 60 desktop Tauri tests.
- `npm run build` in `apps\desktop\ui` — passed TypeScript checking and Vite production build.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy -j 1 --all-targets -p fetchx-downloader -p fetchx-storage -p fetchx-core-services -p fetchx-queue -p fetchx -p fetchx-remote-server -p fetchx-android-bridge` — passed without warnings after fixing two warnings in the new/related native magnet selection test.
- `cargo clippy -j 1 --all-targets --manifest-path apps\desktop\src-tauri\Cargo.toml` — passed with two existing warnings: `collapsible_if` in `apps\desktop\src-tauri\src\app.rs` and `too_many_arguments` in `commands.rs`.

The Android bridge was covered by its Rust tests in the multi-package command; no Android Kotlin/Gradle device test was run because no Android source changed. No remote native-selection behavior was tested because the remote metadata/selection API does not exist. No device, process-kill, packaged desktop, public swarm, or live aria2 daemon validation was performed. `git diff --check` is run as a final worktree check.

### Prompt 20 validation

- `cargo fmt --all` — passed; `cargo fmt --all -- --check` — passed after formatting.
- `cargo clippy -j 1 --all-targets -p fetchx-remote-protocol -p fetchx-core-services -p fetchx-remote-server` — passed without warnings after applying the `ok_or` simplification in the CoreServices selection path.
- `cargo test -j 1 -p fetchx-remote-protocol -p fetchx-core-services -p fetchx-remote-server` — passed: CoreServices 35 unit tests plus 2 integration tests, remote protocol 11 unit tests, remote server 21 unit tests plus 3 integration tests, and the CoreServices doctest. No failures; 4 live extraction tests were ignored.
- `cargo test -j 1 -p fetchx-downloader native_magnet -- --nocapture` — passed: 3 native magnet tests, including the controlled-loopback metadata, selection, selected-only transfer, pause/recovery/interruption/retry/cancellation lifecycle case and metadata cleanup cases.
- `npm test` in `apps\remote-ui` — passed: 2 test files, 9 tests.
- `npm run build` in `apps\remote-ui` — passed TypeScript checking and Vite production build.
- `git diff --check` — passed. Git emitted only line-ending normalization warnings for Rust files.

These are automated unit/integration tests and a local controlled-peer test. No live remote network/server, Android device, or packaged desktop runtime was tested. The UI was not connected to an older or current live server; compatibility is covered by capability-gated behavior and UI/protocol tests.

## Prompt 21 — native torrent defaults and validation

New magnet and local `.torrent` admissions now default to native Rust at the shared CoreServices boundary. Desktop exposes aria2 as an explicit fallback when available; CLI retains `--engine aria2`; Android has no aria2 runtime or selector; and the current remote UI chooses native only when both metadata and selection capabilities are advertised. Older remote clients that omit the engine retain the server's existing aria2-first route. Browser/native-host admission remains HTTP(S)-only. Existing download rows continue to use their persisted engine and existing explicit `native_torrent.enabled = false` configuration is preserved. No non-torrent routing, aria2 settings, packaging, RPC, or records were changed.

The new-magnet regression asserts that admission persists a native-engine queued record with `metadata_state = pending`, no selected indexes, and `selection_confirmed = false`. When queue dispatch is paused by the test, the record correctly remains `Queued`; once dispatched, metadata acquisition precedes selection, and transfer requires an explicit valid selection. The CoreServices tests also cover native local torrent defaulting and explicit aria2 non-substitution when unavailable.

Exact validation:

- `cargo test -j 1 -p fetchx-models -p fetchx-downloader -p fetchx-storage -p fetchx-queue -p fetchx-core-services -p fetchx -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-android-bridge -p fetchx-desktop` — passed. Includes 293 downloader unit tests (3 ignored), 2 downloader integration tests, 36 CoreServices tests, 26 CLI tests, 25 Android bridge tests, 60 desktop Tauri tests, remote protocol/server, storage, queue, models, and doctests; no failures.
- `cargo clippy -j 1 --all-targets -p fetchx-models -p fetchx-downloader -p fetchx-storage -p fetchx-queue -p fetchx-core-services -p fetchx -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-android-bridge -p fetchx-desktop` — passed. The desktop target reports two existing warnings: `collapsible_if` in `apps\desktop\src-tauri\src\app.rs` and `too_many_arguments` in `commands.rs`; no new warning was reported.
- `cargo fmt --all -- --check` and `git diff --check` — passed.
- `npm test` and `npm run build` in `apps\remote-ui` — passed (2 test files, 10 tests; TypeScript and Vite production build).
- `npm run build` in `apps\desktop\ui` — passed TypeScript and Vite production build; there is no desktop UI test script.
- `.\gradlew.bat :downloads:test` in `apps\android` — passed. This is Android host-side Gradle test/build validation, not a device/emulator run.

The controlled torrent evidence remains the synthetic loopback-peer test described under Prompt 19; no real swarm, live remote server/older-server interoperability, aria2 daemon, Android device, or packaged desktop runtime was exercised. The test verifies explicit selected-only transfer, pause and session recreation, interruption, cancellation cleanup, retry, and completion, but not exact piece reuse before peer data arrives or process/device restart recovery. Native torrent remains a partial aria2 replacement; aria2 RPC, supervision, settings, helper packaging, and persisted aria2 jobs remain supported.

## Prompt 22 — remaining aria2 dependency audit

The audit covered the Rust workspace, Android and desktop sources, CLI and remote adapters, browser/native-host sources, build/package configuration, README, and migration/architecture/roadmap documentation. The worktree was clean at `4a08ab9` (`feat: default to native torrent engine for new magnet and local .torrent admissions`) before these documentation updates. No code change is justified in this phase: no new non-torrent admission defaults to aria2, and the remaining uses serve explicit compatibility, persisted jobs, or diagnostics/configuration rather than an unmigrated default download path.

### Remaining aria2 dependency inventory

| Classification | Remaining dependency | Active surfaces / compatibility significance |
|---|---|---|
| Required active execution | `Aria2Engine` and `Aria2Client` submit, control, inspect, and reconcile downloads explicitly stored as `EngineKind::Aria2`; `EngineDispatcher` routes those records to aria2. | Required for CLI/API explicit aria2 jobs, desktop torrent fallback, remote magnet fallback for older clients that omit `engine`, and persisted aria2 jobs. It is not the default for new non-torrent downloads. |
| Existing-download recovery | The persisted `engine_kind = "aria2"` remains authoritative. Aria2 torrent metadata stores `aria2_gid`; recovery queries aria2 status and reconciles task state/GIDs instead of silently resubmitting to another engine. | Required where a local aria2 runtime is available. If unavailable, records are preserved but cannot be resumed through a different backend automatically. Android's current bridge passes no aria2 executable, so legacy Android records persisted as aria2 are not locally executable; the app's separate **Remote aria2** screen only probes a separately configured remote service and does not attach it as the record's engine. |
| Explicit compatibility selection | CoreServices/dispatcher preserve `EngineKind::Aria2`; CLI accepts `--engine aria2`; desktop exposes aria2 as a torrent-only fallback in Add; current remote UI allows aria2 torrent selection. | User-visible behavior intentionally retained. Current remote clients choose native when both selection RPCs are advertised; clients omitting the engine keep legacy aria2-first magnet routing if aria2 is available. Remote HTTP creation is restricted to `direct_http`. |
| Process supervision and RPC | `Aria2Supervisor` starts and owns an optional local child; `Aria2Client` uses its loopback JSON-RPC endpoint for execution and lifecycle controls. The supervisor uses a generated runtime secret and bounded shutdown/reconciliation. | Required whenever local aria2 is enabled and for local aria2 tasks. This is not a second FetchX queue: QueueManager still owns scheduling and admission limits. |
| Configuration and UI | `EngineConfig.aria2` keeps enabled state, executable path, RPC port, and local RPC secret. Desktop Settings edits/tests it; CLI reports non-sensitive config; helper status/install surfaces expose the optional binary. | Desktop helper download is an explicit Windows-only per-user installation flow, not an installer-bundled or automatic dependency. Existing config and secrets remain intact. Android disables aria2 bundling and its bridge only enables the local engine when an executable path is supplied. |
| Diagnostics/status only | Runtime engine availability is reported by engine status and `fetchx engines`; desktop can test its configured local daemon and probe a separate remote endpoint (`getVersion`, then `getGlobalStat`). Android has a separate remote-endpoint probe for version/global statistics. | These probes are not download execution and do not provide full remote aria2 task management through FetchX's remote JSON-RPC. Endpoint secrets are not included in the result. CLI output exposes only `has_secret`; download export contains no engine configuration or RPC credential. |
| Queue/resource policy | Shared queue limits, priority, cancellation, and bandwidth policy apply to all engine records; ResourcePolicy retains an aria2-specific download bandwidth field. | aria2 task transfer remains engine-owned, while FetchX owns queue/concurrency policy. There is no evidence that native implements every aria2-specific transfer option, upload/global statistic, or peer behavior. |
| Storage/export contract | The engine enum/database mapping remains stable, and the torrent metadata export includes the aria2 GID field for backward-compatible torrent rows. | Required for existing records and backups. This is metadata, not a credential. App engine configuration and RPC secrets are outside the download export shape. |
| Packaging/build | The desktop helper catalog contains a separately installable aria2c Windows x64 package. Tauri bundle resources do not bundle aria2; Android's bridge explicitly passes no executable and disables extraction. No aria2 Rust crate or automatic build-time install was found. | Keep helper and packaging support until the explicit fallback and existing-record requirements are retired by a separately approved compatibility plan. |
| Remote/browser boundaries | FetchX remote `engine.runtimeStatus` reports sanitized availability; the remote server's HTTP path is direct HTTP only. The restricted browser/native-host RPC allowlist has no aria2 methods; browser URL intake is HTTP(S)-only. | Remote server is not a general aria2 RPC proxy. Android's remote aria2 profile and desktop's remote probe are diagnostics, not FetchX queue integration. |
| Dead/duplicate code | No aria2 execution, persistence, helper, or diagnostic code was proven dead by this audit. | Similar endpoint probes on Android and desktop are distinct platform settings surfaces; neither is safe to remove as unused. Existing engine and GID tests cover compatibility behavior. |

### New-download routing by protocol and target

| Protocol/operation | Rust-native route | New-download default and aria2 path |
|---|---|---|
| HTTP/HTTPS | `DirectHttpEngine` (segmented/resumable); HLS content/manifest can resolve to HLS when using the automatic direct route. | Direct HTTP is the default across the supported download surfaces. CLI can explicitly request aria2; remote HTTP creation rejects non-`direct_http` engines. No implicit aria2 fallback occurs. |
| FTP/FTPS | `FtpEngine` | Android and CLI scheme routing use the shared FTP/FTPS engines; other hosts expose only the protocols supported by their adapters. Aria2 is not the default. |
| SFTP | `SftpEngine` | Native shared engine when the host admits SFTP; no automatic aria2 routing. |
| BitTorrent/magnet | `NativeTorrentEngine` (`librqbit`) | Native by default for new admissions in CoreServices, Android, desktop, CLI, and capability-aware current remote UI. Explicit aria2 remains available where the surface supports it; older remote clients that omit `engine` preserve the legacy server route. |
| Metalink | `MetalinkEngine` orchestrating shared HTTP transfers | Native CoreServices engine; CLI recognizes `.metalink`/`.meta4` when no engine is specified. It does not default to aria2. |
| HLS/DASH and yt-dlp | `HlsEngine` for HLS; configured yt-dlp runtime for DASH/media extraction | Automatic HLS/DASH routing applies to supported direct-HTTP admission; media extraction selects yt-dlp. No implicit aria2 default. |
| Queue, concurrency, retry, resume | QueueManager/resource policy plus each selected Rust engine; aria2-specific jobs retain aria2's own RPC-backed task lifecycle. | Shared Rust scheduling controls all newly admitted tasks. Resume/retry semantics remain protocol/engine-specific; persisted aria2 tasks remain with aria2 rather than being reinterpreted as native. |

### Compatibility and next decision

No behavior migration is made: the normal HTTP and specialized protocol defaults are already Rust-native, and the remaining aria2 paths are necessary to preserve explicit user selection and persisted work. Removing or redirecting them now would violate the compatibility requirement, particularly because aria2 GID reconciliation and native torrent crash/process recovery are not equivalent or proven interchangeable. No tests/builds were run for this documentation-only audit; source inspection established the routing and dependency classifications above. `git diff --check` is the final validation.

Next migration decision: before reducing the explicit aria2 option, define a deprecation and transfer policy for generic CLI/CoreServices explicit jobs, desktop/remote torrent fallbacks, and aria2 records that cannot run on Android. In parallel, close the native acceptance gaps already recorded here—process/device restart recovery and exact partial-piece reuse—then test persisted aria2 recovery and current/older remote behavior with live runtimes. Keep aria2 RPC, supervisor, settings, helper, and persisted fields until those conditions and a user migration/rollback policy are satisfied.

## Migration phases still outstanding

1. **Phase A — coverage and acceptance**: strengthen exact piece-reuse and process/device-restart tests, and validate explicit destination-collision behavior after the default change.
2. **Phase B — parity**: implement the next missing native behavior only when a FetchX workflow needs it; arbitrary peer-failure policy and broader aria2 option parity remain open. Prompt 22 found no non-torrent default path left to migrate.
3. **Phase C — adapters**: validate live platform runtimes and old/new remote-server interoperability for the new native defaults without changing persisted engine choices; decide how unsupported Android aria2 records are handled.
4. **Phase D — removal**: remove aria2 RPC, process supervision, helper packaging, and settings only after native acceptance tests cover existing downloads, recovery, controls, diagnostics, and every supported target, and after explicit fallback users have a migration/rollback path.

**Status:** native Rust is the default for new torrent admissions on supported FetchX targets; aria2 remains an explicit compatibility backend and is not removed. Remote magnet metadata/selection is implemented through the authenticated JSON-RPC/UI boundary. The next slice is runtime compatibility/recovery validation: exact partial-piece reuse, process/device recovery, collision behavior, and live old/new remote-server compatibility.

## Prompt 23 — legacy aria2 job recovery assessment

This assessment follows persisted `DownloadSource.engine()` through startup recovery, `EngineDispatcher`, `Aria2Engine`, torrent metadata storage, and the Android bridge. The worktree was clean at `d7280b5` before this documentation-only update. No code change is justified: no shared-core operation safely proves that an aria2 partial can be adopted by librqbit or another native engine, and Android does not have a local aria2 runtime.

### Recovery-state matrix

| Persisted job state | aria2 recovery behavior | Native adoption status | Android result |
|---|---|---|---|
| Queued, no progress | Dispatch continues to use the persisted `aria2` engine. It can start with aria2 when its runtime is available. | Not automatic. A new native admission can use the source, but changing this record would silently change its persisted engine and destination/partial semantics. | No local aria2 executable is configured. Dispatch failure becomes a failed row; the sanitized message is generic rather than an Android-specific recovery explanation. |
| Paused | It remains paused across startup. Explicit resume queues the same persisted engine. A GID can identify a torrent task only while that aria2 task is reachable; generic-download GIDs are runtime-only. | Unsafe without first validating source, bytes, destination, and engine-specific resume state. | No local aria2 engine to resume. Retry is guarded by engine availability and does not replace the record when unavailable. |
| Preparing/downloading/metadata-acquiring/recovering interrupted by app shutdown | Queue recovery requeues the record without changing its engine and leaves partial files in place. Dispatcher still selects aria2. | Not supported. A persisted progress count is not evidence that the native engine can verify or resume those bytes. | Requeued work cannot execute through the unconfigured local aria2 runtime and is surfaced as failure. |
| aria2 HTTP/FTP/SFTP or other non-torrent partial | The supervised aria2 path requires a regular sibling `.aria2` control file before resubmitting a job with nonzero progress. Without it, it returns reconciliation-required. Unsupervised aria2 cannot resubmit a nonzero-progress job without a GID. | Not proven safe. aria2 control data and output/partial layout are not translated into native engine state. | No local aria2 recovery path. Preserve the existing record and data. |
| Torrent/magnet with a valid GID and matching metadata | `Aria2Engine` validates the persisted GID and, when available, the info hash through aria2 RPC. It can reconcile a task only when that GID still resolves in the relevant aria2 runtime. | Not supported. A GID is aria2-specific, not a librqbit session or torrent identifier. | The Android bridge cannot query a local aria2 runtime. Its separate remote aria2 endpoint probe is not wired as the engine for persisted downloads. |
| Torrent/magnet with missing, malformed, stale, or no-longer-resolvable GID | An invalid-GID response clears the stale GID; malformed GIDs or an info-hash mismatch return reconciliation-required. A removed task clears the GID and may be re-admitted when source/metainfo, persisted selection, destination, and runtime allow. This is not a guarantee of byte-level continuation; a nonzero-progress job without usable aria2 resume state is rejected for reconciliation. | Not supported. Metadata and file indexes alone do not authenticate existing files or prove piece completeness. | Cannot be repaired by local aria2. Preserve metadata, row, and files for a future recovery flow. |
| Seeding/interrupted seed | Startup turns interrupted seeding into queued recovery while retaining `EngineKind::Aria2`; resumption remains an aria2 operation. | Not supported; native seeding state/session recovery is not equivalent or verified. | Cannot resume locally; the persisted row remains available for diagnosis/removal. |
| Completed | No engine is needed to retain the completed result. The destination file is independent of future engine availability. | No adoption needed. Do not redownload or rewrite the record. | Completed record/file remains usable. |
| Failed or cancelled | A retry follows the persisted engine; it is not an implicit native conversion. A progressed job still needs valid aria2 reconciliation data. | No safe conversion path established. | Failed rows show a sanitized failure reason and retry/remove controls. Retry checks engine availability before replacing the record, so an unavailable aria2 job remains intact. |
| Removed | The queue removes the record, not its download/partial files. | No record-based adoption path remains. | Files are preserved by queue removal; a future recovery tool would need explicit user-directed file discovery and validation. |

### Platform differences and user-facing behavior

Desktop and CLI can execute persisted aria2 jobs only when their configured local aria2 runtime is available. The FetchX remote server also dispatches records by persisted engine; authenticated remote aria2 endpoint probes are diagnostics, not a general remote execution/recovery binding. Android's `BridgeInitializer` explicitly passes no aria2 executable. Its preference for a separately configured remote aria2 endpoint does not bind that service to download rows. On Android, a legacy aria2 record is therefore retained but not locally recoverable; its failed-row error is sanitized but does not explain the platform-specific runtime gap.

The generic-download GID is held in the aria2 runtime map and is not persisted for every protocol. Torrent metadata stores an aria2 GID, selected file indexes, and metainfo, but a GID is valid only in aria2's task context. The supervisor creates a private runtime configuration with `continue=true`; the inspected configuration does not configure aria2 session input/output files. Thus a process restart must not be treated as proof the old aria2 task/GID still exists. Where a supervised aria2 resubmission is attempted, a nonzero-progress job is gated on its `.aria2` control file. Exact continuation after process restart is not established by the current validation.

Android maps the persisted failure diagnostic to a visible failed row and currently marks failed rows retryable. The bridge checks engine availability before retry replacement, preventing the old record from being discarded when its engine is absent. This is data-preserving but not a sufficient recovery explanation; a follow-up should provide an explicit, actionable "aria2 runtime unavailable; data and record retained" state and avoid implying Retry can switch engines.

### Safe follow-up plan

1. Add a read-only shared-core recovery assessment for each persisted aria2 row: engine availability, source class, progress, torrent metadata/GID status, and whether required aria2 control state is present. It must not mutate engine identity or files.
2. Surface that assessment and an actionable unavailable-runtime explanation in Android and other clients. Keep retry bound to the persisted engine.
3. Validate same-runtime pause/resume and process-restart recovery against a real supervised aria2 daemon, including generic partials with/without `.aria2` control files and torrents with current/stale GIDs.
4. Only then design an opt-in migration as a new native download, retaining the original aria2 row and files until the user verifies the replacement. Torrent conversion additionally requires info-hash, selected-file, destination-collision, and piece-integrity validation.
5. Keep existing schemas, aria2 settings, credentials, supervisor, RPC, helper packaging, and records unchanged until rollback and removal gates are satisfied.

No persisted engine identity or GID was reinterpreted, and no schema or download file was changed. This documentation-only phase ran source inspection and `git diff --check`; no tests/builds or live aria2, Android device, or process-restart validation were run.

## Prompt 24 — read-only legacy aria2 recovery assessment

Prompt 24 implements the read-only assessment in CoreServices and exposes it through Android UniFFI and desktop Tauri. The persisted engine remains authoritative: the assessment never changes a download, clears/replaces a GID, starts or restarts aria2, dispatches through native Rust, or mutates a destination, partial, `.aria2` control file, or torrent metadata row.

| Assessment state | Evidence and meaning |
|---|---|
| `not_aria2` | The persisted download is not assigned to aria2; no aria2 task inspection is attempted. |
| `not_required` | The persisted aria2 download is completed; engine recovery is unnecessary. |
| `runtime_unavailable` | The configured runtime is disabled/unavailable on this host. The lookup is a non-starting supervisor peek. |
| `ready_to_start` | aria2 is reachable and the queued item has no partial progress to reconcile. |
| `job_available` | aria2 reports the persisted torrent task and its info hash matches the saved metadata when that evidence is available. This is not a guarantee of byte-level continuation. |
| `missing_torrent_metadata`, `missing_gid`, `stale_gid` | Torrent recovery lacks metadata/GID, or aria2 reports an invalid/removed/error task or mismatching info hash. An inconclusive response remains `unknown`. |
| `missing_control_file`, `control_file_present` | The expected sibling `.aria2` control file is absent/present for supervised partial work. Presence is explicitly not proof that resume will succeed. |
| `unknown` | Available RPC/filesystem evidence is inconclusive, including generic work without a persisted/reachable runtime GID. |

CoreServices returns a typed state with a sanitized explanation and next step. Missing download IDs return no assessment. Android shows a per-aria2-row action and the result/next step; desktop exposes the same operation in download details. Both surfaces explicitly state that assessment is read-only. The Android remote aria2 endpoint preference is not treated as the local runtime for persisted jobs. No CLI, FetchX remote JSON-RPC, or browser/native-host operation was added; those clients do not currently expose this assessment.

Focused source coverage verifies missing/malformed and stale/available torrent GIDs, ready/unknown generic jobs, control-file presence/absence, and that CoreServices assessment preserves the persisted record, torrent metadata, partial output, and sidecar bytes. The local Rust checks passed:

```text
cargo fmt --all
cargo check -j 1 --tests -p fetchx-downloader -p fetchx-core-services -p fetchx-android-bridge -p fetchx-desktop
cargo test -j 1 -p fetchx-downloader recovery_assessment -- --test-threads=1
  5 passed, 0 failed
cargo test -j 1 -p fetchx-core-services aria2_recovery_assessment_preserves_records_and_download_files -- --test-threads=1
  1 passed, 0 failed
cargo test -j 1 -p fetchx-android-bridge aria2_recovery_assessment_dto_preserves_state_and_guidance -- --test-threads=1
  1 passed, 0 failed
cargo clippy -j 1 --tests --no-deps -p fetchx-downloader -p fetchx-core-services -p fetchx-android-bridge -p fetchx-desktop
cargo fmt --all -- --check
git diff --check
```

`Set-Location apps\desktop\ui; npm run build` passed TypeScript checking and the Vite production build. The first Android Gradle attempt failed during UniFFI generation with a Rust allocation/stack-buffer-overrun error. Retrying with a single Cargo job, incremental compilation disabled, debug info disabled, and 256 codegen units succeeded:

```powershell
$env:CARGO_BUILD_JOBS='1'
$env:CARGO_PROFILE_DEV_INCREMENTAL='false'
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_PROFILE_DEV_CODEGEN_UNITS='256'
Set-Location apps\android
.\gradlew.bat :downloads:testDebugUnitTest :bridge:testDebugUnitTest
```

Result: `BUILD SUCCESSFUL`; bridge mapping and downloads ViewModel unit tests ran, including assessment loading/result/error UI state. UniFFI-generated Kotlin emitted two redundant-conversion warnings. Android tests/builds are host-side only; no device run was performed. A real supervised aria2 daemon, process restart, and live network recovery were not tested.

The final Rust test-module type-check, Clippy, formatting, and diff checks passed. Clippy reported two existing desktop warnings outside this change: `collapsible_if` in `app.rs` and `too_many_arguments` in `commands.rs`.

The next dependency boundary remains same-runtime aria2 recovery validation with a live supervised daemon, including GID changes/staleness and partial files with/without sidecars. Do not design cross-engine replacement from an assessment state alone; preserve the aria2 record and files until any future opt-in replacement independently verifies source, selected files, destination, and integrity.

## Prompt 25 — remote and CLI aria2 recovery assessment

The read-only `CoreServices::assess_aria2_recovery` operation is now available through authenticated remote JSON-RPC and the local/remote CLI. The server handler delegates to CoreServices and maps only the typed state/guidance; it does not inspect aria2 settings or endpoint profiles itself. The remote method is `download.aria2Recovery` with `{ "id": "<download-id>" }`. Its typed response carries `assessment_version: 1`, the download ID, one shared assessment state, and sanitized `explanation`/`next_step` strings. It contains no local path, GID, RPC credential, or raw aria2 error.

The method is advertised by `system.discover` as an additive `fetchx.rpc.v1` capability; the protocol version string and existing operations are unchanged. Existing clients remain unaffected. The CLI checks discovery for the exact capability before sending the new method, and refuses without sending it when connected to an older server. The server applies the existing `downloads.read` permission (Admin, Operator, and ReadOnly roles). The RPC is intentionally absent from `BROWSER_RPC_METHODS` and the browser/native-host allowlist.

`fetchx aria2-recovery <id>` uses `CoreServices::start_for_read_only_assessment`: it defers queue dispatch, does not start a local aria2 process, and returns `unknown` when the runtime was not safely checked in this short-lived CLI context. It does not infer a runtime from an unrelated remote aria2 profile. `fetchx remote aria2-recovery <id>` returns the remote server's own CoreServices assessment; thus runtime availability is based only on evidence available to that server.

Tests added cover protocol method parsing and response version/shape, the read-only role and discovery capability, sanitized unavailable-runtime response and persisted-record/partial-file preservation, CLI state rendering, and an older server that lacks the capability (verified to receive no assessment RPC). The serial focused command passed:

```text
cargo test -j 1 -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-core-services -p fetchx -- --test-threads=1
  CLI: 30 passed; CoreServices: 38 passed; remote protocol: 12 passed;
  remote server: 22 passed; remaining selected package test targets passed.
```

Validation: `cargo clippy -j 1 --tests --no-deps -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-core-services -p fetchx` passed; `cargo fmt --all -- --check` passed; and `git diff --check` passed. Git emitted only line-ending normalization warnings for existing modified Rust files. No live remote network/device or real aria2 daemon was used. The remaining limitation is real-runtime/process-restart recovery evidence, not client API availability; no cross-engine adoption or aria2 removal is implied.

## Prompt 26 — aria2-sourced native-engine parity audit

### Verified baseline and scope

At audit start, `master` was clean at `e3afb1d` (`feat: add aria2 recovery assessment RPC and CLI commands`). Prompt 25's remote/CLI assessment contract, `downloads.read` authorization, discovery capability, older-server CLI gate, and browser/native-host exclusion are present. New torrent admission routes were verified per surface:

| Surface | New magnet / `.torrent` default | aria2 compatibility |
|---|---|---|
| Android | Bridge calls shared CoreServices native admission; initializer supplies no local aria2 executable. | No local runtime by default. Bridge accepts an optional path from a host, but the current app initializer passes `null`. |
| Desktop | Add UI's “automatic” choice invokes the shared native default and exposes aria2 only as an explicit fallback. | Explicit aria2 remains selectable when available. |
| CLI | Magnet and `.torrent` default to native when no `--engine` is given. | `--engine aria2` remains explicit. |
| Remote UI/server | Current UI selects native when both magnet metadata and file-selection capabilities are advertised. Older clients/servers retain omitted-engine compatibility; server dispatch still chooses aria2 first if available, otherwise native. | Explicit aria2 remains accepted. Do not equate older-server compatibility with native default. |
| Browser/native host | No magnet/aria2 execution path was found in its restricted host surface. | Not supported there. |

Non-torrent admissions route to the Rust engines or configured yt-dlp, not aria2 by default. Persisted engine identity remains authoritative.

One Android packaging qualification was verified: the workspace currently contains `apps/android/app/src/main/assets/bin/aria2c-arm64-v8a` (69,107,592 bytes), but `apps/android/.gitignore` explicitly ignores developer-downloaded `aria2c-*` assets. The asset is not tracked or reproducibly supplied by the repository. Android's `BridgeInitializer` passes `aria2ExecutablePath = null`, logs that bundling is disabled, and leaves extraction commented out; a separate Android aria2 screen stores and calls a user-configured remote endpoint, not the persisted-download runtime. The ignored asset may be copied into an APK when present under the default assets directory, but no extraction/launch wiring was found. Its exact build provenance/version and distribution notices are unverified and must not be inferred from its filename.

### Remaining aria2 dependency inventory

| Dependency class | Current FetchX locations / behavior | Native Rust equivalent and disposition |
|---|---|---|
| Runtime invocation and supervision | `crates/downloader/src/aria2.rs` implements JSON-RPC download submission, polling, controls and reconciliation; `supervisor.rs` launches a private `aria2c`, generates runtime RPC credentials, checks readiness/process ownership, monitors and restarts it. `CoreServices::init_aria2` initializes it only when enabled/discovered. | No native process is required for HTTP, FTP/SFTP, Metalink, HLS/DASH, or native BitTorrent, but there is no native equivalent to aria2's job table/RPC/supervisor. Keep for explicit aria2 jobs and persisted aria2 rows. |
| Explicit fallback and engine selection | `EngineKind::Aria2` remains a stable serialized value. `enqueue_magnet_with_engine` and torrent-file admission accept explicit aria2; desktop Add/settings/helper UI, CLI engine option, and remote admission can preserve explicit choices where supported. `EngineDispatcher` routes using the persisted engine. | Native protocol engines cover new defaults, not automatic reinterpretation. Explicit aria2 selection and `EngineKind::Aria2` are compatibility contracts until a consented exit path exists. |
| Persisted work, GIDs and recovery | SQLite stores the engine with each download; torrent metadata stores `aria2_gid`, source/selection metadata and destination. `Aria2Engine` restores/reconciles the GID and info hash and inspects `.aria2` sidecars. CoreServices assessment is read-only. Exports retain the aria2 engine/GID fields. | Native torrent uses its own database metadata and librqbit handles, never an aria2 GID. It cannot consume aria2's `.aria2` control format. No safe cross-engine partial adoption exists. |
| Settings and secrets | `EngineConfig` persists enabled/path/RPC port/secret; desktop Tauri Settings and helper manager validate, install/discover and configure the helper. The Android bridge DTO can accept an executable path, but the shipped initializer passes none. The Android remote aria2 endpoint/secret is a separate preference profile. | Rust engines have their own availability/configuration; no equivalent need for the aria2 RPC credential. Do not delete or reinterpret either configuration profile in this phase. |
| Build, packaging and platform restrictions | Desktop Tauri resources currently include the browser native-host binary, not aria2. The managed aria2 installer code points to a Windows x64 release ZIP; system `aria2c` discovery/configuration remains separate. Android's ignored asset is developer-supplied; bridge runtime is deliberately disabled because of app-sandbox/SELinux execution constraints. | Native protocol engines are Rust workspace dependencies. Their behavioral parity is not sufficient to remove helper manager/settings or legacy recovery. Confirm platform/release-specific helper availability before any packaging change. |
| Remote RPC, CLI, diagnostics | CLI can select aria2 and query torrent/aria2 recovery; authenticated FetchX remote allows explicit engine selection and exposes only the typed read-only `download.aria2Recovery` assessment, not raw aria2 RPC. `engine.runtimeStatus` and desktop settings/helper screens provide status/configuration. Android's separate remote profile can probe/manage an external aria2 server. | Shared-core protocol execution replaces aria2 for new default routes; no FetchX remote equivalent to the full aria2 management RPC or external endpoint profile exists. Browser/native-host does not expose aria2 recovery or engine mutations. |
| Tests and support | `aria2.rs` has mocked JSON-RPC/reconciliation tests; supervisor tests use an injected fake process launcher; CoreServices has config/recovery tests. The native-torrent tests use controlled loopback peers. No real aria2 daemon or device test is claimed. | Hermetic tests prove code paths, not daemon-specific recovery, OS packaging, or network interoperability. Keep live-runtime gates open. |

### aria2 source and licensing review

Reviewed aria2 at commit [`9e7273583f83e881e3ec067b523ba88724088d2f`](https://github.com/aria2/aria2/tree/9e7273583f83e881e3ec067b523ba88724088d2f), using the manual and these focused source modules as behavioral references:

| Upstream source | Observed behavior used as a requirement reference |
|---|---|
| [`src/SegmentMan.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/SegmentMan.cc) | Segment ownership is coordinated with piece storage; cached writes are flushed and failures invalidate blocks rather than treating an incomplete segment as done. |
| [`src/RequestGroup.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/RequestGroup.cc) and [`src/RequestGroupMan.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/RequestGroupMan.cc) | Download groups own transfer context, progress/control-file handling, integrity completion and active-group policy; a paused group saves recovery state, while completed groups remove control state after completion/integrity handling. |
| [`src/DownloadEngine.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/DownloadEngine.cc) | A command/event loop drives asynchronous work; graceful end-of-run closes files and saves request-group state. This is materially different from merely persisting a progress counter. |
| [`src/MetalinkPostDownloadHandler.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/MetalinkPostDownloadHandler.cc) and [`src/Metalink2RequestGroup.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/Metalink2RequestGroup.cc) | A fetched Metalink manifest is expanded into child request groups and mirror/file metadata; this is more than downloading an XML document. |
| [`src/BtPostDownloadHandler.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/BtPostDownloadHandler.cc) | Acquired torrent metainfo is parsed and converted into following BitTorrent request groups, with group/metadata relationships maintained. |
| [`src/RpcMethodImpl.cc`](https://github.com/aria2/aria2/blob/9e7273583f83e881e3ec067b523ba88724088d2f/src/RpcMethodImpl.cc) | aria2 RPC exposes task lifecycle/status/options and global/session operations; FetchX deliberately exposes only its own authenticated, typed remote API. |

The pinned repository's `COPYING` is GNU GPL version 2; reviewed source headers state GPL-2.0-or-later and include an OpenSSL linking exception in those files. That exception is not a blanket relicensing grant. No aria2 source was copied, translated, or mechanically adapted. If future work incorporates upstream code or distributes modified/combined aria2 components, pause for a file-by-file licensing and distribution review with counsel; inspect each dependency and binary notice independently. The intended approach is clean-room behavioral requirements implemented against FetchX architecture and appropriately licensed Rust dependencies, not code translation. This is an engineering note, not legal advice.

### Prioritized behavior parity matrix

“Native exists” means there is an implementation path, not that aria2 parity has been proven. Evidence is intentionally separated from feature-name similarity.

| Priority | Behavior area | aria2 behavior to preserve | FetchX native status and evidence | Gap / removal relevance |
|---|---|---|---|---|
| **P0** | BitTorrent/magnet recovery, selected files and integrity | Persisted task identity, pause/restart recovery, selected-file transfers, piece integrity and seeding lifecycle. | FetchX storage retains confirmed selection and validates info hash/index bounds; `librqbit` session/DHT persistence remain disabled. A Windows child-process harness now kills a worker during a file-backed SQLite download, reopens the same record/output in a fresh worker, verifies the selected payload and unchanged selection/engine/info hash, and measures loopback peer bytes. | Aggregate peer-byte evidence shows partial reuse, but does not identify exact piece/block reuse. Android process-death behavior and queue-actor dispatch remain untested. |
| **P0** | Persisted aria2 jobs/GIDs/sidecars | Same-engine GID reconciliation and aria2 `.aria2` control-file semantics. | Shared recovery assessment reports bounded evidence only and preserves records/files. | No native equivalent; never map GIDs or sidecars to librqbit. Keep aria2 until legacy rows have a tested, user-safe disposition. |
| **P1** | HTTP/HTTPS multi-source and segmented transfer | Multi-URI selection/failover, parallel segments, retries and resumable partial state. | `DirectHttpEngine` has validator-checked range resume, up to 64 segments with adaptive concurrency, per-segment retries and staged publication. Metalink has explicit mirror failover. | Generic HTTP admission represents one source URL; arbitrary URI lists/multi-source selection are not an equivalent API. Existing segment tests do not establish parity for every server, proxy or crash point. |
| **P1** | FTP/FTPS and SFTP | Protocol negotiation, authentication, resume, retry, TLS/SSH host behavior and remote errors. | Shared `FtpEngine`/`SftpEngine` use Rust libraries; FTP documents REST-offset resume, and bounded retry policy exists. | Protocol implementations exist, but no behavior-level cross-server/daemon parity matrix or broad live fixture evidence was established in this audit. Preserve Rust route; test server variants before claiming parity. |
| **P1** | BitTorrent metadata and selection | Magnet metadata lifecycle, file selection, download, verify, seed and recover. | Native explicit metadata selection and selected-only transfer are implemented and covered by controlled peers; the migration report records lifecycle cases. | Native telemetry leaves some aria2 fields unavailable; librqbit session persistence is disabled. See P0 restart gap. |
| **P1** | Retry, pause/resume, scheduling, bandwidth and connection limits | Configurable retry behavior, per-job/global controls, queue slots and scheduling. | Rust queue/scheduler and resource policy own admission, retry and scheduling. HTTP/FTP have bounded retry; managed pause/resume/cancel exists. Torrent applies per-download rate and peer limits. | Exact aria2 retry-count/backoff, global upload/rate, option inheritance and all-protocol restart semantics are not equivalent or proven. Global Rust controls are not proof of aria2 option parity. |
| **P1** | Integrity, allocation and restart recovery | Checksum/hash validation, allocation policy, control-file recovery and safe finalization. | Metalink verifies declared supported hashes; BitTorrent verifies protocol pieces; HTTP validates range validators and stages output. | Generic checksum coverage, aria2 allocation modes, crash consistency, and partial-file exact reuse are not comprehensively equivalent. Native writes must not be assumed compatible with `.aria2` control files. |
| **P2** | Metalink | Manifest versions, per-file metadata, mirrors/priorities, checksums and follow-on downloads. | `MetalinkEngine` parses v3/v4 input and delegates HTTP mirrors/checksum verification to DirectHTTP. | Only FetchX's implemented subset is covered; no exhaustive aria2 option/version/mirror parity or external-server matrix is available. |
| **P2** | HLS/DASH and media extraction | aria2 protocol handling where applicable plus associated transfer orchestration. | HLS uses Rust `HlsEngine`; DASH/media extraction uses configured yt-dlp. These are not new aria2 routes. | yt-dlp is a separate optional executable/runtime; this is not a native DASH parser. No aria2 removal claim follows from it. |
| **P2** | Remote management and diagnostics | aria2 JSON-RPC task/status/options, global stats and daemon lifecycle. | CoreServices/FetchX remote expose engine status and a sanitized, read-only recovery assessment; CLI and desktop retain helper/configuration flows. | No native aria2-compatible management plane or remote endpoint profile equivalence exists. Keep aria2 RPC/UI/settings until product decides whether those surfaces are required. |

### Prompt 26 decision and Prompt 27 process-death evidence

Prompt 26 made no engine behavior change and correctly deferred persistence changes pending stronger evidence. Prompt 27 added only a test harness: the engine still disables librqbit session/DHT persistence, and the legacy aria2 sidecar remains unrelated to native recovery. The test validates disk-backed payload reuse via librqbit's normal torrent initialization; it does not enable librqbit session persistence or alter user records/files.

The Windows child-process validation passed:

```text
cargo test -p fetchx-downloader native_torrent_process_death_recovers_selected_files -- --nocapture
  1 passed on Windows; parent test spawned two fresh worker processes, forcibly killed
  the first after persisted progress, requeued the record, and verified completion in
  the second. Child outputs also report their own successful test-harness exits.
```

The test uses fixed multi-file metainfo, a loopback-only seeder, disabled DHT/trackers/local discovery, file-backed SQLite, a persistent explicit selection of file index 1, and a 4 MiB selected payload. It waits until the DB reports at least 128 KiB progress, kills worker A without graceful engine cleanup, verifies the first 16 KiB selected-file piece is present and correct, then calls the same state-machine `recover_interrupted` transition used by queue startup (`Downloading` to `Queued`). Worker B reopens the DB and output path and resumes the same native record. Final payload bytes and SHA-1 match; the unselected file is absent or zero bytes (librqbit may create an empty placeholder); engine identity, info hash, destination, selection and empty aria2 GID remain unchanged. Two successful runs reported 3,620,864 and 3,653,632 uploaded bytes after restart against a 4,194,304-byte selected payload (540,672–573,440 fewer bytes). This aggregate traffic result supports partial reuse, but is not piece/block-level proof and must not be described as exact piece reuse. Queue actor dispatch itself, Android process death, and real-device filesystem behavior are not tested by this harness.

### Removal gate

Do not remove aria2 runtime, supervisor/RPC, helper installation, configuration/secrets, packaging/support, storage fields or legacy recovery until all of these are decided and evidenced:

1. The file-backed child-process engine and CoreServices/QueueManager startup-recovery harnesses pass selected-file integrity on supported Windows hosts; add piece/block-level evidence before claiming exact reuse and validate Android process-death behavior on-device.
2. The parity matrix's required protocol cases have hermetic tests and targeted live interoperability evidence (HTTP multi-source/segment/range change, FTP/FTPS/SFTP resume/auth, Metalink mirror/hash failure, scheduler/limits and crash/finalization cases). Explicitly waive any aria2-only behavior the product will retire.
3. A real supervised aria2 daemon test validates current/stale GIDs, sidecar present/absent, pause/resume, crash/restart and shutdown for existing records, with database and file preservation.
4. Product defines an opt-in migration or retention/support policy for explicit aria2 users and every persisted aria2 row, including Android legacy records; prove rollback and destination/integrity safety. Never silently convert or delete them.
5. Desktop/Android binary provenance, platform packaging, dependency licenses and distribution notices are reviewed. Only then can settings, credentials, helper/install flow and RPC be considered for removal.

As recorded at the end of Prompt 27, only the native downloader test module and migration documents had changed; no database, adapter, binary, runtime setting, engine default or download behavior changed. The engine-layer Windows OS-process kill/reopen harness passed, but queue-actor process-restart validation had not yet been performed. Prompt 28's additional CoreServices/QueueManager evidence is recorded below. Android device, live aria2 and desktop package validation remain unperformed.

### Prompt 28 — CoreServices and QueueManager startup recovery

The Prompt 28 end-to-end harness in `crates/core-services/src/lib.rs` admits a local `.torrent` through `CoreServices::enqueue_torrent_file_with_selection_and_engine` with an explicit native engine and selected-file index. A fixed multi-file, private torrent uses a single loopback HTTP tracker and a loopback seeder; librqbit 9.0.1 suppresses DHT and local discovery for private torrents. The seeder and both workers use temporary directories and file-backed SQLite.

Worker A uses the production CoreServices and QueueManager, writes the generated download ID, and waits for persisted `Downloading` progress before the parent forcibly terminates the child. The parent confirms the row remains `Downloading`, with native engine identity and persisted torrent metadata/selection intact. Worker B then calls only `CoreServices::start` against the same database and directories: it does not enqueue, resume, or call the engine directly. Its queue snapshot must show the recovered download in `QueueManager`'s active set before it waits for `Seeding`/`Completed`. This distinguishes startup recovery plus queue dispatch from a direct engine restart. The terminal row and torrent metadata retain the same engine, info hash, destination, selected index `[1]`, confirmed selection and empty aria2 GID. The selected file's bytes and SHA-1 match; the unselected file is absent or empty.

Validation passed on Windows four focused runs, plus one run as part of the full CoreServices library suite:

```text
cargo test -p fetchx-core-services core_services_queue_recovers_native_torrent_after_process_death -- --nocapture
  1 passed per run; each parent test forcibly killed the admission worker and
  observed the fresh CoreServices worker complete through QueueManager dispatch.
```

In the two explicit repeated runs, loopback peer-upload counters increased by 3,391,488 and 3,096,576 bytes after restart for a 4,194,304-byte selected file. This is aggregate network-byte evidence only: it is consistent with reuse of existing data, but it does not establish exact piece/block identity. No exact piece reuse claim is made. The test validates desktop Windows process recovery and shared startup logic only; Android device process-death, live aria2, public trackers and packaged application execution were not tested. No production recovery behavior needed a fix; no runtime defaults, database schema, persisted user data, aria2 compatibility, or aria2 code changed.

Update the native-torrent removal gate accordingly: CoreServices/QueueManager startup recovery now has child-process Windows evidence, while exact piece/block reuse and Android device recovery remain open. All protocol parity, legacy aria2 runtime/GID/sidecar, explicit-user disposition, rollback, and packaging/license gates remain unchanged. No aria2 source was copied or translated; the existing pinned GPL-2.0-or-later source review remains reference-only.

## Prompt 29 — opt-in aria2-to-native HTTP restart assessment

No conversion action was implemented. The current model is insufficient to identify a safely replayable legacy aria2 HTTP(S) job:

- `DownloadSource` persists the source URL and engine, and its model comment explicitly says it deliberately contains no credentials. It has no provenance marker, capture time/expiry state, or record of transfer-affecting aria2 options such as global headers, cookies, or authentication configuration.
- `DownloadItem` persists a destination path, but that alone does not establish that it is still the correct destination for a new job or that the source URL remains valid.
- FetchX's aria2 `addUri` path rejects URL userinfo and fragments and sets a bounded set of per-job options; the persisted legacy row does not prove that the job was originally created through that exact path or did not depend on external aria2 configuration.
- A query-free HTTP(S) URL can still expire or depend on server-side state. Safely probing it would itself contact an external server and cannot prove that the eventual download will have equivalent authorization or content.

Consequently, filtering only on `EngineKind::Aria2`, HTTP(S) scheme, status, and a collision-free new path would not satisfy the requested source-safety guarantee. No row, file, GID, setting, schema, or client behavior was changed. Existing aria2 records remain available through their existing recovery paths; users can add a current URL as a separate native download through normal admission.

The next prerequisite is an explicit, user-confirmed current-source flow for legacy jobs, rather than replaying an unverified stored URL. Any future opt-in flow must receive the current URL from the user, use the standard native CoreServices admission path, and require a separately chosen non-colliding destination. A future persisted provenance/options model may improve eligibility for newly created jobs, but cannot retroactively prove the freshness or configuration of existing rows. This assessment is documentation-only; no tests or builds were run. All aria2-removal gates above remain open.

## Prompt 30 — user-confirmed native restart

Added a narrow, opt-in action on desktop and Android. The user must enter/confirm the current HTTP(S) URL, choose a separate destination, and affirm that this is a direct file download rather than a torrent, Metalink, or media/streaming job. The stored URL is only prefilled as a suggestion when its structure is safe; it is never treated as proof of freshness. The platform clients call a dedicated shared CoreServices admission method, which validates the persisted source record and current URL, then creates a separate `DirectHttp` record with a new identifier through the standard batch admission/queue path. The old item’s status and engine are not changed.

Eligibility is deliberately conservative: the persisted row must still exist, use the aria2 engine, have an HTTP(S) source, not be completed/seeding/stopped, and not look like a known media endpoint by source path or filename. The current URL must validate as HTTP(S), without embedded credentials or a fragment, and known media extensions are rejected. The user confirmation is mandatory in both UI and shared-core API. No torrent, Metalink, FTP/SFTP, or yt-dlp record can pass this boundary.

The destination must be an absolute, separately supplied path. CoreServices does not auto-rename it: it checks all persisted destinations plus their aria2 sidecar and partial-output paths, checks the selected destination and its `.aria2`/`.part` companions on disk, creates the parent if necessary, and performs a create/remove write-permission probe before admission. Collision or permission errors reject the action. Android restricts the supplied target to a direct child of FetchX's app-specific downloads directory. The old output, `.aria2` sidecar, partial bytes, record, and settings are never read for transfer or modified. The HTTP `DownloadItem` model does not persist an aria2 GID; this path does not query or reinterpret a GID or torrent metadata.

Focused CoreServices tests verify separate native ID/engine/source/destination, rejected invalid URLs and confirmation, non-HTTP/completed/media eligibility, target/sidecar/partial collisions, and byte-for-byte preservation of the old row, partial output, and `.aria2` sidecar. On Windows, `cargo test -p fetchx-core-services --lib` passed (41 tests), `cargo test -p fetchx-android-bridge` passed (26 tests), Android `:bridge:testDebugUnitTest`, `:downloads:testDebugUnitTest`, and `:app:compileDebugKotlin` passed, and desktop UI `npm run build` plus `cargo check -p fetchx-desktop` passed. CoreServices/Android bridge strict Clippy and Rust formatting passed. The combined strict desktop Clippy run is blocked by two existing unrelated warnings; desktop Clippy passed with only those lint classes allowed. `git diff --check` passed. No Android device or live aria2/server test was run. The media safeguards are intentionally conservative but cannot infer the content type of an extensionless HTTP URL; the explicit direct-file confirmation is required and remains a user assertion, not a server-side content guarantee.

Remaining aria2 removal gates are unchanged: real-daemon recovery of legacy IDs/sidecars, cross-platform/native torrent recovery, exact torrent piece reuse, protocol behavior parity, user disposition/rollback policy for remaining legacy jobs, and packaging/license review. No aria2 source was copied or translated; upstream remains behavioral reference only. The existing GPL-2.0-or-later licensing review remains required before any upstream-derived code or binary redistribution.

## Prompt 31 — user-confirmed native restart for aria2 torrents

Implemented a conservative magnet-only restart path for unfinished persisted aria2 magnet and local-torrent jobs on desktop and Android. The user must enter a current magnet URI and explicitly confirm it; a saved magnet is not prefilled as trusted current input. CoreServices verifies that the old item is still an aria2 torrent, that its torrent metadata row agrees with the item destination and source type, and that its persisted info hash (and any stored metainfo hash) is a valid 40-character hexadecimal hash matching the supplied magnet. Invalid sources, mismatches, unsupported base32 hashes, completed/seeding/stopped records, and unavailable native torrent runtime are rejected. Supplying a current `.torrent` file is not implemented in this phase.

The user must also choose a new, non-existing destination directory under FetchX's configured downloads root. The shared core rejects destinations overlapping any persisted download destination or its `.aria2`/`.part` companions, including parent/child directory overlaps. Native torrent path validation still enforces the configured root and rejects symlinks/reparse points. Android additionally restricts the choice to a direct child of its app-specific downloads directory. The standard native magnet admission then creates a new ID and metadata-acquiring job; it does not copy the aria2 file selection. The new job's normal metadata view requires a fresh explicit file selection before transfer. The native engine verifies acquired metadata against the magnet hash. The legacy row, GID, selection, metadata, destination, output, partial data, and sidecar are not changed or reused. No database/schema migration was added.

The CoreServices tests cover successful separate native admission from both legacy magnet and local-torrent records, mandatory confirmation, malformed and mismatched sources, completed-job rejection, legacy-destination overlap, and preservation of the old record, GID, metadata, selected indexes, output bytes, and `.aria2` sidecar. On Windows, `cargo test -p fetchx-core-services --lib` passed (43 tests), `cargo test -p fetchx-android-bridge` passed (26 tests), strict CoreServices/Android-bridge Clippy passed, formatting passed, desktop UI `npm run build` passed, and `cargo check -p fetchx-desktop` passed. Android Gradle tests/compile could not complete: UniFFI generation's Rust compilation failed from host memory pressure (allocation failure; one attempt also reported insufficient paging-file capacity), before Android tests or Kotlin compilation ran. No Android device, live tracker/seeder, or live aria2 daemon test was run.

This proves hash-gated admission and old-data preservation, not that the current magnet still serves the same payload beyond its matching info hash, that selected content matches the legacy selection, or that partial torrent pieces are reused. Exact piece reuse and Android device behavior remain unverified. Aria2 removal gates remain open, including legacy GID/sidecar recovery, protocol parity, Android runtime validation, exact piece-reuse evidence, user disposition/rollback, and packaging/license review. The aria2 manual/source remains behavioral reference only; no upstream code was copied or translated.

## Prompt 32 — current `.torrent` files and base32 v1 magnets

The native restart input now accepts either a user-confirmed current v1 magnet URI or a user-selected current `.torrent` file. Magnet `btih` hashes may be 40-character hexadecimal or 32-character RFC 4648 base32; both forms decode to the canonical lowercase hex info hash before comparison with the persisted aria2 hash. Base32 magnets are rewritten to the equivalent hex `btih` form only for native-engine admission; existing hex magnet input is preserved. `btmh`, malformed values, duplicate `xt` identifiers, v2-only identifiers, and hybrid/mixed identifiers are rejected. The current `.torrent` parser computes v1 identity as SHA-1 over the original bencoded `info` dictionary and requires v1 piece metadata; it does not claim v2/hybrid support.

For `.torrent` input, the user must inspect the newly selected file, explicitly select at least one file from its fresh file list, confirm that selection, and supply a distinct new destination directory. CoreServices validates the current source hash against the saved aria2 info hash, snapshots the supplied metainfo into FetchX-managed restart-source storage, rechecks the snapshot hash, and admits the selected file indexes as a separate native record. Desktop accepts a local path in the restart dialog and inspects it before presenting file checkboxes. Android uses the document picker, stages the selected document in app-private storage, inspects it, and presents file checkboxes. Platform adapters continue to constrain destination roots; CoreServices checks existing jobs and old outputs/partials/sidecars for path overlap.

The old aria2 row, GID, selection, metadata, destination, output, partial bytes, and `.aria2` sidecar are not changed or reused. No schema migration was added. Focused tests cover hex/base32 identity equivalence, malformed and unsupported identifiers, valid and invalid `.torrent` input, mismatches, explicit selection, collisions, separate native admission, and preservation of old records and bytes. On Windows, `cargo test -p fetchx-models --lib` passed (40 tests), `cargo test -p fetchx-core-services --lib` passed (45 tests), `cargo test -p fetchx-android-bridge` passed (26 tests), Android `:bridge:testDebugUnitTest`, `:downloads:testDebugUnitTest`, and `:app:compileDebugKotlin` passed, desktop UI `npm run build` passed, desktop `cargo check -p fetchx-desktop` passed, Rust formatting and `git diff --check` passed, and strict CoreServices/Android-bridge Clippy passed. The full strict Clippy command including desktop fails only on two existing unrelated warnings: collapsible `if` at `apps/desktop/src-tauri/src/app.rs:301` and too many arguments in the existing `add_download_batch` command at `commands.rs:1114`; desktop Clippy passes when only those two lint classes are allowed. No Android device, live tracker/seeder, or live aria2 daemon test is claimed.

The aria2 project and documentation remain behavioral references only: https://github.com/aria2/aria2 and https://aria2.github.io/. No aria2 source was copied or mechanically translated. Aria2 runtime, RPC, packaging, settings, GID/sidecar recovery, protocol parity, user disposition/rollback, Android device validation, exact torrent-piece reuse, and licensing/redistribution review remain removal gates.

## Prompt 33 — native multi-protocol behavior parity gate

This is a behavior-level audit of the shared Rust engines, not a feature-name parity claim. `cargo test -p fetchx-downloader` on Windows passed: 299 tests passed and three remained ignored; two downloader doc tests passed. The passing suite includes hermetic loopback HTTP/FTP and Metalink fixtures and native torrent lifecycle tests. `cargo fmt --all -- --check` and `cargo clippy -p fetchx-downloader --all-targets -- -D warnings` passed. The CoreServices QueueManager restart test passed with `cargo test -p fetchx-core-services --lib core_services_queue_recovers_native_torrent_after_process_death -- --nocapture`.

| Protocol / behavior | Status | Verified behavior and limits |
|---|---|---|
| HTTP(S) segmentation and range handling | Implemented and tested | Loopback fixtures verify range layout, bounded/adaptive concurrency, retry limits, exact assembly, server range rejection/ignored ranges, invalid range metadata, and safe single-stream fallback. |
| HTTP(S) resume, retry, redirects and finalization | Implemented and tested | Tests cover cancellation/resume, validators (`ETag`/`Last-Modified`), `If-Range`, 416 and unknown totals, transient retry, truncation, publication collisions, and rejection of redirects to private addresses. The public-redirect test is ignored because it requires external network access; proxy and broad server interoperability are not established. |
| FTP download, REST resume and cancellation | Implemented and tested | An in-process FTP server exercises download and REST-offset resume; cancellation and retry classification are tested. The separate ignored FTP integration test is redundant with the active local fixture tests. No fixture verifies server-side credential acceptance or interrupted-transfer retry/resume end to end. |
| FTPS TLS and authentication | Explicit FTPS implemented and locally tested, with limits | Prompt 36 adds a hermetic explicit-FTPS TLS FTP server. Private-CA certificate acceptance, untrusted-certificate rejection, credentials, transfer integrity, REST resume, cancellation/preservation, size mismatch, and handshake timeout are covered. Implicit FTPS and external-server interoperability remain unverified. |
| SFTP host keys, authentication, resume and interruption | Implemented and hermetically tested, with limits | Prompt 35 replaces the empty placeholder with an in-process russh/russh-sftp server. The fixture verifies password and public-key authentication, changed host-key rejection, content transfer, prefix-validated resume, cancellation/manual retry, changed remote content across retry, size mismatch, in-transfer metadata change, and destination/partial preservation. Automatic SFTP retry and external-server interoperability remain unverified. |
| BitTorrent metadata, selection, integrity and restart | Implemented and tested, with limits | Local native-peer tests verify metadata/selection and selected output; the CoreServices child-process test exercises QueueManager startup recovery against a loopback seeder and verifies selected content/hash and persisted identity. Aggregate peer bytes suggest partial reuse but do not prove exact block reuse. No Android device, public swarm, or live tracker test was run; a corrupt-piece rejection scenario is not independently asserted by these tests. |
| Metalink parsing, checksum and mirror behavior | Implemented and tested as a subset | Unit tests cover v3/v4 parsing and MD5/SHA-1/SHA-256/SHA-512 computation. Loopback end-to-end tests cover SHA-256 success/mismatch, a 500-to-success mirror failover, multiple files, and cancellation. The explicit failover-on-checksum-mismatch branch and exhaustive aria2 manifest/option compatibility are not proven. |
| Generic HTTP multi-source selection | Implemented, tested (bounded contract) | CoreServices admits 2–16 ordered HTTP(S) sources for one DirectHTTP job. Sources are attempted sequentially; segmentation is per source only. Cross-source range mixing is unsupported, and caller-supplied SHA-256 is required to prove expected content before failover from a completed but incorrect representation. |
| FTP active mode | Intentionally different / unsupported | FetchX uses passive mode only; `FtpEngineConfig::passive_mode` documents that active mode is not implemented. |

No transfer logic was changed: the clearest gaps are actual FTPS/SFTP server interoperability and SFTP restart semantics, for which the current suite does not provide a trustworthy wire-level oracle. The reproducible local harness is `cargo test -p fetchx-downloader`; its existing HTTP, FTP, Metalink, and torrent fixtures are loopback-only. A follow-up SFTP gate must replace the ignored placeholder with a real in-process SSH/SFTP server and cover known/unknown host keys, password and public-key authentication, successful content, offset resume, remote-file change during resume, and interruption/retry. FTPS requires a local TLS FTP fixture covering certificate validation and auth. Until then these rows remain unverified, not parity.

The CoreServices focused test initially invoked `cargo test -p fetchx-core-services core_services_queue_recovers_native_torrent_after_process_death -- --nocapture`, which caused Cargo to compile unrelated integration-test targets and rustc failed with “only metadata stub found” for `std`/`core`. Retrying the library test explicitly with `--lib` passed; the initial failure was a test-target/build-metadata issue, not a failing recovery assertion. No Android device, live aria2 daemon, public tracker, or external server was used. Desktop/Android adapters were not changed, so no adapter-specific build was required for this documentation-only gate.

The upstream aria2 source and documentation remain behavioral references (including the already recorded `SegmentMan`, request-group, Metalink, and BitTorrent paths); no source was copied or mechanically translated. Runtime/RPC/settings/packaging, existing GID and sidecar recovery, explicit user disposition and rollback, platform runtime validation, exact torrent-piece reuse, protocol interoperability, and licensing review remain removal gates.

## Prompt 35 — native SFTP resume and transfer integrity

The shared SFTP engine now verifies that an existing `.part` file is a prefix of the current remote file before seeking to its resume offset. It rejects a partial larger than the current remote size or with any mismatching byte, then requires the received byte count to equal the initial remote size and re-stats the file before publication. A changed size or available modification time prevents publishing. Failure leaves the existing destination untouched; a stale partial is not modified when prefix validation rejects it.

The hermetic fixture in `crates/downloader/src/sftp.rs` covers real local SSH/SFTP exchanges: password and public-key auth success, bad password rejection, changed host-key rejection, transfer, offset resume, same-size changed-prefix rejection after interruption, a remote shrink below the partial length, metadata change during transfer, cancellation with partial preservation, explicit retry/resume, and collision preservation. `cargo test -p fetchx-downloader sftp_ -- --nocapture` passed (31 focused tests). The full downloader test command passed: 252 library tests passed, two remained ignored, both `native_torrent_lifecycle` tests passed, and the child-process torrent recovery test passed. Strict downloader Clippy, `cargo fmt --all -- --check`, and `git diff --check` passed.

Behavioral reference review used aria2 commit `9e7273583f83e881e3ec067b523ba88724088d2f`: `src/SftpNegotiationCommand.cc` (`executeInternal`, `onFileSizeDetermined`) for stat/length validation and offset selection; `src/SSHSession.cc` (`sftpStat`, `sftpSeek`, `readData`) for metadata and transfer operations; `src/SftpDownloadCommand.cc` (`prepareForNextSegment`) for segment completion; and `src/SftpFinishDownloadCommand.cc` (`execute`) for graceful close and retry handling. These files carry aria2/Tatsuhiro Tsujikawa copyright notices and GPL-2.0-or-later licensing language (including the upstream OpenSSL exception). They were inspected as behavioral references only: no code was copied, ported, or mechanically translated, no aria2-derived source was added, and no runtime dependency was introduced. The user-provided authorization for selected reuse is acknowledged; future incorporation still requires the applicable notices and licensing review.

This improves safe resume but does not establish full SFTP parity. There is no automatic SFTP retry loop; a later explicit engine invocation can resume after validation. The content-prefix comparison detects source changes before resume, and final size/mtime checks detect many changes during a transfer, but SFTP does not provide a portable immutable snapshot or strong content validator; same-size mutations that preserve/coarsen modification time during an active transfer cannot be ruled out universally. No external SSH server, Android device, or desktop/remote platform build was run; shared APIs and adapters were unchanged. External aria2 remains removed, and this work adds no executable, daemon, RPC, supervisor, or fallback.

## Prompt 36 — native FTPS TLS and authentication validation

The shared FTP engine now accepts optional DER-encoded additional trust roots while retaining WebPKI roots, bounds the FTPS control/data socket operations with configured timeouts, classifies secure-session failures as non-retryable TLS errors, and verifies the final received byte count against FTP `SIZE` before publishing. The retry path now reloads the current persisted item so a transient failure after entering `Downloading` does not attempt an invalid second `Downloading` transition. TLS failures remain FTPS failures; there is no plaintext fallback.

The in-process explicit-FTPS fixture uses a generated local certificate and covers trusted private-CA validation and complete transfer, rejection of an untrusted certificate with pre-existing destination/partial preservation, valid and invalid credentials, `REST` offset resume, cancellation and preservation, remote size mismatch, and bounded TLS handshake timeout. The two previously failing certificate/timeout tests pass after TLS upgrade errors were mapped at the secure-session boundary. This does not test implicit FTPS, active-mode FTP, a third-party FTP server, all certificate name/chain combinations, or automatic retry/recovery after every network failure; FTP is passive-mode only.

Aria2 was reviewed at commit `9e7273583f83e881e3ec067b523ba88724088d2f`, particularly `src/FtpNegotiationCommand.cc` (TLS negotiation and connection setup) and `src/FtpDownloadCommand.cc` (transfer/resume behavior). These GPL-2.0-or-later sources were behavioral references only; no source was copied or ported, and no new aria2 runtime or dependency was added. The user's previously provided authorization for selected-code reuse is acknowledged but was not needed for this change.

On Windows, `cargo test -p fetchx-downloader ftps_ -- --nocapture` passed (10 tests); `cargo test -p fetchx-downloader` passed (259 library tests, 2 ignored; 2 lifecycle integration tests, including nested process recovery); strict downloader Clippy, `cargo fmt --all -- --check`, and `git diff --check` passed. No platform adapter API changed, so no additional desktop/Android/remote build was required; no device run was performed.

## Prompt 37 — native HTTP multi-source failover and segmentation

CoreServices now admits one DirectHTTP queue item with 2–16 distinct ordered HTTP(S) sources and an optional expected SHA-256. The first canonical URL remains the existing persisted `DownloadSource`; a versioned `.fetchx.sources` sidecar stores the ordered list, avoiding a schema change and preserving the single-URL record/API. Admission rejects credentials, non-HTTP schemes, duplicates, media manifest URLs, invalid checksum shapes, and collisions with related partial/metadata files.

The downloader tries one source at a time. Each source uses the existing HTTP retries, redirect/private-address policy, range scheduler, and segmented validation. If a candidate fails, FetchX removes only that attempt's validated `.part` and known segment files before switching; cancellation/pause preserves the current source's resumable partial and metadata. If range metadata is inconsistent or the server ignores ranges, the existing single-source fallback remains in effect. The source list is removed only after successful publication and retained on failed/cancelled jobs for retry.

FetchX deliberately does not combine segments across URLs. Equal lengths, equal/weak ETags, and Last-Modified do not establish cross-source identity. Same-URL range/resume validators remain scoped to that URL. When a caller supplies SHA-256, the existing pre-publication hash check rejects a complete wrong object (including same-size content with the same ETag) and allows the next source to be attempted. Without a checksum, ordered failover treats the first source that completes the ordinary HTTP validations as authoritative; it cannot detect semantically different but valid content from that source. This is safe because no bytes from distinct URLs are mixed, but it is not aria2's full multi-URI segmented behavior.

Hermetic tests cover a failing source during active range work followed by segmented transfer from an identical-content source; exact no-gap/no-overlap range coverage; wrong same-size bytes with a matching ETag rejected by SHA-256; inconsistent range totals and ignored ranges falling back to one source; cancellation with same-source resume; retry exhaustion with source metadata retained; and destination collision preservation. CoreServices tests validate source-count and duplicate rejection. No aria2 code was reused. The selected behavior references at commit `9e7273583f83e881e3ec067b523ba88724088d2f` were `src/SegmentMan.cc` (`checkoutSegment`, `getSegment`, `getSegmentWithIndex`, `cancelSegmentInternal`, `completeSegment`), `src/SegmentMan.h`, `src/InorderURISelector.cc` (`select`), `src/RequestGroup.cc` (in-order selector initialization), and `src/HttpDownloadCommand.cc` (`prepareForNextSegment`). These files carry GPL-2.0-or-later notices and the upstream OpenSSL exception; they were inspected only. The user-provided selected-code authorization is acknowledged but no port was needed.

The implementation uses FetchX's existing deterministic `segments::plan_segments` partition and retry state, not aria2's peer/piece allocator. The fixture proves this bounded behavior only; external mirrors/proxies, validator variants, Android device execution, and cross-source parallel ranges are not claimed. External aria2 remains removed.

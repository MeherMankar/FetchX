# aria2 to Native Rust Migration

## Scope and evidence

This is a staged migration report, not a claim that aria2 has been removed.

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

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

## Migration phases still outstanding

1. **Phase A — coverage and acceptance**: strengthen exact piece-reuse and process/device-restart tests, and validate explicit destination-collision behavior after the default change.
2. **Phase B — parity**: implement the next missing native behavior only when a FetchX workflow needs it; arbitrary peer-failure policy and broader aria2 option parity remain open.
3. **Phase C — adapters**: validate live platform runtimes and old/new remote-server interoperability for the new native defaults without changing persisted engine choices.
4. **Phase D — removal**: remove aria2 RPC, process supervision, helper packaging, and settings only after native acceptance tests cover existing downloads, recovery, controls, diagnostics, and every supported target.

**Status:** native Rust is the default for new torrent admissions on supported FetchX targets; aria2 remains an explicit compatibility backend and is not removed. Remote magnet metadata/selection is implemented through the authenticated JSON-RPC/UI boundary. The next slice is runtime compatibility/recovery validation: exact partial-piece reuse, process/device recovery, collision behavior, and live old/new remote-server compatibility.

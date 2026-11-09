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
| Magnet and local `.torrent` transfers | Partial | `librqbit` is integrated into the shared dispatcher. This phase makes backend choice explicit and passes confirmed local torrent file indexes to `AddTorrentOptions.only_files`. Magnet metadata selection is still not applied by the native engine. |
| Local torrent file selection | Covered for the migrated path | Persisted confirmed indexes are revalidated against the current `.torrent` info hash and file list before being supplied to librqbit; unconfirmed, empty, changed, and out-of-range selections are rejected. Magnet metadata does not restrict the local-file add. |
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
| Android | Existing shared-core native-torrent fallback remains available when aria2 is not configured. No Kotlin/UniFFI change was needed for this slice. Native magnet file selection is not migrated. |
| Desktop | Explicit engine selection reaches CoreServices; the local torrent file-selection path selects native torrent. aria2 settings and helper management remain available. |
| CLI | Explicit `native-torrent` selection is supported for magnets and local torrent files; current aria2 default is preserved. |
| Remote server | Explicit native or aria2 magnet selection is honored. An omitted engine preserves aria2-first/native-fallback behavior. |
| Browser/native host | No protocol or RPC contract change; no new native engine-selection UI was added. |
| Existing data/settings | Preserved. There is no migration away from aria2 GIDs or aria2-specific configuration, and no existing download is rewritten. |

## Prompt 18 — native torrent runtime evidence

The deterministic fixture is generated in the test from two synthetic 16 KiB files and a private multi-file `.torrent`. A local librqbit session verifies and seeds those files. The downloader connects to the seeder's loopback peer address through a private test seam; DHT, trackers, and local service discovery are disabled. No public swarm, external content, or aria2 daemon is involved.

End-to-end assertions confirm that local torrent metainfo can be inspected without queue admission, a confirmed index is persisted and applied, only the selected 16 KiB file is written, downloaded-byte progress reaches the selected file size, and the download transitions to `Seeding`. A CoreServices regression check enqueues the same local torrent twice and verifies that the generated default destinations differ.

The validation exposed stale-selection risk when `.torrent` bytes change after selection. The engine now reparses the current file and rejects a different info hash, empty or unconfirmed selections, and indexes outside the current file list. Focused tests cover these rejections and the controlled-peer transfer.

The acceptance scope is still incomplete: the peer transfer does not exercise pause/resume mid-transfer, retry, cancellation during a live transfer, or app shutdown/restart recovery. Existing lifecycle tests cover pause then cancellation against an unavailable tracker and stopping a completed seed without deleting its payload; CoreServices separately tests generic persisted-queue recovery. Magnet metadata acquisition and magnet file selection are not exercised by this fixture. Explicit user-supplied destination collisions are not covered; only unique generated default destinations were verified.

No default change was made. Defaults remain adapter-specific: desktop torrent admission explicitly selects native torrent, while the CLI and legacy CoreServices/remote paths preserve aria2-first behavior (with the existing native fallback where configured). This evidence is not sufficient to promote native torrent globally or remove aria2.

## Tests and validation

Completed checks for this validation slice:

- `cargo fmt --all` — passed.
- `cargo test -j 1 -p fetchx-downloader -p fetchx-storage -p fetchx-core-services -p fetchx-queue -p fetchx -p fetchx-remote-server -p fetchx-android-bridge` — passed. This includes the controlled-loopback transfer and stale-selection regression, CLI engine-default test, local metadata inspection and destination uniqueness checks, Android bridge tests, remote-server tests, storage tests, and associated doctests.
- `cargo test -j 1 --manifest-path apps\desktop\src-tauri\Cargo.toml` — passed: 60 desktop Tauri tests.
- `cargo clippy -j 1 --all-targets -p fetchx-downloader -p fetchx-storage -p fetchx-core-services -p fetchx-queue -p fetchx -p fetchx-remote-server -p fetchx-android-bridge` — passed.
- `cargo clippy -j 1 --all-targets --manifest-path apps\desktop\src-tauri\Cargo.toml` — passed with two existing warnings: a collapsible `if` in `app.rs` and the existing batch command's argument count in `commands.rs`.

These are compilation and automated test results, including a real transfer between two local peer sessions. No public/live swarm, aria2 daemon, Android device/emulator, packaged desktop application, or external network transfer was exercised. Kotlin/TypeScript UI checks were not needed because this slice changes no Android or desktop UI source.

## Migration phases still outstanding

1. **Phase A — coverage and acceptance**: retain this table as the baseline; add focused tests for native magnet selection, live-transfer pause/resume/cancel/retry, and torrent-specific queue/restart behavior before changing defaults.
2. **Phase B — parity**: implement the next missing native behavior only when a FetchX workflow needs it; magnet file selection needs metadata acquisition followed by a persisted selection before file transfer.
3. **Phase C — adapters**: migrate each adapter only after its explicit selection, fallback, settings, and backward-compatibility behavior is tested. Do not silently change the current default.
4. **Phase D — removal**: remove aria2 RPC, process supervision, helper packaging, and settings only after native acceptance tests cover existing downloads, recovery, controls, diagnostics, and every supported target.

**Status:** aria2 is partially supplemented by native Rust, not fully replaced. The next recommended slice is native magnet metadata/file selection, with a deterministic lifecycle test before any adapter default changes.

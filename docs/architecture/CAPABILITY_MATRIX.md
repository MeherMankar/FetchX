# FetchX Capability Matrix

## Reporting model

Capabilities have two independent states:

- **implementation availability**: the Rust engine or adapter can run;
- **platform availability**: current OS, permissions, executable runtime, and bundled resources support it.

`engine_status.rs` reports only engines with an active implementation/runtime. `EngineKind::Aria2` remains a persisted compatibility value for old records and is not included in current runtime status. Capability reporting should extend the shared projection rather than infer status from UI lists.

## Repository-evidenced capabilities

| Capability | Rust implementation | Android | Desktop | CLI/server |
|---|---|---|---|---|
| HTTP/HTTPS segmented download | Available | Available | Available | Available |
| FTP/FTPS | Available in downloader | Available, conditional on binding/configuration | Available, conditional on binding/configuration | Available |
| SFTP | Available in downloader with russh | Available, conditional on host policy | Available, conditional on host policy | Available |
| HLS native segments | Available | Available through shared dispatcher/runtime | Available through shared dispatcher/runtime | Available |
| Direct DASH MPD dispatch | Direct HTTP(S) `.mpd` URLs route to the configured yt-dlp engine; no native DASH engine | Shared dispatcher; requires the configured yt-dlp runtime | Shared dispatcher; requires the configured yt-dlp runtime | Shared dispatcher for CLI/server/browser-submitted URLs; requires yt-dlp |
| Metalink | Available | Core implementation; advertise from runtime status | Core implementation; advertise from runtime status | Available |
| Native BitTorrent/torrent | Available | Available; default for new admissions | Available; native default | Core implementation; host availability as configured |
| Legacy aria2 records | Historical engine identity is parsed/displayed; no runtime adapter or dispatch exists | Preserved rows displayed unavailable; explicit native restart is available for eligible records | Preserved rows displayed unavailable; explicit native restart is available for eligible records | Historical IDs/GIDs/metadata remain inspectable; new aria2 selection is rejected |
| aria2 runtime / RPC / packaging / settings | Removed; `EngineKind::Aria2` is compatibility-only and always unavailable | No executable, RPC screen, or bundled helper | No local/remote aria2 settings, helper, process, or RPC probe | No engine selector, CLI engine option, or aria2 daemon client |
| yt-dlp | Available with process runner | Android runtime adapter supplies implementation | Optional executable/helper adapter | Optional executable/configuration |
| yt-dlp audio extraction (MP3/M4A) | Typed source option and shared argument/lifecycle handling | Browser picker via UniFFI; bundled FFmpeg is passed for extraction | Tauri Add dialog supports MP3/M4A | Local CLI `--audio-format`; remote API/UI and browser extension/native host do not expose media-specific selection |
| FFmpeg/FFprobe | Available process orchestration | Android bundles statically linked FFmpeg runtime | Optional helper executable | Optional executable/configuration |
| Media format extraction | Available in media-extractor | Android runtime adapter | Shared Rust plus process adapter | Shared Rust plus process adapter |
| Pre-creation resource resolution | No general resolver; media extraction and local torrent inspection are specialized operations | UniFFI exposes media formats and local `.torrent` inspection; magnet selection follows metadata acquisition | Tauri exposes media formats and local `.torrent` inspection; magnet selection follows metadata acquisition | CLI supports local torrent selection and post-admission magnet selection; FetchX remote has no resolve operation; browser capture routes URLs without resource inspection |
| Queue/scheduler/recovery | Available | Available | Available | Available |
| SQLite persistence | Available | App-private database | OS app-data database | Caller-selected database |
| Remote JSON-RPC | Available server transport | Android browser-RPC transport exists | Desktop IPC should remain separate unless exposed | Available HTTP/WebSocket |
| Engine runtime status | CoreServices projection of all active engine kinds; aria2 is omitted | No engine-status operation or UI consumer | Local settings consume Tauri engine status; remote engine status is a separate API | Authenticated `engine.runtimeStatus` is consumed by the remote UI; not exposed to the CLI or browser/native-host allowlist |
| Persisted download runtime status | Compact projection of persisted `DownloadItem` fields: `id`, `status`, `progress`, and `updated_at` | Status/progress already consumed via list/events; separate UniFFI method has no caller | Status/progress/update time already consumed via list/events; separate Tauri/TypeScript wrapper has no UI caller | CLI list/monitor commands consume persisted status; no separate runtime-status command | Status/progress/update time already consumed via `download.list`; separate RPC/helper has no remote UI caller; older servers need no new capability |
| Background execution | Platform adapter responsibility | Foreground service/Android lifecycle | OS application lifecycle | Long-running process |
| Browser integration | Protocol/native host/modules | GeckoView/browser module | Desktop browser IPC/native host | Browser extension client via server |
| Plugins | Policy/API crates; Android service has no execution adapter | Policy evaluation only, no execution capability | Depends on future/host adapter | Policy/manager if wired |
| Normalized-URL duplicate lookup | CoreServices repository-backed query; reports a match without enforcing creation policy | UniFFI lookup is consumed by the Add dialog as a confirmation warning; adding anyway remains possible | Tauri command and TypeScript API are exposed, but no UI caller was found | Local/remote CLI and authenticated `download.findDuplicate` RPC are available; browser RPC allowlist permits it, but no extension UI consumer was found |
| Multi-URL batch admission | CoreServices validates each URL/options, deduplicates within the batch, reserves unique output names, and returns per-input results; additive explicit-destination operation serves platform adapters | UniFFI and bulk Add UI use CoreServices admission with per-item destinations, media selection, checksum, public staging, and partial outcomes | Add dialog extracts explicit links from pasted text and queues them through Tauri; shared priority, package, and yt-dlp options | Core API only; no CLI command, FetchX remote JSON-RPC method, remote UI, or browser RPC method |
| Batch package grouping | CoreServices can place links in explicit packages or derive a package per source using the shared package-name helper | Bulk Add preserves Android's user-selected package directory; per-source package inference is not used by this adapter | Batch links group by source when the suggested value is unchanged; an edited package name groups them together | No shared-core batch adapter; browser extension's batch UI submits links individually |
| HTTP segmented transfer with validator-safe resume | Shared Rust range engine; stale segments are discarded if current resource validator is absent or mismatched | Same Rust engine via CoreServices | Same Rust engine via CoreServices | Same Rust engine for direct HTTP downloads |
| Manual queued-download ordering | Rust queue actor moves queued items up/down within their priority group; SQLite queue order is updated transactionally and restored on restart | UniFFI service methods and queued-row controls | Tauri commands, queue-order sort, and queued-row controls | CLI commands, authenticated `queue.moveUp` / `queue.moveDown`, remote UI controls |
| Interrupted Android transfer recovery | Shared state machine and queue recover interrupted work from persisted records | Rust startup recovery plus sticky foreground-service restart when the bridge is available | Existing Rust startup recovery; Android service policy does not apply | Existing Rust startup recovery |

Manual queue-ordering constraints:

- Only items in `Queued` state can move; active, paused, and terminal items are rejected.
- A move affects the relative order only among queued items with the same priority; priority remains the primary dispatch order.
- Boundary moves are idempotent and do not change the stored ordering.
- The browser extension/native-host RPC allowlist remains read-only and does not expose these queue mutations. The authenticated remote web UI does.
- Existing `queue_order` storage is reused; there is no schema migration.

Android background recovery:

- The foreground service uses `START_STICKY` when the Rust bridge is available, so process recreation can reconnect to the repository and Rust can requeue persisted interrupted work.
- This does not cover force-stop or boot-time autostart and has not been runtime-tested on a device.

Remote aria2 preflight:

- The desktop probe reuses `Aria2Client`; HTTPS is required for remote hosts, while HTTP is accepted only for an IP loopback address. Embedded credentials, query strings, fragments, redirects, and proxy use are rejected or disabled.
- A successful `aria2.getVersion` probe displays the daemon version and `enabledFeatures`, then reads `aria2.getGlobalStat` for active, waiting, and stopped-result counts plus download/upload bytes per second. If the statistics call fails or is unsupported, the daemon remains reported as reachable and the UI explicitly notes that statistics could not be read.
- Authentication and transport errors do not return the configured secret. Statistics are transient and are not persisted.
- This is connectivity/capability discovery only. It does not configure remote aria2 as the download engine or expose arbitrary endpoint testing through FetchX's authenticated remote API.
- The Tauri diagnostic response has an additive optional statistics field. Android, CLI, FetchX remote JSON-RPC, and browser/native-host surfaces are unchanged. No persistence or migration is involved.

Remote engine runtime status:

- `CoreServices::engine_runtime_statuses()` projects every engine as `available`, `disabled`, or `unavailable`, with an optional version. The authenticated `engine.runtimeStatus` RPC is advertised through capability discovery and is readable by Operator and ReadOnly roles.
- The remote UI requests the method only when the server advertises the capability and refreshes it with the existing polling cycle. Older servers continue to display the unavailable state.
- The response omits internal failure reasons, executable paths, and configuration. This read-only status does not add persistence, migration, or a new credential flow.
- The method is deliberately excluded from `BROWSER_RPC_METHODS`; it is a remote management view, not a browser page-submission operation.

Availability notes:

- “Available” for core protocols means implemented, not necessarily selectable without host configuration.
- Optional executables must report `Disabled`, `Available`, or `Unavailable(reason)` at runtime. UI lists must not assume installation.
- Archive extraction is not presently evidenced as a shared core capability despite the capability examples in Phase 0.
- WebDAV is not evidenced by a dedicated implementation; a WebDAV endpoint may currently use HTTP-compatible behavior only if validated separately.

## EngineKind values

`EngineKind` currently includes:

- `DirectHttp`
- `Aria2`
- `YtDlp`
- `Hls`
- `NativeTorrent`
- `Ftp`
- `Ftps`
- `Sftp`
- `Metalink`

Capabilities must validate that every reported engine kind is implemented, initialized, and permitted on the active host. Persisted unavailable records must remain recoverable without being dispatched as available.

## Required capability record

Use a stable record rather than UI booleans:

```rust
pub struct CoreCapabilityStatus {
    pub capability: CapabilityId,
    pub implementation: ImplementationAvailability,
    pub platform: PlatformAvailability,
    pub reason: Option<CapabilityReason>,
    pub config_key: Option<String>,
}
```

Unknown or unavailable capabilities are explicit values, never omitted as if supported.

## Runtime status projection

FetchX exposes lightweight runtime-only download state through shared Rust models and CoreServices. Android UniFFI, desktop Tauri/TypeScript, and remote JSON-RPC consume the same `id`, `status`, nested progress, and `updated_at` millisecond timestamp shape. The restricted browser RPC allowlist includes the read-only method. Removed downloads are absent because no record remains.

## HTTP segment recovery integrity

Direct HTTP uses the Rust downloader's parallel byte ranges, bounded adaptive connection scheduling, retries, pause/cancel handling, and persisted partial segment files. A fragment is reused on a later invocation only when the URL and non-empty server validator match the saved identity. With no current validator, segments are restarted to avoid assembling mixed server representations. This requires no schema migration or adapter-specific logic.

## Persisted duplicate lookup

`CoreServices::find_duplicate_download` compares a candidate URL with persisted download sources using the shared URL normalizer. It returns a privacy-limited match (ID, filename, status, and update time), or `None`. It neither blocks creation nor changes lifecycle state. Android UniFFI, desktop Tauri, local/remote CLI, remote JSON-RPC, and the browser RPC route expose this same Rust behavior. No migration is required.

## yt-dlp audio extraction

The shared `YtDlpAudioFormat` model allows only MP3 or M4A on a yt-dlp source and rejects combining audio conversion with a selected video format. `YtDlpEngine` selects `bestaudio/best`, runs extraction through the existing isolated process/workspace, validates the output suffix, and retains existing cancellation/error/finalization behavior. SQLite schema version 12 persists this optional mode. Android, desktop, and local CLI expose it; the authenticated remote API and browser extension/native host do not expose media-specific extraction. Android's Kotlin runtime supplies its bundled FFmpeg executable when extraction is requested; other hosts rely on their configured yt-dlp/FFmpeg runtime.

## Multi-URL batch admission

`CoreServices::enqueue_download_batch` admits explicit HTTP(S) URL requests independently and returns one result per input. Invalid URLs, duplicate normalized URLs within the batch, unavailable engines, invalid media options, and destination/queue failures are reported per item; they do not prevent later valid items from being admitted. Existing queue destinations and earlier destinations in the same batch are reserved, with a numbered suffix used when suggested names collide. If the operation is cancelled, already-enqueued items remain in the ordinary persisted queue lifecycle and work not yet admitted is not started.

`CoreServices::enqueue_download_batch_with_destinations` is additive to the existing batch APIs and lets platform adapters supply explicit absolute destinations, filenames, engine/media options, and optional checksums. Android's generated UniFFI request uses this operation: Kotlin retains destination and public-storage policy, stages eligible public targets before the call, and preserves each selected package path; Rust owns URL/format/checksum validation, within-batch duplicate rules, collision resolution, queue admission, and one result per input. Rust writes the established checksum sidecar before queue admission, so it exists before dispatch can begin. Completion events continue through Android's existing reconciliation and publication path. The original CoreServices batch APIs remain unchanged for desktop callers.

This is batch admission, not playlist or page-media discovery. FetchX's `extract_download_links` extracts explicit HTTP(S) links from supplied text; `MediaExtractor` extracts metadata/formats for one URL at a time. The desktop and Android Add dialogs consume batch admission. Local/remote CLI, authenticated JSON-RPC, remote UI, and browser RPC do not expose a batch operation. No schema migration or new dependency is required. Rust core and Android bridge unit tests and Android bridge/downloads host unit tests pass; no Android device validation was performed.

When desktop batch admission is invoked without an explicit package name, CoreServices derives each item's package using the shared `suggest_download_package_name` rule, so links with the same suggested source package share a folder. An explicitly edited package name remains a shared override. Existing single-download package suggestions and package sanitization are reused; the adapter does not implement grouping rules.

## Direct DASH manifest dispatch

`EngineDispatcher` recognizes valid HTTP(S) URLs whose path ends in `.mpd` (case-insensitive) and routes Direct HTTP sources to the existing yt-dlp engine. This complements the existing native HLS URL dispatch and `MediaProtocol::Dash` mapping; FetchX does not add a native DASH parser or engine. The configured yt-dlp runtime owns manifest extraction, representation selection, and download lifecycle. If yt-dlp is unavailable, dispatch returns the existing engine-unavailable error rather than saving the MPD as a generic HTTP file. The queue checks pause support against the effective source route and uses cancel-and-requeue at shutdown for this unpausable yt-dlp path. No model, migration, export/import, or adapter API change is required because Android, desktop, CLI, remote, and browser submissions already use CoreServices' shared dispatcher.

Detection is based on the URL path only; opaque manifest URLs without an `.mpd` suffix are not detected. Browser-captured request headers and cookies are not forwarded by this change. See `docs/research/XDM_FEATURE_ANALYSIS.md` for source review, tests, and runtime limitations.

## Pre-creation resource resolution

Prompt 16 verified Gopeed's separate resolve/create workflow against revision `8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b`. FetchX has specialized media-format discovery and local torrent-file inspection, while ordinary HTTP/FTP/SFTP/HLS/Metalink jobs generally discover metadata after admission and magnet file selection is post-admission. Batch admission and duplicate lookup do not inspect remote resources. There is no shared `resolve_download` API in Rust or the host adapters.

A common resolver would provide pre-add metadata and file selection without replacing existing direct-add behavior. Its protocol probes, reusable transient handles, security/lifetime policy, and Android/desktop/CLI/remote boundaries are too broad for a safe partial implementation in this phase. No API or runtime behavior was changed. The verified source contract, per-protocol comparison, proposed contract, acceptance criteria, and tests reviewed are in [`GOPEED_RESOLVE_TASK_ANALYSIS.md`](../research/GOPEED_RESOLVE_TASK_ANALYSIS.md). Gopeed's GPL-3.0 source was not copied.

## Cross-phase integration audit

The Prompt 13 target-by-target audit is recorded in [`CROSS_PHASE_INTEGRATION_AUDIT.md`](../research/CROSS_PHASE_INTEGRATION_AUDIT.md). It distinguishes a Rust/API implementation from a target that actually consumes it. In particular:

- Android now consumes persisted duplicate lookup as a pre-add warning; duplicate creation remains allowed after explicit confirmation.
- Persisted download-runtime-status wrappers remain exposed but have no direct Android, desktop, or remote UI consumer. Their fields duplicate the ordinary download records already consumed by those UIs; adding another fetch would not provide additional status information.
- Android, CLI, FetchX remote UI, and browser/native-host still do not call the shared Rust batch-admission API. Browser bulk submission issues individual requests.
- Per-source batch package grouping remains consumed only by the desktop batch UI.
- Engine runtime status is consumed by desktop settings and the authenticated remote UI; it is not exposed through the browser/native-host allowlist.
- Android sticky restart has host-side unit coverage only; real process-recreation behavior remains unverified.

At the Prompt 14 baseline, phase work was committed through `573c30b`; Prompt 13's duplicate-warning integration and Prompt 14's Android batch integration and documentation are uncommitted. No schema migration or new dependency was introduced. Android batch adapter/device behavior is host-tested only; browser-build, source-access, and desktop Clippy limitations remain documented in the audit and phase reports.

## aria2/native torrent migration status

| Capability | Current status |
|---|---|
| Torrent backend defaults and explicit fallback | New CoreServices magnet/local `.torrent` admissions default to native Rust. Desktop offers an explicit aria2 fallback; CLI retains `--engine aria2`; remote UI chooses native when selection RPCs are advertised and aria2 remains selectable. Android uses native only (aria2 is not bundled/selectable); browser/native host does not accept torrent inputs. Existing records keep their persisted engine. |
| Native local `.torrent` file selection | Confirmed indexes are revalidated against current metainfo (info hash and file bounds) before librqbit `only_files`; stale, unconfirmed, empty, and out-of-range selections are rejected. Local peer transfer verifies selected-file progress/completion; generated default destinations are distinct per download. |
| Native magnet metadata and file selection | Implemented in shared Rust: acquire metadata with librqbit, persist it, wait for an explicit non-empty selection, revalidate info hash/indexes, and transfer selected files. Android (native fallback), desktop (Tauri/UI), CLI (explicit native admission), and authenticated remote JSON-RPC can inspect and confirm. Remote metadata requires `DownloadsRead`; confirmation requires `DownloadsControl`. Calls are capability-gated for older servers and excluded from the restricted browser/native-host allowlist. |
| Other aria2 RPC/process/settings behavior | Retained for compatibility; no removal or migration of existing records. |

Prompt 19's deterministic, non-private synthetic magnet fixture acquired metadata from one manually supplied loopback peer, persisted an explicit file choice, and transferred only the selected file with trackers, DHT, and local discovery disabled. It exercised partial progress, pause, engine/session recreation and completion, seeder removal, cancellation/handle cleanup, seeder restoration, and retry to completion. The test does not prove exact piece reuse before resumed peer data, crash/process recovery, explicit destination-collision handling, real adapter runtimes, or peer-failure retry policy. Synthetic private-torrent magnet exchange is unsupported in this librqbit test setup. Prompt 20 adds authenticated remote metadata inspection and selection confirmation dispatch and a capability-gated UI; no live remote network/device validation was performed.

Prompt 21 makes native Rust the default for new shared-core magnet and local `.torrent` admissions. The default does not fall back silently if native is disabled/unavailable; aria2 remains an explicit option. Current remote UI selects native when both selection RPC capabilities are advertised, while older clients that omit the engine retain the remote server's legacy aria2-first route. Android uses native and has no aria2 chooser/runtime; browser/native-host torrent admission remains unsupported because its RPC input is HTTP(S)-only. Persisted download engine kinds and aria2 settings are unchanged. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for per-target defaults, exact validation, and limitations.

Prompt 21 host validation passed for serial Rust tests (including the controlled-loopback torrent lifecycle), remote UI tests/build, desktop UI build, Android downloads Gradle tests, formatting, Clippy, and diff checks. No device/emulator, packaged desktop, real aria2 daemon, public swarm, or live old/new remote-server interoperability run was performed. Exact commands and the two existing desktop Clippy warnings are recorded in the migration report.

## Prompt 22 — aria2 dependencies and protocol defaults

| Protocol / surface | Default route for new work | Remaining aria2 dependency |
|---|---|---|
| HTTP/HTTPS | Rust direct HTTP (plus existing HLS/DASH/yt-dlp resolution) | Explicit CLI/CoreServices selection only; remote HTTP admits `direct_http` only. |
| FTP/FTPS, SFTP | Shared Rust FTP/FTPS/SFTP engines where exposed by the host | No automatic aria2 route. |
| BitTorrent/magnet | Native Rust (`librqbit`) | Explicit fallback on supported surfaces; older remote clients omitting engine retain the aria2-first magnet behavior. |
| Metalink | Shared Rust Metalink engine | No automatic aria2 route. |
| Queue, retry, resume | Rust queue/scheduler and selected engine lifecycle | Existing `EngineKind::Aria2` records require aria2's dispatcher/RPC reconciliation; engine identity is not rewritten. |
| Diagnostics/configuration | Shared runtime status and host settings | Optional local aria2 supervisor/client, desktop Windows helper/settings, desktop and Android separate remote-daemon probes. No general aria2 RPC is exposed through FetchX remote/browser APIs. |

Prompt 22 found no non-torrent download default still routed through aria2 and no aria2 code proven dead. No implementation was removed or migrated: explicit fallback, aria2 GID persistence/recovery, and settings remain required. Android disables local aria2 bundling; legacy Android records persisted as aria2 remain unchanged but cannot execute locally without an aria2 runtime. Credentials/configuration are preserved and excluded from download export and sanitized diagnostics. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for the full dependency inventory, evidence, and removal gates.

## Prompt 23 — persisted aria2 job recovery

| State / capability | Shared Rust behavior | Android capability |
|---|---|---|
| Persisted `EngineKind::Aria2` | Remains the dispatch and retry identity. No transparent native fallback or engine rewrite. | Persisted and displayed; not locally executable because the bridge supplies no aria2 runtime. |
| Interrupted job recovery | Preparing/downloading/metadata-acquiring/recovering/seeding work is requeued and retains its persisted engine. aria2 reconciliation is required to continue it. | A requeued row can fail through the unavailable local engine; row/data are retained. |
| aria2 GID recovery | Torrent GIDs are persisted in torrent metadata and reconciled with aria2 RPC/info hash; generic GIDs are runtime-only. A GID is never a native identifier. | No local RPC recovery. Separate remote aria2 endpoint configuration does not dispatch these download records. |
| Partial-file continuation | Supervised aria2 resubmission with progress requires aria2's `.aria2` control file. Progress alone is not sufficient. Cross-engine partial adoption is unverified and unsupported. | No local aria2 resume path. Retry does not replace the persisted record when the engine is unavailable. |
| Completed/removed data | Completed output needs no engine recovery. Queue removal preserves output and partial files, though it removes the record. | Completed results remain; removed files are not implicitly imported as downloads. |
| User-visible unavailable state | Persisted failure diagnostics are sanitized; no structured recovery assessment currently identifies platform/runtime unavailability. | Failed rows show a sanitized reason and retry/remove controls, but the reason is generic and not Android-specific. |

No native adoption capability is claimed for any persisted aria2 state. The safe next slice is a read-only shared-core recovery assessment, an actionable UI explanation, and tests with a real supervised aria2 runtime before any opt-in conversion design. See the Prompt 23 recovery matrix and validation boundary in [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md).

## Prompt 24 — read-only persisted aria2 recovery assessment

| Surface | Capability and limitation |
|---|---|
| Shared Rust CoreServices | `assess_aria2_recovery(id)` returns a typed state, sanitized explanation, and next step. It checks only an already-running aria2 runtime and read-only filesystem evidence; it never changes the record, engine, GID, torrent selection, or files. |
| Android | UniFFI exposes the typed result. Each aria2 row offers a read-only recovery check and displays its explanation/next step. No local aria2 runtime is configured by default; the separate remote aria2 settings probe is not used for persisted-download dispatch. |
| Desktop | Tauri exposes the same assessment in download details. The UI labels the action read-only and preserves normal engine-bound controls. |
| CLI | Local `aria2-recovery <id>` calls CoreServices in an assessment-only startup mode: it does not start aria2 or dispatch queued work, and reports `unknown` when runtime evidence is not checked. `remote aria2-recovery <id>` uses the server operation after capability discovery. |
| FetchX remote server/client | Authenticated `download.aria2Recovery` delegates to CoreServices and returns assessment schema version 1. It is discovered under the unchanged `fetchx.rpc.v1` protocol, requires `downloads.read`, and is capability-gated by the CLI for older servers. It reports only the remote server's evidence; it does not infer state from a separate aria2 endpoint profile. |
| Browser/native-host | Unsupported. `download.aria2Recovery` remains excluded from the restricted browser RPC allowlist. |
| Evidence boundary | States distinguish runtime unavailability, queued work ready to start, available/stale/missing torrent GID or metadata, sidecar presence/absence, completed/not-aria2 rows, and unknown. Task availability or sidecar presence does not prove that continuation will succeed. |

Five focused downloader tests, the CoreServices data/file-preservation test, Android bridge DTO mapping, and the Prompt 25 protocol/server/CLI suites passed. Rust test modules type-check; the desktop UI production build and Android bridge/downloads unit tests pass (the Android run required constrained Rust build settings for UniFFI generation). No real aria2 daemon, Android device, or process-restart recovery was validated. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for exact commands and limitations.

Prompt 25 extended only the read-only assessment boundary: remote and local/remote CLI clients now consume the same CoreServices state. Older remote servers omit the new capability and are not sent the new RPC by CLI. Existing remote clients and protocol v1 methods remain compatible.

## Prompt 26 — native parity and remaining aria2 dependencies

| Dependency / capability | Current implementation | Native coverage and evidence limit |
|---|---|---|
| Runtime and persisted engine | `Aria2Engine` + `Aria2Supervisor` remain for explicit selections and persisted `EngineKind::Aria2` jobs; torrent GIDs and `.aria2` control files remain aria2-owned. | Rust protocol engines cover new defaults; no GID/sidecar compatibility or safe cross-engine partial adoption. |
| Desktop support | Native torrent is Add UI default; managed aria2 helper/settings, engine status and explicit Add fallback remain. Default Tauri resources do not bundle aria2; the managed aria2 package reviewed is Windows x64. | Keep until explicit users, old rows, and helper/settings product policy have a tested exit path. |
| Android support | Native torrent is default. Core bridge config passes no local aria2 path; runtime extraction is disabled. The separate remote aria2 screen is an external-server profile. A local `aria2c` asset is ignored/untracked and not wired to bridge runtime. | Legacy aria2 records are not locally executable by default. The ignored asset is not reproducible release evidence. |
| CLI / remote | CLI defaults torrent admissions to native and supports explicit aria2/recovery. Current remote UI selects native when the server advertises metadata and selection; old calls omitting an engine retain aria2-first server behavior. Authenticated FetchX remote supports explicit admission plus `download.aria2Recovery`, not raw aria2 management RPC. | Browser/native-host does not expose torrent or aria2 management. |

| Priority | Parity area | Evidence status |
|---|---|---|
| P0 | Native BitTorrent restart, crash recovery, exact partial-piece reuse | Prompt 28 now tests full CoreServices/QueueManager startup recovery in child processes on Windows: worker A admits via CoreServices, is killed, and worker B only starts CoreServices; the recovered item appears active in QueueManager and completes with selected-file SHA-1 and persisted identity invariants intact. Aggregate peer bytes suggest partial reuse. Exact piece/block reuse and Android device restart remain unverified. |
| P0 | Existing aria2 job recovery | Same-engine assessment/reconciliation is preserved; no native adoption. Needs real aria2 daemon tests for GID/sidecar/restart. |
| P1 | HTTP/HTTPS multi-source and segmentation | DirectHTTP has bounded ordered failover; digest-bearing jobs may use cross-source ranges only after every mirror passes a same-total one-byte range probe. Segment ranges are disjoint, retries are source-aware, and final SHA-256 gates atomic no-replace publication. |
| P1 | FTP/FTPS/SFTP, retry, pause/resume, scheduler, bandwidth/connection limits | Shared Rust paths and controls exist; exact aria2 option/protocol behavior and interoperability are not exhaustive. |
| P1/P2 | Integrity, file allocation, Metalink versions/mirrors, HLS/DASH, remote diagnostics | Native features exist by subsets (Metalink hashes, BitTorrent piece verification, HLS, yt-dlp for DASH); allocation policy and global aria2 management are not fully replaced. |

Prompt 28's Windows test passed four focused runs and once in the full CoreServices suite with a file-backed SQLite database, fixed private multi-file metainfo, one loopback tracker and seeder, and selected index 1. The first child admitted through CoreServices and was killed after persisted progress; the second child only called CoreServices startup and confirmed the recovered row was active in QueueManager before completion. It verified selected output bytes/SHA-1 and engine, info hash, destination, selection, and GID invariants. Two repeated runs measured 3,391,488 and 3,096,576 peer-uploaded bytes after restart for a 4,194,304-byte selected file; this is aggregate evidence consistent with some data reuse, not exact piece/block proof. The private torrent disables DHT and local service discovery in librqbit; its only tracker and all peers are loopback. The standalone Prompt 27 test remains engine-layer evidence and should not be conflated with this queue test. Android device process death, live aria2, public trackers and packaged binaries remain untested. `librqbit` session persistence remains disabled, and no production recovery code or engine runtime behavior needed changing. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for the harness and remaining removal gates. Aria2 sources were reviewed at a pinned upstream commit only as behavioral requirements; no code was copied or translated. GPL-2.0-or-later source headers and the limited OpenSSL exception require a separate licensing review before any upstream-derived code or binary redistribution.

### Prompt 29 — legacy aria2 HTTP restart

Not supported. Persisted URL and destination fields do not prove source freshness, job provenance, or absence of external aria2 credentials/options. The desktop and Android clients do not offer a restart-with-native action. The safe next step is a user-confirmed current-source flow and a separately selected collision-free destination, with admission through CoreServices; do not infer eligibility from engine, scheme, and status alone.

### Prompt 30 — user-confirmed native HTTP restart

Supported in desktop and Android for unfinished persisted aria2 HTTP(S) records only. The user enters/confirms the current URL, chooses a separate exact destination, and affirms it is a direct file rather than media/streaming, torrent, or Metalink work. CoreServices validates the persisted row and URL, refuses known media extensions, checks database and on-disk collisions for the destination and `.aria2`/`.part` companions, and verifies destination-directory write access. It creates a new native `DirectHttp` item with its own ID and never mutates the aria2 item or reuses legacy bytes. Desktop accepts a user-entered absolute path; Android restricts paths to its app-specific downloads directory. Content behind an extensionless URL cannot be identified without a network request; user confirmation is required but cannot prove server content. No schema migration was added. CoreServices preservation/eligibility tests, Android bridge/downloads unit suites, Android app Kotlin compilation, desktop UI build, and desktop Tauri `cargo check` passed on Windows. Device and live aria2/server tests remain outstanding; existing aria2 runtime, daemon, GID/sidecar, parity, rollback, and packaging/license gates remain.

### Prompt 31 — user-confirmed native torrent restart

Supported in shared CoreServices, desktop, and Android for unfinished persisted aria2 magnet or local-torrent records when a user supplies and confirms a current 40-character-hex magnet matching the saved info hash. The current `.torrent` file cannot be supplied in this phase; base32 hashes and incomplete/inconsistent legacy metadata are rejected. The user must choose a new, non-existing destination directory under FetchX downloads. CoreServices rejects overlap with old or other persisted outputs and aria2 sidecar/partial paths; platform adapters keep destination selection within their supported roots. Admission creates a distinct native magnet record with no copied file indexes. The standard native metadata/file-selection UI requires an explicit fresh selection before transfer. Legacy row, GID, selection, files, and sidecar remain unchanged. No schema migration was added.

Windows validation: CoreServices 43 tests, Android-bridge Rust 26 tests, strict CoreServices/Android-bridge Clippy, formatting, desktop UI build, and desktop Tauri `cargo check` passed. Android Gradle unit tests/app compile did not run because UniFFI generation's Rust compilation failed with host allocation/paging-file exhaustion. No device, live tracker/seeder, or live aria2 daemon test ran. This is hash-matched new admission and preservation evidence only—not source availability, selection equivalence, or piece-reuse proof. All aria2-removal gates remain open.

### Prompt 32 — current `.torrent` files and base32 v1 magnets

Supported in shared CoreServices, desktop, and Android for unfinished persisted aria2 torrent jobs. Users may confirm a current v1 magnet using hex or base32 `btih`, or choose a current local `.torrent` file. Hashes normalize to lowercase 40-character hex and are compared with the persisted aria2 info hash; base32 magnets are passed to the native engine as an equivalent hex `btih`. `btmh`, malformed, duplicate, v2-only, and hybrid/mixed identifiers are rejected. The local metainfo parser verifies v1 identity from the original bencoded `info` bytes.

`.torrent` input requires a fresh inspection and explicit file selection, plus a separate non-existing destination directory. Desktop uses an inspected local path and selection checkboxes; Android stages a document-picker selection privately and presents the same file list. CoreServices verifies the source hash, snapshots and rechecks the input, rejects destination collisions with legacy/other output and sidecar/partial paths, and creates a separate native record. The aria2 row, GID, selection, metadata, destination, output, partial bytes, and `.aria2` sidecar remain unchanged. No schema migration was added.

Validation on Windows: model (40), CoreServices (45), and Android-bridge Rust (26) tests passed; Android bridge/downloads unit tests and app Kotlin compile passed; desktop UI build and Tauri `cargo check` passed; Rust formatting and `git diff --check` passed; strict CoreServices/Android-bridge Clippy passed. Full strict Clippy including desktop reports two unrelated existing warnings (`collapsible_if` in `app.rs:301` and `too_many_arguments` in existing `commands.rs:1114`); desktop Clippy passes with those two lint classes allowed. No Android device, live tracker/seeder, or aria2 daemon testing is claimed. Aria2 runtime, daemon/RPC, GID/sidecar recovery, protocol parity, Android runtime validation, exact piece-reuse evidence, rollback/user disposition, and packaging/license review remain required before aria2 removal. The aria2 source and documentation are behavioral references only; no source code was copied or translated.

### Prompt 33 — verified native protocol behavior

| Protocol area | Status from local evidence | Remaining limitation |
|---|---|---|
| HTTP(S) ranges, segments, retries, resume, redirects | Implemented and hermetically tested, including range rejection, validator changes, 416 handling, and private-address redirect rejection. | Public redirect is ignored; no exhaustive proxy/server parity or generic multi-source URI list. |
| FTP | Download, REST resume, cancellation, and retry classification tested against local fixtures. | No end-to-end authenticated interruption/retry fixture; passive mode only. |
| FTPS | Explicit TLS and authentication have hermetic local fixture coverage for certificate validation, transfer, resume, cancellation, integrity, and timeout. | Implicit FTPS, external-server interoperability, and full aria2 parity are not established. |
| SFTP | In-process russh/russh-sftp fixtures verify auth, host-key mismatch rejection, transfer, offset resume, and bounded retry after transient interruption. Retries are capped at 4 total attempts / 120 seconds with 250 ms, 500 ms, then 1 s backoff (+ up to 249 ms jitter; shared cap 8 s); a 30 s inactivity timeout detects stalled SSH sessions. Cancellation and pause interrupt backoff. | Permanent auth, host-key, malformed path, permission, integrity/metadata, local I/O, and destination errors are not retried. Every retry rechecks remote size/mtime and byte-compares the partial prefix. External-server interoperability is unverified; same-size mutations hidden by unavailable/coarse mtimes during transfer cannot be universally detected. |
| BitTorrent | Local-peer metadata/selection/download and Windows CoreServices queue-recovery tests pass. | No exact block-reuse proof, Android device recovery, live tracker/swarm, or independent corrupt-piece test. |
| Metalink | v3/v4 parsing, hash functions, checksum success/mismatch, mirror failover, multi-file and cancellation fixtures pass. | Only FetchX's subset; checksum-mismatch failover and exhaustive aria2 compatibility remain unverified. |

The Windows downloader suite passed (299 tests; 3 ignored) with strict downloader Clippy and formatting. The focused CoreServices recovery test passed with `--lib`; an initial wider test invocation failed compiling unrelated integration targets due rustc reporting metadata stubs for `std`/`core`. No platform adapter changed. These results do not close aria2-removal gates: retain runtime/RPC, settings/packaging, persisted GID/sidecar recovery, explicit legacy-job disposition, and require SFTP/FTPS interoperability plus Android runtime and exact-piece-reuse evidence.

### Prompt 34 — external runtime removal

The current runtime has no aria2 executable, RPC, supervisor, helper/package, engine setting UI, or fallback. New and resumed native work uses shared Rust engines. Legacy `EngineKind::Aria2` rows are parsed for compatibility, excluded from queue dispatch, shown unavailable if unfinished, and retained without changes. The HTTP(S) and torrent native-restart flows remain opt-in, create separate records, and preserve legacy GIDs, metadata, destinations, output, partial bytes, and `.aria2` sidecars. Remote UI declines to create a magnet when the server does not advertise native metadata/selection support.

Native behavior is not described as full aria2 parity. HTTP(S), native BitTorrent, and Metalink have tested subsets; FTPS explicit TLS and SFTP have bounded hermetic wire-level tests but broader interoperability remains unverified; DASH continues through yt-dlp. Native failures do not fall back to aria2. The full Windows `cargo test --workspace` suite, Android host tests/app compilation, desktop and remote UI builds/tests, desktop Tauri `cargo check`, formatting, and `git diff --check` passed in the previous runtime-removal validation. No Android device, live daemon, tracker/seeder, or public protocol tests were performed.

### Prompt 35 — SFTP behavior and evidence

The shared Rust SFTP engine verifies an existing partial file byte-for-byte against the current remote prefix before resuming, rejects a shorter remote object, requires the final byte count to match the initial size, and re-stats size/modification time before publication. Local fixture coverage includes successful and rejected password authentication, public-key authentication, changed host keys, transfer, resume, remote content changes between interruption and retry, in-transfer metadata changes, cancellation, explicit retry, and destination/partial preservation.

`cargo test -p fetchx-downloader sftp_ -- --nocapture` passed (31 tests). The full downloader suite passed (252 library tests, 2 ignored; 2 lifecycle integration tests; nested process-recovery test passed); strict downloader Clippy, formatting, and `git diff --check` passed. Aria2’s `src/SftpNegotiationCommand.cc`, `src/SSHSession.cc`, `src/SftpDownloadCommand.cc`, and `src/SftpFinishDownloadCommand.cc` were reviewed at commit `9e7273583f83e881e3ec067b523ba88724088d2f` as behavioral references. No aria2 source was copied or ported; its GPL-2.0-or-later notices were noted. No platform adapters changed, no external server/device run was performed, and external aria2 remains removed.

Resume is not a content snapshot protocol: same-size mutations during an active transfer that do not alter available/coarse mtime cannot be universally detected. Automatic retries handle only transient transport failures; external server interoperability remains unverified.

### Prompt 36 — FTPS behavior and evidence

The shared Rust engine now trusts optional configured DER private roots in addition to WebPKI roots, labels failures during the FTPS secure-session path as TLS errors, and checks the final byte count against FTP `SIZE` before publishing. Local explicit-FTPS fixture coverage includes trusted/untrusted certificates, credentials, successful transfer, REST resume, cancellation, preservation of existing files, size mismatch, and handshake timeout. TLS failures do not downgrade to plaintext. FTP remains passive-mode only; implicit FTPS and external-server interoperability are unverified.

Aria2 commit `9e7273583f83e881e3ec067b523ba88724088d2f` was reviewed at `src/FtpNegotiationCommand.cc` and `src/FtpDownloadCommand.cc` as a behavioral reference. No source was reused or ported. The retry-state fix reloads the persisted item before retry attempts to avoid an invalid `Downloading`-to-`Downloading` transition. `cargo test -p fetchx-downloader ftps_ -- --nocapture` passed (10 tests); the full downloader suite passed (259 library tests, 2 ignored; 2 lifecycle integration tests); strict Clippy, formatting, and `git diff --check` passed. No platform adapter API changed, and external aria2 remains removed.

### Prompt 37 — HTTP multi-source contract and evidence

CoreServices exposes ordered HTTP(S) source sets of 2–16 distinct URLs with an optional expected SHA-256. A single queue row and primary source URL are preserved; ordered alternatives are stored in `.fetchx.sources`. DirectHTTP attempts one source at a time and uses its existing range segmentation/retry behavior. It never writes ranges from different URLs into one output.

An optional SHA-256 verifies final content before publication and enables safe failover from a complete but wrong representation. Without a checksum, the first source that passes normal HTTP validation is authoritative; equal size, weak/strong ETag text, and Last-Modified do not establish cross-source identity. Source-bound partials resume only for the same URL. On source change, the partial and known segment files are discarded. Cancellation preserves the active source's partial; source metadata remains until successful publication.

Hermetic tests cover active range failure and segmented failover, exact segment coverage, same-size/different-content matching ETag rejection by SHA-256, inconsistent range totals, ignored ranges, one-source fallback, cancellation/resume, exhaustion, and collision preservation. CoreServices validates source count and duplicate rejection. Aria2 commit `9e7273583f83e881e3ec067b523ba88724088d2f` review covered `src/SegmentMan.cc` (`checkoutSegment`, `getSegment`, `getSegmentWithIndex`, `cancelSegmentInternal`, `completeSegment`), `src/SegmentMan.h`, `src/InorderURISelector.cc` (`select`), `src/RequestGroup.cc`, and `src/HttpDownloadCommand.cc` (`prepareForNextSegment`). No source was reused or ported; FetchX's deterministic segment planner and ordered failover are independent Rust code. External aria2 remains removed. Cross-source parallel ranges and broad remote interoperability remain unverified.

## Prompt 38 — checksum-gated mirror range scheduling

Cross-source ranges require a valid caller-supplied SHA-256 and a successful range probe from every source: `206`, exact one-byte range `bytes 0-0/<known-total>`, a one-byte body, and the same known total across all sources. Probes use the shared HTTP client and redirect/private-address policy. If any probe disagrees or does not support ranges, the engine starts no cross-source work and uses ordered whole-source failover. Validators and equal lengths are not identity proofs.

The shared scheduler assigns distinct fixed ranges round-robin to validated mirrors. Failed ranges discard their partial segment and rotate through other sources; whole-file SHA-256 is checked before atomic no-replace hard-link publication. The version-1 bounded resume identity binds segment artifacts to exact sources, checksum, total length, and layout. A mismatch removes assembled/segment artifacts; hard-link publication failure does not fall back to a copy. Tests cover two-mirror range coverage, conflicting totals, same-size/different bytes, active-range reassignment, cancellation/resume, exhaustion, and collision preservation.

Aria2 commit `9e7273583f83e881e3ec067b523ba88724088d2f` references were `src/SegmentMan.cc` (`checkoutSegment`, `getSegment`, `cancelSegmentInternal`, `completeSegment`), `src/SegmentMan.h`, and `src/InorderURISelector.cc` (`select`). `SegmentMan` is coupled to aria2 `PieceStorage`, CUID/peer ownership, and block-cache lifecycle, so direct transplantation is not suitable for FetchX's fixed HTTP ranges. No source was reused or ported; FetchX independently adapts disjoint ownership, ordered source assignment, and failed-range reassignment. External aria2 remains removed. Server interoperability beyond loopback fixtures, Android/device behavior, and hard-link support on other filesystems remain unverified.
### Prompt 40 — Metalink chunk checksums

The shared downloader supports v3/v4 Metalink chunk checksums using MD5, SHA-1, SHA-256, or SHA-512. Piece lengths, digest format, hash-count bounds, and declared file-size boundaries are validated. Each piece must match its manifest digest before it is retained; missing pieces may be recovered from later ordered mirrors. The assembled output is rechecked piece-by-piece and against any whole-file hash before publication. Failed integrity checks do not replace an existing destination, and manifests without chunk hashes are unchanged. Temporary verified pieces do not survive cancellation or process restart.

Coverage is hermetic loopback only. The full downloader suite passed (284 tests, 2 ignored; 2 lifecycle integration tests passed), as did strict downloader Clippy, formatting, and `git diff --check`; the desktop consumer check passed. The workspace check is blocked by an unrelated existing Android-bridge match omission for two `BatchDownloadError` variants. DirectHTTP still performs whole-file requests per mirror; this is safe but not piece-specific range fetching. Aria2 parser/checksum implementation files were reviewed at commit `9e7273583f83e881e3ec067b523ba88724088d2f`; no source was reused. No aria2 runtime, fallback, or platform-specific scheduler was added.

### Prompt 41 — Metalink piece-range retrieval

For Metalink files with piece hashes, DirectHTTP now requires an exact `206` probe and exact per-piece `Content-Range`, known consistent total size, identity encoding, and the expected response byte count before a range is usable. It requests only missing pieces; each is streamed to an attempt file and retained after all declared piece hashes pass. Valid pieces are rechecked and survive mirror switches, cancellation, and restart. Unsupported/malformed range sources are skipped; if none can safely serve pieces, existing whole-file verification is used as fallback. The assembled file passes piece and whole-file hashes before atomic no-replace publication. No-checksum documents are unchanged.

Loopback fixtures cover bad/short/oversized/wrong-offset ranges, inconsistent totals, ignored ranges and whole-file fallback, mirror switching with only missing-piece requests, verified-piece resume after cancellation, whole-file mismatch, and destination preservation. Aria2's indexed offset, final short piece, and per-piece validation behavior were adapted from the pinned `IteratableChunkChecksumValidator.cc`; no aria2 source was copied. Full external protocol and device coverage remains unverified.

Windows validation passed: 22 focused Metalink tests, 286 downloader tests (2 ignored), 2 lifecycle integration tests, strict downloader Clippy, formatting, diff check, and desktop `cargo check`. Prompt 42 subsequently fixed the Android-bridge mappings for `InvalidSourceSet` and `InvalidMultiSourceUrl`; `cargo check --workspace` now passes.

### Prompt 42 — Android batch error mapping

The Android bridge converts `InvalidSourceSet` and `InvalidMultiSourceUrl` into existing stable DTO codes (`InvalidFormat` and `InvalidUrl`). The DTO carries no raw message or input values, preventing URLs, credentials, paths, and server text from leaking. Focused mapping tests and bridge/CoreServices/downloader suites pass; `cargo check --workspace`, strict Clippy, formatting, and diff checks pass. No CoreServices behavior or platform API changed. Android device validation remains unperformed, and external aria2 remains removed.

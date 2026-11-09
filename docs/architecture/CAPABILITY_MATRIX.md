# FetchX Capability Matrix

## Reporting model

Capabilities have two independent states:

- **implementation availability**: the Rust engine or adapter can run;
- **platform availability**: current OS, permissions, executable runtime, and bundled resources support it.

`engine_status.rs` is the runtime-status source for aria2, yt-dlp, native torrent, FTP, SFTP, and Metalink. `EngineRuntimeStatus::entries()` projects all nine `EngineKind` values, including built-in Direct HTTP/HLS and shared FTP/FTPS status. Capability reporting should continue to extend this shared projection rather than infer status from UI lists.

## Repository-evidenced capabilities

| Capability | Rust implementation | Android | Desktop | CLI/server |
|---|---|---|---|---|
| HTTP/HTTPS segmented download | Available | Available | Available | Available |
| FTP/FTPS | Available in downloader | Available, conditional on binding/configuration | Available, conditional on binding/configuration | Available |
| SFTP | Available in downloader with russh | Available, conditional on host policy | Available, conditional on host policy | Available |
| HLS native segments | Available | Available through shared dispatcher/runtime | Available through shared dispatcher/runtime | Available |
| Direct DASH MPD dispatch | Direct HTTP(S) `.mpd` URLs route to the configured yt-dlp engine; no native DASH engine | Shared dispatcher; requires the configured yt-dlp runtime | Shared dispatcher; requires the configured yt-dlp runtime | Shared dispatcher for CLI/server/browser-submitted URLs; requires yt-dlp |
| Metalink | Available | Core implementation; advertise from runtime status | Core implementation; advertise from runtime status | Available |
| Native BitTorrent/torrent | Available | Available; Android selects native fallback | Available; desktop native-torrent route | Core implementation; host availability as configured |
| aria2 engine adapter | Shared `Aria2Client`/`Aria2Engine`; `CoreServices` starts a configured local supervisor | Available only through the host's configured runtime; no remote endpoint UI in this phase | Local engine settings plus a non-persistent HTTPS remote RPC probe | Local configured engine; CLI remote commands target FetchX, not aria2 |
| Remote aria2 endpoint preflight | Shared Rust JSON-RPC validation and `getVersion` / `getGlobalStat` parsing | Client code exists, but no UniFFI probe operation or Android UI | Test endpoint without saving it; show version, `enabledFeatures`, activity counts, and transfer rates | No CLI, FetchX remote RPC, remote UI, or browser-native-host probe |
| yt-dlp | Available with process runner | Android runtime adapter supplies implementation | Optional executable/helper adapter | Optional executable/configuration |
| yt-dlp audio extraction (MP3/M4A) | Typed source option and shared argument/lifecycle handling | Browser picker via UniFFI; bundled FFmpeg is passed for extraction | Tauri Add dialog supports MP3/M4A | Local CLI `--audio-format`; remote API/UI and browser extension/native host do not expose media-specific selection |
| FFmpeg/FFprobe | Available process orchestration | Android bundles statically linked FFmpeg runtime | Optional helper executable | Optional executable/configuration |
| Media format extraction | Available in media-extractor | Android runtime adapter | Shared Rust plus process adapter | Shared Rust plus process adapter |
| Pre-creation resource resolution | No general resolver; media extraction and local torrent inspection are specialized operations | UniFFI exposes media formats and local `.torrent` inspection; magnet selection follows metadata acquisition | Tauri exposes media formats and local `.torrent` inspection; magnet selection follows metadata acquisition | CLI supports local torrent selection and post-admission magnet selection; FetchX remote has no resolve operation; browser capture routes URLs without resource inspection |
| Queue/scheduler/recovery | Available | Available | Available | Available |
| SQLite persistence | Available | App-private database | OS app-data database | Caller-selected database |
| Remote JSON-RPC | Available server transport | Android browser-RPC transport exists | Desktop IPC should remain separate unless exposed | Available HTTP/WebSocket |
| Engine runtime status | CoreServices projection of all nine engine kinds | No engine-status operation or UI consumer | Local settings consume Tauri engine status; remote engine status is a separate API | Authenticated `engine.runtimeStatus` is consumed by the remote UI; not exposed to the CLI or browser/native-host allowlist |
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
| Explicit torrent backend choice | CoreServices validates aria2/native-torrent selection; desktop, CLI, and remote magnet paths pass explicit choices. Legacy/default behavior remains aria2-first where previously used. |
| Native local `.torrent` file selection | Confirmed persisted indexes are passed to librqbit `only_files`; metadata lookup is filtered by download ID. |
| Native magnet file selection | Not implemented; native metadata selection remains a follow-up. |
| Other aria2 RPC/process/settings behavior | Retained for compatibility; no removal or migration of existing records. |

This migration has Rust unit and integration test coverage but no device, packaged-desktop, swarm, or live-daemon runtime proof. Detailed platform status and acceptance work are in [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md).

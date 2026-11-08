# FetchX Capability Matrix

## Reporting model

Capabilities have two independent states:

- **implementation availability**: the Rust engine or adapter can run;
- **platform availability**: current OS, permissions, executable runtime, and bundled resources support it.

`engine_status.rs` is the existing runtime-status source for aria2, yt-dlp, native torrent, FTP, SFTP, and Metalink. Capability reporting should be unified and extended rather than inferred from UI lists or `EngineKind`.

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
| Remote aria2 endpoint preflight | Shared Rust JSON-RPC validation and `getVersion` parsing | Client code exists, but no UniFFI probe operation or Android UI | Test endpoint without saving it; show version and `enabledFeatures` | No CLI, FetchX remote RPC, remote UI, or browser-native-host probe |
| yt-dlp | Available with process runner | Android runtime adapter supplies implementation | Optional executable/helper adapter | Optional executable/configuration |
| yt-dlp audio extraction (MP3/M4A) | Typed source option and shared argument/lifecycle handling | Browser picker via UniFFI; bundled FFmpeg is passed for extraction | Tauri Add dialog supports MP3/M4A | Local CLI `--audio-format`; remote API/UI and browser extension/native host do not expose media-specific selection |
| FFmpeg/FFprobe | Available process orchestration | Android bundles statically linked FFmpeg runtime | Optional helper executable | Optional executable/configuration |
| Media format extraction | Available in media-extractor | Android runtime adapter | Shared Rust plus process adapter | Shared Rust plus process adapter |
| Queue/scheduler/recovery | Available | Available | Available | Available |
| SQLite persistence | Available | App-private database | OS app-data database | Caller-selected database |
| Remote JSON-RPC | Available server transport | Android browser-RPC transport exists | Desktop IPC should remain separate unless exposed | Available HTTP/WebSocket |
| Background execution | Platform adapter responsibility | Foreground service/Android lifecycle | OS application lifecycle | Long-running process |
| Browser integration | Protocol/native host/modules | GeckoView/browser module | Desktop browser IPC/native host | Browser extension client via server |
| Plugins | Policy/API crates; Android service has no execution adapter | Policy evaluation only, no execution capability | Depends on future/host adapter | Policy/manager if wired |
| Normalized-URL duplicate lookup | CoreServices repository-backed query | UniFFI `find_duplicate_download` | Tauri command and TypeScript API | CLI and `download.findDuplicate` JSON-RPC; browser RPC allowlisted |
| Multi-URL batch admission | CoreServices validates each URL/options, deduplicates within the batch, reserves unique output names, and returns per-input results | Not exposed through UniFFI or Android UI | Add dialog extracts explicit links from pasted text and queues them through Tauri; shared priority, package, and yt-dlp options | Core API only; no CLI command, FetchX remote JSON-RPC method, remote UI, or browser RPC method |
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
- A successful `aria2.getVersion` probe displays the daemon version and `enabledFeatures`; authentication and transport errors do not return the configured secret.
- This is connectivity/capability discovery only. It does not configure remote aria2 as the download engine or expose arbitrary endpoint testing through FetchX's authenticated remote API.
- No persistence or migration is involved.

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

This is batch admission, not playlist or page-media discovery. FetchX's `extract_download_links` extracts explicit HTTP(S) links from supplied text; `MediaExtractor` extracts metadata/formats for one URL at a time. The desktop Add dialog is the only currently wired user-facing batch adapter. Android UniFFI, local/remote CLI, authenticated JSON-RPC, remote UI, and browser RPC do not expose a batch operation yet. No schema migration or new dependency is required.

## Direct DASH manifest dispatch

`EngineDispatcher` recognizes valid HTTP(S) URLs whose path ends in `.mpd` (case-insensitive) and routes Direct HTTP sources to the existing yt-dlp engine. This complements the existing native HLS URL dispatch and `MediaProtocol::Dash` mapping; FetchX does not add a native DASH parser or engine. The configured yt-dlp runtime owns manifest extraction, representation selection, and download lifecycle. If yt-dlp is unavailable, dispatch returns the existing engine-unavailable error rather than saving the MPD as a generic HTTP file. The queue checks pause support against the effective source route and uses cancel-and-requeue at shutdown for this unpausable yt-dlp path. No model, migration, export/import, or adapter API change is required because Android, desktop, CLI, remote, and browser submissions already use CoreServices' shared dispatcher.

Detection is based on the URL path only; opaque manifest URLs without an `.mpd` suffix are not detected. Browser-captured request headers and cookies are not forwarded by this change. See `docs/research/XDM_FEATURE_ANALYSIS.md` for source review, tests, and runtime limitations.

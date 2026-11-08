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
| Metalink | Available | Core implementation; advertise from runtime status | Core implementation; advertise from runtime status | Available |
| Native BitTorrent/torrent | Available | Available; Android selects native fallback | Available; desktop native-torrent route | Core implementation; host availability as configured |
| aria2 engine adapter | Shared `Aria2Client`/`Aria2Engine`; `CoreServices` starts a configured local supervisor | Available only through the host's configured runtime; no remote endpoint UI in this phase | Local engine settings plus a non-persistent HTTPS remote RPC probe | Local configured engine; CLI remote commands target FetchX, not aria2 |
| Remote aria2 endpoint preflight | Shared Rust JSON-RPC validation and `getVersion` parsing | Client code exists, but no UniFFI probe operation or Android UI | Test endpoint without saving it; show version and `enabledFeatures` | No CLI, FetchX remote RPC, remote UI, or browser-native-host probe |
| yt-dlp | Available with process runner | Android runtime adapter supplies implementation | Optional executable/helper adapter | Optional executable/configuration |
| FFmpeg/FFprobe | Available process orchestration | Android bundles statically linked FFmpeg runtime | Optional helper executable | Optional executable/configuration |
| Media format extraction | Available in media-extractor | Android runtime adapter | Shared Rust plus process adapter | Shared Rust plus process adapter |
| Queue/scheduler/recovery | Available | Available | Available | Available |
| SQLite persistence | Available | App-private database | OS app-data database | Caller-selected database |
| Remote JSON-RPC | Available server transport | Android browser-RPC transport exists | Desktop IPC should remain separate unless exposed | Available HTTP/WebSocket |
| Background execution | Platform adapter responsibility | Foreground service/Android lifecycle | OS application lifecycle | Long-running process |
| Browser integration | Protocol/native host/modules | GeckoView/browser module | Desktop browser IPC/native host | Browser extension client via server |
| Plugins | Policy/API crates; Android service has no execution adapter | Policy evaluation only, no execution capability | Depends on future/host adapter | Policy/manager if wired |
| Normalized-URL duplicate lookup | CoreServices repository-backed query | UniFFI `find_duplicate_download` | Tauri command and TypeScript API | CLI and `download.findDuplicate` JSON-RPC; browser RPC allowlisted |
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

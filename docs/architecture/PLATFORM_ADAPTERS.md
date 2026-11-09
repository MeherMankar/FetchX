# FetchX Platform Adapters

## Adapter contract

A platform adapter is a trust boundary, filesystem/lifecycle integration, and presentation transport. It must not implement download-engine logic or queue decisions.

## Android

Evidence: `apps/android`, Android Gradle modules, UniFFI Rust library, API 26 minimum, target SDK 35.

Responsibilities:

- Android permissions and SAF/public-media publication;
- foreground service and notification lifecycle;
- WorkManager/foreground scheduling if added;
- connectivity and data-saver policy;
- app-private database and runtime directories;
- Android yt-dlp runtime supplied to the Rust engine;
- native sharing and file URI handling;
- Kotlin UI and module repository adapters.

Shared-core consumer:

```text
Kotlin UI -> Kotlin DownloadRepository -> UniFFI DownloadService -> CoreServices
```

Android-specific risks:

- Public-download reconciliation belongs in an Android storage adapter and must validate metadata published by core.
- Kotlin fallback parsing should call Rust only and should not diverge from desktop behavior.
- Android foreground service state must not duplicate queue state.
- Background restart must restore from SQLite through `CoreServices`.

## Desktop

Evidence: one `apps/desktop/src-tauri` Tauri 2 application, React UI, and `tauri.conf.json` bundle targets for NSIS/MSI, deb/AppImage, and DMG.

Responsibilities:

- Tauri OS path resolution and native window lifecycle;
- native dialogs and notifications;
- desktop engine-helper downloads and executable installation;
- OS-specific permission and security policy;
- browser IPC/native extension integration;
- desktop database/runtime/download directories.

Shared-core consumer:

```text
React -> desktop API wrappers -> Tauri commands -> CoreServices
                                              -> Rust event bus -> Tauri events
```

Windows, Linux, and macOS use the same desktop Rust adapter and React UI. Platform helpers may differ; engines and lifecycle do not.

## CLI/headless

The CLI constructs `CoreServices` through the same bootstrap used by long-running hosts.

Responsibilities:

- argument parsing;
- terminal output;
- short-lived maintenance/database commands;
- host shutdown;
- executable discovery for explicitly configured engines.

Maintenance commands must not start incompatible live runtime services while operating on a database.

## Remote server and web UI

The remote server constructs shared services and exposes only JSON-RPC operations advertised by discovery. It owns authentication, role authorization, TLS, CORS, WebSocket ticket exchange, and listener binding.

The remote UI owns presentation and protocol rendering only. It must remain a protocol client and not contain SQLite, engine, queue, or filesystem logic. Native magnet metadata and file confirmation use capability-gated `download.torrentMetadata` and `download.selectTorrentFiles` calls. Read-only roles can inspect metadata; operator/admin roles can confirm a selection. The new mutation is intentionally excluded from the restricted browser/native-host RPC allowlist.

## Browser extension/native host

Responsibilities:

- capture browser URLs and media/page context;
- enforce browser extension permissions;
- route requests to the native host;
- present browser-native UI;
- Android GeckoView integration where applicable.

Download content validation, naming, queueing, and engine selection remain in Rust. Browser extension and Android browser modules may supply metadata and contexts but must not make transfer decisions.

## Future adapter rule

A new application may be added only by proving:

1. It uses the shared models and `CoreServices` operations.
2. Its generated DTO mapping is contract-tested.
3. It handles OS permission/background APIs only.
4. It does not reimplement URL/category/name, engine, queue, schedule, retry, state-machine, or persistence logic.
5. It reports capabilities only when the core runtime and platform adapter can support them.

## Torrent engine selection during aria2 migration

Desktop, CLI, and remote adapters may request the native torrent backend through the existing Rust CoreServices boundary; adapters do not implement torrent transfer logic. Explicit requests are validated in CoreServices, while legacy/default selection remains unchanged for compatibility. Android's existing native fallback is unchanged. No target may remove aria2 setup, settings, or persisted records until native magnet selection, restart/recovery, and the applicable runtime acceptance tests are complete. Current target status and limitations are documented in [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md).

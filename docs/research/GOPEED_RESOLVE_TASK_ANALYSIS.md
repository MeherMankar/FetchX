# Gopeed `resolve_task` and FetchX Resolution Gap

## Scope and conclusion

This review compares Gopeed's pre-creation resolution flow with FetchX's existing protocol engines, metadata extractors, file-selection APIs, and add paths. It is source inspection only: no Gopeed code was copied, no Gopeed tests were run, and no FetchX runtime/network behavior is claimed.

**Conclusion:** FetchX has a real, bounded gap in general pre-creation resource inspection and file selection. It already has useful specialized flows for yt-dlp media formats and torrent metadata, but they are not a common resolver and do not cover the rest of the supported protocols. A shared `resolve_download` capability would add value, particularly for HTTP metadata, Metalink file sets, HLS variants, and magnet metadata before admission. This phase does **not** implement it: safely composing those distinct network protocols, selection semantics, transient handles, security policy, and all host boundaries is not a single narrow change. Implementing only HTTP or adding an adapter-only shim would leave the contract partial.

## Gopeed source and license

- Repository: [`GopeedLab/gopeed`](https://github.com/GopeedLab/gopeed).
- Pinned revision: [`8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b`](https://github.com/GopeedLab/gopeed/commit/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b), the `main` revision observed on 2026-11-06.
- License: [`LICENSE`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/LICENSE) is GNU GPL version 3. No code was copied; this report records behavior and FetchX's proposed independent design only.

Relevant source paths at that revision:

- [`README.md`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/README.md) lists `resolve_task` separately from `create_task`.
- [`pkg/mcpserver/server.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/mcpserver/server.go) declares MCP request/output schemas and the resolve/create tools.
- [`pkg/api/service.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/api/service.go) implements REST resolution and creation handlers.
- [`pkg/rest/api.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/rest/api.go) registers `/api/v1/resolve` and `/api/v1/tasks`.
- [`pkg/rest/model/task.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/rest/model/task.go) defines the Resolve and Create request payloads.
- [`pkg/base/model.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/base/model.go) defines `Resource`, `FileInfo`, `Request`, and `Options`.
- [`pkg/download/downloader.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/download/downloader.go) implements `Downloader.Resolve` and `CreateWithOptions`.
- Built-in protocol resolver implementations inspected: [`internal/protocol/http/fetcher.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/internal/protocol/http/fetcher.go), [`bt/fetcher.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/internal/protocol/bt/fetcher.go), [`ftp/fetcher.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/internal/protocol/ftp/fetcher.go), [`hls/fetcher.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/internal/protocol/hls/fetcher.go), and [`ed2k/fetcher.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/internal/protocol/ed2k/fetcher.go).
- Contract tests inspected: [`pkg/api/service_test.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/api/service_test.go) and [`pkg/download/downloader_test.go`](https://github.com/GopeedLab/gopeed/blob/8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b/pkg/download/downloader_test.go).

## Verified Gopeed contract

### Resolve and create are separate operations

1. REST `POST /api/v1/resolve` accepts a request (`req`) and invokes `Downloader.Resolve`.
2. Resolve first runs the `onResolve` extension hook. If the hook supplies a resource with files, Gopeed returns that resource and suggested category directory directly.
3. Otherwise, Gopeed selects a built-in fetcher by URL, invokes its protocol-specific `Resolve`, and returns a generated resource ID (`id`/`rid`), resource (`res`), and suggested download directory. The resolved fetcher is kept in an in-memory cache under that ID.
4. `POST /api/v1/tasks` accepts either the resolved `rid` or a direct `req`, plus options. Creating from a `rid` consumes the cached resolver state; invalid file indexes are rejected. The existing direct-create path remains available.
5. Errors from request decoding, protocol selection/resolution, or task creation are returned through the REST error response. An unsupported protocol therefore does not produce a successful metadata-shaped fallback.

The cached fetcher is useful for retaining protocol state (including an HTTP response body for one-time URLs) between inspection and creation, but the ID is process-local, not a durable handle. A failed create returns the fetcher to the cache; a successfully persisted task consumes it. This behavior creates explicit lifecycle, expiry, resource-bound, and restart questions for any analogous FetchX design.

### Metadata, files, and options

`base.Resource` contains a resource name, total size, range support, files, and hash. Each `FileInfo` carries a name, relative path, size, optional creation time, and optional per-file request. The schema does not expose a general MIME/type field. Resolve returns the suggested file names through `res.files`; it does not return a separate generic supported-options list.

The create options include output name/path, default-path/category controls, and zero-based `selectFiles` indexes. The MCP schema also exposes protocol/task-specific options: HTTP request method/headers/body and HTTP connection/checksum/auto-torrent/auto-extract settings, or BitTorrent trackers. Archive extraction and archive password are post-download task options, not archive-member inspection/selection.

### Built-in protocol behavior observed

| Gopeed built-in | Resolve behavior verified in source |
|---|---|
| HTTP/HTTPS | Performs a normal request (without `Range`) to obtain response metadata; parses range support and size and retains the response for reuse where possible. The HTTP fetcher has protocol-specific request options. |
| BitTorrent/magnet | Adds the torrent and updates resource metadata before returning; resource files support selection. Trackers are a protocol-specific request option. |
| FTP/FTPS/FTPES | Connects/logs in and resolves a remote target into resource metadata; the FTP resolver has files and selected-index support. |
| HLS | Fetches and parses the playlist during Resolve and builds a single assembled output resource from its segment plan. |
| ED2K | Parses the link locally and builds a single-file resource from it. |

These are built-in protocol managers in the inspected source tree, not a claim that every Gopeed extension has the same resolver contract. SFTP, DASH, Metalink, and aria2 are not among those built-in protocol directories; extension behavior is separate. HTTP and torrent are the cases with direct end-to-end resolution contract tests reviewed here; the tests do not establish exhaustive failure behavior across every protocol. `TestResolveThenCreateWithDownloadOptions` verifies an HTTP response's filename/size and that a resolved ID can be created with a chosen output path/name and selected index. `TestDownloader_Resolve` verifies HTTP size, filename, and range support.

## FetchX comparison

FetchX uses the Rust shared core for download execution and has separate APIs for media extraction, torrent metadata, batch admission, and duplicate lookup. There is no general `CoreServices::resolve_download` or resolved-resource handle. Current behavior by area:

| Area | Existing FetchX behavior | Assessment against pre-creation `resolve_task` |
|---|---|---|
| HTTP and HTTPS | Direct HTTP downloads negotiate response metadata, range support, and size during engine execution after queue admission; the add path suggests a filename from the URL. No shared metadata-only preflight returns the final name/type/size first. | **Missing** general preflight; transfer support is implemented. |
| FTP and FTPS | Shared FTP engine handles file transfers and retries after a persisted queue item is dispatched. There is no FTP listing/metadata inspection or multi-file selection API before admission. | **Missing** preflight and directory/file selection. |
| SFTP | Shared SFTP engine performs host-key checks and file transfer after dispatch. No shared preflight metadata or file-list selection API exists. | **Missing** preflight; transfer support is implemented. |
| HLS | `HlsEngine` fetches and parses manifests in the download job after queue admission, follows master playlists, and selects the highest-bandwidth variant automatically. It does not expose variants/files for pre-add selection. | **Partial** protocol parsing exists, but not a user-facing preflight/selection flow. |
| DASH | Direct HTTP(S) `.mpd` URLs route to yt-dlp; no native DASH resolver/engine exists. `MediaExtractor` can expose media formats for supported extractable URLs, but this is not a common DASH-resource resolver. | **Partial** via yt-dlp; no generic manifest inspection or selection contract. |
| yt-dlp media | `CoreServices::extract_media` returns title, duration, thumbnail, extractor and formats before task creation; adapters can select a format ID or MP3/M4A audio mode. | **Adequate for its media-format use case**, but specialized and not a general protocol resolver. |
| aria2 | Ordinary URI tasks enter the queue before aria2 work. For a magnet, metadata is acquired asynchronously after the task is created. | **Partial**; task creation precedes metadata/file selection. |
| Torrent | `.torrent` files can be inspected before queue admission, then selected by zero-based file index. Magnet tasks are admitted first; metadata and file selection are exposed after metadata acquisition. | **Partial**; local torrent inspection is useful, magnet resolution is post-creation. |
| Metalink | The shared engine fetches and parses the manifest while a queued job is preparing, then processes its files/mirrors. The parser is not exposed as a pre-creation inspection API and there is no file-selection UI contract. | **Partial** parser/transfer exists; preflight and selective admission are missing. |
| Batch admission | CoreServices validates and independently admits each input, resolving within-batch duplicates and destination collisions. It does not resolve remote resource metadata before admission. | **Partial** admission, not resource resolution. |
| Duplicate lookup | CoreServices performs advisory normalized-URL lookup against persisted downloads. It does not inspect the remote resource or create a resolution handle. | **Unsuitable as a substitute** for resource resolution. |

### Host boundaries

- **Android:** UniFFI exposes media-format discovery and local torrent-file inspection/selection. Magnet metadata is handled after adding the task. The Kotlin UI is an adapter; there is no general resolve operation.
- **Desktop:** Tauri exposes the same media-format and local torrent inspection paths. Magnet selection follows metadata acquisition. No general resolve operation exists.
- **CLI:** supports selected indexes when adding a local `.torrent` file and post-admission `torrent files/select` commands for magnet metadata; it has no general protocol resolver or media-format discovery command.
- **Remote server/UI:** `download.create` admits ordinary URLs or magnet links; no protocol metadata/file-inspection resolve operation is exposed. Remote clients therefore cannot use the local UI's media/torrent inspection path.
- **Browser/native host:** browser media interception recognizes/captures URL contexts and routes downloads; it does not inspect remote resource metadata or present a shared file-selection result. Browser requests use normal shared-core download submission.

## Decision and proposed contract

A common resolver is meaningful because it would allow clients to inspect supported resources and choose from actual files/options before creating a persisted download, instead of duplicating protocol-specific discovery in each application. The most concrete missing use cases are Metalink file selection and HLS/remote torrent inspection; HTTP/FTP/SFTP metadata preflight would also improve name/size presentation.

Do not implement it as a generic opaque stringly typed request or a UI-only helper. A future design should start with a typed `CoreServices::resolve_download` request/result and explicitly specify:

- protocol/engine identity and resolution capabilities;
- safe suggested name, optional size/content type/range metadata, and relative file entries with stable selection indexes;
- typed protocol-specific options, while reusing existing media and torrent models rather than flattening them into one generic metadata bag;
- an opaque, bounded-lifetime resolution handle only where the protocol needs state reuse (e.g. one-time HTTP response or acquired torrent metadata), plus a create-from-resolution operation; existing direct create APIs must continue to work;
- no queue/storage/lifecycle mutation during resolution, and explicit handling for unsupported protocols, network/authentication errors, cancellation, timeouts, stale handles, and invalid selections;
- redaction of credentials, cookies, private URLs, temporary paths, and raw engine diagnostics; existing URL/private-address, redirect, resource-size, and metadata bounds must apply to probes too;
- runtime/capability discovery so a client can fall back to existing add flows when the server does not advertise resolution.

### Acceptance criteria before implementation

1. Core tests cover supported/unsupported schemes, malformed input, probe/network/auth failures, timeout/cancellation, redaction, and prove that resolve alone creates no persisted item or queue work.
2. Protocol tests cover HTTP redirects/headers/filename/unknown size and one-time response handling; FTP/FTPS and SFTP metadata under their authentication/host-key policies; HLS master/media playlists; yt-dlp format compatibility; Metalink multi-file/mirror selection; local torrent file and magnet metadata; and aria2 where supported. Unsupported adapters must return typed unsupported results rather than success-shaped defaults.
3. File indexes are validated against the returned resource; selection and estimates match the actual eventual create behavior. Resolved handles have bounded capacity/expiry, cannot be reused incorrectly, and are released on error, cancellation, create, and shutdown.
4. Existing direct add, batch admission, duplicate warning, engine routing, and destination behavior remain unchanged when resolution is unused.
5. Adapter coverage includes UniFFI/Kotlin, desktop Tauri/UI, CLI, remote capability negotiation/older-server fallback, and browser/native-host only where product UX uses the operation. Tests distinguish host compilation/unit tests from device, external-server, and real-network validation.

No public API, adapter, migration, dependency, or runtime behavior changed in this research phase.

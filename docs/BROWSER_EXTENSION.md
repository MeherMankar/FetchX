# Browser Extension Integration (Phase 17)

## Architecture

```
Browser (Chrome / Edge)
        │
        │  Native Messaging (stdin/stdout, 4-byte LE length + JSON)
        ▼
apps/browser/native-host   (fetchx-native-host binary)
        │
        │  Loopback TCP  127.0.0.1:<random port>
        │  Session token (per-desktop-startup UUID)
        ▼
apps/desktop  (FetchX desktop application)
        │
        │  Existing application path (same as UI add_download)
        ▼
crates/queue  →  EngineDispatcher  →  Download engine
```

The browser extension is a **capture-only** layer.  It submits a URL; all
download logic (engine selection, filename derivation, queueing, retries,
persistence) remains in the Rust core.

## Components

| Path | Description |
|------|-------------|
| `crates/browser-protocol/` | Shared Rust crate: protocol types, URL validation, native-messaging framing |
| `apps/browser/native-host/` | Rust binary: reads stdin, connects to desktop, writes stdout |
| `apps/browser/extension/` | TypeScript Manifest V3 extension |

## Protocol

Version: `1`

### Request

```json
{
  "version": 1,
  "type": "add_download",
  "url": "https://example.com/file.zip",
  "request_id": "optional-uuid-for-deduplication"
}
```

Supported types: `add_download`, `ping`

### Success response

```json
{ "version": 1, "ok": true, "download_id": "uuid" }
```

### Error response

```json
{
  "version": 1,
  "ok": false,
  "error": { "code": "unsupported_scheme", "message": "Only HTTP and HTTPS URLs are supported." }
}
```

### Error codes

| Code | Meaning |
|------|---------|
| `invalid_request` | Malformed JSON or unknown fields |
| `unsupported_version` | Protocol version mismatch — update FetchX |
| `unsupported_message` | Unknown message type |
| `invalid_url` | URL is syntactically invalid |
| `credentials_in_url` | URL contains `user:pass@` |
| `unsupported_scheme` | Not `http://` or `https://` |
| `desktop_unavailable` | FetchX is not running or not responding |
| `download_rejected` | Desktop queue or engine rejected the download |
| `internal_error` | Unexpected server-side error |

## Native Messaging framing

```
┌──────────────────────┬───────────────────────────────┐
│  4 bytes (LE uint32) │  N bytes UTF-8 JSON             │
│  message length = N  │  { "version": 1, "type": … }   │
└───────────────────────┴───────────────────────────────┘
```

Maximum payload: **64 KiB**.  Larger frames are rejected before allocation.

## Desktop IPC

The desktop application exposes a **loopback-only** TCP server on a random OS-assigned
port.  At startup it writes connection details to a user-private state file:

```json
{ "port": 54321, "token": "per-session-uuid", "app_version": "0.1.0" }
```

State file locations:

| Platform | Path |
|----------|------|
| Windows  | `%LOCALAPPDATA%\com.fetchx.desktop\browser-ipc.json` |
| Linux    | `$XDG_RUNTIME_DIR/fetchx/browser-ipc.json` |
| macOS    | `~/Library/Application Support/com.fetchx.desktop/browser-ipc.json` |

The native host reads this file to learn the port and token.  The file is
removed on desktop shutdown.

## Security model

- The IPC server binds to `127.0.0.1` only — never `0.0.0.0`.
- A per-session UUID token is required on every request.  It is compared in
  constant time to prevent timing attacks.
- URL validation is performed **twice**: lightweight in the extension/host for
  fast UX, authoritatively in the desktop application.
- Only `http://` and `https://` URLs are accepted.  Credentials in URLs are rejected.
- No cookies, session data, authentication tokens, or request headers are forwarded.
- Response messages never contain filesystem paths, process arguments, RPC secrets,
  SQL errors, or stack traces.
- No shell execution anywhere in the extension, host, or desktop integration.
- Extension permissions: `contextMenus`, `nativeMessaging` only.

## Development setup

See [`apps/browser/extension/README.md`](../apps/browser/extension/README.md) and
[`apps/browser/native-host/README.md`](../apps/browser/native-host/README.md).

## Known limitations (Phase 17)

- Firefox is not tested.  The native messaging API is compatible but the manifest
  format differs slightly; Firefox support is deferred.
- Automatic download interception (capturing every browser download) is not
  implemented.  The user must explicitly right-click or use the popup.
- Engine selection (aria2/yt-dlp) is not exposed through the extension.  All
  browser-initiated downloads use Direct HTTP.  The user can change the engine in
  the desktop UI after the download is created.
- Production packaging (installer, signed extension) is deferred to Phase 22.


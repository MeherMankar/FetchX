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

The browser extension is a browser adapter. It submits an HTTP(S) URL and, when
available, a suggested filename; authoritative validation, filename
sanitization, queueing, retries, and persistence remain in the Rust core.

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
- The extension already has an optional browser-download interception setting.
  Phase 21I does not introduce or expand interception or download-engine behavior;
  broader browser download handling remains outside this integration phase.
- Engine selection (aria2/yt-dlp) is not exposed through the extension.  All
  browser-initiated downloads use Direct HTTP.  The user can change the engine in
  the desktop UI after the download is created.
- Production packaging (installer, signed extension) is deferred to Phase 22.

## Phase 21I — Browser remote integration

On Windows, the extension selects either the local desktop or a configured
remote FetchX server. Requests use the shared `fetchx.rpc.v1` protocol. Local
requests go through native messaging, the desktop's authenticated loopback IPC,
and CoreServices. Remote requests go through the native host to an HTTPS server;
the host validates discovery/capabilities and currently forwards only
`system.discover`, `download.create`, and `download.list`.

Remote bearer credentials are stored by the Windows native host in Windows
Credential Manager, associated with both the target reference and endpoint.
They are not stored in extension local storage. Pairing codes, device discovery,
refresh tokens, and device/session management are not provided.

On Android, the FetchX app bundles a trusted GeckoView extension. It dispatches
the supported local browser RPC operations through the in-process Android bridge
to CoreServices. The built-in extension declares GeckoView's `geckoViewAddons`
permission, required for its native-message delegate. The browser menu's
**FetchX Remote** entry opens the bundled extension's actual GeckoView
browser-action popup, which supports connection discovery, listing downloads,
and submitting an HTTP(S) URL. This is same-device, in-app browser integration;
Android does not expose an HTTP/WebSocket/LAN listener for external browser
extensions.
External Chrome/Firefox-to-Android connectivity and Android remote-target
selection are not implemented.

## Phase 22A — Browser download interception

**Status: COMPLETE — implemented, tested, and live validated on Windows and
Android.** Chrome interception acceptance and explicit rejection/fallback were
observed live on Windows; the Android GeckoView handoff, completed download,
and relaunch persistence were also observed live.

The Windows extension can intercept eligible HTTP(S) downloads when
`autoIntercept` is enabled. It pauses the browser item before submitting
`download.create`; it cancels the browser item only after a correlated success
response, resumes it after an explicit rejection, and leaves it paused with a
notification when the result is ambiguous. Chrome/Edge report a download after
it starts, so the extension cannot guarantee that no bytes transfer before the
pause takes effect.

On Android, GeckoView external responses are queued for a user confirmation.
Acceptance submits `download.create` through the Android bridge; cancellation
cannot resume the response because GeckoView has already reported that it
cannot handle it. Each response is presented in order rather than replacing an
earlier pending response.

The Android handoff was live-validated on the `Pixel_10_Pro` emulator with an
`httpbin.org/response-headers` response. The accepted request completed as a
136-byte `direct_http` download; its SHA-256 was
`7a13f3e34ecf508e1c5ed12e26a37c4546b34cbfef7ed0f5d307e8640bcb0549`.
The app database record and destination file persisted after force-stop and
relaunch.

Both adapters submit only the HTTP(S) URL and a suggested filename. The shared
Rust path validates and sanitizes the filename and enqueues the request through
CoreServices. Browser adapters reject magnet and other non-HTTP schemes. Cookies,
credentials, referrers, and other request headers are not forwarded.
On Windows, Chrome 156 live validation confirmed the browser item was paused,
then canceled only after FetchX accepted the request. The completed
`direct_http` item contained 174/174 bytes and had SHA-256
`d42cbf89145094b020e1804e58ef80b769038f13937251193179062c50a15426`, matching
the fixture byte-for-byte; the database record and file persisted after a
graceful desktop relaunch. An explicit remote-target rejection resumed Chrome's
download, which completed at 168 bytes without creating a FetchX record.
Chrome initially reports an empty filename at `downloads.onCreated`; the
extension now waits for the resolved filename before submitting the RPC.

The browser MIME value is not forwarded: the Direct HTTP engine obtains
`Content-Type` from its own response, and Android uses response content type
only when deriving a suggested filename. This does not prevent correct engine
selection or response metadata handling. Referrer forwarding is intentionally
excluded because it can expose browsing context and requires an explicit
user-consent policy. Cookies, credentials, and other request headers are also
not forwarded.

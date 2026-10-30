# FetchX Browser Extension

A minimal Manifest V3 browser extension that lets you send HTTP/HTTPS links
from Chrome or Microsoft Edge to the FetchX desktop application.

## Features

- **Context menu**: right-click any link → "Send to FetchX"
- **Popup**: paste a URL and click "Send to FetchX"
- Select the local FetchX desktop or a configured remote FetchX server
- Displays connection status for the selected target
- Existing optional download interception and page-resource scanning features

Remote target support only adds explicit target selection and RPC forwarding. It
does not add or change the extension's existing interception or page-scanning
behavior.

The extension does not download files itself, access cookies or browser
passwords, or bypass DRM, CAPTCHA, or access controls.

Downloads are submitted through FetchX's shared `fetchx.rpc.v1` protocol. The
selected local desktop or remote FetchX server owns download processing.

## Prerequisites

- FetchX desktop application must be **running**.
- The [native messaging host](../native-host/README.md) must be installed.
- Remote targets require a compatible FetchX server and a bearer token. Remote
  tokens are stored by the Windows native host in Windows Credential Manager;
  they are not saved in extension storage.

## Targets

The popup's **Settings → FetchX target** selector chooses between **This PC**
and manually configured remote server origins. Remote origins must use HTTPS,
and must not contain a path, query, fragment, or embedded credentials. Adding a
remote target sends the bearer token to the native host once for storage in
Windows Credential Manager. Later RPC calls send only the target ID and
endpoint; the native host retrieves the endpoint-bound token and sends it as a
bearer credential to the configured server.

The native host validates the remote server's `fetchx.rpc.v1` protocol and
allows only `system.discover`, `download.create`, and `download.list` for this
browser integration. Other RPC methods are not exposed through the extension.
Credential-backed remote targets are currently supported by the Windows host.

The Android GeckoView integration is separate: its built-in extension dispatches
local `system.discover`, `download.create`, and `download.list` through an
in-process bridge. It does not open a LAN listener or expose remote target
configuration.

## Building

```sh
cd apps/browser/extension
npm install
npm run build
```

Build output goes to `dist/`.

## Loading as an unpacked extension (development)

1. Open `chrome://extensions/` in Chrome (or `edge://extensions/` in Edge).
2. Enable **Developer mode** (top right).
3. Click **Load unpacked**.
4. Select the `apps/browser/extension/` directory (the one containing `manifest.json`).
5. Note the extension ID shown on the card — you will need it when installing the native host.

## Protocol

The extension communicates with FetchX via Native Messaging.  Each
request is a JSON object framed with a 4-byte little-endian length prefix:

```json
{
  "version": 1,
  "type": "add_download",
  "url": "https://example.com/file.zip",
  "request_id": "optional-uuid"
}
```

Successful response:

```json
{
  "version": 1,
  "ok": true,
  "download_id": "...",
  "request_id": "optional-uuid"
}
```

Error response:

```json
{
  "version": 1,
  "ok": false,
  "error": {
    "code": "desktop_unavailable",
    "message": "FetchX is not running."
  }
}
```

## Supported browsers

| Browser | Status |
|---------|--------|
| Google Chrome | ✅ Supported |
| Microsoft Edge | ✅ Supported (Chromium) |
| Firefox | ⚠  Not tested in Phase 17 |

## Permissions used

| Permission | Reason |
|-----------|--------|
| `contextMenus` | "Send to FetchX" right-click menu |
| `nativeMessaging` | Communicate with the native host |
| `downloads` | Existing optional automatic download interception |
| `tabs` | Prefill the popup from the active tab |
| `storage` | Save extension settings and non-secret target details |
| `notifications` | Report submission/interception results |

The existing page scanner uses a content script and `<all_urls>` host access.
The extension does not request cookie, history, or webRequest permissions.

## Security

- Only `http://` and `https://` URLs are forwarded.
- URLs containing embedded credentials are rejected client-side and server-side.
- No cookies, session data, or browser passwords are accessed or forwarded.
- Remote bearer tokens are entered by the user, passed directly to the native
  host for OS-backed storage, and never written to extension storage.
- The native host validates the loopback session token for local desktop requests.
- The desktop application or remote server performs authoritative URL validation
  regardless of what the extension sends.

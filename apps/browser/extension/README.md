# FetchX Browser Extension

A browser extension that sends HTTP/HTTPS links from Chrome, Microsoft Edge,
or Firefox to the FetchX desktop application. Chrome and Edge use Manifest V3;
Firefox uses its separate Manifest V2 package.

## Features

- **Context menu**: right-click any link → "Send to FetchX"
- **Popup**: paste a URL and click "Send to FetchX"
- Select the local FetchX desktop or a configured remote FetchX server
- Displays connection status for the selected target
- Fetch and queue individual yt-dlp video formats for the local **This PC** target
- Existing optional download interception and page-resource scanning features

Remote target support only adds explicit target selection and RPC forwarding.
Firefox uses the same browser-interception and native messaging behavior via a
small browser API adapter; download logic remains shared.

The extension does not download files itself, access cookies or browser
passwords, or bypass DRM, CAPTCHA, or access controls.

Downloads are submitted through FetchX's shared `fetchx.rpc.v1` protocol. The
selected local desktop or remote FetchX server owns download processing.

## Prerequisites

- FetchX desktop application must be **running**.
- The [native messaging host](../native-host/README.md) must be installed.
- yt-dlp must be available in FetchX to fetch video qualities.
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

For the local **This PC** target, click **Fetch video qualities** to list
supported yt-dlp formats, then choose a format or **Download all**. This option
is not available for remote targets.

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

Build output goes to `dist-chromium/` and `dist-firefox/`.

## Loading as an unpacked extension (development)

1. In FetchX Settings → Browser extensions, click **Open install page** next to Chrome or Edge.
2. Enable **Developer mode** (top right).
3. Click **Load unpacked**.
4. Select the Chromium extension folder shown in FetchX Settings.
5. Restart the browser after the native host is registered.

## Loading in Firefox (development)

1. In FetchX Settings → Browser extensions, click **Open install page** next to Firefox.
2. Click **Load Temporary Add-on** and select `manifest.json` from the Firefox
   extension folder shown in FetchX Settings.
3. Keep in mind that Firefox removes temporary add-ons when it exits. Permanent
   installation requires a signed add-on published through Firefox Add-ons.

Firefox uses the fixed extension ID `FetchX@FetchX.app` and a separate native
host manifest/registry entry from Chrome and Edge. To remove its registration,
run `.\install-firefox-windows.ps1 -Uninstall`.

FetchX does not yet have browser-store listings. Until it is published, Chromium
extensions must be loaded unpacked and Firefox extensions can only be loaded
temporarily; the desktop installer does not claim that a registry entry installs
the add-on.

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
| Firefox | ✅ Supported (Firefox 115+, Manifest V2) |

Firefox 159 was live-validated on Windows for successful native-host handoff,
download completion, file integrity, and persistence across FetchX relaunch.
When an intercepted Firefox download must fall back, the origin must support
HTTP range requests for Firefox to resume it; otherwise FetchX reports that the
browser download could not be resumed.

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

# MyDownloader Browser Extension

A minimal Manifest V3 browser extension that lets you send HTTP/HTTPS links
from Chrome or Microsoft Edge to the MyDownloader desktop application.

## Features

- **Context menu**: right-click any link → "Send to MyDownloader"
- **Popup**: paste a URL and click "Send to MyDownloader"
- Displays connection status (connected / not running)

## What the extension does NOT do

The extension is intentionally restricted:

- It does not download files itself.
- It does not intercept browser downloads automatically.
- It does not access cookies, passwords, or authentication tokens.
- It does not track browsing history.
- It does not inject scripts into web pages.
- It does not bypass DRM, CAPTCHA, or access controls.

All downloading is handled by the MyDownloader desktop application.

## Prerequisites

- MyDownloader desktop application must be **running**.
- The [native messaging host](../native-host/README.md) must be installed.

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

The extension communicates with MyDownloader via Native Messaging.  Each
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
    "message": "MyDownloader is not running."
  }
}
```

## Supported browsers

| Browser | Status |
|---------|--------|
| Google Chrome | ✅ Supported |
| Microsoft Edge | ✅ Supported (Chromium) |
| Firefox | ⚠ Not tested in Phase 17 |

## Permissions used

| Permission | Reason |
|-----------|--------|
| `contextMenus` | "Send to MyDownloader" right-click menu |
| `nativeMessaging` | Communicate with the native host |

No host permissions, cookies, history, tabs (except for popup URL pre-fill),
or webRequest are requested.

## Security

- Only `http://` and `https://` URLs are forwarded.
- URLs containing embedded credentials are rejected client-side and server-side.
- No cookies, session data, or authentication headers are ever accessed or forwarded.
- The native host validates the session token on every request.
- The desktop application performs authoritative URL validation regardless of what
  the extension sends.

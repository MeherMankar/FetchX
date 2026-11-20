# FetchX Browser Extension — Privacy Policy

**Last updated: 2025**

## What data the extension collects

The FetchX browser extension does not collect, store, or transmit any personal data to external servers.

## What the extension does

- **Download interception**: The extension monitors browser download events and, when configured to do so, redirects them to the locally running FetchX desktop application via Native Messaging.
- **Page resource scanning**: When you click "Scan this page", the extension reads the current tab's DOM and network requests to list downloadable resources. This data is shown only in the popup and is never sent anywhere except the local FetchX desktop app when you explicitly add a download.
- **Media detection**: The extension passively monitors network requests on visited pages to detect HLS/DASH streams and video or audio files. Detected streams are stored temporarily in memory, scoped to the browser tab, and are discarded when the tab closes or navigates.
- **Context menus**: Right-clicking a link and choosing "Download link with FetchX" sends that URL to the locally running FetchX desktop application.
- **Remote targets**: If you configure a remote FetchX instance, the extension stores the server URL in `chrome.storage.local`. Authentication tokens are stored in the operating system credential store via Native Messaging — they are never stored in extension storage or sent to any third party.
- **Settings**: User preferences (auto-intercept toggle, minimum file size, notifications toggle, intercept rules) are stored locally in `chrome.storage.local`.

## What the extension does NOT do

- Does not send any data to the extension developer or any third-party server.
- Does not track browsing history.
- Does not use analytics or telemetry.
- Does not modify page content.
- Does not collect personal information.

## Permissions used

| Permission | Reason |
|---|---|
| `nativeMessaging` | Communicate with the locally installed FetchX desktop application |
| `downloads` | Detect and intercept browser-initiated downloads |
| `tabs` | Read the current tab URL when you click "Use tab" or scan a page |
| `storage` | Save settings and target configuration locally |
| `webRequest` | Detect media streams (HLS, DASH, video, audio) in network traffic |
| `contextMenus` | Add "Download with FetchX" to the right-click menu |
| `notifications` | Show a notification when a download is intercepted (optional, can be disabled) |
| `cookies` | Forward cookies for the intercepted URL to FetchX so the download succeeds |
| `scripting` | Inject the page-scanning content script (Chromium MV3) |
| `*://*/*` | Allow the content script and webRequest listener to run on any HTTP/HTTPS page |

## Contact

For questions about this privacy policy, open an issue in the FetchX repository.

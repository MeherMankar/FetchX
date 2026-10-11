# Android Extension Architecture

> **Phase 20 extension design document.** Describes the WebExtension support
> architecture for MyDownloader Android, based on GeckoView's
> `WebExtensionController` API.

---

## GeckoView WebExtensionController Capabilities

GeckoView exposes the `WebExtensionController` class as part of `GeckoRuntime`.
It provides the following operations:

| Operation | API method |
|---|---|
| Install extension from .xpi URL | `WebExtensionController.install(uri)` |
| Install built-in (bundled) extension | `WebExtensionController.installBuiltIn(uri)` |
| Uninstall extension | `WebExtensionController.uninstall(extension)` |
| Enable extension | `WebExtensionController.enable(extension, source)` |
| Disable extension | `WebExtensionController.disable(extension, source)` |
| List installed extensions | `WebExtensionController.list()` |
| Update extension | `WebExtensionController.update(extension)` |
| Send message to extension | `WebExtension.sendMessage(name, message)` |
| Receive message from extension | `WebExtension.MessageDelegate` |
| Handle browser action clicks | `WebExtension.ActionDelegate` |
| Handle page action clicks | `WebExtension.ActionDelegate` |

`WebExtensionController` is retrieved from `GeckoRuntime.webExtensionController`.

---

## Available WebExtension APIs (GeckoView Surface)

The following WebExtension APIs are available to extensions running in GeckoView,
consistent with the Firefox for Android extension API surface:

| API | Available | Notes |
|---|---|---|
| `content_scripts` | ✅ | Injected into pages per manifest `matches` |
| `background` scripts | ✅ | Persistent background script via `background.scripts` |
| `browser.browserAction` | ✅ | Toolbar button; popup UI |
| `browser.pageAction` | ✅ | Address bar button; popup UI |
| `browser.webRequest` (blocking) | ✅ | Intercept and modify requests before they reach the network |
| `browser.webRequest` (observing) | ✅ | Observe completed/failed requests |
| `browser.cookies` | ✅ | Read/write cookies for accessible origins |
| `browser.storage.local` | ✅ | Extension local key-value storage |
| `browser.storage.session` | ✅ (GV 120+) | In-memory session storage |
| `browser.tabs` | ✅ | Query and manipulate tabs |
| `browser.windows` | ✅ | Query windows (limited on Android: single window) |
| `browser.runtime` | ✅ | Extension lifecycle, messaging, version |
| `browser.i18n` | ✅ | Internationalisation |
| `browser.alarms` | ✅ | Scheduled callbacks |
| `browser.notifications` | ✅ | System notifications from extensions |
| `browser.permissions` | ✅ | Optional permission requests |
| `browser.theme` | ✅ (limited) | Theme colour hints |

---

## What is NOT Available

| Feature | Status | Reason |
|---|---|---|
| `chrome.downloads` API | ❌ Not available to extensions | Download interception in MyDownloader is done at the `GeckoSession.NavigationDelegate` level (app code), not inside an extension. Extensions cannot route a download to MyDownloader via the WebExtension API. |
| Native messaging from extension to Rust | ❌ Not available | GeckoView does not expose WebExtension native messaging to the embedding app. Security boundary: extensions run in the Gecko sandbox and cannot call Android JNI or the Rust bridge. |
| Full `chrome.*` namespace | ❌ Partial | GeckoView exposes `browser.*` (WebExtension standard). Some `chrome.*` aliases exist for compat but are not guaranteed. Extensions that rely on Chrome-only APIs will not work. |
| `browser.devtools` | ❌ Not available on Android | DevTools protocol not exposed in GeckoView mobile |
| `browser.tabGroups` | ❌ Not available | Chrome-specific API |
| `browser.sidePanel` | ❌ Not available | Chrome-specific API |
| `browser.storage.sync` | ⚠️ Limited | Requires Firefox Account (FxA) sync. Not available without FxA integration. `storage.local` is always available. |

---

## Extension Installation Flow

```
User selects .xpi file (file picker or URL)
    │
    ▼
:extensions module validates the .xpi (Phase 20K)
    │
    ▼
WebExtensionController.install(xpiUri): Promise<WebExtension>
    │
    ▼
GeckoView reads manifest.json, validates permissions
    │
    ▼
Phase 20L: PermissionPromptDelegate called with permission list
    │
    ▼
User approves permissions in MyDownloader permission UI
    │
    ▼
Extension installed; background script starts
    │
    ▼
Extension appears in "Installed Extensions" list (:extensions screen)
```

### Built-in extensions (bundled with the app)

MyDownloader may bundle a content-blocking extension as a built-in extension:

```kotlin
// Phase 20K — example
runtime.webExtensionController.installBuiltIn(
    "resource://android/assets/extensions/content-blocker/"
)
```

Built-in extensions are installed at app startup, are not shown in the user
extension list, and cannot be uninstalled by the user.

---

## Permission Model (Phase 20L)

GeckoView calls `WebExtensionController.PromptDelegate.onInstallPrompt()` with a
`WebExtension` object whose `metaData.permissions` and `metaData.origins` fields
describe what the extension requests.

Phase 20L will implement:

1. A `PermissionPromptUI` composable that lists human-readable permission
   descriptions (mirroring Firefox's permission prompt).
2. A `PermissionPromptDelegate` that calls the UI and resolves the GeckoView
   install promise based on user decision.
3. A `GrantedPermissions` data class persisted per-extension in SQLite.
4. Revocation UI: users can disable an extension or revoke host permissions
   post-install (via `WebExtensionController.disable()`).

Permission categories and human-readable descriptions follow the Firefox extension
permission warning system.

---

## Security Isolation

Extensions installed via `WebExtensionController` run inside the GeckoView sandbox:

- Extensions **cannot** call Android JNI.
- Extensions **cannot** read the download queue or interact with the Rust bridge.
- Extensions **cannot** access files outside their own `storage.local`.
- Extensions **cannot** make arbitrary Android API calls.
- Extensions are subject to GeckoView's same-origin policy and content security.

An extension that attempts to exfiltrate data can use `fetch()` / `XMLHttpRequest`
to send data to a remote server — this is governed by the extension's `host_permissions`
and the standard `webRequest` blocking (other extensions or built-in rules can block it).

The MyDownloader built-in content blocker extension will run with elevated trust
(`installBuiltIn`) and is distributed as part of the signed APK.

---

## Content Scripts

Content scripts are injected per the extension's `content_scripts` manifest field.
They run in a separate JavaScript context from the page (isolated world), can read
and modify the DOM, and communicate with the background script via
`browser.runtime.sendMessage`.

Content scripts are subject to the extension's `host_permissions` — they can only
be injected into pages whose origin matches the declared host permissions.

---

## Background Scripts

GeckoView manages the lifecycle of extension background scripts. A persistent
background script (`"persistent": true` in `background`) runs continuously while
GeckoView is running. A non-persistent (event-based) background script is started
on demand and suspended when idle.

GeckoView exposes background script message handling via
`WebExtension.setMessageDelegate(delegate, nativeApp)`, where `nativeApp` is the
application-defined port name for `browser.runtime.connectNative` messages.

---

## Extension Storage

| Storage API | Persistence | Notes |
|---|---|---|
| `browser.storage.local` | Persistent (SQLite) | Per-extension local storage |
| `browser.storage.session` | In-memory (GV 120+) | Cleared when GeckoView session ends |
| `browser.storage.sync` | Requires Firefox Account | Not available without FxA integration — document this limitation to users |

`storage.sync` limitation: extensions that rely on `storage.sync` for cross-device
synchronisation will fall back to `storage.local` on MyDownloader Android unless
FxA integration is added (not planned for Phase 20 scope).

---

## Relationship to Desktop Extension (apps/browser/extension/)

The existing `apps/browser/extension/` is an MV3 Chrome/Firefox browser extension
for the desktop. It communicates with the MyDownloader desktop app via native
messaging.

The Android WebExtension architecture is separate:

| Aspect | Desktop extension | Android WebExtension |
|---|---|---|
| Host | Chrome / Firefox desktop | GeckoView (MyDownloader app IS the browser) |
| Communication | Native messaging (stdin/stdout) | `WebExtensionController.MessageDelegate` (in-process) |
| Intercept mechanism | `chrome.downloads.onCreated` / `browser.downloads` | `GeckoSession.NavigationDelegate.onNewSession` + download MIME detection |
| Distribution | Chrome Web Store / Firefox Add-ons | Bundled with app or sideloaded .xpi |

They share design concepts but are separate implementations targeting different
platforms.

---

## Phase Plan

| Phase | Deliverable |
|---|---|
| 20A (now) | Extension architecture documented; no code yet |
| 20F | GeckoView initialised with `WebExtensionController`; built-in content blocker installed |
| 20K | Extension management UI: install, list, enable/disable, uninstall |
| 20L | Permission prompt UI; per-extension permission revocation; `GrantedPermissions` persistence |

# Android WebExtension Framework

This document describes the implemented Phase 20K foundation in the Android
GeckoView browser. The verified dependency is
`org.mozilla.geckoview:geckoview-arm64-v8a:137.0.20250325171416`.

## Architecture

Extension framework code lives in the existing `:browser` module; Phase 20K
does not introduce a separate Android module.

```text
BrowserScreen
    -> BrowserViewModel
        -> ExtensionManager
            -> ExtensionStore / GeckoExtensionGateway
                -> GeckoRuntime.webExtensionController
```

`ExtensionManager` is obtained from `GeckoRuntimeHolder` and is scoped to the
process-wide `GeckoRuntime`, not to a tab or BrowserViewModel. On initialization
it calls `WebExtensionController.list()` and builds an immutable in-memory
registry. Tab creation, tab switching, and tab closure do not create, remove, or
otherwise mutate extension instances. BrowserViewModel observes manager state;
Compose only renders the list and forwards user actions.

The domain includes validated extension IDs and metadata, explicit lifecycle
states, source classification, and bounded API-permission/host-origin metadata.
`ExtensionStore` rejects duplicate IDs and malformed metadata, returns defensive
snapshots, and caps the registry at 128 entries. GeckoView remains the source of
truth for installed extensions; the FetchX store is not persisted.

## Verified GeckoView 137 API

The exact GeckoView 137 AAR and its matching published source artifact were
inspected. `GeckoRuntime.getWebExtensionController()` exposes
`WebExtensionController`, whose relevant methods are asynchronous
`GeckoResult` operations:

- `list(): GeckoResult<List<WebExtension>>`
- `install(String, String): GeckoResult<WebExtension>`
- `enable(WebExtension, int): GeckoResult<WebExtension>`
- `disable(WebExtension, int): GeckoResult<WebExtension>`
- `uninstall(WebExtension): GeckoResult<Void>`

The implementation uses `INSTALLATION_METHOD_FROM_FILE` for installation and
`EnableSource.USER` for user-triggered enable/disable. Extension metadata is
read from GeckoView's `WebExtension.id`, `isBuiltIn`, and `metaData` fields,
including `name`, `version`, `description`, `enabled`, `requiredPermissions`,
and `requiredOrigins`. The UI does not receive GeckoView objects.

The pinned source documents `install(String, String)` as accepting remote HTTPS
or local `file:` / `resource:` package URIs, and states that ordinary extension
packages must be Mozilla-signed. The app does not use `installBuiltIn`; GeckoView
documents built-ins as APK `resource://android` folders and says they can access
native messaging. No built-in extension is bundled by this phase.

## Installation and permission boundary

Installation is user initiated through Android's document picker. FetchX accepts
only a `content:` URI from that picker, copies it to a generated file in the
app-private cache, enforces a 25 MiB size limit, and passes only the resulting
local `file:` URI to GeckoView. FetchX does not accept extension URLs, arbitrary
filesystem paths, shell commands, or extension-provided native code. GeckoView
performs package and signature validation; FetchX does not bypass those checks.
The staged file is removed when installation completes/fails or the manager is
shut down.

The GeckoView 137 `PromptDelegate` APIs are
`onInstallPromptRequest(WebExtension, String[], String[])`,
`onUpdatePrompt(WebExtension, WebExtension, String[], String[])`, and
`onOptionalPrompt(WebExtension, String[], String[])`.

Until Phase 20L, FetchX applies a deny-by-default policy:

- Installation proceeds only when GeckoView reports **no** requested API
  permissions and **no** host origins. That empty-permission response never
  enables private-browsing access.
- Installation requests containing any permissions or origins are denied.
- New permissions during updates and optional permission requests are denied.
- Permission strings returned in installed-extension metadata are stored only
  as bounded API/host-origin descriptions; malformed values are represented as
  unsupported. FetchX does not grant or revoke them in Phase 20K.

The UI explicitly warns users about this limitation. Permission review,
per-extension grants/revocation, and persistence are Phase 20L responsibilities.

## Lifecycle and failure handling

The manager supports initialization/list refresh, local installation,
enable/disable, removal, and shutdown. State changes are published only after
GeckoView's asynchronous operation succeeds. Failed enable/disable/removal
restores the prior in-memory state. Duplicate, unknown, transient, and built-in
removal requests fail without issuing an unintended GeckoView operation.

GeckoView documents uninstall as deleting the extension and its data. The UI
confirms that effect before removal. Built-in extensions are not offered a
remove action. The runtime-scoped manager survives tab changes and
BrowserViewModel recreation; its shutdown clears FetchX references and the
permission delegate without uninstalling extensions. Android does not guarantee
`Application.onTerminate()` in production, so process termination itself remains
OS-managed.

Errors are surfaced as user-readable state. GeckoView errors are logged by
exception type only; package URIs, permission values, browsing history, cookies,
and credentials are not logged or copied into the extension model.

## Security and current limitations

- No permission is automatically granted to an extension that requests it.
- No remote installation, marketplace, auto-update, native messaging, or
  download interception is implemented by FetchX in this phase.
- No JavaScript injection or custom content-script mechanism is added.
- Extension code executes under GeckoView's WebExtension model and can use only
  what GeckoView exposes and the extension's granted permissions allow; this
  framework does not claim an independent OS-level sandbox or enumerate all
  JavaScript WebExtension API compatibility.
- Extension settings/data stored internally by GeckoView are outside FetchX's
  registry. FetchX does not store browsing history, credentials, or cookies in
  extension records.
- The browser continues to use the existing shared GeckoRuntime, HTTPS
  certificate validation, cleartext policy, tracker blocking, and privacy
  controls; this phase does not weaken them.
- Automated JVM tests cover models, store behavior, manager operation and
  failure paths, permission denial rules, bounded file copying, and shutdown.
  Physical-device installation of a signed extension is not claimed as verified.

## Phase plan

| Phase | Deliverable |
|---|---|
| 20K | Runtime-scoped manager, in-memory registry, local package import, lifecycle UI (implemented) |
| 20L | Permission review UI, explicit grants/revocation, and any required persistence |

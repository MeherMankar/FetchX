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
- `update(WebExtension): GeckoResult<WebExtension>`; returns `null` when no update
  is available.

The implementation uses `INSTALLATION_METHOD_FROM_FILE` for local package
installation, the remote `install(String)` overload for Firefox Add-ons packages,
and `EnableSource.USER` for user-triggered enable/disable. The remote overload
lets GeckoView fetch the HTTPS package and perform its signature validation.
Extension metadata is
read from GeckoView's `WebExtension.id`, `isBuiltIn`, and `metaData` fields,
including `name`, `version`, `description`, `enabled`, `requiredPermissions`,
and `requiredOrigins`. The UI does not receive GeckoView objects.

The pinned source documents `install(String, String)` as accepting remote HTTPS
or local `file:` / `resource:` package URIs, and states that ordinary extension
packages must be Mozilla-signed. The app does not use `installBuiltIn`; GeckoView
documents built-ins as APK `resource://android` folders and says they can access
native messaging. No built-in extension is bundled by this phase.

## Installation and permission boundary

Installation is user initiated either through Android's document picker or the
integrated Firefox Add-ons catalog. Local packages are accepted only as a
`content:` URI from the picker, copied to a generated file in the app-private
cache, limited to 25 MiB, and passed to GeckoView as a local `file:` URI.
Marketplace results come from Mozilla's Add-ons API, and FetchX accepts package
URLs only on `https://addons.mozilla.org/firefox/downloads/file/`; GeckoView
downloads and validates the signed package. FetchX does not accept arbitrary
remote URLs, filesystem paths, shell commands, or extension-provided native
code. The staged file is removed when local installation completes/fails or
the manager is shut down.

The GeckoView 137 `PromptDelegate` APIs are
`onInstallPromptRequest(WebExtension, String[], String[])`,
`onUpdatePrompt(WebExtension, WebExtension, String[], String[])`, and
`onOptionalPrompt(WebExtension, String[], String[])`.

FetchX applies different, explicit policies to local packages and marketplace
installs:

- Local XPI installation proceeds only when GeckoView reports no requested API
  permissions and no host origins, except for the packaged official uBlock
  Origin 1.75.0. Its permissions are accepted only for the
  `uBlock0@raymondhill.net` ID; private-browsing access remains disabled.
- Firefox Add-ons catalog installs show GeckoView's requested API permissions
  and host origins in a confirmation dialog. Only explicit approval grants
  those requested permissions, and private-browsing access remains disabled.
  Requests too large or malformed to display safely are denied.
- New permissions during updates and optional permission requests are denied.
- Permission strings returned in installed-extension metadata are stored only
  as bounded API/host-origin descriptions; malformed values are represented as
  unsupported.

Marketplace approval is per installation only; FetchX does not yet provide
separate permission revocation controls or persist user permission decisions.

## Lifecycle and failure handling

The manager checks each installed non-built-in extension for an available update
when it initializes or the user refreshes the extension list. GeckoView resolves
updates using each extension's own update source and verifies the package. Updates
that request any new API permissions or host origins are denied; an update-check
failure leaves the installed extension in place and is surfaced in the extension
manager.

The manager supports initialization/list refresh, local installation,
update checks, enable/disable, removal, and shutdown. State changes are published only after
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

- Local XPI imports cannot request permissions. The local-package exception
  above is limited to the official uBlock Origin ID;
  the bundled signed release is pinned to 1.75.0 and should be updated with
  FetchX releases. Its GPL-3.0 license and corresponding source are available
  from [uBlock Origin](https://github.com/gorhill/uBlock/tree/1.75.0).
- Marketplace search and install are limited to the Firefox Add-ons API and
  Mozilla's XPI download host; arbitrary remote installation is not exposed.
  Update checks run when the extension manager initializes and when the user
  refreshes its list; an extension can only update if GeckoView can resolve an
  update from its configured source. There is no periodic update scheduler,
  native messaging, or extension download interception.
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

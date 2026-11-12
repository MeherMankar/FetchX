# Brave adblock-rust integration assessment

**Assessment date:** 2026-11-12
**Decision:** Research only; do not add the dependency or change browser behavior yet.

## Summary

FetchX has a GeckoView browser on Android and a Tauri download-manager UI on
desktop. Android already has GeckoView tracking protection and a bundled uBlock
Origin WebExtension; desktop's browser extension observes media/download
activity in browsers installed by the user. Neither platform currently routes
browser subresource decisions through FetchX's Rust core.

The Android browser can block subresources through its WebExtension engine, but
its Rust-facing `ContentBlocking.Delegate` reports GeckoView decisions after
they occur. The Android navigation delegate handles top-level navigation, not
subresource requests. Desktop has no FetchX-owned browsing surface, and the
installed-browser extension does not request the blocking `webRequest` API.
Adding a Rust matcher without a verified request-time adapter and selected,
versioned rule data would therefore add unused policy code or duplicate the
existing Android blocker. No dependency, filter list, or browser code is added
by this assessment.

## Upstream identity, license, and versioning

The correct upstream is [brave/adblock-rust](https://github.com/brave/adblock-rust),
not `brave-core`. Its Cargo package is named [`adblock`](https://crates.io/crates/adblock).

| Item | Finding |
|---|---|
| License | MPL-2.0, declared in the crate metadata and the upstream [`LICENSE`](https://github.com/brave/adblock-rust/blob/master/LICENSE). Record the notice and review MPL source-distribution obligations before shipping. |
| Current crate candidate | `adblock` **0.13.5**, published on crates.io on 2026-10-28. Crates.io checksum: `7a4c8558f27608dec74695c866f9a8ef897ec7a77cb826f8a5baf0b8ba461d2a`. |
| GitHub releases/tags | The GitHub Releases API returned no releases. The tags endpoint listed `v0.13.4` as its newest tag, while crates.io and upstream `master` report 0.13.5. Do not assume the latest crate has a matching Git tag. |
| Version model | Stable-looking semver crate publications on crates.io, with repository tags/commits as a separate stream. Pin the crate version and lockfile, not `master` or an unreviewed moving Git reference. |
| Rust compatibility | The 0.13.5 package uses edition 2024 and does not declare `rust-version` in its published metadata. Edition 2024 requires Rust 1.85 or newer; FetchX declares workspace Rust 1.88. This is a language-level compatibility indication, not a verified build of this dependency or its full transitive dependency graph. |
| Features | Defaults are `embedded-domain-resolver`, `full-regex-handling`, and `single-thread`. The latter makes `Engine` not `Send + Sync`; review threading and feature selection before integrating it into FetchX. Optional features include CSS validation, Apple content-blocking conversion, and uBlock resource assembly. |

The upstream API composes rules in `FilterSet`, parses them with
`ParseOptions`, and creates an `Engine` for network decisions
(`check_network_request`). It supports ABP-style rules (including EasyList and
EasyPrivacy), hosts syntax, and uBlock Origin syntax extensions. Cosmetic
filtering and resource replacements are also available, but require additional
browser integration and, for some rules, resources. Rule changes require
building a replacement engine; the crate does not itself provide a trusted
filter-list update service.

**Candidate only, not selected or added to FetchX:** if implementation is
approved, evaluate `adblock = "=0.13.5"` from crates.io with Cargo.lock updated
in the same reviewed change. There is no FetchX dependency pin or Git revision
from this assessment.

## FetchX browser and interception capability

| Platform/surface | Existing implementation | Subresource blocking capability | Relevance to adblock-rust |
|---|---|---|---|
| Android in-app browser | The `:browser` module uses GeckoView 137. `GeckoRuntimeHolder` configures Gecko content blocking. `PrivacySettings` exposes ad blocking and `BrowserViewModel` synchronizes that setting with the bundled uBlock extension. | GeckoView's extension-based path can block requests; FetchX bundles uBlock Origin. Gecko's `onContentBlocked` callback is used to record already-blocked events. The `NavigationDelegate.onLoadRequest` path handles main-frame navigations such as external schemes and magnets; it is not a subresource filter hook. | No current callback gives FetchX Rust a synchronous decision for each Gecko request. The existing Kotlin/UniFFI bridge is for download/core services, not Gecko network requests. A Rust engine would need a request-time WebExtension adapter, likely using a locally packaged WASM binding, or another adapter proven to meet the blocking callback's timing and API constraints. Do not replace uBlock until this path and behavior parity are tested. |
| Desktop Tauri application | The Tauri window loads FetchX's local React download-manager UI. It is not a general-purpose browser. The separate browser extension/native host forwards browser downloads and detects media in Chrome/Firefox. | No FetchX-owned user browsing engine is present. The Chromium manifest is MV3 and requests `webRequest` without `webRequestBlocking`; the Firefox MV2 manifest also has `webRequest` without `webRequestBlocking`. Current listeners observe media request/response events and do not cancel them. | There is no desktop app request hook for an adblock engine. Do not describe desktop or installed-browser filtering as supported. A future host-browser implementation would require separate Chromium MV3 (`declarativeNetRequest`) and Firefox blocking designs and tests. |

Android code and architecture evidence:

- [GeckoRuntimeHolder](../../apps/android/browser/src/main/kotlin/dev/mydownloader/browser/GeckoRuntimeHolder.kt)
  creates the runtime, configures content blocking, and registers FetchX
  extensions.
- [TabManager](../../apps/android/browser/src/main/kotlin/dev/mydownloader/browser/TabManager.kt)
  consumes `ContentBlocking.Delegate.onContentBlocked` and uses
  `NavigationDelegate.onLoadRequest` for navigation handling.
- [PrivacyController](../../apps/android/browser/src/main/kotlin/dev/mydownloader/browser/PrivacyController.kt),
  [BrowserViewModel](../../apps/android/browser/src/main/kotlin/dev/mydownloader/browser/BrowserViewModel.kt),
  and [BrowserScreen](../../apps/android/browser/src/main/kotlin/dev/mydownloader/browser/BrowserScreen.kt)
  own the ad/tracker controls and their GeckoView/uBlock effects.
- [ANDROID_EXTENSIONS.md](../ANDROID_EXTENSIONS.md) documents the bundled,
  release-pinned uBlock Origin package and extension lifecycle.

Desktop evidence:

- [Tauri configuration](../../apps/desktop/src-tauri/tauri.conf.json) loads the
  local desktop UI.
- [Chromium manifest](../../apps/browser/extension/manifest.json) and
  [Firefox manifest](../../apps/browser/extension/manifest-firefox/manifest.json)
  declare `webRequest` but not `webRequestBlocking`.
- [Media interception](../../apps/browser/extension/src/background/media-interception.ts)
  listens to headers and response-start events to detect media; it does not
  return a blocking response.

The browser-related `crates/core` state machine, `crates/core-services`, and
`crates/browser-protocol` do not currently define a page-request filtering
contract. If this proceeds, keep the matcher in a focused shared Rust crate
(for example, `crates/browser-filter`) rather than putting browser policy in
download orchestration. Platform adapters should only translate each browser
request into the shared matcher input and apply its result.

The source evidence establishes available APIs and current wiring; this
assessment did not exercise live network blocking on an Android device or
browser.

## Safe implementation plan

1. **Prototype Android request-time integration first.** Create an isolated
   shared Rust filter crate and compile/package its engine locally for the
   GeckoView WebExtension path (the upstream project advertises WASM and
   JavaScript bindings). Verify that the extension can synchronously evaluate
   GeckoView subresource requests using the required blocking permission. If
   the runtime/API cannot make that decision before the request proceeds, stop
   rather than falling back to asynchronous native messaging or JavaScript
   reimplementation.
2. **Choose rule data before shipping.** Start with a small, reviewed,
   app-bundled ruleset from an approved source, with its source, revision,
   license, and digest recorded. Do not fetch arbitrary lists or list metadata
   at runtime. Keep the existing uBlock and Gecko behavior intact during the
   prototype; only consider changing the shipped blocker after explicit
   behavior/parity testing and a deliberate migration decision.
3. **Use one user-facing control.** Reuse or deliberately revise Android's
   existing ad-blocking control so it has a clear enabled/disabled state. Keep
   Gecko tracker/fingerprinting/cookie controls distinguishable from network
   ad-filter state. Ensure disabled means the FetchX matcher does not cancel
   requests, and preserve navigation, downloads, and unrelated extension
   behavior.
4. **Keep desktop capability honest.** No desktop in-app browser integration
   is currently available. A separately scoped browser-extension project would
   need Chromium declarative rules and Firefox blocking API support; it must
   not advertise cross-browser support until each engine has been implemented
   and exercised.
5. **Validate end-to-end before enabling by default.** Add Rust tests for
   allow/block/exception rules, resource types, first-/third-party context,
   hosts lists, malformed rules, and disabled state. Add WebExtension tests
   using local test pages/servers to prove subresource cancellation and allowed
   requests. Add Android instrumentation checks for toggle behavior, navigation,
   downloads, and existing uBlock/extension lifecycle. A Rust matcher unit test
   alone does not prove browser blocking.

## Controlled dependency and ruleset updates

- Pin the reviewed crates.io version exactly and commit its Cargo.lock
  resolution in a FetchX change. Monitor crates.io versions with Cargo
  Dependabot (or an equivalent scheduled check) that opens reviewable update
  pull requests; separately watch the upstream repository for security notices
  and commits. Do not auto-merge dependency changes.
- For each update, review the upstream diff, release notes/commits, license,
  declared edition/MSRV, feature changes, and dependency changes. Run
  `cargo fmt --all -- --check`, focused Rust tests and clippy, Android browser
  unit/instrumentation tests and builds, and relevant extension/desktop checks.
  Record platform checks that cannot run in CI. Ship an accepted library update
  only in a FetchX app release.
- Keep filter-list updates independent from crate updates. The initial shipped
  design should package rules with the app and update them only through a
  FetchX release. If a separate runtime list updater is later approved, use an
  approved source and signed metadata verified against a public key pinned in
  the app, enforce version/size limits and anti-rollback, install atomically,
  retain the last-known-good rules for rollback, and never permit remotely
  supplied metadata to select arbitrary URLs.
- Never download or execute upstream library code in an installed browser.
  Library code changes ship only as part of a reviewed FetchX release.

## Sources

- [Upstream repository](https://github.com/brave/adblock-rust)
- [Upstream Cargo manifest](https://raw.githubusercontent.com/brave/adblock-rust/master/Cargo.toml)
- [Upstream README](https://github.com/brave/adblock-rust/blob/master/README.md)
- [Upstream license](https://github.com/brave/adblock-rust/blob/master/LICENSE)
- [Crates.io package metadata for 0.13.5](https://crates.io/api/v1/crates/adblock/0.13.5)
- [adblock 0.13.5 API documentation](https://docs.rs/adblock/0.13.5/adblock/)
- [Mozilla `webRequest` blocking documentation](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/webRequest)

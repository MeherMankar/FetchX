# Android Browser Privacy Controls

This document describes privacy behavior implemented by the Android browser in
FetchX, using the pinned GeckoView dependency
`org.mozilla.geckoview:geckoview-arm64-v8a:158.0.20261007201254`.
It distinguishes native GeckoView settings from behavior that FetchX has not
implemented.

## Implemented controls

The browser uses one process-wide `GeckoRuntime` shared by all tabs. `PrivacyController`
holds immutable settings; `BrowserViewModel` persists them in the browser's
SharedPreferences store, and `GeckoRuntimeHolder` maps GeckoView settings to native
APIs. A change updates the shared runtime without closing or recreating tabs. Reload
open pages after changing a setting.

| Control | Default | Behavior |
|---|---:|---|
| GeckoView tracking protection | On | `ContentBlocking.AntiTracking.AD`, `ANALYTIC`, and `SOCIAL` |
| uBlock Origin | On | Bundled WebExtension enable/disable state managed independently of GeckoView protections |
| Fingerprinting protection | Off | `GeckoRuntimeSettings.setFingerprintingProtection(false)` and the `AntiTracking.FINGERPRINTING` category |
| Cryptomining protection | On | `ContentBlocking.AntiTracking.CRYPTOMINING` |
| Third-party cookie isolation | Off | `CookieBehavior.ACCEPT_ALL`; when on, `ACCEPT_FIRST_PARTY_AND_ISOLATE_OTHERS` |
| HTTPS-only mode | Off | `GeckoRuntimeSettings.HTTPS_ONLY`; when off, `ALLOW_ALL` |

uBlock Origin's XPI and license are packaged in the Android app assets. When the
browser's runtime extension manager starts, it checks GeckoView's installed
extensions and installs the bundled XPI if it is missing. This is an automatic
first-browser-start install from the app package; it does not download executable
code at runtime. The Ad Blocking screen shows the extension's actual GeckoView
enabled state and reports loading or installation errors.

The fingerprinting-protection preference and the fingerprinting anti-tracking
category are distinct GeckoView mechanisms. The preference enables GeckoView's
fingerprinting protections in all tabs; the category blocks network resources
classified as fingerprinting trackers. Neither setting is a promise of anonymity
or complete protection from fingerprinting.

The cookie control selects GeckoView's documented dynamic first-party-isolation
behavior. When disabled, FetchX explicitly selects `CookieBehavior.ACCEPT_ALL`;
that setting accepts all cookies and site data. This control does not clear stored
cookies when toggled.

HTTPS-only mode uses GeckoView's runtime-wide `HTTPS_ONLY` setting, not a
FetchX-maintained URL rule. HTTP-only sites may fail to load. FetchX does not
implement per-site HTTP exceptions. Android's `network_security_config.xml` is a
separate platform policy; it is not the implementation of this browser control.

These controls are global to the shared runtime, not per-tab or per-origin. The
GeckoView Tracking Protection switch and the Tracking Protection switch in the
browser menu share the same preference. Turning it off removes the ad, analytics,
and social anti-tracking categories; separately enabled fingerprinting and
cryptomining protections remain active. uBlock Origin is a separate extension
switch and does not change GeckoView's settings.

## Tracker blocking and statistics

`GeckoSessionSettings.useTrackingProtection` defaults to false.
`TabManager` explicitly enables it on each session before opening it. The
per-session `ContentBlocking.Delegate` forwards GeckoView's native
`onContentBlocked` events to `TrackerBlockingController`; FetchX does not infer
blocked requests with custom URL rules.

Tracker statistics count blocking events in the ad, analytics, and social
categories only. Fingerprinting and cryptomining protection events do not inflate
that tracker counter. For each tab, the controller retains a bounded count and up
to 20 normalized hostnames; it discards request paths and query strings. Statistics
are in memory and removed when a tab closes. No request URL, credentials, cookies,
or page content is logged or sent by this feature.

On a V2135 Android 13 device, loading public HTTPS `https://edition.cnn.com/`
produced real GeckoView callbacks with `isBlocking=true` for ad (category 2) and
analytics (category 4); logcat also recorded that the controller accepted them.
The count changed on enabled reloads and did not increase on a reload with tracker
blocking disabled. A second tab opened with an isolated zero count. The physical
test of cross-tab state after generating events in both tabs, tab cleanup, and the
minimum-one-tab invariant was not completed. Therefore the native callback path
was observed, but those multi-tab lifecycle checks remain unverified.

## Not implemented

- **Private browsing/private tabs:** GeckoView exposes `GeckoSessionSettings.Builder.usePrivateMode`,
  but FetchX does not create private sessions or provide a private-browsing mode.
  Browser tabs currently share the normal runtime profile. Do not interpret this
  browser as a private browser.
- **Per-site exceptions or permissions:** no per-origin privacy settings, HTTPS
  exceptions, camera/microphone/location permission store, or `site_permissions`
  database table is implemented.
- **WebRTC IP handling:** FetchX does not configure or expose a WebRTC IP policy.
- **Custom fingerprint profiles:** FetchX does not spoof navigator, canvas, screen,
  font, audio, or WebGL values, nor does it configure fingerprinting-protection
  overrides.
- **VPN, Tor, or network anonymity:** none of these controls hides the user's IP
  address from network operators or visited sites.

The GeckoView 137 API includes additional privacy and permission settings, but API
availability alone does not mean FetchX configures or exposes them.

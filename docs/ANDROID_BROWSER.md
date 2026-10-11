# Android Browser Engine Comparison

> **Phase 20 decision document.** Evaluated before any browser code was written.
> Chosen engine: **GeckoView (Mozilla Public License 2.0)**.

---

## A. Android System WebView (Chromium-based)

Android System WebView is a system component bundled with AOSP that exposes a
`android.webkit.WebView` API for embedding web content in Android applications.

- **Ships with Android:** yes — updated independently via the Play Store (AOSP Chromium fork)
- **API:** `android.webkit.WebView` / `androidx.webkit.WebViewCompat`
- **Extension support:** NONE natively.  
  JavaScript injection via `evaluateJavascript()` provides a limited workaround, but
  this is **not** the WebExtension API. Extensions cannot access `webRequest`,
  cannot intercept network traffic before it reaches the engine, cannot install their
  own background scripts, and cannot receive real extension messages. Any "extension"
  layer built on top of WebView must be reimplemented entirely in the app itself.
- **Privacy controls:** Limited.  
  No cross-site tracking protection, no built-in content blocking, no fingerprinting
  resistance. The app can disable cookies globally (`CookieManager`) or block mixed
  content, but there is no per-origin, per-tracker, or per-category policy system.
- **Profile isolation:** Effectively one WebView data directory at a time.  
  `WebStorage.deleteAllData()` can wipe state, but there is no runtime profile-switch
  API. Two simultaneous profiles require two separate `WebView` instances backed by
  different data paths, which requires careful management and is not first-class.
- **Private browsing:** Not supported natively. The app must manually clear all
  cookies, localStorage, cache, and history on every session end — no guarantees.
- **Fingerprinting resistance:** None.
- **WebRTC controls:** Exposed with no controls. No ICE policy, no mDNS obfuscation,
  no TURN-only mode.
- **JavaScript controls:** `WebSettings.setJavaScriptEnabled(false)` — coarse global
  toggle only.
- **Storage isolation:** None automatic. All `WebView` instances in the same process
  share the same `WebStorage` unless the app explicitly segregates data directories.
- **Multiple profiles:** Not supported without significant custom scaffolding.
- **APK size impact:** ~0 MB — system component, not bundled.
- **Licensing:** Apache 2.0 / BSD (Chromium), ships as Android system component.
- **Chromium compatibility:** Yes (it is Chromium WebView).
- **Firefox/WebExtension compatibility:** No.
- **RAM usage:** Lowest — shares the system WebView process.
- **minSdk:** 21+
- **Long-term viability:** High — Google maintains, updated independently of OS.
- **Verdict: REJECTED**  
  No real extension API, no privacy controls, no profile isolation, no private
  browsing guarantees. Building what Phase 20 requires on top of WebView would mean
  reimplementing the bulk of what GeckoView already provides.

---

## B. GeckoView (Mozilla)

GeckoView is Mozilla's embeddable Android library that wraps the Gecko browser engine
(the same engine used by Firefox for Android / Fenix, Firefox Focus, Firefox Beta,
Tor Browser for Android, and Mullvad Browser).

- **Used by:** Firefox for Android (Fenix), Firefox Focus, Tor Browser for Android,
  Mullvad Browser for Android.
- **Maven coordinates:**
  ```
  org.mozilla.geckoview:geckoview-androidX-arm64-v8a:133.0.20250116035429
  org.mozilla.geckoview:geckoview-androidX-armeabi-v7a:133.0.20250116035429
  org.mozilla.geckoview:geckoview-androidX-x86_64:133.0.20250116035429
  ```
- **Extension support:** FULL WebExtensions via `WebExtensionController`.  
  Real browser extension API: content scripts, background scripts, `webRequest`
  (blocking and modifying), `browserAction`, `pageAction`, `cookies`, `storage`,
  `tabs`, `windows`, `runtime` messaging. Extensions are installed as `.xpi` files.
  This is the same API surface as Firefox for Android.
- **Privacy controls:**  
  Enhanced Tracking Protection (ETP) with strict/standard/custom modes, built-in
  tracker and fingerprinter databases, crypto-miner blocking, social tracker blocking.
  Configurable via `ContentBlocking.Settings` and `GeckoRuntimeSettings`.
- **Cookie isolation:** Yes — first-party isolation, per-site storage partitioning
  (Total Cookie Protection) configurable per-session via `GeckoSessionSettings`.
- **Private browsing:** Yes — `GeckoSessionSettings.usePrivateMode(true)` creates a
  session with separate storage, no history persistence, no cookie sharing with normal
  sessions.
- **Fingerprinting resistance:** Built-in via ETP fingerprinting protection category
  and `GeckoRuntimeSettings.fingerprintingProtection`. Reduces JS API surface (canvas,
  fonts, screen resolution) to a consistent fingerprint profile.
- **WebRTC controls:** Yes — `GeckoRuntimeSettings.webRtcIpHandling` policy (default,
  default + public only, disable non-proxied UDP, disable non-proxied UDP + TURN only).
  mDNS hostname obfuscation for ICE candidates.
- **JavaScript controls:** Per-session JS disable; per-extension JS policy via
  content scripts.
- **Storage isolation:** Per-profile storage via `GeckoRuntime` + multiple
  `GeckoSession` instances with separate profile paths.
- **Multiple profiles:** Native — create separate `GeckoSession` instances under
  separate runtime profiles. Regular profile and private profile are handled
  transparently.
- **APK size impact:** ~37 MB arm64 ABI (ABI splits via Android App Bundle can
  deliver only the required ABI at install time).
- **Licensing:** Mozilla Public License 2.0 — compatible with this project.  
  MPL 2.0 is a file-level copyleft: modifications to GeckoView files themselves must
  be shared under MPL 2.0, but application code that uses GeckoView is not affected.
  There is no obligation to open-source the MyDownloader application.
- **Chromium compatibility:** High — supports modern web standards (CSS Grid, WebGL,
  Web Workers, Service Workers, WebAssembly, etc.). Not Chromium, but compatible with
  the living standards.
- **Firefox/WebExtension compatibility:** Yes (is the Firefox engine).
- **RAM usage:** Higher than WebView — separate Gecko content process, ~50–80 MB
  baseline additional RAM.
- **minSdk:** 21+ (GeckoView 133.x supports API 21+)
- **Long-term viability:** EXCELLENT — actively maintained by Mozilla as the
  production foundation for Firefox for Android. Not going away.
- **Verdict: SELECTED**  
  The only option with a real WebExtension API, full privacy controls, per-session
  isolation, and private browsing with genuine storage separation.

---

## C. Crosswalk (deprecated)

Crosswalk was an Intel-maintained Chromium-based embedded WebView replacement for
Android, offering a consistent Chromium version across Android versions (when the
system WebView was fragmented before Android 7.0 guaranteed updates).

- **Status: END OF LIFE since March 2017.**  
  Intel discontinued Crosswalk in March 2017. No security patches have been issued
  since then. Using Crosswalk in 2025 means embedding a Chromium version from 2016
  with no security fixes — this is not acceptable for any application, and
  especially not for one with a privacy focus.
- **Verdict: REJECTED — EOL. Do not use.**

---

## D. Custom Chromium Build

This option involves forking the Chromium source and maintaining a custom Chromium
build for Android (e.g., as Brave, Kiwi, and other browsers do).

- **APK size:** +80–150 MB minimum.
- **Build time:** Hours per build cycle on high-end hardware; tens of minutes for
  incremental builds.
- **Security maintenance:** Must manually backport all Chromium security fixes,
  typically dozens per release cycle. Chromium ships security updates roughly every
  4 weeks. Missing one cycle means shipping known CVEs.
- **Team required:** Brave employs a dedicated browser engineering team to maintain
  their Chromium fork. Google employs hundreds of engineers on Chromium itself.
- **Extension support:** WebExtensions can be added via Chromium's extension
  infrastructure, but enabling them in an Android Chromium build is non-trivial
  and is itself a significant engineering project (Kiwi Browser demonstrates this
  is possible but requires sustained effort).
- **Verdict: REJECTED** — completely impractical for any team below browser-vendor
  scale. Maintenance burden is incompatible with the project's resources.

---

## E. Comparison Matrix

| Criterion | WebView (Chromium) | GeckoView | Crosswalk | Custom Chromium |
|---|---|---|---|---|
| Extension support | ❌ None | ✅ Full WebExtensions | ❌ None | ⚠️ Possible but hard |
| Privacy controls | ❌ None built-in | ✅ ETP strict/standard/custom | ❌ None | ⚠️ Must build yourself |
| Profile isolation | ❌ Manual/fragile | ✅ Native per-session | ❌ None | ⚠️ Must build yourself |
| Private browsing | ❌ Manual clear-only | ✅ Native usePrivateMode | ❌ None | ⚠️ Must build yourself |
| Fingerprinting resistance | ❌ None | ✅ Built-in (ETP) | ❌ None | ⚠️ Must build yourself |
| WebRTC controls | ❌ None | ✅ ICE policy + mDNS | ❌ None | ⚠️ Must build yourself |
| APK size impact | ✅ 0 MB (system) | ⚠️ ~37 MB (arm64, ABI split) | ❌ +70 MB | ❌ +80–150 MB |
| RAM usage | ✅ Lowest | ⚠️ +50–80 MB (Gecko process) | ❌ High | ❌ High |
| Licensing | ✅ Apache 2.0 | ✅ MPL 2.0 | ❌ EOL, no clear status | ⚠️ BSD/Apache (Chromium) |
| Long-term viability | ✅ Google-backed | ✅ Mozilla-backed | ❌ EOL | ✅ Community-backed |
| Status | ✅ Active | ✅ Active | ❌ Dead | ✅ Active (with effort) |

---

## Recommendation: GeckoView

**Chosen foundation: GeckoView (MPL 2.0)**

Justifications:

1. **Real WebExtension API** — `WebExtensionController` provides the same WebExtension
   API surface as Firefox for Android. This is not a workaround. This is the
   critical prerequisite for Phase 20K (extension install) and Phase 20L (extension
   permissions). No other embeddable engine offers this without becoming a full
   browser fork.

2. **Enhanced Tracking Protection built-in** — ETP with strict/standard/custom modes,
   tracker blocking, fingerprinter blocking, crypto-miner blocking, and social tracker
   blocking are all available at the engine level. This directly satisfies Phase 20J
   (privacy/tracker protection) without building a custom content-blocking layer.

3. **Per-session profile isolation** — `GeckoSessionSettings.usePrivateMode(true)`
   and separate profile storage paths satisfy Phase 20H (private browsing) with
   genuine storage isolation, not best-effort cookie clearing.

4. **Fingerprinting resistance built-in** — `GeckoRuntimeSettings.fingerprintingProtection`
   and ETP fingerprinting category reduce the JS API surface that fingerprinting
   scripts rely on.

5. **MPL 2.0 compatible** — No copyleft obligation on application code. The license
   is strictly file-level: only modifications to GeckoView source files themselves
   need to be shared under MPL 2.0. The MyDownloader application code is not affected.

6. **Production-tested** — GeckoView is the production engine for Firefox for Android
   (hundreds of millions of users), Tor Browser for Android, and Mullvad Browser for
   Android. It is not experimental.

7. **APK size cost is justified** — The ~37 MB arm64 ABI cost (with ABI splits
   delivering only the device's ABI) is the same cost accepted by Firefox for Android,
   Tor Browser, and Mullvad Browser. For an application whose core value proposition
   is privacy and extension support, this is a reasonable trade-off. A privacy-focused
   browser on WebView would save 37 MB while delivering a product that cannot deliver
   on its privacy or extension promises.

---

## GeckoView Integration Plan (Phase 20F)

GeckoView is **NOT added as a Gradle dependency in Phase 20A**. It will be added when
the `browser` module is created in Phase 20F.

### Maven repository (already declared in settings.gradle.kts)

```
https://maven.mozilla.org/maven2
```

### Planned version catalog entries (libs.versions.toml, Phase 20F)

```toml
[versions]
# Pin to a specific date-versioned build for reproducibility
geckoview = "133.0.20250116035429"

[libraries]
geckoview-arm64 = { group = "org.mozilla.geckoview", name = "geckoview-androidX-arm64-v8a", version.ref = "geckoview" }
geckoview-armeabi = { group = "org.mozilla.geckoview", name = "geckoview-androidX-armeabi-v7a", version.ref = "geckoview" }
geckoview-x86-64 = { group = "org.mozilla.geckoview", name = "geckoview-androidX-x86_64", version.ref = "geckoview" }
```

### ABI split strategy

Use Android App Bundle (AAB) for Play Store distribution. Gradle ABI splits ensure
that each device download contains only the GeckoView native library for its ABI
(arm64, armeabi-v7a, or x86_64), keeping the installed APK size close to the minimum.

```kotlin
// In browser/build.gradle.kts (Phase 20F):
android {
    bundle {
        abi { enableSplit = true }
    }
}
```

### Key GeckoView APIs to use (Phase 20F–20L)

| API | Purpose | Phase |
|---|---|---|
| `GeckoRuntime` | Engine lifecycle, global settings | 20F |
| `GeckoSession` | Individual browser tab | 20F, 20H |
| `GeckoSessionSettings.usePrivateMode` | Private browsing | 20H |
| `WebExtensionController` | Extension install/manage | 20K |
| `ContentBlocking.Settings` | ETP configuration | 20J |
| `GeckoRuntimeSettings.fingerprintingProtection` | Fingerprinting resistance | 20J |
| `GeckoRuntimeSettings.webRtcIpHandling` | WebRTC IP policy | 20J |
| `GeckoSession.NavigationDelegate` | URL interception for download detection | 20G |
| `GeckoSession.ContentDelegate` | Page load events | 20F |
| `GeckoSession.ProgressDelegate` | Loading progress | 20F |

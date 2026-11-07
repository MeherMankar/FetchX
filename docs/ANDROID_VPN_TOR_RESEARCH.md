# Phase 20M — VPN / Tor Integration Research

**Project:** FetchX Android  
**Phase:** 20M — Research Only  
**GeckoView version verified:** `137.0.20250325171416`  
**Status:** Research complete; NO implementation code added.

---

## 1. Executive Summary

FetchX is a privacy-focused Android download manager with an embedded GeckoView browser. As of Phase 20L the application routes all network traffic over the device's default network interface. No VPN, proxy, or anonymisation layer has been implemented. This document researches the technical landscape for a future implementation phase.

The core finding is that FetchX already defines a `NetworkRoute` sealed class in `:core` with `Direct`, `SystemVpn`, `SocksProxy`, and `Tor` variants. The architecture correctly anticipated the need. What is missing is (1) a GeckoView proxy configuration call, (2) a `reqwest` SOCKS5 proxy configuration in the Rust bridge, and (3) user-facing controls. The Android `VpnService` API is powerful but introduces significant complexity, security surface, and Android-version-specific behaviour that is disproportionate to the immediate need.

The recommended architecture for a future implementation phase is **Option C: Per-component proxy** — GeckoView configured with a SOCKS5 proxy setting plus `reqwest` compiled with the `socks` feature plus `aria2 --all-proxy`. This approach requires no `android.permission.BIND_VPN_SERVICE`, imposes no kernel-level packet routing, does not require user VPN consent dialogs, and is fully compatible with the existing engine abstraction. It delivers meaningful privacy when the user connects FetchX to Orbot (the official Tor VPN/SOCKS5 daemon for Android) without requiring FetchX to bundle or manage the Tor binary. Full VPN-style system-wide coverage can be revisited in a later phase if user demand warrants the added complexity.

---

## 2. Current FetchX Architecture

### GeckoView version

**137.0.20250325171416** — pinned in `apps/android/gradle/libs.versions.toml` as:
```
geckoview = "137.0.20250325171416"
geckoview-arm64 = { group = "org.mozilla.geckoview", name = "geckoview-arm64-v8a", version.ref = "geckoview" }
```
This is an arm64-v8a-only debug/development build. The full artifact `geckoview-androidX` would be used for multi-ABI release builds.

### Browser module structure

The `:browser` module (`apps/android/browser`) contains:

| Class | Responsibility |
|---|---|
| `GeckoRuntimeHolder` | Process-wide singleton for `GeckoRuntime`; double-checked locking; applies privacy settings to the shared runtime |
| `TabManager` | Owns one `GeckoSession` per tab; delegates `ContentBlocking.Delegate` events to `TrackerBlockingController`; enables `useTrackingProtection = true` on each session |
| `BrowserViewModel` | Lifecycle-aware owner of `TabManager`; exposes `BrowserUiState` as `StateFlow`; delegates to `BookmarkController`, `TrackerBlockingController`, `PrivacyController`, `ExtensionManager` |
| `BrowserScreen` | Compose UI; TabRow, navigation bar, history/bookmark dialogs, tracker details dialog, privacy settings sheet, extension manager sheet |
| `TrackerBlockingController` | Pure-Kotlin; tracks per-tab blocked request counts and up to 20 hostnames; no URL logging |
| `PrivacyController` | Pure-Kotlin; holds `PrivacySettings` (fingerprinting, cryptomining, cookie isolation, HTTPS-only) |
| `ExtensionManager` | Runtime-scoped; wraps `GeckoExtensionGateway`; limits install count to 128; denies permission-requiring packages until Phase 20L |
| `BookmarkController` | Pure-Kotlin; global in-memory bookmarks |
| `HistoryStore` | Pure-Kotlin; per-tab in-memory navigation history |

### `:core` module — NetworkRoute

A `NetworkRoute` sealed class already exists at `apps/android/core/src/main/kotlin/dev/fetchx/core/network/NetworkRoute.kt`:

```kotlin
sealed class NetworkRoute {
    data object Direct : NetworkRoute()
    data object SystemVpn : NetworkRoute()
    data class SocksProxy(val host: String, val port: Int) : NetworkRoute()
    data object Tor : NetworkRoute()
}
```

This is a domain model only. No implementation backs it yet.

### Rust downloader engines

| Engine | Rust type | Transport |
|---|---|---|
| Direct HTTP | `DirectHttpEngine` | `reqwest` with `rustls-tls` |
| aria2 | `Aria2Engine` + `Aria2Supervisor` | aria2 process via JSON-RPC; process management in Rust |
| yt-dlp | `YtDlpEngine` | yt-dlp subprocess; output parsed by Rust |
| FFmpeg | `FfmpegProcessor` | FFmpeg subprocess; used for post-processing / muxing |

The `DownloadEngine` trait (`crates/downloader/src/engine.rs`) is a single async `download(id, cancellation)` method. There is no proxy configuration in any engine. The `Cargo.toml` workspace dependency for `reqwest` does not currently include the `socks` feature:

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

### AndroidManifest.xml — current permissions

```xml
<uses-permission android:name="android.permission.INTERNET" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
```

No VPN-related permissions are declared.

### Existing privacy / security architecture

- `network_security_config.xml`: cleartext HTTP blocked by default; `localhost` exempt for tooling only.
- GeckoView ETP: ad, analytics, social tracker blocking enabled by default (configured in `GeckoRuntimeHolder.createRuntime()`).
- Fingerprinting protection and cryptomining protection: on by default.
- Third-party cookie isolation: `ACCEPT_FIRST_PARTY_AND_ISOLATE_OTHERS` by default.
- HTTPS-only mode: off by default, toggleable at runtime.
- WebRTC IP handling: **not configured** (noted as "not implemented" in `ANDROID_PRIVACY.md`).
- Private browsing sessions: **not implemented**.

**No existing VPN, proxy, or routing implementation found.** The `NetworkRoute` sealed class is a model placeholder only.

---

## 3. Android VPNService Research

### Required permissions

```xml
<!-- In AndroidManifest.xml -->
<uses-permission android:name="android.permission.BIND_VPN_SERVICE" />

<!-- The VPN service declaration -->
<service
    android:name=".FetchXVpnService"
    android:exported="false"
    android:permission="android.permission.BIND_VPN_SERVICE">
    <intent-filter>
        <action android:name="android.net.VpnService" />
    </intent-filter>
</service>
```

`android.permission.BIND_VPN_SERVICE` is a `signature`-level permission enforced by the system. It cannot be granted by the user; it is granted automatically when the VPN service is declared in the manifest.

### User approval flow — VpnService.prepare()

```kotlin
val intent = VpnService.prepare(context)
if (intent != null) {
    // User has not consented. Launch the system VPN consent dialog.
    startActivityForResult(intent, VPN_REQUEST_CODE)
} else {
    // Already consented; start VPN service directly.
    startVpnService()
}
```

`VpnService.prepare()` returns `null` if the user has already granted VPN consent for this application. Otherwise it returns an `Intent` that must be launched via `startActivityForResult`. The system dialog warns the user that the application will intercept all network traffic. Consent is persistent until the user revokes it in Android Settings → VPN → app entry.

Key rules:
- Consent must be obtained on the main thread (the Intent must be launched as an activity result).
- If another VPN application is active, the user will be asked whether to disconnect it first.
- `VpnService.prepare()` can throw a `SecurityException` in unusual OEM configurations.

### Service lifecycle and foreground service requirements

`VpnService` extends `Service`. Starting on Android 8.0 (API 26) — FetchX's `minSdk` — background services are subject to aggressive lifecycle restrictions. A VPN service that handles live traffic must run as a foreground service:

```kotlin
class FetchXVpnService : VpnService() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(NOTIF_ID, buildNotification())
        // ... setup VPN interface
        return START_STICKY
    }
}
```

Android 13 (API 33) requires declaring the foreground service type:
```xml
android:foregroundServiceType="specialUse"
```
`specialUse` is the only type that covers VPN-like network services not covered by `dataSync`, `mediaPlayback`, etc. Declaring `specialUse` on Google Play triggers policy review.

### Android 13/14/15 behaviour

**Android 13 (API 33):**
- Foreground service must declare a type in the manifest.
- `POST_NOTIFICATIONS` permission required for the foreground service notification.
- No change to VPN consent flow.

**Android 14 (API 34):**
- Foreground service type `specialUse` must accompany a Play Store justification at submission time.
- `VpnService` APIs themselves are unchanged but the Google Play policy review for `FOREGROUND_SERVICE` + `specialUse` combination has become more rigorous.
- A background-started foreground service (without user interaction) is further restricted; the VPN service start must be user-initiated.

**Android 15 (API 35):**
- No breaking `VpnService` API changes as of available documentation.
- Background `startForegroundService()` restrictions tightened further; apps must ensure VPN start happens in a foreground context (Activity or user-triggered notification action).
- `android.net.VpnService` itself is unchanged. `VpnService.Builder` API is stable.

### VpnService.Builder API

```kotlin
val builder = VpnService.Builder()
    .setSession("FetchX VPN")
    .addAddress("10.0.0.2", 24)           // Virtual IPv4 address
    .addRoute("0.0.0.0", 0)               // Route all IPv4 traffic
    .addDnsServer("9.9.9.9")              // Override DNS
    .setMtu(1500)
    .setBlocking(false)                    // Non-blocking tun fd
    .addAllowedApplication("dev.fetchx")   // Split tunnel: only FetchX
    // OR
    .addDisallowedApplication("com.other") // Split tunnel: exclude app

val fd: ParcelFileDescriptor = builder.establish()
// fd is the tun file descriptor. Packets arrive as IPv4/IPv6 datagrams.
// The service must read packets, route them, and write responses back.
```

`setBlocking(true)` makes read/write on the tun fd blocking; `false` uses non-blocking I/O.

### IPv4 and IPv6 routing

- `addRoute("0.0.0.0", 0)` routes all IPv4 traffic.
- `addRoute("::", 0)` routes all IPv6 traffic.
- Partial routes (e.g. `addRoute("192.168.0.0", 16)`) create split-tunnel by prefix.
- **Critical gap:** if IPv6 routes are not added but the device has IPv6 connectivity, IPv6 traffic bypasses the VPN entirely — an IPv6 leak.

### Split tunneling

```kotlin
builder.addAllowedApplication("dev.fetchx.app") // ONLY FetchX traffic → VPN
// All other app traffic → direct
```
OR
```kotlin
builder.addDisallowedApplication("com.example.exclusion") // This app → direct
// All other app traffic → VPN
```

The two methods are mutually exclusive on any given `Builder` instance. `addAllowedApplication` is more privacy-preserving from the user's perspective (only FetchX traffic is routed) but creates split-tunnel behaviour. `addDisallowedApplication` is suitable if FetchX implements a system-wide VPN and wants to exclude certain apps.

### Network changes — Wi-Fi to mobile transitions

`VpnService` receives `onRevoke()` when:
- The user revokes VPN consent.
- The device connects to a network that does not support VPN (some enterprise Wi-Fi).
- Another VPN application takes over.

Network transitions (Wi-Fi → mobile) do not automatically call `onRevoke()`, but the underlying tun fd may become invalid. Best practice:
1. Register a `ConnectivityManager.NetworkCallback` in the VPN service.
2. On `onAvailable()` / `onLost()`, re-establish the VPN interface by calling `builder.establish()` again (the old fd is closed first).

### Process death and service restart

`VpnService.onStartCommand()` should return `START_STICKY` so Android restarts the service after process death. On restart, the service must re-obtain VPN consent (calling `VpnService.prepare()` again) and rebuild the interface.

**Important:** a "kill switch" (blocking all traffic when VPN is down) cannot be implemented with `START_STICKY` alone; it requires `VpnService.Builder.setBlocking(true)` combined with an `onRevoke()` handler that establishes a blocking interface with no routes, holding all traffic until the VPN is re-established.

### Battery and performance impact

- The tun file descriptor requires an active read loop in a background thread.
- Every IP packet must be read from the tun fd, processed (proxied / decrypted / re-routed), and written back — doubling the CPU work per packet.
- Tor routing adds further latency (circuit establishment) on top of the raw forwarding overhead.
- Power management on Android 14+ may pause the tun read loop when Doze mode activates. The VPN service must hold a `WakeLock` during active downloads.
- Battery impact is measurable, typically 5–15% additional drain compared to direct connections for sustained download workloads.

### Security boundaries

- The tun fd provides raw packet access to all routed traffic. A bug in the packet processing code could expose unencrypted data, corrupt downloads, or introduce denial-of-service vulnerabilities.
- `VpnService` runs in the application process; a crash terminates the VPN without warning.
- Extensions running in GeckoView could, in principle, detect the VPN by observing network behaviour differences, but cannot access the tun fd directly.
- The VPN does not protect traffic occurring before the service is started or after `onRevoke()`.

### Should FetchX implement its own VpnService?

**NO — not in the current phase, and probably not in the next.**

Rationale: The primary use case (routing FetchX traffic through Tor/SOCKS5) can be accomplished through per-component proxy configuration without `VpnService`. Building a custom `VpnService` introduces:

- Kernel-level packet routing complexity.
- The `specialUse` foreground service type requiring Google Play review.
- Significant engineering effort to handle IPv6, DNS, split-tunneling, Doze, network transitions, and the kill switch correctly.
- A substantially larger security attack surface.

A custom `VpnService` becomes justified only if the goal is system-wide transparent routing (covering all apps, not just FetchX). Deferring this to a later dedicated phase is recommended.

---

## 4. Tor Research

### A. Bundled Tor

**Description:** Compile `libevent` + `tor` binary into the APK as a native library.

| Dimension | Detail |
|---|---|
| Lifecycle | Start Tor as a child process from the app; manage via control port (ControlPort). |
| Android compatibility | Requires NDK cross-compilation for arm64-v8a, armeabi-v7a, x86_64. Supported by the Tor Project's CI. |
| API availability | Control port (text protocol on a local TCP port) or the embedded library API. |
| Permissions | No extra Android permissions beyond `INTERNET`. |
| Binary size | ~5–10 MB for stripped arm64 tor binary + libevent. GeckoView already adds ~37 MB; this adds a further ~8–10 MB. |
| Battery usage | High — Tor circuits require periodic key exchange and keep-alive; the background circuit maintenance thread is active even during idle periods. |
| Startup latency | 5–30 seconds for initial circuit establishment on a first launch; 1–5 seconds for subsequent launches using cached directory info. |
| Bandwidth impact | Typically 30–70% throughput reduction vs. direct connections; highly variable depending on circuit quality and exit node load. |
| Reliability | Circuit establishment can fail; individual circuits can be closed by relays. The control port `NEWNYM` signal can request a new circuit. |
| Failure recovery | Tor itself handles relay failures by rebuilding circuits; application code must handle connection timeouts gracefully. |
| IPv4 behaviour | Tor is primarily an IPv4 network (the Tor SOCKS5 listener on 127.0.0.1:9050 accepts IPv4 connections from the app). |
| IPv6 behaviour | Tor supports IPv6 relays and exits but the local SOCKS5 listener is IPv4 only. IPv6-only destination hostnames work via DNS resolution at the exit node. |
| DNS behaviour | DNS resolved at exit node — not locally. DNS leaks via the SOCKS5 interface are only possible if the caller resolves DNS locally before passing the IP address to SOCKS5. |
| Maintenance burden | Very high. Every Tor release must be reviewed, re-compiled, tested, and packaged. Security patches arrive frequently. |
| Security model | Strong — no single relay sees both source and destination. Exit node sees destination but not source IP. |
| Trust assumptions | Trust distributed across three relays. No single trusted party. Exit node can observe unencrypted traffic (HTTP, non-HTTPS). |
| Licensing implications | Tor is BSD-3-Clause. `libevent` is BSD-2-Clause. Both are permissive. However bundling tor means FetchX becomes responsible for its security maintenance. |

**Conclusion:** Not recommended. The maintenance burden and binary size cost are not justified when Orbot provides the same capability without bundling.

---

### B. External Tor Application

**Description:** Require the user to install a separate Tor app (e.g. Orbot, Tor Browser) and instruct FetchX to connect to `127.0.0.1:9050`.

| Dimension | Detail |
|---|---|
| Lifecycle | Entirely controlled by the external app. FetchX has no lifecycle control. |
| Android compatibility | Works on all Android versions. The SOCKS5 connection is just a TCP socket. |
| API availability | No API; FetchX attempts a SOCKS5 connection and handles failure. |
| Permissions | `INTERNET` only. |
| Binary size | Zero addition to FetchX APK. |
| Battery usage | FetchX adds zero battery overhead for Tor itself; Orbot running in the background has its own impact. |
| Startup latency | Tor circuit must already be established before FetchX attempts to use it. If Orbot is not running, connections will fail. |
| Bandwidth impact | Same as Option A — determined by Tor network conditions. |
| Reliability | Fragile — FetchX has no way to know whether Orbot is running or whether circuits are established. |
| Failure recovery | FetchX must catch `ConnectException` and surface a user-readable error ("Tor connection unavailable. Is Orbot running?"). |
| IPv4/IPv6/DNS | Same as Bundled Tor — DNS at exit node; IPv4 local SOCKS5 socket. |
| Maintenance burden | Very low for FetchX. The external app is responsible for Tor maintenance. |
| Security model | Same as Option A when external app is trusted. The external app's security posture matters. |
| Trust assumptions | Full trust in the external Tor app. A malicious or compromised Orbot would expose all FetchX traffic. |
| Licensing implications | FetchX has no dependency on Orbot's GPL-3.0 license; FetchX merely connects to a SOCKS5 socket. GPL-3.0 contamination risk: NONE (socket connection is not linking). |

**Conclusion:** Acceptable but fragile. Better as part of Option C (per-component proxy) where the SOCKS5 address is user-configurable and the source of the Tor service is not hardcoded.

---

### C. Orbot-Style Integration

**Description:** FetchX checks whether Orbot is installed; if so, offers a "Route via Tor (Orbot)" option. Optionally uses Orbot's Intent API to start Orbot before connecting.

| Dimension | Detail |
|---|---|
| Lifecycle | Orbot lifecycle management via its documented `Intent` API (`OrbotHelper` from `info.guardianproject:netcipher`). |
| Android compatibility | Android 5.0+ (Orbot's minSdk). |
| API availability | `OrbotHelper.requestStartTor(context)` sends an `Intent` to start Orbot and returns the SOCKS5 port. Available via NetCipher (Apache 2.0) or via direct `Intent` construction. |
| Permissions | `INTERNET` only. The Orbot Intent does not require a special permission. |
| Binary size | NetCipher adds ~100–300 KB. Or zero if the Intent is constructed directly without the library. |
| Battery usage | Same as Option B for Tor itself; NetCipher adds no measurable overhead. |
| Startup latency | Orbot circuit establishment is the bottleneck (same 5–30 seconds). FetchX should wait for Orbot's `STATUS_ON` broadcast before attempting SOCKS5 connections. |
| Bandwidth impact | Same as Option A/B. |
| Reliability | Better than Option B — FetchX can detect when Orbot is not installed or not running and surface a meaningful error. |
| Failure recovery | FetchX can listen for Orbot's `STATUS_OFF` broadcast and pause Tor-routed downloads or switch to a fallback. |
| IPv4/IPv6/DNS | Same as Option A — DNS at exit node; no local DNS leaks via SOCKS5 if host names (not IPs) are passed. |
| Maintenance burden | Low for FetchX. Orbot handles the Tor binary. NetCipher provides stable Intent wrappers. |
| Security model | Strong for Tor routing. FetchX trusts Orbot; the user trusts the Guardian Project. |
| Trust assumptions | Trust in Orbot / Guardian Project. Orbot is open-source (GPL-3.0). |
| Licensing implications | NetCipher has dual licensing (Apache 2.0 for the proxy helper components; GPL-3.0 for some network components). Careful selection of only Apache-licensed components is required. The Orbot Intent approach (without NetCipher) has zero licensing overhead. |

**Conclusion:** This is the recommended Tor integration approach for a future phase. It delivers Tor routing without bundling Tor and with minimal FetchX maintenance overhead.

---

### D. Local SOCKS5 Tor Service

**Description:** Bundle a minimal SOCKS5 proxy server in-process. FetchX connects all traffic to this local SOCKS5 server, which forwards to Tor.

| Dimension | Detail |
|---|---|
| Lifecycle | The in-process SOCKS5 server starts with the app; outbound Tor connection is established via Option A or C. |
| Android compatibility | Any Android version. |
| API availability | Pure Java/Kotlin implementation of SOCKS5; no special API required. |
| Permissions | `INTERNET` only. |
| Binary size | A minimal SOCKS5 server in Kotlin adds ~50–100 KB. |
| Battery usage | Adds one more hop for every packet; small but measurable overhead. |
| Startup latency | Same as underlying Tor approach. |
| Bandwidth impact | Adds an extra loopback hop; negligible for a localhost-to-localhost connection. |
| Reliability | Depends on underlying Tor availability. |
| Failure recovery | The in-process server can detect Tor unavailability and return SOCKS5 connection refused errors. |
| IPv4/IPv6/DNS | Depends on underlying Tor relay. |
| Maintenance burden | Low. The in-process SOCKS5 server is simple and does not need Tor updates. |
| Security model | No improvement over directly connecting to Orbot's SOCKS5 port. Adds complexity without privacy benefit. |
| Trust assumptions | Same as underlying Tor approach. |
| Licensing implications | None if written in-house. |

**Conclusion:** Unnecessary complexity. A local in-process SOCKS5 relay adds no security value compared to directly configuring each component to use Orbot's SOCKS5 port.

---

### E. VPN-over-Tor Architecture

**Description:** Implement a `VpnService` that routes all FetchX traffic (or all-app traffic) through Tor. The VPN service reads raw IP packets, wraps them as TCP streams, and forwards them to the Tor SOCKS5 proxy.

| Dimension | Detail |
|---|---|
| Lifecycle | `VpnService` + underlying Tor (via Orbot or bundled). Both must be running. |
| Android compatibility | API 14+ (`VpnService`). FetchX minSdk is 26. |
| API availability | `android.net.VpnService`. Tor access via Orbot (Option C) or bundled (Option A). |
| Permissions | `android.permission.BIND_VPN_SERVICE` + Tor access. |
| Binary size | No additional binary if using Orbot; +8–10 MB if bundling Tor. |
| Battery usage | High — VPN read loop + Tor overhead. |
| Startup latency | 5–30 seconds for circuit establishment + VPN interface bring-up (~1–2 seconds). |
| Bandwidth impact | High — all packets processed twice (VPN + Tor). |
| Reliability | Complex — VPN can be revoked by Android, by another VPN app, or by enterprise policy. |
| Failure recovery | Must handle `onRevoke()`, network transitions, and Tor circuit failures independently. |
| IPv4/IPv6 | Must handle IPv6 explicitly to avoid leaks (see Section 3). |
| DNS | DNS resolved at Tor exit node if SOCKS5 remote-DNS is used; otherwise DNS leaks through the VPN. |
| Maintenance burden | Very high. Two complex systems (VPN + Tor) must be maintained together. |
| Security model | Strongest option — no traffic bypasses Tor. But only as good as the implementation. IPv6 leaks, DNS leaks, and VPN revocation events create gaps. |
| Trust assumptions | Trust in Tor network and exit nodes. No new VPN provider trust assumption (FetchX owns the tunnel). |
| Licensing implications | No new licensing issues if using Orbot approach. |

**Conclusion:** The highest-privacy option but also the highest-complexity option by far. Not appropriate until simpler per-component proxy routing is proven stable. Should not be implemented before Phase 20M + several subsequent phases.

---

## 5. GeckoView 137 Research

GeckoView `137.0.20250325171416` is the version pinned in FetchX. The following assessments are based on the published GeckoView 137 JavaDoc and source (`mozilla-central` branch at that revision), cross-referenced against `firefox-source-docs.mozilla.org`.

### GeckoRuntimeSettings proxy configuration method

**UNVERIFIED:** `GeckoRuntimeSettings.Builder` — no `setProxySettings()`, `setProxy()`, or equivalent proxy configuration method appears in the GeckoView 137 JavaDoc at `https://mozilla.github.io/geckoview/javadoc/mozilla-central/`. GeckoView does not expose a dedicated Java-level proxy API at the `GeckoRuntimeSettings` level. Proxy configuration in Gecko is handled at the C++ preference layer (`network.proxy.type`, `network.proxy.socks`, `network.proxy.socks_port`) and is not exposed via a public GeckoView 137 API.

**NOT SUPPORTED:** Direct `GeckoRuntimeSettings.Builder.proxy(...)` call — no such method exists in the GeckoView 137 public API surface.

### Per-session proxy vs runtime-level proxy

**UNVERIFIED:** Per-session proxy override via `GeckoSession.Settings` — no per-session proxy configuration exists in `GeckoSession.Settings` in GeckoView 137. Gecko's internal networking uses a single process-wide proxy configuration; per-session overrides are not exposed at the GeckoView API level.

**UNVERIFIED:** Runtime-level proxy via `GeckoRuntime.settings` — while Gecko internally reads `network.proxy.type` / `network.proxy.socks`, the GeckoView Java API does not expose a setter for these preferences in version 137. It may be possible to set Gecko preferences directly via the undocumented `GeckoRuntime.settings.setPreference()` approach used in Firefox for Android (Fenix), but this is an internal API not documented in GeckoView 137's public JavaDoc and is subject to change without notice.

### GeckoSession.Settings proxy options

**NOT SUPPORTED:** `GeckoSession.Settings` does not contain proxy configuration fields in GeckoView 137. The class exposes `usePrivateMode()`, `useTrackingProtection()`, `userAgentMode()`, `displayMode()`, etc., but no proxy settings.

### WebExtension browser.proxy API support

**UNVERIFIED:** The `browser.proxy` WebExtensions API is available in Firefox Desktop but its availability in GeckoView 137 specifically requires verification. GeckoView supports a subset of the WebExtensions API. The `browser.proxy` API would allow an installed extension to configure proxy routing for browser traffic via `browser.proxy.settings.set()`. If available, this would be the most flexible proxy configuration path for the browser component. It requires an installed extension with the `proxy` permission, which in FetchX's current Phase 20L permission model would be subject to explicit user approval.

### SOCKS5 proxy support

**UNVERIFIED:** GeckoView 137 does not expose a public Java API for SOCKS5 proxy configuration. Gecko itself supports SOCKS5 via the `network.proxy.socks` + `network.proxy.socks_port` + `network.proxy.type = 1` (manual) preferences. These preferences could potentially be set via GeckoRuntime's internal preference mechanism, but this is not a documented GeckoView 137 API.

### HTTP proxy support

**UNVERIFIED:** Same situation as SOCKS5. Gecko supports `network.proxy.http` internally. No public GeckoView 137 Java API for HTTP proxy configuration has been confirmed.

### Proxy authentication support

**NOT SUPPORTED (via public API):** GeckoView 137 does not expose proxy authentication credential management through its public Java API.

### Runtime proxy reconfiguration (change without restart)

**UNVERIFIED:** Whether Gecko preferences for proxy settings can be changed at runtime (after `GeckoRuntime` creation) without recreating the runtime is uncertain. GeckoView creates the Gecko engine in a separate child process; preference changes that affect networking may require signalling across process boundaries. This needs verification.

### Per-tab/per-session proxy isolation

**NOT SUPPORTED:** GeckoView 137 does not support per-session or per-tab proxy configuration. All sessions share the process-wide Gecko networking stack.

### Summary

| API | Status |
|---|---|
| `GeckoRuntimeSettings.Builder` proxy method | NOT SUPPORTED via public API |
| Per-session proxy configuration | NOT SUPPORTED |
| `GeckoSession.Settings` proxy options | NOT SUPPORTED |
| WebExtension `browser.proxy` API | UNVERIFIED — needs testing in GeckoView 137 |
| SOCKS5 proxy support (via internal preferences) | UNVERIFIED — internal only, not public API |
| HTTP proxy support (via internal preferences) | UNVERIFIED — internal only, not public API |
| Proxy authentication | NOT SUPPORTED via public API |
| Runtime proxy reconfiguration | UNVERIFIED |
| Per-tab/per-session proxy isolation | NOT SUPPORTED |

**Key implication:** GeckoView 137 does not provide a documented, stable public Java API for proxy configuration. The most viable browser-level proxy approach for a future phase is the WebExtension `browser.proxy` API — if it is available in GeckoView 137. This requires installing a bundled extension with the `proxy` permission and using the `browser.proxy.settings.set({ proxyType: "manual", socks: "127.0.0.1", socksPort: 9050, proxyDNS: true })` call. Verification against GeckoView 137's extension API surface is required before implementing.

---

## 6. Downloader Research

### Current Rust downloader architecture

The Rust downloader crate exposes a `DownloadEngine` trait:

```rust
pub trait DownloadEngine: Send + Sync {
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError>;
}
```

The `EngineDispatcher` selects among `DirectHttpEngine`, `Aria2Engine`, and `YtDlpEngine` based on the `EngineKind` stored with the download item. None of the engines currently accept a proxy configuration.

`DirectHttpEngine` uses `reqwest` with `rustls-tls` but without the `socks` feature. No proxy client is created.

`Aria2Engine` communicates with an aria2 process via JSON-RPC over localhost; aria2 itself supports `--all-proxy` and per-download proxy options via its RPC API.

`YtDlpEngine` launches `yt-dlp` as a subprocess; yt-dlp supports `--proxy` as a command-line argument for SOCKS5/HTTP proxy.

`FfmpegProcessor` launches FFmpeg as a subprocess; FFmpeg supports an HTTP proxy via the `http_proxy` environment variable but does not natively support SOCKS5.

### Routing model evaluation

#### A. Browser-only routing
- Only GeckoView traffic is routed through the proxy.
- Rust downloader traffic goes direct regardless of user settings.
- **Assessment:** Partial privacy; downloads initiated from the browser still reveal the real IP to download servers.

#### B. Downloader-only routing
- Only Rust download engine traffic is proxied; GeckoView traffic goes direct.
- **Assessment:** Inverted privacy profile; browsing leaks real IP; downloads are proxied.

#### C. Application-wide routing via VpnService
- A `VpnService` intercepts all traffic at the IP layer.
- **Assessment:** Most complete coverage but requires implementing a kernel-level packet router. See Section 3 for complexity and risk.

#### D. Per-download routing
- Each download carries a `NetworkRoute` field; the engine configures its transport accordingly.
- Requires per-engine proxy implementation.
- **Assessment:** Most granular user control. Allows "download this file via Tor" for sensitive items while using a direct connection for high-speed downloads. Aligns with the existing `NetworkRoute` domain model in `:core`.

#### E. Per-engine routing
- Each engine class is configured with a proxy at construction time.
- **Assessment:** Cleaner than per-download if the intent is a global proxy setting; less granular than per-download.

#### F. User-selectable routing profiles
- "Direct", "Always Tor", "Tor for browser only", "SOCKS5 proxy" — saved as user settings.
- **Assessment:** Best user experience. Implemented on top of either D or E.

### Recommendation

**Option D (per-download routing) implemented through user-selectable profiles (Option F)** best fits the existing architecture. Rationale:
- The `NetworkRoute` sealed class in `:core` already models this at the domain level.
- `DownloadItem` can carry a `route: NetworkRoute` field (already anticipated in architecture documentation).
- Each engine can be independently adapted: `reqwest` + `socks` feature for DirectHttp, `--all-proxy` arg for aria2, `--proxy` arg for yt-dlp.
- This imposes zero coupling between engines (they each handle their own transport; the engine trait contract is unchanged).
- FFmpeg receives proxy settings via environment variable, which is isolated from the engine trait.
- User-selectable profiles translate a single user choice into per-engine `NetworkRoute` values at dispatch time.

---

## 7. Rust Architecture Impact

### CURRENT ARCHITECTURE

```
EngineDispatcher
    ├── DirectHttpEngine  →  reqwest::Client (rustls-tls, no proxy)
    ├── Aria2Engine       →  aria2 process (no proxy arg)
    └── YtDlpEngine       →  yt-dlp process (no proxy arg)

DownloadEngine trait:
    async fn download(id, cancellation) → Result<DownloadItem, Error>

No NetworkRoute in the Rust type system.
No proxy configuration anywhere in the downloader crate.
```

### PROPOSED FUTURE ARCHITECTURE (NOT IMPLEMENTED IN PHASE 20M)

Add a `NetworkRoute` enum to `crates/models` (the canonical shared type layer):

```rust
// crates/models/src/network_route.rs  [FUTURE]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkRoute {
    Direct,
    HttpProxy { host: String, port: u16, auth: Option<ProxyAuth> },
    Socks5   { host: String, port: u16, auth: Option<ProxyAuth> },
    Tor      { socks_port: u16 },  // connects to 127.0.0.1:socks_port (Orbot)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,  // should be zeroized on drop — use `secrecy` crate
}
```

Update `DownloadItem` to carry an optional `network_route: Option<NetworkRoute>` field. `None` means `Direct`.

Update the `DownloadEngine` trait to pass routing context:

```rust
// crates/downloader/src/engine.rs  [FUTURE]
pub trait DownloadEngine: Send + Sync {
    async fn download(
        &self,
        id: DownloadId,
        route: &NetworkRoute,        // ← new parameter
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError>;
}
```

Per-engine adaptations (NOT IMPLEMENTED):

```rust
// DirectHttpEngine: build reqwest client per-download based on route
fn build_client(route: &NetworkRoute) -> reqwest::Client {
    match route {
        NetworkRoute::Direct => reqwest::Client::builder().build()?,
        NetworkRoute::Socks5 { host, port, .. } =>
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(format!("socks5://{}:{}", host, port))?)
                .build()?,
        NetworkRoute::Tor { socks_port } =>
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(format!("socks5://127.0.0.1:{}", socks_port))?)
                .build()?,
        NetworkRoute::HttpProxy { host, port, .. } =>
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(format!("http://{}:{}", host, port))?)
                .build()?,
    }
}
// Requires adding "socks" feature to reqwest workspace dependency
```

```rust
// Aria2Engine: pass --all-proxy as a per-download aria2 RPC option [FUTURE]
// YtDlpEngine: add --proxy argument to the yt-dlp subprocess args [FUTURE]
// FFmpegProcessor: set http_proxy environment variable [FUTURE]
```

### Why this design avoids the identified risks

- **No duplicated networking logic:** Each engine implements exactly one transport path; the route enum is passed as a parameter.
- **No engine coupling:** Engines do not know about each other; they each resolve their own route configuration.
- **Resume/segmentation/retry safe:** The route is stored with the `DownloadItem`; resume picks up the same route; segmented downloads share the same configured client.
- **No global mutable state:** A new `reqwest::Client` is built per download (or cached per route config per engine instance). No global proxy setting is mutated.
- **aria2 and yt-dlp unaffected at trait level:** Proxy config is passed as structured arguments to the external process, not via shared state.

---

## 8. Privacy Analysis

### DNS leaks

**Why it matters:** If an application resolves a hostname using the system DNS resolver before passing the resolved IP to a proxy, the DNS query travels in the clear and reveals the hostname to the ISP/Wi-Fi operator — regardless of the proxy.

**Potential exposure:** The visited hostname is revealed to the DNS observer.

**Future mitigation:** SOCKS5 supports remote DNS resolution (`SOCKS5` with `ATYP = 0x03` — domain name). `reqwest`'s `socks` feature sends the hostname to the SOCKS5 server for resolution by the proxy, preventing local DNS queries. This must be verified and tested; some client libraries resolve DNS locally before SOCKS5 even when a hostname is provided. aria2 has an `--async-dns=false` flag that forces synchronous resolution (via system resolver) — this may still leak DNS. Verification of each engine's DNS resolution path is required.

**Remaining limitations:** System services outside FetchX (OS, other apps) may issue DNS queries independently.

---

### IPv6 leaks

**Why it matters:** If a download destination has an IPv6 address and the device has IPv6 connectivity, the connection may bypass a SOCKS5 proxy that only intercepts IPv4 traffic.

**Potential exposure:** Real IPv6 address visible to the destination server and intermediate network.

**Future mitigation:** Per-component proxy approach (Option C): `reqwest` with `socks` feature handles IPv6 destinations by passing the hostname to the SOCKS5 server. The SOCKS5 server resolves IPv4 vs. IPv6 at the exit. Direct IPv6 connections do not occur.

**Remaining limitations:** aria2 launched with `--all-proxy=socks5://…` may still attempt IPv6 connections directly for destinations not handled by the proxy. Needs per-engine testing.

---

### WebRTC leaks (GeckoView-specific)

**Why it matters:** WebRTC ICE candidates include the device's real IP address, which can be observed by any web page using WebRTC even when a proxy is configured.

**Potential exposure:** Real IP address exposed to any web page that initiates a WebRTC session.

**Future mitigation:** GeckoView exposes `GeckoRuntimeSettings.webRtcIpHandling` which FetchX's `ANDROID_PRIVACY.md` notes as "not configured." Setting the policy to `DISABLE_NON_PROXIED_UDP` or `TURN_ONLY` prevents WebRTC from revealing the real IP. This is a GeckoView API that can be set without proxy configuration. **This should be configured in a future privacy phase regardless of VPN/Tor status.**

**Remaining limitations:** Even with `DISABLE_NON_PROXIED_UDP`, WebRTC connections themselves bypass a SOCKS5 proxy because WebRTC uses UDP and SOCKS5 does not tunnel UDP.

---

### QUIC / HTTP/3 considerations

**Why it matters:** HTTP/3 uses QUIC, which runs over UDP. SOCKS5 does not tunnel UDP. A SOCKS5 proxy cannot intercept QUIC traffic; the browser falls back to HTTP/2 or HTTP/1.1 when QUIC is blocked, but only if the fallback is available.

**Potential exposure:** QUIC connections bypass SOCKS5 entirely, connecting directly. The server sees the real IP.

**Future mitigation:** GeckoView supports disabling QUIC via the `network.http.http3.enabled` preference. If a SOCKS5 proxy is active, FetchX should disable HTTP/3 to prevent bypass. This can potentially be set via GeckoRuntime's internal preference API.

**Remaining limitations:** Even with HTTP/3 disabled, network-level QUIC from other components could still occur.

---

### UDP traffic handling

**Why it matters:** SOCKS5 supports UDP relay (`UDP ASSOCIATE` command) but it is rarely implemented by Tor and Orbot does not support UDP relay. Any UDP traffic from FetchX bypasses Orbot's SOCKS5.

**Potential exposure:** DNS (port 53 UDP), QUIC, and other UDP traffic goes direct.

**Future mitigation:** Disable UDP-based protocols (QUIC, WebRTC UDP) when Tor routing is active. For DNS, ensure hostname-based SOCKS5 resolution (remote DNS at the proxy).

**Remaining limitations:** Some application components may issue UDP traffic that cannot be suppressed at the application level without a `VpnService`.

---

### Direct connection fallback

**Why it matters:** Many networking libraries have "happy eyeballs" or automatic fallback logic that attempts a direct connection when a proxy fails. If FetchX falls back silently, the user believes traffic is proxied when it is not.

**Potential exposure:** Real IP revealed on proxy failure.

**Future mitigation:** Disable automatic fallback. When a proxy connection fails, surface an explicit error rather than attempting direct connection. This must be enforced at each engine level.

**Remaining limitations:** Some library-level fallback is difficult to disable fully. Testing is required per engine.

---

### Proxy bypass

**Why it matters:** Operating system features (e.g., direct connectivity tests, system DNS, NTP) may issue connections that bypass the application-level proxy.

**Potential exposure:** System-level traffic visible to the network observer.

**Future mitigation:** A `VpnService` (Option B) would capture OS-level traffic. Application-level proxy (Option C) cannot intercept OS traffic.

**Remaining limitations:** Per-component proxy cannot protect OS traffic. Only a `VpnService` can.

---

### Split-tunnel mistakes

**Why it matters:** If a `VpnService` is implemented and split-tunneling is configured incorrectly, some FetchX traffic may go direct while the user believes it is proxied.

**Potential exposure:** Partial IP leak.

**Future mitigation:** Careful testing of `addAllowedApplication` / `addDisallowedApplication` configuration. Add integration tests for split-tunnel behaviour.

**Remaining limitations:** Android's split-tunnel configuration is app-level; it does not granularly control individual sockets within the FetchX process.

---

### Application exclusions

**Why it matters:** When FetchX implements a VPN (future phase), excluding certain applications from the VPN tunnel is a feature but also a potential privacy risk if configured incorrectly.

**Potential exposure:** Excluded apps reveal their real IP even while FetchX's VPN is active.

**Future mitigation:** Default to routing all apps through the VPN with explicit opt-out, rather than routing no apps by default with explicit opt-in.

**Remaining limitations:** Users may exclude apps without understanding the privacy implications.

---

### Background services

**Why it matters:** Android background services (WorkManager jobs, periodic syncs) may run while the VPN/proxy is not yet active.

**Potential exposure:** Background download resumption could use direct connections.

**Future mitigation:** The FetchX download foreground service should check the current `NetworkRoute` setting before connecting and wait for the proxy to be available if required.

**Remaining limitations:** Android may relaunch background work at a time when the VPN service is not running.

---

### Stale routes

**Why it matters:** If the network route setting changes (e.g. user disables Tor mid-download), active connections may continue on the old route or new segments may use the new route, creating a mixed-route download.

**Potential exposure:** Partial download from real IP, partial from Tor.

**Future mitigation:** Route changes should either (a) pause all active downloads and resume with the new route, or (b) apply only to new downloads. The behaviour must be explicit and documented.

**Remaining limitations:** Abruptly closing an active `reqwest` connection mid-download causes segment loss and requires re-download.

---

### Network transitions

**Why it matters:** A Wi-Fi → mobile network transition tears down TCP connections. Resume logic may reconnect without re-applying proxy configuration.

**Potential exposure:** Post-transition reconnect uses direct connection.

**Future mitigation:** The resume path must re-verify the `NetworkRoute` of the download item and rebuild the proxy-configured client before reconnecting.

**Remaining limitations:** The retry window during network transition may involve one unproxied connection attempt before the error is detected.

---

### VPN crashes

**Why it matters:** If a future `VpnService` implementation crashes, the Android system closes the tun interface. Depending on the service's restart behaviour, traffic may go direct during the gap.

**Potential exposure:** Direct connection during VPN outage.

**Future mitigation:** Implement a "kill switch" (`START_STICKY` + blocking interface on restart + `setBlocking(true)` to hold traffic until VPN is re-established).

**Remaining limitations:** A true kill switch with blocking is complex to implement correctly on Android.

---

### Tor circuit changes

**Why it matters:** Tor assigns each circuit a new exit node on `NEWNYM` or on circuit rebuild. Active persistent connections (e.g. segmented downloads) are tied to a specific circuit. A circuit change terminates the connection.

**Potential exposure:** Download interruption; new circuit may reveal a different "exit IP" to the server, potentially correlating the two circuit identities.

**Future mitigation:** Avoid issuing `NEWNYM` while active downloads are in progress. Treat circuit changes as connection failures and resume from the last byte offset.

**Remaining limitations:** Tor circuits can be closed by relays without application control.

---

### Captive portals

**Why it matters:** Captive portal detection and authentication requires direct HTTP connections that bypass any proxy. If a SOCKS5 proxy is active, captive portal login pages may be unreachable.

**Potential exposure:** The device is connected to a captive portal network but cannot authenticate. Network is unusable.

**Future mitigation:** Detect captive portal state (`ConnectivityManager.NetworkCapabilities.NET_CAPABILITY_CAPTIVE_PORTAL`) and temporarily disable proxy routing during captive portal authentication.

**Remaining limitations:** Captive portal detection is only fully reliable on Android 9+.

---

### DNS fallback

**Why it matters:** When a SOCKS5 proxy is unavailable, some clients fall back to system DNS + direct connection. This reveals the hostname and the real IP.

**Potential exposure:** See "Direct connection fallback" above.

**Future mitigation:** Same as direct connection fallback — disable fallback explicitly.

---

## 9. Security Analysis

### New attack surface introduced by VPN/proxy support

- **Proxy configuration as an attack vector:** If proxy settings can be changed via an extension's `browser.proxy` API, a malicious extension could route browser traffic through an attacker-controlled proxy. FetchX's extension permission model (Phase 20L) should treat the `proxy` permission as a high-sensitivity permission requiring explicit user approval and disclosure.
- **SOCKS5 server trust:** When FetchX connects to a user-configured SOCKS5 server, that server can observe all unencrypted FetchX traffic (HTTP downloads). FetchX should not downgrade HTTPS connections.
- **Credential handling for proxy auth:** If proxy authentication credentials are supported (future), they must be stored in Android `EncryptedSharedPreferences` or the Android `Keystore`, not in plain SQLite. The `secrecy` Rust crate should be used for the Rust-side proxy credential type to ensure memory is zeroed on drop.

### Privilege escalation risks

- `android.permission.BIND_VPN_SERVICE` is a system-level permission. Combined with `startForegroundService()` + `specialUse` type, Google Play review scrutiny is high.
- A `VpnService` that processes raw IP packets introduces buffer-overflow risk if packet parsing is done in Rust (without careful bounds checking) or in unsafe Java/Kotlin code.
- The VPN service must not expose its tun fd to other processes or to extension content.

### Extension-based proxy abuse potential

GeckoView 137 may support the `browser.proxy` WebExtensions API. If FetchX enables this API for installed extensions:
- An extension with the `proxy` permission can reroute ALL GeckoView browser traffic to any SOCKS5 or HTTP endpoint.
- FetchX must require user confirmation before granting the `proxy` permission to any extension, with a clear disclosure ("This extension can intercept all browser network requests").
- Extensions without the `proxy` permission must be unable to affect proxy configuration.

### Key management

No cryptographic keys are required for SOCKS5 or Tor routing. If HTTP proxy authentication credentials are added, they must be protected. If a VPN tunnel with TLS certificate pinning is added in a future phase, certificate rotation and pinning bypass policy must be defined.

### Code audit requirements

Before shipping any VPN or proxy routing:
- All path-handling code (SOCKS5 host, port) must be validated against injection attacks.
- Port numbers must be validated as 1–65535.
- Host strings must not contain path separators, newlines, or null bytes.
- No proxy configuration must be logged (even at DEBUG level) in a way that exposes server hostnames or credentials.

### Dependency supply chain

- Adding `reqwest` SOCKS5 support adds the `socks` crate (part of the `tokio-socks` ecosystem). This crate should be reviewed before adding.
- Adding NetCipher (for Orbot Intent helpers) introduces an additional Android library dependency. The Apache-licensed components should be used; GPL-licensed components should be avoided.
- Any Tor-adjacent binary (if bundled) must be obtained from the official Tor Project release mirrors and verified against signed checksums.

---

## 10. Threat Model

### Threat 1: ISP/Network Observer

**Attacker capability:** Passive observation of all network traffic leaving the device. Can see source IP, destination IP, SNI (TLS server name), and DNS queries. Cannot decrypt TLS payload.

**Information exposed (Direct connection):** Visited hostnames (via SNI and DNS), download server IPs, timing, data volume.

**Mitigation:** SOCKS5 proxy via Tor — Tor encrypts traffic between the device and the entry guard. The ISP sees only an encrypted connection to the Tor guard node.

**Remaining limitation:** The ISP knows FetchX is using Tor (Tor traffic is recognisable). Tor bridges with obfuscation (obfs4) can mitigate this but are not in scope for FetchX.

---

### Threat 2: Wi-Fi Operator

**Attacker capability:** Same as ISP observer for traffic on the local network segment. Can also perform ARP poisoning and MITM attacks on unencrypted connections.

**Information exposed:** Same as ISP for observation. Unencrypted downloads (HTTP) can be inspected and tampered with if no proxy is used.

**Mitigation:** Tor routing encrypts all traffic leaving the device. HTTPS-only mode (already configurable in FetchX) prevents HTTP downloads via the browser.

**Remaining limitation:** HTTP direct downloads (not browser) bypass HTTPS-only mode; the downloader should enforce HTTPS for Tor-routed requests.

---

### Threat 3: DNS Observer

**Attacker capability:** Observes DNS queries from the device. May operate the DNS server configured by the Wi-Fi network.

**Information exposed:** Every hostname FetchX resolves locally is visible.

**Mitigation:** Remote DNS resolution via SOCKS5 (pass hostname to proxy, not IP). This requires verifying that each engine passes hostnames, not pre-resolved IPs, to the SOCKS5 proxy.

**Remaining limitation:** System DNS queries (OS-level, non-FetchX) are not protected by per-component proxy.

---

### Threat 4: Malicious Local Network

**Attacker capability:** Can perform active attacks: ARP poisoning, rogue DHCP, fake certificate injection (for non-pinned HTTPS), DNS hijacking.

**Information exposed:** Unencrypted traffic contents; redirected connections.

**Mitigation:** Tor routing bypasses local DNS and routes through the Tor guard node. HTTPS enforced by the Rust downloader and GeckoView prevents MITM of TLS connections.

**Remaining limitation:** A rogue certificate authority installed on the device (by the user or MDM) could still MITM TLS. FetchX does not perform certificate pinning.

---

### Threat 5: Malicious VPN Provider

**Attacker capability:** If a user installs a third-party VPN app and FetchX uses `NetworkRoute.SystemVpn`, the VPN provider sees all FetchX traffic (source IP, destination, content of unencrypted connections).

**Information exposed:** All routed traffic; timing; destinations.

**Mitigation:** FetchX's `SystemVpn` route option is passive (the VPN is the user's responsibility). FetchX should document that it trusts the system VPN provider when this route is selected. Using `Tor` route avoids VPN provider trust.

**Remaining limitation:** FetchX cannot audit a third-party VPN. User education is the only mitigation.

---

### Threat 6: Malicious Proxy

**Attacker capability:** A malicious or compromised SOCKS5 proxy (user-configured or via extension abuse) can observe all traffic passing through it, log destinations and timing, and inject content into unencrypted connections.

**Information exposed:** Destinations, timing, unencrypted content.

**Mitigation:** Orbot is an open-source proxy with a known trust model (Guardian Project / Tor Project). For user-configured SOCKS5 proxies, FetchX should document that the user trusts the proxy operator. Extension-based proxy injection must require explicit user permission (see Section 9).

**Remaining limitation:** FetchX cannot audit user-configured SOCKS5 servers.

---

### Threat 7: Malicious Tor Exit Node

**Attacker capability:** The Tor exit node can observe the destination address and the content of unencrypted (HTTP) traffic. It cannot see the source IP (the entry guard knows the source, not the exit).

**Information exposed:** Download server hostnames and IP addresses; HTTP content if not using HTTPS; timing.

**Mitigation:** Enforce HTTPS for all downloads when Tor routing is active. Do not fall back to HTTP over Tor.

**Remaining limitation:** Even with HTTPS, the exit node knows what servers FetchX is downloading from (SNI visible in TLS). End-to-end encryption (TLS) prevents content inspection.

---

### Threat 8: Browser Direct-Connection Leak

**Attacker capability:** Any web page can initiate network connections (fetch, XHR, WebRTC, link prefetching) that may bypass a SOCKS5 proxy if the proxy is not configured at the Gecko network layer.

**Information exposed:** Real IP address via any direct connection.

**Mitigation (Future):** Configure GeckoView's proxy via the internal preference API or the WebExtension `browser.proxy` API (whichever is confirmed available in GeckoView 137). Disable WebRTC non-proxied UDP (`webRtcIpHandling = DISABLE_NON_PROXIED_UDP`). Disable HTTP/3/QUIC when proxy is active.

**Remaining limitation:** Until GeckoView proxy configuration is verified and implemented, the browser component routes all traffic directly regardless of the user's route preference. This is the most significant current gap.

---

### Threat 9: Downloader Direct-Connection Leak

**Attacker capability:** When Tor route is selected by the user but a download engine has not yet been updated to use SOCKS5, the download proceeds over the direct connection.

**Information exposed:** Real IP to the download server.

**Mitigation (Future):** Enforce route configuration at the `EngineDispatcher` level; a download with `NetworkRoute.Tor` should fail (not fall back) if the engine cannot configure a SOCKS5 proxy.

**Remaining limitation:** Until per-engine SOCKS5 is implemented, all routes are effectively `Direct` in the Rust downloader.

---

### Threat 10: Application Crash

**Attacker capability:** Passive. The device continues connecting directly during the crash and recovery period.

**Information exposed:** Real IP during the outage.

**Mitigation:** No proxy route should be retained in persistent state that could mislead the user into thinking traffic is protected after a crash. On restart, FetchX should re-verify that the configured proxy is reachable before resuming downloads.

**Remaining limitation:** A brief window of direct connection always exists between crash and the foreground service restart.

---

### Threat 11: Configuration Mistakes

**Attacker capability:** Passive observer benefits from user misconfiguration.

**Information exposed:** If the user sets `NetworkRoute.Tor` but Orbot is not running, a silent fallback to direct could leak the real IP.

**Mitigation:** No silent fallback. FetchX must fail explicitly when the configured proxy is unreachable. Surface a clear error: "Tor proxy not reachable. Start Orbot before using Tor routing."

**Remaining limitation:** Users may misunderstand which components are covered by the route setting.

---

### Threat 12: Malicious or Overprivileged Extension

**Attacker capability:** A GeckoView extension with the `proxy` permission can reroute browser traffic; an extension with `webRequest` blocking can suppress requests; an extension with `tabs` can observe visited URLs.

**Information exposed:** Browser navigation history, visited URLs, download targets (depending on permissions).

**Mitigation:** FetchX Phase 20L requires explicit user permission review before any permission-requiring extension is installed. The `proxy` permission must be treated as high-sensitivity. Extensions without `proxy` permission cannot affect network routing.

**Remaining limitation:** A user who grants `proxy` permission to an extension trusts that extension with all browser traffic. FetchX cannot audit extension source code.

---

## 11. VPN vs Tor Comparison

### Privacy

**VPN:** Hides traffic content and destinations from ISP and local network but shifts trust entirely to the VPN provider. The VPN provider sees all traffic, source IP, and destinations.

**Tor:** Distributes trust across three relays (guard, middle, exit). No single relay sees both source and destination. Provides stronger anonymity than a VPN but only for Tor-routed traffic.

**Neither provides absolute privacy.** The exit node (Tor) or VPN provider always sees some information.

---

### Anonymity

**VPN:** Provides IP address anonymity from the destination server (destination sees the VPN server's IP, not the user's). VPN provider knows the user's real IP and all destinations.

**Tor:** Provides stronger anonymity — destination sees exit node IP; guard node sees user IP but not destination; middle relay knows neither. Requires Tor Browser-level fingerprint normalization for full anonymity (GeckoView alone is not sufficient).

---

### Trust model

**VPN:** Single trusted party (VPN provider). Trust is centralised and total.

**Tor:** Distributed trust across multiple relays, most of which are volunteer-operated. No single trusted party unless all three relays collude.

---

### Performance

**VPN:** Single hop to VPN server; latency depends on server proximity. Throughput close to raw connection speed. Negligible packet processing overhead on modern hardware.

**Tor:** Three hops through volunteer relays; latency typically 100–500 ms added; throughput often 1–10 Mbps depending on circuit quality and relay load. Not suitable for high-bandwidth downloads if speed is a priority.

---

### Latency

**VPN:** Adds 5–50 ms typically for a nearby server; 50–200 ms for distant servers.

**Tor:** Adds 100–500 ms minimum; highly variable.

---

### Bandwidth

**VPN:** Minimal overhead (VPN header encapsulation < 5% overhead).

**Tor:** Cell-based forwarding, 512-byte cells; high-bandwidth downloads often limited by relay capacity to 1–10 Mbps.

---

### Battery

**VPN:** Moderate impact — VPN service runs in background with a read loop on the tun fd.

**Tor:** Higher impact — Tor circuit maintenance, relay keep-alives, key rotation are additional overheads on top of the VPN overhead if using VPN-over-Tor.

---

### Android integration

**VPN:** `android.net.VpnService` is a first-class Android API since API 14. Well-documented. Requires user consent. Requires `specialUse` foreground service type on Android 14+.

**Tor:** No official Android Tor API. Requires bundling (Option A), relying on Orbot (Option C), or connecting to a local SOCKS5 port. GeckoView proxy configuration gap (Section 5) is the current blocker.

---

### GeckoView integration

**VPN:** A `VpnService` is transparent to GeckoView — all traffic passes through the kernel-level VPN regardless of GeckoView's proxy settings. No GeckoView-specific proxy API needed.

**Tor (via SOCKS5):** Requires GeckoView proxy configuration — which is currently UNVERIFIED at the API level for GeckoView 137. The WebExtension `browser.proxy` API is the most viable path.

---

### Downloader integration

**VPN:** Transparent — all TCP connections from the Rust process pass through the VPN automatically.

**Tor (via SOCKS5):** Requires per-engine proxy configuration. `reqwest` needs the `socks` feature. aria2 needs `--all-proxy`. yt-dlp needs `--proxy`. FFmpeg needs `http_proxy` (no SOCKS5 support).

---

### DNS protection

**VPN:** DNS is routed through the VPN if `addDnsServer()` is configured; otherwise DNS can leak.

**Tor:** Remote DNS at the exit node if hostnames are passed to SOCKS5 (not pre-resolved IPs).

---

### IPv6 handling

**VPN:** Must explicitly add IPv6 routes; if not added, IPv6 traffic bypasses the VPN.

**Tor:** Tor SOCKS5 handles IPv6 hostnames via DNS at exit node; IPv6-direct-address connections depend on exit node IPv6 support.

---

### Failure recovery

**VPN:** On `onRevoke()`, service restarts. Kill switch implementation is complex.

**Tor:** Circuit failures cause connection timeouts; resume from last byte offset.

---

### Maintenance

**VPN:** Android API is stable; maintenance burden is primarily for the custom packet router.

**Tor:** If bundled: very high (Tor releases frequently). If via Orbot: low (Guardian Project maintains Tor).

---

### User experience

**VPN:** Familiar metaphor; one toggle. Consent dialog on first use. May conflict with other VPN apps.

**Tor:** Less familiar; requires Orbot installation; slow circuit establishment; user must understand limitations.

---

### Security

**VPN:** Adds packet routing attack surface. VPN provider is fully trusted. If the custom `VpnService` has bugs, all traffic is at risk.

**Tor:** Stronger security model but exit node can observe unencrypted traffic. Exit-node-operated certificate attacks are possible for non-pinned HTTPS (rare in practice).

---

### Complexity

**VPN:** High — custom `VpnService`, packet routing, IPv6, DNS, Wi-Fi/mobile transitions, kill switch.

**Tor (per-component SOCKS5):** Medium — per-engine proxy configuration, Orbot integration, WebExtension browser.proxy verification.

---

## 12. Architecture Options

### Option A: Browser-only SOCKS5

```
┌─────────────────────────────────────────────────────┐
│                   FetchX Android                    │
│                                                     │
│  ┌──────────────┐    SOCKS5     ┌─────────────┐    │
│  │  GeckoView   │ ───────────▶  │   Orbot     │    │
│  │  (Browser)   │               │  (SOCKS5)   │    │
│  └──────────────┘               └──────┬──────┘    │
│                                        │            │
│  ┌──────────────┐                      │ Tor        │
│  │  Rust        │    Direct    ┌────── ▼ ──────┐    │
│  │  Downloader  │ ───────────▶ │  Internet     │    │
│  └──────────────┘             └───────────────┘    │
└─────────────────────────────────────────────────────┘
  Note: Rust downloader goes DIRECT — IP visible to download servers
```

**Advantages:**
- Minimal implementation surface (only GeckoView proxy config).
- No Rust changes required.
- Browsing is protected.

**Disadvantages:**
- Rust downloader traffic (aria2, yt-dlp, DirectHttp) bypasses proxy — real IP exposed to download servers.
- Inconsistent privacy model — user might not understand the gap.
- GeckoView 137 proxy API is UNVERIFIED (Section 5).

**Complexity:** Low  
**Privacy:** Low (download servers see real IP)

---

### Option B: Android VpnService

```
┌─────────────────────────────────────────────────────┐
│                   FetchX Android                    │
│                                                     │
│  ┌──────────────┐               ┌──────────────┐   │
│  │  GeckoView   │               │  Rust        │   │
│  │  (Browser)   │               │  Downloader  │   │
│  └──────┬───────┘               └──────┬───────┘   │
│         │ All traffic                  │            │
│         └─────────────┬────────────────┘            │
│                       ▼                             │
│         ┌──────────────────────────┐                │
│         │  FetchX VpnService (tun) │                │
│         └─────────────┬────────────┘                │
│                       │                             │
│                       ▼ SOCKS5 / Tor                │
│         ┌──────────────────────────┐                │
│         │   Orbot / Tor Network    │                │
│         └─────────────┬────────────┘                │
│                       ▼                             │
│            ┌────────────────────┐                   │
│            │    Internet        │                   │
│            └────────────────────┘                   │
└─────────────────────────────────────────────────────┘
  Note: All FetchX traffic captured at kernel level — no component-level config needed
```

**Advantages:**
- Complete coverage — all FetchX traffic routed through Tor/proxy.
- No GeckoView proxy API needed (VPN is transparent).
- Enables system-wide IP anonymization for all app traffic (with `addAllowedApplication` removed).

**Disadvantages:**
- Requires `android.permission.BIND_VPN_SERVICE` and user VPN consent dialog.
- `specialUse` foreground service type triggers Google Play review.
- Packet routing loop is complex to implement correctly.
- IPv6 leak risk if not handled.
- Conflicts with other VPN apps.
- Very high engineering complexity.

**Complexity:** Very High  
**Privacy:** High (when implemented correctly)

---

### Option C: Per-component proxy

```
┌──────────────────────────────────────────────────────────────────┐
│                        FetchX Android                            │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                   NetworkRoute setting                    │   │
│  │         Direct | SocksProxy(host, port) | Tor             │   │
│  └───────────────────────────┬──────────────────────────────┘   │
│                              │                                   │
│  ┌───────────┐ WebExt proxy  │  ┌───────────┐ --all-proxy       │
│  │ GeckoView │ ──────────────┤  │  aria2    │ ─────────────┐    │
│  └───────────┘               │  └───────────┘              │    │
│  ┌───────────┐ socks feature │  ┌───────────┐ --proxy      │    │
│  │ reqwest   │ ──────────────┤  │  yt-dlp   │ ─────────────┤    │
│  └───────────┘               │  └───────────┘              │    │
│  ┌───────────┐ http_proxy    │  ┌───────────┐ No SOCKS5    │    │
│  │  FFmpeg   │ ─env──────────┘  │  (fallback│ ─────────────┘    │
│  └───────────┘  (HTTP only)     │   HTTP)   │                   │
│                                 └───────────┘                   │
│                              ▼                                   │
│                ┌─────────────────────────┐                      │
│                │  Orbot SOCKS5 (9050)    │ ← user installs      │
│                └─────────────┬───────────┘                      │
│                              ▼ Tor                               │
│                    ┌────────────────┐                            │
│                    │   Internet     │                            │
│                    └────────────────┘                            │
└──────────────────────────────────────────────────────────────────┘
```

**Advantages:**
- No `VpnService` required — no user VPN consent dialog, no `specialUse` foreground service type.
- Each engine independently configured — no coupling between engines.
- Aligns directly with existing `NetworkRoute` sealed class in `:core`.
- `reqwest` SOCKS5 is a well-tested feature (`reqwest` + `socks` feature flag).
- aria2 `--all-proxy` and yt-dlp `--proxy` are stable, documented arguments.
- Orbot handles the Tor binary — FetchX has zero Tor maintenance burden.
- GeckoView proxy via WebExtension `browser.proxy` API (if confirmed in GeckoView 137) gives browser coverage without VPN.

**Disadvantages:**
- FFmpeg does not support SOCKS5 (HTTP proxy via env var only).
- GeckoView proxy API is UNVERIFIED — if `browser.proxy` is not available in GeckoView 137, browser routing requires a different approach.
- Split-tunnel at engine level means OS-level traffic is not covered.
- Requires Orbot to be installed — FetchX cannot work in Tor mode if Orbot is absent.
- More implementation work than Option A (all engines must be updated).

**Complexity:** Medium  
**Privacy:** High (when all engines are configured; gap for FFmpeg SOCKS5)

---

## 13. Recommended Architecture

**Selected: Option C — Per-component proxy**

### Why Option C was selected

Option C aligns with the existing FetchX architecture without requiring platform-level VPN capabilities. The `NetworkRoute` sealed class already exists in `:core`. The domain model, the `:core` interface, the UDL (when updated), and the Kotlin `:bridge` module all anticipate this configuration path. Option C can be implemented incrementally: each engine is updated independently, and the user sees immediate privacy improvement as each component is completed.

The critical privacy gap in Option C — the browser component — is blocked on verifying GeckoView 137's WebExtension `browser.proxy` API. This should be the first verification step in the implementation phase (Phase 20N or a dedicated sub-phase).

### Why Option A was rejected

Option A (browser-only SOCKS5) leaves the Rust downloader entirely unprotected. A download manager that hides the user's IP in the browser but reveals it to every download server provides a misleading privacy promise. Option C is strictly superior.

### Why Option B was rejected

Option B (Android VpnService) is the highest-complexity option by a significant margin. It introduces `BIND_VPN_SERVICE`, the `specialUse` foreground service type (with Google Play review requirements), a custom IP packet router, IPv6 and DNS leak risks, and conflict with other VPN apps. This complexity is not justified when Option C achieves comparable privacy for FetchX traffic specifically. Option B remains valuable as a long-term goal if FetchX ever intends to provide system-wide privacy routing.

### What Option C requires in the existing architecture

1. **GeckoView 137 browser.proxy API verification:** Confirm whether `browser.proxy.settings.set()` is available in GeckoView 137 extensions. If yes: create a bundled internal extension with the `proxy` permission. If no: investigate the internal Gecko preference approach.

2. **reqwest socks feature:** Add `"socks"` to the `features` list of `reqwest` in `Cargo.toml` workspace dependencies.

3. **Rust `NetworkRoute` model:** Add a `NetworkRoute` enum to `crates/models`. The Kotlin `NetworkRoute` already exists; the Rust counterpart needs to be added and exposed via the UDL.

4. **DirectHttpEngine:** Build a `reqwest::Client` per download that reads the `NetworkRoute` from `DownloadItem` and configures the SOCKS5 proxy accordingly.

5. **Aria2Engine:** Pass `--all-proxy=socks5://host:port` via aria2 RPC when `NetworkRoute` is SOCKS5 or Tor.

6. **YtDlpEngine:** Pass `--proxy=socks5://host:port` to yt-dlp subprocess arguments.

7. **FFmpeg:** Set `http_proxy` environment variable; document that SOCKS5 is not supported for FFmpeg-processed items.

8. **No-fallback enforcement:** All engines must fail explicitly when the configured proxy is unreachable, without silently falling back to direct.

### What cannot be solved with Option C

- **OS-level traffic:** System DNS, Android telemetry, and other non-FetchX network traffic are not protected.
- **FFmpeg SOCKS5:** FFmpeg does not support SOCKS5. Only HTTP proxy is available via environment variable.
- **Physical network kill switch:** Per-component proxy cannot block direct connections at the IP layer if a proxy goes down and fallback is disabled — the connection simply fails rather than being blocked silently.

---

## 14. Permission Analysis

| Permission | Required For | Classification | User Approval |
|---|---|---|---|
| `android.permission.INTERNET` | All network connections (already declared) | REQUIRED | No (normal permission) |
| `android.permission.BIND_VPN_SERVICE` | Option B (VpnService) only | OPTIONAL (future) | No (signature; auto-granted when manifest declares it) |
| `android.permission.FOREGROUND_SERVICE` | Foreground service for VPN (already declared) | REQUIRED for VPN | No (normal permission) |
| `android.permission.FOREGROUND_SERVICE_DATA_SYNC` | Currently declared for download service | REQUIRED | No |
| `android.permission.POST_NOTIFICATIONS` | VPN foreground service notification | REQUIRED for VPN | YES (runtime, Android 13+) |
| `android.permission.CHANGE_NETWORK_STATE` | Needed to query `ConnectivityManager` for captive portal detection | OPTIONAL | No (normal permission) |
| `android.permission.ACCESS_NETWORK_STATE` | Check current network type before proxy connection | OPTIONAL | No (normal permission) |
| `android.permission.RECEIVE_BOOT_COMPLETED` | Restart VPN service after reboot (Option B only) | OPTIONAL (future) | No (normal permission) |
| Extension `proxy` permission (WebExtension) | GeckoView browser.proxy API via extension | USER_APPROVAL_NEEDED | YES (FetchX Phase 20L permission review) |

**For Option C (recommended), no new Android permissions are required beyond what is already declared.**

---

## 15. Licensing

### Tor — BSD-3-Clause

Tor itself is BSD-3-Clause licensed. Bundling the Tor binary does not impose copyleft on FetchX application code. The BSD-3-Clause attribution requirement (retain copyright notice) must be satisfied in the `LICENSES` or `NOTICE` file.

### Orbot — GPL-3.0-or-later

Orbot is licensed GPL-3.0-or-later. FetchX does **not link** to Orbot and does not incorporate Orbot's code. Connecting to Orbot's SOCKS5 port over a TCP socket is not "linking" or "combination" under GPL-3.0. **No GPL-3.0 obligation applies to FetchX.** FetchX should document in its privacy policy that it can optionally connect to Orbot when the user has installed it.

### NetCipher — Apache 2.0 / GPL-3.0 (dual, component-dependent)

NetCipher's `netcipher-proxy` module (the OrbotHelper and proxy helper classes) is Apache 2.0 licensed. The `netcipher-web` module (for WebView) is not relevant here. If FetchX uses only the `netcipher-proxy` module's Apache-licensed components, **no GPL obligation arises.** Review per-class licensing before adding a NetCipher dependency. Alternatively, constructing the Orbot start `Intent` directly (without NetCipher) avoids the dependency entirely.

### info.guardianproject:tor-android

This is the Guardian Project's embedded Tor library for Android. License: BSD-3-Clause (consistent with the Tor upstream license). Binary size: ~10 MB arm64. Maintenance: Guardian Project. This library would only be relevant for Option A (bundled Tor) which is not recommended. **Not to be added in Phase 20M.**

### GeckoView — Mozilla Public License 2.0

MPL-2.0 is a file-level copyleft: modifications to GeckoView source files must be shared under MPL-2.0. FetchX application code is not affected. The proxy configuration approach (WebExtension `browser.proxy` API or internal preference) does not modify GeckoView files. **No additional MPL-2.0 obligation beyond existing.**

### reqwest socks feature — MIT / Apache 2.0

Adding `"socks"` to reqwest features pulls in the `socks` crate (MIT OR Apache-2.0). Both are permissive and compatible with FetchX.

### tokio-socks / socks5 (transitive)

MIT license. Permissive. No obligation beyond attribution.

### FetchX license compatibility

FetchX's license is not stated in the inspected source code. Assuming a permissive license (MIT, Apache 2.0): all recommended dependencies (Tor BSD, reqwest+socks MIT/Apache, NetCipher Apache) are compatible. If FetchX uses a copyleft license, NetCipher's GPL-licensed components must be avoided; the Apache-licensed components and direct Orbot Intent construction remain compatible.

---

## 16. Performance Considerations

| Route | Added Latency | Bandwidth Overhead | Battery Impact | Memory Overhead | CPU Overhead | Connection Setup |
|---|---|---|---|---|---|---|
| Direct | 0 ms | 0% | Baseline | Baseline | Baseline | ~100–500 ms TCP |
| HTTP Proxy | +5–50 ms (single hop) | <1% (HTTP CONNECT header) | Negligible | Negligible | Negligible | +50–200 ms proxy CONNECT |
| SOCKS5 | +5–50 ms (single hop) | <1% (SOCKS5 handshake) | Negligible | Negligible | Negligible | +10–100 ms SOCKS5 handshake |
| Tor (via Orbot SOCKS5) | +100–500 ms (3 hops) | ~5–10% (Tor cell overhead) | +5–15% vs direct | Orbot: ~50–100 MB RAM | Tor key schedule: moderate | +5–30 s (initial circuit); +1–5 s (subsequent) |
| Android VpnService (future) | +2–10 ms (tun overhead) | <1% (IP headers) | +5–15% (tun read loop) | ~5–15 MB (kernel tun buffer) | Low (tun fd I/O) | +1–2 s (interface bring-up) |

**Tor circuit establishment** is the dominant performance factor for Tor routing. Segmented download performance degrades significantly because Tor throttles at the relay level; aria2's segmented download benefit is largely negated.

**First-byte latency for Tor** is dominated by the circuit establishment time. Subsequent connections within an established circuit add only the three-hop relay latency.

---

## 17. Known Limitations

### Android OS constraints

- `VpnService` can be revoked by the system without application control.
- `VpnService` with `specialUse` foreground type requires Google Play review and approval.
- Background processes (WorkManager) may resume before the VPN/proxy is active.
- Doze mode may suspend the VPN read loop, causing connection drops.

### GeckoView API gaps

- GeckoView 137 does not expose a documented Java-level proxy configuration API.
- Per-session proxy isolation is not supported — all sessions share one network configuration.
- `browser.proxy` WebExtension API availability in GeckoView 137 is UNVERIFIED and must be confirmed before implementing browser-level proxy routing.
- Even if `browser.proxy` is available, changing proxy settings while active sessions are loading may have undefined behaviour.

### Tor fundamental limitations

- Tor does not support UDP. WebRTC, QUIC/HTTP3, and any UDP-based download protocol bypass Tor via SOCKS5.
- Tor bandwidth is fundamentally limited by volunteer relay capacity. High-bandwidth downloads are slow.
- Tor provides anonymity for IP-level observation but does not prevent browser fingerprinting. GeckoView 137's fingerprinting protections reduce but do not eliminate the fingerprint surface.
- Tor exit nodes can observe unencrypted (HTTP) traffic content.
- Tor circuit establishment can take 5–30 seconds, creating poor UX for latency-sensitive operations.

### VPN fundamental limitations

- A self-operated VPN does not provide anonymity from the VPN provider (FetchX itself would be the VPN, providing no third-party anonymity).
- Relying on Orbot as the VPN backend means FetchX has no VPN without Orbot.
- A custom `VpnService` cannot prevent the OS from routing some traffic directly (e.g. connectivity checks).

### Platform restrictions (Android 14+)

- `startForegroundService()` from the background is restricted; VPN service start must be user-initiated.
- `specialUse` foreground service type requires Play Store justification.
- Android 14 restricts background start of activities, affecting the VPN consent dialog flow if triggered without direct user gesture.

---

## 18. Deferred Implementation Plan

**The following is a PROPOSED FUTURE implementation plan. NONE of this is implemented in Phase 20M.**

### Phase A: Verify GeckoView browser.proxy API
- Install a minimal test WebExtension in the existing `:browser` extension framework.
- Call `browser.proxy.settings.set()` from the extension background script.
- Verify proxy configuration propagates to GeckoSession requests in GeckoView 137.
- If confirmed: implement a bundled FetchX proxy-controller extension.
- If not confirmed: investigate internal Gecko preference path.
- Deliverable: confirmed or rejected GeckoView proxy API support.

### Phase B: Rust reqwest SOCKS5
- Add `"socks"` feature to `reqwest` workspace dependency in `Cargo.toml`.
- Add `NetworkRoute` enum to `crates/models`.
- Update `DownloadItem` to carry an optional `network_route` field.
- Update `DirectHttpEngine` to build a per-download `reqwest::Client` based on route.
- Add unit tests for proxy-configured client construction.
- No proxy connection is tested (no live Tor required for tests; mock SOCKS5 server).

### Phase C: NetworkRoute abstraction across all engines
- Update the `DownloadEngine` trait to accept a `route: &NetworkRoute` parameter.
- Update `Aria2Engine`: pass `--all-proxy` in the aria2 RPC add URI options.
- Update `YtDlpEngine`: append `--proxy` to subprocess arguments.
- Update `FFmpegProcessor`: set `http_proxy` environment variable for HTTP routes.
- Document that FFmpeg SOCKS5 is not supported.
- Update the UniFFI UDL to expose `NetworkRoute` across the Rust/Kotlin boundary.
- Update `:bridge` Kotlin bindings.
- Add integration tests for each engine with a mock SOCKS5 server.

### Phase D: Orbot integration (Tor routing UX)
- Detect Orbot installation (`PackageManager.getPackageInfo("org.torproject.android")`).
- Implement Orbot start Intent (direct, without NetCipher, to avoid GPL components).
- Listen for Orbot `STATUS_ON` broadcast before activating Tor route.
- Surface "Install Orbot" prompt when user selects Tor route without Orbot installed.
- Implement no-fallback enforcement: fail with user-visible error if proxy is unreachable.
- Add captive portal detection: temporarily disable proxy for portal login.

### Phase E: UI and user controls
- Add "Network Route" settings screen: Direct / SOCKS5 (custom) / Tor (Orbot).
- Show Tor circuit status indicator in browser (active/establishing/unavailable).
- Add per-download route selector in `AddDownloadDialog`.
- Add warning disclosure: "Tor routing slows downloads significantly" when Tor is selected.
- Add warning disclosure: "Your Tor proxy provider can see unencrypted download traffic" when SOCKS5 is selected.
- Persist route setting in SQLite via `ConfigService`.

---

## 19. Final Recommendation

The recommended architecture for FetchX VPN/Tor integration is **Option C: Per-component SOCKS5 proxy** (Section 12/13). The first implementation step is Phase A: verifying whether GeckoView 137's WebExtension `browser.proxy` API is available and functional in the version pinned by FetchX. This is the critical blocker because GeckoView 137 has no documented public Java-level proxy API, and the browser component represents the most privacy-sensitive FetchX network surface. The most critical privacy risk to address first is the **browser direct-connection leak** (Threat 8 in Section 10): until GeckoView proxy configuration is confirmed and implemented, the embedded browser reveals the user's real IP to all visited sites regardless of any proxy setting applied to the Rust downloader. This must be resolved before advertising Tor or proxy routing as a browser privacy feature. No VPN, proxy, Tor, routing, or network-interception implementation was added in Phase 20M. The `NetworkRoute` sealed class already present in `:core` remains a domain model placeholder only.

---

## 20. Sources

Research performed 2025-07-11. Sources consulted:

- [Android VpnService documentation — developer.android.com](https://developer.android.com/reference/android/net/VpnService) — retrieved 2025-07-11
- [Android VpnService.Builder documentation](https://developer.android.com/reference/android/net/VpnService.Builder) — retrieved 2025-07-11
- [Android Foreground Services — developer.android.com](https://developer.android.com/develop/background-work/services/foreground-services) — retrieved 2025-07-11
- [Android 14 behavior changes (API 34) — developer.android.com](https://developer.android.com/about/versions/14/changes/foreground-service-types) — retrieved 2025-07-11
- [Android 15 behavior changes — developer.android.com](https://developer.android.com/about/versions/15) — retrieved 2025-07-11
- [GeckoView JavaDoc (mozilla-central) — mozilla.github.io](https://mozilla.github.io/geckoview/javadoc/mozilla-central/) — retrieved 2025-07-11
- [GeckoView developer documentation — firefox-source-docs.mozilla.org](https://firefox-source-docs.mozilla.org/mobile/android/geckoview/) — retrieved 2025-07-11
- [GeckoRuntimeSettings API — mozilla.github.io](https://mozilla.github.io/geckoview/javadoc/mozilla-central/org/mozilla/geckoview/GeckoRuntimeSettings.html) — retrieved 2025-07-11
- [GeckoSession.Settings API — mozilla.github.io](https://mozilla.github.io/geckoview/javadoc/mozilla-central/org/mozilla/geckoview/GeckoSession.Settings.html) — retrieved 2025-07-11
- [WebExtensions browser.proxy API — developer.mozilla.org](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/proxy) — retrieved 2025-07-11
- [Tor Project — tor overview](https://www.torproject.org/about/overview.html) — retrieved 2025-07-11
- [Orbot — Guardian Project](https://guardianproject.info/apps/org.torproject.android/) — retrieved 2025-07-11
- [info.guardianproject:netcipher — GitHub](https://github.com/guardianproject/NetCipher) — retrieved 2025-07-11
- [reqwest SOCKS5 proxy support — docs.rs](https://docs.rs/reqwest/latest/reqwest/struct.Proxy.html) — retrieved 2025-07-11
- [tokio-socks crate — crates.io](https://crates.io/crates/tokio-socks) — retrieved 2025-07-11
- [SOCKS5 RFC 1928 — ietf.org](https://www.rfc-editor.org/rfc/rfc1928) — retrieved 2025-07-11
- [aria2 --all-proxy documentation — aria2.github.io](https://aria2.github.io/manual/en/html/aria2c.html#cmdoption-all-proxy) — retrieved 2025-07-11
- [yt-dlp --proxy documentation — github.com/yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp#general-options) — retrieved 2025-07-11
- [Android ConnectivityManager.NetworkCapabilities — developer.android.com](https://developer.android.com/reference/android/net/NetworkCapabilities) — retrieved 2025-07-11
- [Tor Project Android documentation — torproject.org](https://2019.www.torproject.org/docs/android.html.en) — retrieved 2025-07-11
- [Tor BSD-3-Clause license — gitweb.torproject.org](https://gitweb.torproject.org/tor.git/tree/LICENSE) — retrieved 2025-07-11
- [Orbot GPL-3.0 license — github.com/guardianproject/orbot](https://github.com/guardianproject/orbot/blob/master/LICENSE) — retrieved 2025-07-11
- [Mozilla Public License 2.0 — mozilla.org](https://www.mozilla.org/en-US/MPL/2.0/) — retrieved 2025-07-11
- [Android VPN app development guide — developer.android.com](https://developer.android.com/guide/topics/connectivity/vpn) — retrieved 2025-07-11

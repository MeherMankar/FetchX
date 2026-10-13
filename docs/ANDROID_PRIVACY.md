# Android Privacy Architecture

> **Phase 20 privacy design document.** Establishes what privacy properties the
> FetchX Android application provides, how they are implemented, and what
> the clear boundaries of each privacy mechanism are.

---

## Threat Model

### What we protect against

1. **Third-party trackers** — scripts and pixels loaded by visited pages that report
   user behaviour to data brokers and advertising networks. Mitigated by GeckoView
   Enhanced Tracking Protection (ETP).

2. **Cross-site tracking via cookies** — third-party cookies that persist a user's
   identity across unrelated websites. Mitigated by GeckoView Total Cookie Protection
   (first-party isolation).

3. **Browser fingerprinting** — scripts that enumerate device properties
   (canvas, fonts, screen, WebGL, audio) to identify the user without cookies.
   Mitigated by GeckoView `fingerprintingProtection` and ETP fingerprinting category.

4. **Insecure network traffic** — HTTP connections that expose content and metadata.
   Mitigated by HTTPS-first mode and `network_security_config.xml`
   (`cleartextTrafficPermitted="false"` by default).

5. **Cross-session browsing history leakage** — normal browsing history visible to
   other users of the device. Mitigated by private browsing mode (GeckoSession private
   flag).

6. **WebRTC IP leakage** — WebRTC ICE gathering exposing the device's local or
   public IP address without user intent. Mitigated by `GeckoRuntimeSettings.webRtcIpHandling`.

### What we do NOT claim

- **Anonymity** — private browsing is not anonymity. The network operator, ISP, and
  destination server can still see connection metadata. See the "Private Browsing ≠ 
  VPN ≠  Tor" section below.
- **Protection from Tor deanonymisation** — Tor routing is an optional feature
  (Phase 20M) and requires Orbot or a bundled Tor binary. The browser does not
  provide Tor by default.
- **VPN leak protection** — if the user has a system VPN active, we route through it
  via `NetworkRoute.SystemVpn`, but we do not audit or enforce VPN kill-switch behaviour.
  That is the VPN app's responsibility.
- **Device compromise** — if the device is compromised (malware, physical access),
  no browser-level privacy measure protects against data exfiltration.
- **Government-level surveillance** — we do not claim protection against
  nation-state adversaries.

---

## Browser Engine Privacy: GeckoView Enhanced Tracking Protection

GeckoView provides Enhanced Tracking Protection (ETP) via `ContentBlocking.Settings`
and the built-in Disconnect.me tracking protection database.

### ETP modes

| Mode | Blocks |
|---|---|
| Strict | Tracking cookies, tracking content (in all pages), cryptominers, fingerprinters, social trackers |
| Standard | Tracking cookies, cryptominers, fingerprinters, social trackers (tracking content only in private windows) |
| Custom | User-configurable combination of the above categories |

Phase 20I enables GeckoView's built-in ad, analytics, and social tracking
categories by default. The categories are selected with
`ContentBlocking.Settings.Builder.antiTracking(...)`; fingerprinting and
cryptomining categories are not enabled by this tracker-blocking control.
The user can toggle these tracker categories for all open sessions without
recreating tabs. The setting is in-memory and resets to enabled on process restart.

### ETP categories

- `ANTI_TRACKING` — tracking cookies and tracking-classified scripts
- `CRYPTOMINING` — cryptocurrency mining scripts
- `FINGERPRINTING` — canvas fingerprinting, font enumeration, etc.
- `SOCIAL_TRACKING` — social media tracking pixels and scripts
- `STRICT_SOCIAL` — stricter social tracking blocking (strict mode)

GeckoView 137 is configured through `GeckoRuntimeSettings.Builder()`:

```kotlin
val settings = GeckoRuntimeSettings.Builder()
    .contentBlocking(
        ContentBlocking.Settings.Builder()
            .antiTracking(
                ContentBlocking.AntiTracking.AD or
                    ContentBlocking.AntiTracking.ANALYTIC or
                    ContentBlocking.AntiTracking.SOCIAL
            )
            .build()
    )
    .build()
```

---

## Cookie Isolation

GeckoView Total Cookie Protection (dFPI — dynamic First-Party Isolation) partitions
cookie storage by the top-level site. A cookie set by a third-party resource loaded
on `example.com` cannot be read when the same resource is loaded on `other.com`.

Configuration:

```kotlin
ContentBlocking.CookieBehavior.ACCEPT_FIRST_PARTY_AND_ISOLATE_OTHERS
```

This is the same mechanism Firefox for Android uses when ETP Strict is active.

---

## Private Browsing

### What IS isolated in a private GeckoSession

- Browsing history (not written to the Rust storage / SQLite)
- Cookies (separate storage context, not persisted to disk)
- localStorage and sessionStorage (in-memory only)
- IndexedDB (in-memory only)
- Cache (separate cache context, discarded on session close)
- Form data (not auto-filled from the regular profile)

### What is NOT isolated

- **Downloads saved to disk** — a file downloaded in a private session is still
  written to the device storage. The fact that a download occurred is not hidden.
- **Bookmarks explicitly saved by the user** — if the user taps "Bookmark this page"
  in a private tab, that is an explicit action and is persisted.
- **Network-level metadata** — the ISP, DNS resolver, and network operator can still
  observe that a connection was made to an IP address.
- **Downloaded URLs in the system's download manager** — downloads are visible in the
  Android downloads notification (mitigated in Phase 20N with a private notification
  channel).

### Session lifetime

A private GeckoSession's storage is a separate in-memory context. When the session
is closed (`GeckoSession.close()`), GeckoView discards the in-memory storage.
No explicit `clearData()` call is required for the private session itself.

Regular session data is never contaminated by private session data — the two storage
contexts are completely separate within GeckoView's architecture.

---

## Tracker Blocking

Tracker blocking uses GeckoView's native `ContentBlocking` implementation and
its bundled tracking-protection data. In GeckoView 137, configuring categories on
the runtime is not sufficient: each session must also set
`GeckoSessionSettings.useTrackingProtection` to true (its default is false).
Each session forwards `ContentBlocking.Delegate.onContentBlocked` events to the
browser's in-memory statistics controller. GeckoView determines tracker
classification and first-/third-party context; FetchX does not reimplement those
rules or infer site relationships from host suffixes.

On a V2135 Android 13 device, public HTTPS `https://edition.cnn.com/` produced
real `isBlocking=true` callbacks in logcat for GeckoView categories 2 (ad) and 4
(analytics), accepted by the controller. After enabling the missing per-session
setting, the displayed counter rose from 22 on initial page load to 123 after
enabled reloads; a reload with protection disabled left the displayed count at
95. Per-tag INFO logging was temporarily enabled because the device's global
`log.tag` threshold was `E`. Physical cross-tab isolation, tab cleanup, and the
minimum-one-tab invariant remain unverified.

The browser records only a per-tab blocked-request count and up to 20 distinct
normalized hostnames. It discards request paths and query strings immediately and
does not retain timestamps, cookies, headers, credentials, or page content. Stats
are removed when a tab closes. No request URLs are logged or sent to a network
service.

The tracker category toggle is global across sessions, affects subsequent requests,
and does not destroy or recreate tabs. It is currently in-memory and defaults to
enabled at each application start.

FetchX does not ship a separate custom tracker-rule list in this phase. GeckoView's
public native API uses its bundled tracking-protection data rather than accepting
arbitrary app-provided host/path rules. The bundled list changes only when the
GeckoView dependency is updated with an app release; tracker blocking does not
contact an external list service.

The fresh post-fix device run confirmed bridge initialization, GeckoRuntime and
browser startup, and the enabled indicator. The enable/disable control was
toggled off and back on in the running browser. A temporary loopback page loaded
script resources from Google Analytics, Facebook, and Google Publisher Services
over a validated mobile network, but the displayed blocked count remained zero.
This did not demonstrate whether GeckoView blocked those requests or delivered
the callback; a localhost top-level origin is not a deterministic substitute
for a public-site tracker context. A real end-to-end tracker callback therefore
remains unverified.

**No telemetry is sent** by the tracker blocking logic itself. GeckoView does not
report blocking decisions to Mozilla unless the application explicitly enables
GeckoView telemetry, which FetchX does NOT enable.

---

## Fingerprinting Resistance

GeckoView's fingerprinting protection reduces the JavaScript API surface that
fingerprinting scripts rely on:

- Canvas API returns a consistent, slightly randomised result
- Screen dimensions report a reduced resolution
- Font enumeration is limited
- Audio API fingerprinting is mitigated
- Navigator properties are normalised

Note: FetchX uses a **consistent fingerprint profile** rather than random
spoofing on each page load. Research by privacy engineers (notably the Tor Project
and EFF's Cover Your Tracks) has shown that random spoofing can make a browser
*more* identifiable because the randomly-spoofed values are themselves unusual.
A consistent reduced profile is harder to distinguish from the population of other
GeckoView-based browser users.

---

## HTTPS-First

GeckoView HTTPS-only mode (`GeckoRuntimeSettings.Builder.allowInsecureConnections`)
causes the browser to:

1. Upgrade HTTP requests to HTTPS automatically.
2. Show a warning page if the HTTPS upgrade fails.
3. Allow the user to explicitly proceed with HTTP (with a visible warning).

HSTS (HTTP Strict Transport Security) is respected natively by GeckoView.

The `network_security_config.xml` in the Android application independently enforces
`cleartextTrafficPermitted="false"` at the Android OS level for all non-browser
network traffic (including the Rust download engine). Only `localhost` is exempt
(for development tooling).

---

## WebRTC Controls

GeckoView exposes `GeckoRuntimeSettings.webRtcIpHandling` with these policy values:

| Policy | Description |
|---|---|
| `DEFAULT` | Standard browser behaviour — may expose local IP |
| `DEFAULT_PUBLIC_INTERFACE_ONLY` | Only the public interface IP |
| `DEFAULT_PUBLIC_AND_PRIVATE_INTERFACES` | Public + private IP |
| `DISABLE_NON_PROXIED_UDP` | Only relay via TURN; requires TURN server |

Default for FetchX: `DEFAULT_PUBLIC_INTERFACE_ONLY` (prevents local network
topology leakage while keeping WebRTC functional). Configurable in Settings
(Phase 20N).

ICE candidate obfuscation: GeckoView replaces real local IP addresses in ICE
candidates with mDNS hostnames by default, preventing JavaScript from reading the
local IP directly.

---

## Referrer Policy

Default referrer policy for GeckoView sessions: `strict-origin-when-cross-origin`.

- Same-origin requests: full URL in referrer.
- Cross-origin requests: only the origin (scheme + host + port), not the path.
- Downgrade (HTTPS → HTTP): no referrer sent.

This prevents destination sites from learning which HTTPS page referred the user
when the destination is HTTP.

---

## Per-Site Settings Model

Phase 20J will implement per-site settings stored in the Rust SQLite database:

| Setting | Scope | Default |
|---|---|---|
| ETP exception | Per-origin | Off (ETP on for all sites) |
| JavaScript enabled | Per-origin | Enabled |
| Camera permission | Per-origin | Prompt |
| Microphone permission | Per-origin | Prompt |
| Location permission | Per-origin | Prompt (or Deny if not needed) |
| Notification permission | Per-origin | Prompt |
| Autoplay (audio/video) | Per-origin | Block audio autoplay |
| HTTPS-only exception | Per-origin | None (HTTPS enforced) |

Permissions are stored as rows in a `site_permissions` table in the existing
SQLite database (schema addition in Phase 20J).

---

## What Is NOT in Scope for This Document

- **Tor routing** (Phase 20M) — tunnelling all browser and download traffic through
  Tor via Orbot or a bundled tor binary. Architecture is separate from the browser
  privacy layer documented here.
- **VPN integration** (Phase 20M) — detecting an active VPN and routing downloads
  through it. This is handled at the `NetworkRoute` level in `:core`.
- **Extension sandbox** (Phase 20L) — permissions model for WebExtensions installed
  via `WebExtensionController`. Extensions run in GeckoView's sandbox and cannot
  access Android JNI or the Rust bridge directly.

---

## ⚠️ Private Browsing ≠ VPN ≠ Tor

This distinction must be communicated clearly to users.

| Mode | Protects against | Does NOT protect against |
|---|---|---|
| **Private Browsing** | History on this device, cookie tracking, localStorage | Network operator, ISP, destination server, downloads on disk |
| **VPN** | ISP traffic monitoring, IP-based geo-blocking | The VPN provider itself, app-level tracking, fingerprinting |
| **Tor** | IP-based identification, ISP monitoring | End-to-end content surveillance (if destination is HTTP), malicious exit nodes, correlation attacks |

Private Browsing in FetchX protects your **on-device browsing history and
cookie state**. It does not hide your traffic from your network provider or from
websites that track you by fingerprint rather than cookie.

Use a VPN (Phase 20M) if you need to hide your IP from destination servers.
Use Tor (Phase 20M) if you need to hide your IP from your ISP and destination servers
simultaneously (with the trade-offs Tor involves).

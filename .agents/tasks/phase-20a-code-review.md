# Phase 20A — Android Foundation

Phase 20A establishes the Android project skeleton: a three-module Gradle project (`app`, `core`, `downloads`), domain models, the `DownloadRepository` interface, a four-tab downloads UI scaffold, and four documentation files covering browser engine selection, architecture, privacy, and extension strategy. GeckoView is chosen as the browser engine but deliberately not added as a Gradle dependency yet — that is deferred to Phase 20F. The Rust workspace is untouched; all new files are Kotlin, Gradle, XML, and Markdown.

Watch for: **confirmed** — `DownloadsViewModel` derives its four tab lists by calling derived property getters on every recomposition, with no caching. This is harmless at stub stage but becomes a correctness hazard in Phase 20B when the `StateFlow` is live and those properties re-filter the full list on every Compose frame. The pattern needs to change before real data arrives. **confirmed** — `FOREGROUND_SERVICE_DATA_SYNC` is declared in the manifest but this permission did not exist until API 34; because `minSdk=26`, devices on API 26–33 will silently ignore it — document or guard this. **possible** — the `MyDownloaderTheme` composable hardcodes `darkTheme = false` with no system-preference hook; users whose system is in dark mode will see a light theme regardless.

**Verdict**: APPROVED

---

## High-level view

The SDK triad (`minSdk=26`, `targetSdk=35`, `compileSdk=35`) matches the Phase 20A specification exactly and is consistently declared across all three module build files. Every dependency is pinned to an exact version in `libs.versions.toml`; no open ranges exist. The module graph (`app → core ← downloads`) is clean and correctly prevents `:downloads` from depending on `:app`. GeckoView is comment-documented in the version catalog but not added as a Gradle dependency, satisfying the explicit constraint.

The `DownloadRepository` interface covers the full API surface specified by Phase 20B requirements — add, pause, resume, cancel, remove, retry, priority, schedule, get — and `DownloadEvent` carries all seven event variants. The sealed class / data object pattern used throughout `:core` is idiomatic Kotlin 2.x and will map cleanly to UniFFI-generated types in Phase 20B.

Navigation is correctly wired: `AppNavigation` registers `Screen.Downloads.route` and calls `DownloadsScreen()`. The `DownloadItemRow` renders all six download states with contextually appropriate action buttons for each.

The network security config blocks cleartext globally with a localhost-only exception. One downstream consequence to flag for Phase 20B: cleartext HTTP download URLs will be blocked at the Android OS network layer before reaching the Rust engine, which is a behavioural difference from desktop that the engine integration must handle or document.

The four documentation files are accurate and honest about limitations. The browser engine comparison covers all four required options with a full matrix, and each rejection is specific rather than generic. The privacy document explicitly lists what private browsing does *not* protect against. The extensions document is correct about `chrome.downloads` being unavailable and the `storage.sync` / Firefox Account dependency.

---

<details>
<summary>Issues (4)</summary>

1. **Derived tab-filter properties called on every recomposition** — `activeDownloads`, `queuedDownloads`, `completedDownloads`, and `failedDownloads` are unguarded derived properties on `DownloadsViewModel` that filter the full list on every access. `DownloadsScreen` reads all four on every composition pass. Before Phase 20B wires real data, convert these to `StateFlow` properties derived with `stateIn` so filtering runs once per state update, not once per Compose frame.

2. **`FOREGROUND_SERVICE_DATA_SYNC` declared at minSdk 26** — This permission was introduced in API 34. On API 26–33 the OS silently ignores unknown `uses-permission` entries, so the app won't crash, but the foreground service type won't be enforced as intended. In Phase 20E, guard the foreground service type declaration with a `maxSdk` attribute or document the API-level conditional in the manifest comment.

3. **`MyDownloaderTheme` ignores system dark-mode preference** — `darkTheme` defaults to `false` with no `isSystemInDarkTheme()` hook. All users see the light theme regardless of system setting. Pass `darkTheme = isSystemInDarkTheme()` at the callsite in `MainActivity`.

4. **Cleartext HTTP download URLs will be blocked by network security config** — `cleartextTrafficPermitted="false"` applies to all app traffic including the Rust download engine. A cleartext HTTP download URL will be rejected at the Android OS network layer before reaching the Rust engine. Phase 20B must decide whether to surface this as a user-facing error, attempt an HTTPS upgrade, or add a documented exception — this is a behavioural difference from desktop that users will encounter.

</details>

---

<details>
<summary>Details</summary>

## ViewModel derived-property recomposition hazard (confirmed)

`DownloadsViewModel` exposes four properties — `activeDownloads`, `queuedDownloads`, `completedDownloads`, `failedDownloads` — each implemented as a plain getter that calls `_downloads.value.filter { ... }`. `DownloadsScreen` reads all four unconditionally on every composition pass to populate `filtered`:

```kotlin
val filtered = when (selectedTab) {
    0 -> viewModel.activeDownloads
    1 -> viewModel.queuedDownloads
    2 -> viewModel.completedDownloads
    3 -> viewModel.failedDownloads
    else -> emptyList()
}
```

At stub stage with an empty `StateFlow` this is invisible. Once Phase 20B connects real data and progress events drive frequent recompositions, each composable frame will run four filter passes over the full download list. The fix is to derive these as `StateFlow` properties in the ViewModel using `stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())`, so the filtering work happens once per state update rather than once per frame.

## `FOREGROUND_SERVICE_DATA_SYNC` permission scope (confirmed)

The manifest unconditionally declares `android.permission.FOREGROUND_SERVICE_DATA_SYNC`, which was introduced in Android 14 (API 34). With `minSdk=26`, the app targets eleven Android versions below this threshold. The Android permission system silently ignores unrecognised `uses-permission` entries, so no crash occurs, but a foreground service started on API 26–33 without this type being enforced will behave as a generic foreground service. Phase 20E must handle this with a runtime API-level check when starting the service with `FOREGROUND_SERVICE_TYPE_DATA_SYNC`.

## Dark theme wiring gap (possible)

`MyDownloaderTheme` accepts a `darkTheme: Boolean = false` parameter but `MainActivity` calls it as `MyDownloaderTheme { AppNavigation() }`, leaving the default. Users on Android 10+ who have system dark mode active will see the light colour scheme. The fix is `MyDownloaderTheme(darkTheme = isSystemInDarkTheme()) { ... }` in `MainActivity`.

## Network cleartext scope

`<base-config cleartextTrafficPermitted="false">` covers all app traffic including the Rust download engine. A cleartext HTTP download URL handed to the Rust engine will fail at the Android OS network layer before any bytes are transferred — this is a behaviour change relative to desktop that Phase 20B's engine integration must document. The localhost carve-out (`includeSubdomains="false"` on all three loopback addresses) is correctly scoped.

</details>

---

<details>
<summary>File map</summary>

| File | Change |
|---|---|
| `apps/android/gradle/libs.versions.toml` | New — version catalog; all deps pinned; GeckoView commented out |
| `apps/android/settings.gradle.kts` | New — includes `:app`, `:core`, `:downloads`; Mozilla Maven declared |
| `apps/android/app/build.gradle.kts` | New — compileSdk/targetSdk/minSdk=35/35/26; release minify on |
| `apps/android/core/build.gradle.kts` | New — library module; no Compose plugin |
| `apps/android/downloads/build.gradle.kts` | New — library module; Compose enabled |
| `apps/android/app/src/main/AndroidManifest.xml` | New — 4 permissions; network security config wired |
| `apps/android/app/src/main/res/xml/network_security_config.xml` | New — cleartext blocked; localhost exempt |
| `apps/android/app/src/main/kotlin/.../MainActivity.kt` | New — single Activity; edge-to-edge; AppNavigation |
| `apps/android/app/src/main/kotlin/.../MyDownloaderApp.kt` | New — Application stub; Phase 20B hooks commented |
| `apps/android/app/src/main/kotlin/.../navigation/AppNavigation.kt` | New — NavHost; Downloads route registered |
| `apps/android/app/src/main/kotlin/.../navigation/Screen.kt` | New — sealed class; Downloads destination; future screens commented |
| `apps/android/app/src/main/kotlin/.../ui/theme/MyDownloaderTheme.kt` | New — Material 3; dynamic colour on API 31+ |
| `apps/android/core/src/main/kotlin/.../DownloadItem.kt` | New — data class; 10 fields |
| `apps/android/core/src/main/kotlin/.../DownloadState.kt` | New — sealed class; 6 variants |
| `apps/android/core/src/main/kotlin/.../DownloadEvent.kt` | New — sealed class; 7 event types |
| `apps/android/core/src/main/kotlin/.../DownloadPriority.kt` | New — enum; 4 levels; DEFAULT companion |
| `apps/android/core/src/main/kotlin/.../DownloadRepository.kt` | New — interface; 11 methods |
| `apps/android/core/src/main/kotlin/.../NetworkRoute.kt` | New — sealed class; 4 routing modes |
| `apps/android/downloads/src/main/kotlin/.../DownloadsScreen.kt` | New — 4-tab Scaffold; LazyColumn; empty state |
| `apps/android/downloads/src/main/kotlin/.../DownloadItemRow.kt` | New — Card row; all 6 states; progress/speed/ETA |
| `apps/android/downloads/src/main/kotlin/.../DownloadsViewModel.kt` | New — stub ViewModel; empty StateFlow; 4 derived properties |
| `docs/ANDROID_BROWSER.md` | New — 4-option comparison; matrix; GeckoView recommendation |
| `docs/ANDROID_ARCHITECTURE.md` | New — layered architecture; module breakdown; data/event flow |
| `docs/ANDROID_PRIVACY.md` | New — threat model; ETP; cookie isolation; private browsing; WebRTC |
| `docs/ANDROID_EXTENSIONS.md` | New — WebExtensionController; available APIs; install flow; security isolation |
| `docs/ROADMAP.md` | Modified — Phase 20A entry expanded |

Full diff: `git show HEAD` from the repository root.

</details>

# Phase 20A Implementation Plan

## Browser Engine Decision

### Comparison Research Summary

| Criterion | Android WebView | GeckoView |
|---|---|---|
| **Chromium compatibility** | Full — it *is* Chromium WebView | Partial — Gecko engine, not Blink |
| **Firefox/WebExtension support** | None — Chrome-for-Android has no user extensions | Full — Firefox for Android v123+ ships WebExtensions via GeckoView; bundled extensions work natively |
| **Extension APIs** | None exposed to embedders | Full WebExtension API surface via `WebExtensionController`; bundled and user-installed XPIs supported |
| **Privacy controls** | Minimal — no built-in tracker blocking, no fingerprint resistance, no user-visible privacy mode | Enhanced Tracking Protection (ETP), fingerprinting resistance via `GeckoRuntimeSettings`, `AntiTracking` API |
| **Content blocking** | Only via JavaScript injection or proxy interception | Native content blocking via built-in WebExtension or `ContentBlockingController`; `uBlock Origin` runs unmodified |
| **Cookie isolation** | Shared cookie store unless the app carefully segregates `CookieManager` — no automatic isolation across tabs | First-party isolation, total cookie protection configurable per-session via `GeckoSessionSettings` |
| **Private browsing** | Manual flag on `WebView`; no real isolation guarantees without extra work | `GeckoSessionSettings.usePrivateMode(true)` — native private session with separate storage |
| **Fingerprinting resistance** | None — WebView exposes same UA/screen/canvas as the host Chrome version | `GeckoRuntimeSettings.Builder.fingerprintingProtection(true)` + resistFingerprinting preference |
| **WebRTC controls** | No built-in controls; JS or proxy workarounds only | `GeckoRuntimeSettings.webRtcIpHandling` policy; can disable or restrict |
| **JavaScript controls** | `WebSettings.setJavaScriptEnabled(false)` — coarse only | Per-session JS disable; per-extension JS policy |
| **Storage isolation** | Single `WebStorage` shared across all WebViews by default | Per-profile storage via `GeckoRuntime` + multiple `GeckoSession` profiles |
| **Multiple profiles** | Not supported without major hacks | Native: create separate `GeckoSession` instances under separate runtime profiles |
| **Maintenance burden** | Maintained by Google; auto-updated via Play Store system component — low burden for embedder | Mozilla-maintained, released in sync with Firefox stable/beta/nightly; embedder upgrades manually via Maven version bump |
| **APK size** | Zero overhead — system component | ~50 MB ABI-split native library per architecture; with ABI splits in AAB: ~50 MB per architecture, ~50 MB for the selected ABI at install time |
| **RAM usage** | Shares process memory with system WebView; relatively lower overhead | Separate Gecko process; ~50–80 MB baseline additional RAM |
| **Android compatibility** | API 21+ | API 21+ (minSdk 21) |
| **Licensing** | Apache 2.0 (AOSP) | MPL 2.0 — compatible with this project; no copyleft obligation on application code |
| **Long-term viability** | Google-backed; extremely stable | Mozilla-backed; actively developed as Firefox for Android foundation; not going away |

### Engine Decision: **GeckoView**

Rationale: The primary design goal of Phase 20 is a privacy-focused browser. Android WebView has no first-party isolation, no fingerprinting resistance, no built-in content blocking, and no extension support. GeckoView delivers all of these natively. The APK size penalty (~50 MB per ABI) is acceptable for a dedicated download+browser application — 1DM and Firefox themselves carry this cost. The MPL 2.0 licence places no obligations on application code. GeckoView is the correct foundation; WebView would require reimplementing privacy features that Gecko already has.

---

## SDK Versions

- **minSdk: 26** (Android 8.0 Oreo) — GeckoView technically supports API 21, but Android 8.0 is the cutoff for JobScheduler reliability, `NotificationChannel` (required for background downloads), and `StrictMode` network improvements. API 26 represents ~97% of active Android devices as of 2025.
- **targetSdk: 35** (Android 15) — Google Play now requires targetSdk 35 for new app submissions.
- **compileSdk: 35** — always equal to targetSdk for new projects.

---

## Module Structure Decision

The repo already has `apps/browser/` (desktop browser extension) and `apps/desktop/` (Tauri desktop app). The Android app goes under `apps/android/`. Rather than creating every possible module up front, only the modules needed for Phase 20A (foundation) and 20B (Rust bridge scaffold) are created. Later phases add modules as needed.

Modules created now:
- `app` — application entry point, DI wiring, Activity, Compose NavHost
- `core` — domain models, service interfaces, coroutine utilities (no Android deps)
- `bridge` — UniFFI-generated Kotlin bindings + `DownloadBridge` wrapper
- `ui` — shared Compose theme, Material 3 tokens, common widgets

Modules deferred (created in their respective phases):
- `downloads` — 20C/20D: download list UI and ViewModel
- `browser` — 20F: GeckoView browser shell
- `privacy` — 20J: tracker protection configuration
- `extensions` — 20K: WebExtension management

This avoids creating empty modules that have no content yet and would only cause build failures.

---

## Rust Mobile API Crate Decision

A new `crates/mobile-api` crate is the UniFFI facade. It depends on `mydownloader-core-services` and re-exports a stable, Android-facing API. It does **not** expose internal types from `queue`, `scheduler`, `storage`, or `downloader` directly — only opaque `DownloadHandle` / `DownloadInfo` / `DownloadEvent` types. This keeps the mobile API stable across internal refactors.

`crates/mobile-api` is **not** added to `Cargo.toml [workspace.members]` in Phase 20A itself — it is scaffolded as a stub (`lib.rs` + `Cargo.toml`) with the UniFFI proc-macro dependency declared but no actual FFI exports yet. The full implementation is Phase 20B.

---

## Implementation Plan

- [ ] 1. Create the `.agents/tasks/` directory and verify it exists (already done by this planning step).
      Files: `d:\Projects\MyDownloader\.agents\tasks\` (directory only)
      Verify: `Test-Path d:\Projects\MyDownloader\.agents\tasks` returns `True`.

- [ ] 2. Write `docs/ANDROID_BROWSER.md` — browser engine comparison and decision.
      Document the WebView vs GeckoView comparison table above, record the decision (GeckoView),
      and explain the rationale (privacy features, extension support, MPL 2.0 licence).
      Include the APK size trade-off and the minSdk/targetSdk/compileSdk choices with justification.
      Files: `docs/ANDROID_BROWSER.md` (new file)
      Verify: File exists and is non-empty; `cargo check` is unaffected (no Rust changes yet).

- [ ] 3. Write `docs/ANDROID_ARCHITECTURE.md` — Android application architecture document.
      Document the layered architecture:
      ```
      Android UI (Jetpack Compose)
        ↓
      Android ViewModel / use-case layer (Kotlin coroutines)
        ↓
      crates/mobile-api (UniFFI Kotlin bindings)
        ↓
      crates/core-services (existing Rust service graph)
        ↓
      crates/queue / crates/scheduler / crates/downloader / ...
      ```
      Document the module breakdown (`app`, `core`, `bridge`, `ui`), SDK versions,
      background download strategy (foreground service in Phase 20E), and how
      `CoreServices` is reused without duplication.
      Files: `docs/ANDROID_ARCHITECTURE.md` (new file)
      Verify: File exists and is non-empty.

- [ ] 4. Write `docs/ANDROID_PRIVACY.md` — privacy and tracker protection design document.
      Cover: GeckoView Enhanced Tracking Protection configuration, first-party isolation,
      fingerprinting resistance settings, private browsing sessions, WebRTC IP handling,
      cookie isolation strategy, and what Phase 20J will implement vs. what is configured
      at the foundation level. Reference the GeckoView API surface by name
      (`GeckoRuntimeSettings`, `ContentBlockingController`, `AntiTracking`).
      Files: `docs/ANDROID_PRIVACY.md` (new file)
      Verify: File exists and is non-empty.

- [ ] 5. Write `docs/ANDROID_EXTENSIONS.md` — extension architecture document.
      Cover: WebExtension support via GeckoView `WebExtensionController`, bundled
      built-in extensions (content blocker), user-installed extension flow (Phase 20K),
      relation to the existing desktop MV3 extension in `apps/browser/extension/`,
      and what the Phase 20K implementation will deliver.
      Files: `docs/ANDROID_EXTENSIONS.md` (new file)
      Verify: File exists and is non-empty.

- [ ] 6. Update `docs/ROADMAP.md` — expand Phase 20A entry.
      Replace the current one-line `Phase 20A — Android foundation` entry with a
      multi-line entry that mirrors the style of the Phase 18 and Phase 19 entries.
      Include: architecture summary, engine decision (GeckoView, MPL 2.0), SDK versions,
      module list, UniFFI bridge crate scaffold, new docs created.
      Do **not** change any other entries.
      Files: `docs/ROADMAP.md` (modify existing)
      Verify: `cargo check` passes (no Rust changes); the ROADMAP entry for Phase 20A
      is expanded and all other entries are unchanged.

- [ ] 7. Create the Gradle wrapper and root project files for `apps/android/`.
      This is the Android multi-module project root. Create:
      - `apps/android/settings.gradle.kts` — declares `rootProject.name`, includes
        `:app`, `:core`, `:bridge`, `:ui` modules.
      - `apps/android/build.gradle.kts` — root build file; applies `com.android.application`
        and `org.jetbrains.kotlin.android` plugins via `plugins {}` block with
        `apply false`; declares no top-level dependencies.
      - `apps/android/gradle.properties` — sets `android.useAndroidX=true`,
        `android.nonTransitiveRClass=true`, `org.gradle.jvmargs=-Xmx2g`,
        `kotlin.code.style=official`.
      - `apps/android/local.properties.example` — documents `sdk.dir` and `ndk.dir`
        variables; not committed as `local.properties` (add that to `.gitignore`).
      - `apps/android/.gitignore` — ignores `local.properties`, `build/`, `.gradle/`,
        `**/*.iml`.
      - `apps/android/gradle/libs.versions.toml` — version catalog declaring:
        `agp = "8.5.2"`, `kotlin = "2.0.21"`, `compose-bom = "2024.09.03"`,
        `geckoview = "131.0.20241001211527"` (latest stable as of late 2024),
        `material3` (via compose BOM), `activity-compose`, `navigation-compose`,
        `lifecycle-viewmodel-compose`, `kotlinx-coroutines-android`,
        `junit`, `espresso-core`, `compose-ui-test`.
      - `apps/android/gradlew` and `apps/android/gradlew.bat` — Gradle wrapper
        scripts (standard Gradle 8.x wrapper scripts; generate via
        `gradle wrapper --gradle-version 8.9` if Gradle is available, otherwise
        create the standard boilerplate manually).
      - `apps/android/gradle/wrapper/gradle-wrapper.properties` — points to
        Gradle 8.9 distribution URL.
      Files: all files listed above under `apps/android/`
      Verify: `cd apps\android && .\gradlew.bat tasks --all` completes without error
      (lists available tasks). If Gradle is not installed globally, verify by inspecting
      `settings.gradle.kts` includes all four modules and `libs.versions.toml` is
      syntactically valid.

- [ ] 8. Create the `:core` Android module.
      This module has no Android framework dependency — it is a pure Kotlin library
      (`com.android.library` with no Android-specific APIs in `src/main`).
      Create:
      - `apps/android/core/build.gradle.kts` — applies `com.android.library` and
        `org.jetbrains.kotlin.android`; `minSdk = 26`; no Android deps in `dependencies {}`.
      - `apps/android/core/src/main/AndroidManifest.xml` — minimal manifest
        (`package` attribute only).
      - `apps/android/core/src/main/kotlin/com/mydownloader/core/DownloadInfo.kt` —
        data class with fields: `id: String`, `url: String`, `filename: String`,
        `destinationPath: String`, `status: DownloadStatus`, `bytesDownloaded: Long`,
        `totalBytes: Long?`, `speedBps: Long?`, `etaSeconds: Long?`, `engine: String`,
        `error: String?`.
      - `apps/android/core/src/main/kotlin/com/mydownloader/core/DownloadStatus.kt` —
        enum: `QUEUED`, `PREPARING`, `DOWNLOADING`, `PAUSED`, `COMPLETED`, `FAILED`,
        `CANCELLED`. Matches `DownloadStatus` in `crates/models/src/download.rs`.
      - `apps/android/core/src/main/kotlin/com/mydownloader/core/DownloadEvent.kt` —
        sealed class with subclasses: `Started(id)`, `Progress(id, bytesDownloaded,
        totalBytes, speedBps, etaSeconds)`, `Paused(id)`, `Resumed(id)`,
        `Completed(id, destinationPath)`, `Failed(id, error)`, `Cancelled(id)`.
      - `apps/android/core/src/main/kotlin/com/mydownloader/core/DownloadService.kt` —
        interface with suspend functions: `addDownload(url, filename, destination,
        engine): String`, `pauseDownload(id)`, `resumeDownload(id)`,
        `cancelDownload(id)`, `removeDownload(id)`, `getDownload(id): DownloadInfo?`,
        `listDownloads(): List<DownloadInfo>`, `setPriority(id, priority: Int)`,
        `scheduleDownload(id, atEpochMillis: Long)`. Plus `val events: Flow<DownloadEvent>`.
      - `apps/android/core/src/test/kotlin/com/mydownloader/core/DownloadStatusTest.kt` —
        unit test verifying all enum values exist and their `name` strings are stable.
      Files: all files listed above under `apps/android/core/`
      Verify: `cd apps\android && .\gradlew.bat :core:assembleDebug` succeeds.

- [ ] 9. Create the `:bridge` Android module stub.
      This module will host the UniFFI-generated bindings in Phase 20B.
      In Phase 20A it provides a `StubDownloadService` that implements `DownloadService`
      with no-op or in-memory implementations, so the `:app` module can compile.
      Create:
      - `apps/android/bridge/build.gradle.kts` — applies `com.android.library` and
        `kotlin`; `minSdk = 26`; depends on `:core`.
      - `apps/android/bridge/src/main/AndroidManifest.xml` — minimal manifest.
      - `apps/android/bridge/src/main/kotlin/com/mydownloader/bridge/StubDownloadService.kt` —
        implements `DownloadService` with:
        - `addDownload(...)` returns a `UUID.randomUUID().toString()` and logs a warning
          that the real bridge is not yet built.
        - All other operations are no-ops or return empty/null values.
        - `events` is `MutableSharedFlow<DownloadEvent>()` that never emits.
        - Every function logs `"[StubDownloadService] Not yet connected to Rust engine"`
          via `android.util.Log.w`.
      Files: all files listed above under `apps/android/bridge/`
      Verify: `cd apps\android && .\gradlew.bat :bridge:assembleDebug` succeeds.

- [ ] 10. Create the `:ui` Android module — shared Compose theme.
       Create:
       - `apps/android/ui/build.gradle.kts` — applies `com.android.library` + kotlin +
         compose; `minSdk = 26`; depends on compose BOM, `material3`,
         `activity-compose`.
       - `apps/android/ui/src/main/AndroidManifest.xml` — minimal manifest.
       - `apps/android/ui/src/main/kotlin/com/mydownloader/ui/theme/Color.kt` —
         Material 3 colour tokens for a dark-capable privacy-focused palette
         (deep navy primary, neutral surface, red error). No branding/logo assets.
       - `apps/android/ui/src/main/kotlin/com/mydownloader/ui/theme/Typography.kt` —
         Material 3 `Typography` object using system fonts only (no custom font assets).
       - `apps/android/ui/src/main/kotlin/com/mydownloader/ui/theme/Theme.kt` —
         `MyDownloaderTheme` composable wrapping `MaterialTheme` with light/dark
         support via `isSystemInDarkTheme()`.
       Files: all files listed above under `apps/android/ui/`
       Verify: `cd apps\android && .\gradlew.bat :ui:assembleDebug` succeeds.

- [ ] 11. Create the `:app` Android module.
       This is the application entry point. It hosts `MainActivity`, the Compose
       `NavHost`, and the Hilt (or manual) DI wiring. Use manual DI (no Hilt) in
       Phase 20A to avoid adding an annotation-processor dependency that isn't yet
       justified.
       Create:
       - `apps/android/app/build.gradle.kts` — applies `com.android.application` +
         kotlin + compose; `namespace = "com.mydownloader.app"`;
         `minSdk = 26`, `targetSdk = 35`, `compileSdk = 35`;
         `versionCode = 1`, `versionName = "0.1.0"`;
         depends on `:core`, `:bridge`, `:ui`; depends on `activity-compose`,
         `navigation-compose`, `lifecycle-viewmodel-compose`, `kotlinx-coroutines-android`.
       - `apps/android/app/src/main/AndroidManifest.xml` — declares `MainActivity`
         with `intent-filter` for `MAIN`/`LAUNCHER`; declares
         `android:name=".MyDownloaderApp"` application class; includes
         `INTERNET` permission only (no storage, no foreground-service yet —
         those are Phase 20E).
       - `apps/android/app/src/main/kotlin/com/mydownloader/app/MyDownloaderApp.kt` —
         `Application` subclass; initialises `StubDownloadService` as a singleton
         stored in a companion object (manual service locator, replaced in Phase 20B
         with real bridge).
       - `apps/android/app/src/main/kotlin/com/mydownloader/app/MainActivity.kt` —
         `ComponentActivity`; calls `setContent { MyDownloaderTheme { AppNavHost() } }`.
       - `apps/android/app/src/main/kotlin/com/mydownloader/app/navigation/AppNavHost.kt` —
         `NavHost` with two routes: `"home"` (placeholder `HomeScreen`) and
         `"downloads"` (placeholder `DownloadsScreen`). Navigation between them is
         not yet wired to any UI affordance — that is Phase 20D.
       - `apps/android/app/src/main/kotlin/com/mydownloader/app/ui/HomeScreen.kt` —
         a minimal composable showing the app name ("MyDownloader") in a `Scaffold`
         with a `TopAppBar`. No download list, no browser — those are later phases.
       - `apps/android/app/src/main/kotlin/com/mydownloader/app/ui/DownloadsScreen.kt` —
         a minimal composable showing "Downloads — not yet implemented" placeholder text.
       - `apps/android/app/src/main/res/values/strings.xml` — defines `app_name =
         "MyDownloader"`.
       - `apps/android/app/src/main/res/mipmap-*/ic_launcher*.png` — placeholder
         launcher icon files (use Android Studio's default adaptive icon or a single
         color PNG; do NOT use any third-party branding assets).
       Files: all files listed above under `apps/android/app/`
       Verify: `cd apps\android && .\gradlew.bat :app:assembleDebug` succeeds and
       produces `app/build/outputs/apk/debug/app-debug.apk`.

- [ ] 12. Scaffold `crates/mobile-api` — the future UniFFI bridge crate.
       Phase 20A creates the crate skeleton only. No UniFFI proc-macros or FFI exports
       yet — that is Phase 20B.
       Create:
       - `crates/mobile-api/Cargo.toml` — name `mydownloader-mobile-api`;
         edition/version/rust-version from workspace; depends on
         `mydownloader-core-services`, `mydownloader-models`, `tokio` (workspace),
         `serde` (workspace), `thiserror` (workspace); `[lib] crate-type = ["cdylib",
         "staticlib", "rlib"]`; `[lints] workspace = true`.
       - `crates/mobile-api/src/lib.rs` — `#![forbid(unsafe_code)]`; a single
         `// TODO(Phase 20B): UniFFI exports` comment and a public `pub mod api;`
         declaration.
       - `crates/mobile-api/src/api.rs` — defines placeholder types:
         `pub struct DownloadInfo { ... }` (mirroring `crates/core/` model fields)
         and an empty `pub struct MobileApi;` with a `pub fn version() -> String`
         returning `env!("CARGO_PKG_VERSION")`. No `#[uniffi::export]` yet.
       - Do **not** add `crates/mobile-api` to `Cargo.toml [workspace.members]`
         yet — the crate uses `cdylib` which requires cross-compilation targets that
         aren't configured in the workspace CI. It is added to the workspace in
         Phase 20B when the Android NDK toolchain is set up.
       Files: `crates/mobile-api/Cargo.toml`, `crates/mobile-api/src/lib.rs`,
       `crates/mobile-api/src/api.rs`
       Verify: `cargo check -p mydownloader-mobile-api` fails with "not a workspace
       member" at this stage — that is expected and correct. Instead verify:
       `cargo check` (workspace) still passes unchanged, confirming no existing crate
       was broken.

- [ ] 13. Run the workspace Rust build and confirm no regressions.
       Files: none (validation only)
       Verify: `cargo check --workspace` passes with zero errors. `cargo test
       --workspace` passes with zero new failures. Run from `d:\Projects\MyDownloader`.

- [ ] 14. Run the Android build end-to-end and confirm the debug APK is produced.
       Files: none (validation only)
       Verify: `cd apps\android && .\gradlew.bat :app:assembleDebug` exits 0 and
       `apps/android/app/build/outputs/apk/debug/app-debug.apk` exists.

---

## Order of Operations Summary

1. Docs first (items 2–5) — these have no build dependencies.
2. ROADMAP update (item 6) — after docs are written so the entry can reference them.
3. Gradle root setup (item 7) — Android project skeleton before any module.
4. Library modules in dependency order: `:core` (item 8) → `:bridge` (item 9) →
   `:ui` (item 10) → `:app` (item 11).
5. Rust mobile-api scaffold (item 12) — independent of Android build; done last to
   avoid any accidental workspace build breakage during Android setup.
6. Validation (items 13–14) — confirm both build systems are clean.

---

## Files to Create / Modify

### New documentation files
- `docs/ANDROID_BROWSER.md`
- `docs/ANDROID_ARCHITECTURE.md`
- `docs/ANDROID_PRIVACY.md`
- `docs/ANDROID_EXTENSIONS.md`

### Modified documentation files
- `docs/ROADMAP.md` — expand Phase 20A entry only

### New Android project files
```
apps/android/
  .gitignore
  settings.gradle.kts
  build.gradle.kts
  gradle.properties
  local.properties.example
  gradlew
  gradlew.bat
  gradle/
    libs.versions.toml
    wrapper/
      gradle-wrapper.jar
      gradle-wrapper.properties
  app/
    build.gradle.kts
    src/main/
      AndroidManifest.xml
      kotlin/com/mydownloader/app/
        MyDownloaderApp.kt
        MainActivity.kt
        navigation/AppNavHost.kt
        ui/HomeScreen.kt
        ui/DownloadsScreen.kt
      res/
        values/strings.xml
        mipmap-mdpi/ic_launcher.png
        mipmap-hdpi/ic_launcher.png
        mipmap-xhdpi/ic_launcher.png
        mipmap-xxhdpi/ic_launcher.png
        mipmap-xxxhdpi/ic_launcher.png
  core/
    build.gradle.kts
    src/main/
      AndroidManifest.xml
      kotlin/com/mydownloader/core/
        DownloadInfo.kt
        DownloadStatus.kt
        DownloadEvent.kt
        DownloadService.kt
    src/test/kotlin/com/mydownloader/core/
      DownloadStatusTest.kt
  bridge/
    build.gradle.kts
    src/main/
      AndroidManifest.xml
      kotlin/com/mydownloader/bridge/
        StubDownloadService.kt
  ui/
    build.gradle.kts
    src/main/
      AndroidManifest.xml
      kotlin/com/mydownloader/ui/theme/
        Color.kt
        Typography.kt
        Theme.kt
```

### New Rust crate files (Phase 20B scaffold, not added to workspace yet)
```
crates/mobile-api/
  Cargo.toml
  src/
    lib.rs
    api.rs
```

---

## Phase 20A Explicit Non-Goals

The following are explicitly **not** in Phase 20A scope:

- UniFFI proc-macro attributes and actual FFI exports (Phase 20B)
- Android NDK / cross-compilation configuration (Phase 20B)
- `crates/mobile-api` added to workspace (Phase 20B)
- Real download UI with list, progress, speed, ETA (Phase 20D)
- Background foreground service (Phase 20E)
- GeckoView browser shell (Phase 20F)
- Download interception (Phase 20G)
- Tabs / private browsing (Phase 20H)
- History / bookmarks (Phase 20I)
- Tracker protection / privacy controls (Phase 20J)
- Extension management (Phase 20K)
- Any Tor/VPN integration (Phase 20M)
- Production signing / release build (Phase 20P)
- Any desktop `AppState` migration

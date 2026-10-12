# Phase 20C — Android Rust Native Build & Runtime Integration

## Status: PARTIAL

The Rust native libraries are cross-compiled and packaged into the APK for both
arm64-v8a and x86_64. The physical device install succeeds, but bridge
initialization fails at runtime due to a JNA configuration issue in the
generated Kotlin bindings. Bridge activation is not confirmed on device.

---

## Environment Setup

- **NDK version:** 27.2.12479018 (r27)
- **NDK path:** `D:\Android\Sdk\ndk\27.2.12479018`
- **cargo-ndk version:** 4.1.2
- **Rust Android targets:**
  - `aarch64-linux-android` (arm64-v8a) — installed
  - `x86_64-linux-android` (x86_64) — installed
- **`.cargo/config.toml`:** Updated with Android NDK linker paths for both targets.
  - arm64 linker: `D:\Android\Sdk\ndk\27.2.12479018\toolchains\llvm\prebuilt\windows-x86_64\bin\aarch64-linux-android33-clang.cmd`
  - x86_64 linker: `D:\Android\Sdk\ndk\27.2.12479018\toolchains\llvm\prebuilt\windows-x86_64\bin\x86_64-linux-android33-clang.cmd`

---

## Native Build

- **arm64-v8a library:** `apps/android/bridge/src/main/jniLibs/arm64-v8a/libmydownloader_android_bridge.so` — 12,569,768 bytes
- **x86_64 library:** `apps/android/bridge/src/main/jniLibs/x86_64/libmydownloader_android_bridge.so` — 12,257,992 bytes
- **x86_64 built:** Yes
- **Build command:**
  ```
  cargo ndk -t arm64-v8a -o apps/android/bridge/src/main/jniLibs build --release -p mydownloader-android-bridge
  ```
- **Build script:** `scripts/build-android-native.ps1`
  - Usage: `.\scripts\build-android-native.ps1 -Release`
  - Builds both ABIs and copies `.so` files to `jniLibs`.

---

## APK Packaging

- **APK path:** `apps/android/app/build/outputs/apk/debug/app-debug.apk`
- **APK size:** 35,605,441 bytes (~34 MB)
- **Contains arm64-v8a .so:** Yes (12,569,768 bytes in APK)
- **Contains x86_64 .so:** Yes (12,257,992 bytes in APK)
- **jniLibs location:** `apps/android/bridge/src/main/jniLibs`
- **Gradle wiring:** `:bridge` module `sourceSets { main { jniLibs.srcDirs("src/main/jniLibs") } }`
- **:bridge JVM unit tests:** All pass

---

## Physical Device Test

- **Device connected:** Yes
- **APK installed:** Yes
- **App launched:** Yes (UI visible briefly)
- **UnsatisfiedLinkError:** Yes — JNA's `libjnidispatch.so` not found
- **Bridge load confirmed:** No
- **Bridge init confirmed:** No
- **Overall result:** FAILED

### Device Test Details

Two distinct runtime failures were observed:

**ISSUE 1 — Bridge load failure (primary blocker)**

```
java.lang.UnsatisfiedLinkError: Native library
(com/sun/jna/android-aarch64/libjnidispatch.so)
not found in resource path (.)
```

The generated `android_bridge.kt` uses a JNA-based UniFFI scaffold. JNA requires
its own `libjnidispatch.so` to be bundled inside the APK. This library comes from
the JNA Android AAR variant (`net.java.dev.jna:jna:X.Y.Z:@aar`).

Root cause options:
- The `:bridge` module depends on `net.java.dev.jna:jna` (the plain JVM JAR) rather
  than the Android AAR variant that bundles `libjnidispatch.so`.
- Or the Kotlin bindings were generated with a uniffi-bindgen version that does not
  match the Rust `uniffi 0.29` crate. UniFFI 0.29 should not require JNA at all —
  it uses its own FFI scaffold. If the bindings were generated with an older
  uniffi-bindgen they may reference JNA classes that are unnecessary.

The `libmydownloader_android_bridge.so` was present in the APK and did not cause
any error — the failure fires at JNA initialization before `System.loadLibrary` is
even reached.

**ISSUE 2 — ViewModel factory crash (secondary)**

```
FATAL EXCEPTION: main
java.lang.RuntimeException: Cannot create an instance of class
dev.mydownloader.downloads.ui.DownloadsViewModel
Caused by: java.lang.NoSuchMethodException: DownloadsViewModel.<init> []
```

`DownloadsViewModel` requires a constructor argument (a `DownloadRepository`)
but was instantiated via `viewModel()` without a `ViewModelProvider.Factory`.
This is independent of the native build issue.

---

## Validation Results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace` | PASS |
| `cargo test --workspace` | PASS |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS |
| Android `assembleDebug` | PASS |
| Android `:bridge:test` | PASS |

---

## Files Added/Modified

| File | Change |
|---|---|
| `.cargo/config.toml` | Added Android NDK linker configuration for `aarch64-linux-android` and `x86_64-linux-android` |
| `scripts/build-android-native.ps1` | New: one-shot build script for Rust Android native libraries |
| `apps/android/bridge/build.gradle.kts` | Added `jniLibs.srcDirs` configuration |
| `apps/android/bridge/src/main/jniLibs/arm64-v8a/.gitkeep` | Placeholder to preserve directory in git |
| `apps/android/bridge/src/main/jniLibs/x86_64/.gitkeep` | Placeholder to preserve directory in git |
| `.gitignore` | Added `*.so` exclusion for `jniLibs` directories |
| `.agents/tasks/phase-20c-setup.json` | Phase artifact: NDK/toolchain setup record |
| `.agents/tasks/phase-20c-native-build.json` | Phase artifact: native build record |
| `.agents/tasks/phase-20c-apk.json` | Phase artifact: APK packaging record |
| `.agents/tasks/phase-20c-device-test.json` | Phase artifact: physical device test record |
| `docs/ANDROID_ARCHITECTURE.md` | Added "Native Build Pipeline (Phase 20C)" section |
| `docs/ROADMAP.md` | Updated Phase 20B limitations note; added Phase 20C entry |

---

## Remaining Limitations

1. **JNA libjnidispatch.so missing from APK** — The primary blocker. The bridge
   cannot initialize on device. Fix: switch to the JNA Android AAR dependency
   (`net.java.dev.jna:jna:5.x.x:@aar`) or regenerate `android_bridge.kt` using
   the exact `uniffi-bindgen` version that corresponds to the Rust `uniffi = "0.29"`
   crate dependency (UniFFI 0.29 bindings should not use JNA).

2. **DownloadsViewModel missing factory** — The ViewModel is instantiated without
   a `ViewModelProvider.Factory`. Fix: add a `companion object` factory or use
   Hilt's `@HiltViewModel`.

3. **Debug .so size** — The `libmydownloader_android_bridge.so` files are ~12 MB
   each in debug mode. Release builds with `strip = true` in the Cargo profile will
   reduce this significantly (expected <3 MB stripped).

4. **DownloadPriority.CRITICAL unmapped** — Inherited from Phase 20B; mapped to
   `High` at the bridge layer.

5. **No ProcessLifecycleOwner shutdown** — Bridge lifetime is coextensive with the
   process. `Application.onTerminate()` is emulator-only; proper shutdown hook not
   yet wired.

---

## Recommended Next Phase

Before continuing to Phase 20D (Background downloads), the two runtime failures
should be resolved as a Phase 20C fixup:

1. **Resolve JNA / UniFFI binding mismatch** — Determine the exact
   `uniffi-bindgen` version, regenerate `android_bridge.kt`, and verify the
   bindings do not reference JNA. If JNA is legitimately required by the chosen
   UniFFI version, add `net.java.dev.jna:jna:5.14.0:@aar` to `:bridge`
   dependencies. This is the critical path item.

2. **Fix DownloadsViewModel factory** — Add a `ViewModelProvider.Factory` or
   migrate to Hilt so the ViewModel can receive its `DownloadRepository` argument.

3. **Re-run device test** after both fixes to confirm bridge initialization and
   a clean smoke test: library load → bridge init → CoreServices init → SQLite
   init → listDownloads.

Once runtime bridge activation is confirmed on device, Phase 20D (ForegroundService,
WorkManager, background download continuity) is the logical next step.

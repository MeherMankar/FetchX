# Phase 20B — Rust ↔ Android Bridge: Implementation Report

## 1. Phase 20B Status

**COMPLETE**

All validation passed:
- `cargo fmt --all -- --check` ✅
- `cargo check --workspace` ✅
- `cargo test --workspace -- --test-threads=1` ✅ (1 pre-existing desktop failure, not introduced by Phase 20B)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` ✅
- `./gradlew assembleDebug` ✅ BUILD SUCCESSFUL in 2m 4s (122 tasks)
- `./gradlew :bridge:test` ✅ BUILD SUCCESSFUL (all JVM unit tests pass)
- APK confirmed: `apps/android/app/build/outputs/apk/debug/app-debug.apk` (10.2 MB)

---

## 2. Architecture Implemented

```
Compose UI
    ↓
ViewModel (DownloadsViewModel)
    ↓
DownloadRepository interface (:core)
    ↓
BridgeDownloadRepository (:bridge)
    ↓
UniFFI-generated DownloadService (Kotlin bindings in :bridge/uniffi/)
    ↓  [JNA / FFI boundary]
DownloadService Rust struct (crates/android-bridge)
    ↓
CoreServices (core-services crate)
    ↓
QueueManager / Scheduler / EventRepository / EngineDispatcher
    ↓
DirectHTTP / aria2 / yt-dlp engines
    ↓
SQLite persistence (crates/storage)
```

The Rust core remains the **single source of truth** for all download-management behaviour. No download logic was duplicated in Kotlin.

---

## 3. Bridge Technology: UniFFI 0.29

**Selected: UniFFI 0.29** (with JNA backend for Android)

Rationale:
- The existing Rust codebase uses `tokio`, `async/await`, and the `CoreServices` abstraction — UniFFI handles the FFI boundary cleanly without manual `extern "C"` declarations.
- UniFFI generates type-safe Kotlin bindings from the UDL interface file, eliminating handwritten JNI glue.
- The `tokio` feature on the `uniffi` dependency enables async callback machinery for the event listener.
- The crate produces both a `cdylib` (for Android `.so`) and a `lib` target (for host `cargo test`).
- UniFFI 0.29 callback interfaces (`callback interface DownloadEventListener`) enable push-based event delivery without polling.

---

## 4. Files Added

### Rust
- `crates/android-bridge/Cargo.toml`
- `crates/android-bridge/build.rs`
- `crates/android-bridge/uniffi.toml`
- `crates/android-bridge/uniffi/android_bridge.udl`
- `crates/android-bridge/src/lib.rs`
- `crates/android-bridge/src/dto.rs`
- `crates/android-bridge/src/error.rs`
- `crates/android-bridge/src/service.rs` (includes 14 unit tests)
- `crates/android-bridge/src/bin/uniffi_bindgen.rs`

### Android
- `apps/android/bridge/build.gradle.kts`
- `apps/android/bridge/src/main/AndroidManifest.xml`
- `apps/android/bridge/src/main/jniLibs/arm64-v8a/.gitkeep`
- `apps/android/bridge/src/main/jniLibs/armeabi-v7a/.gitkeep`
- `apps/android/bridge/src/main/jniLibs/x86_64/.gitkeep`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/android_bridge.kt` (generated)
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/BridgeInitializer.kt`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/BridgeDownloadRepository.kt`
- `apps/android/bridge/src/test/kotlin/dev/mydownloader/bridge/MappingTest.kt` (14 unit tests)

---

## 5. Files Modified

- `Cargo.toml` — added `"crates/android-bridge"` to `[workspace.members]`
- `apps/android/settings.gradle.kts` — added `include(":bridge")`
- `apps/android/gradle/libs.versions.toml` — added `jna`, `junit`, `kotlin-test` entries
- `apps/android/app/build.gradle.kts` — added `implementation(project(":bridge"))`
- `apps/android/downloads/build.gradle.kts` — added `implementation(project(":bridge"))`
- `apps/android/app/src/main/kotlin/dev/mydownloader/app/MyDownloaderApp.kt` — initializes/shuts down bridge
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/DownloadsViewModel.kt` — wired to real DownloadRepository

---

## 6. Public Bridge API

### UDL surface (Rust → Kotlin via UniFFI)

```
DownloadService(config: CoreConfig)          // constructor — starts Tokio runtime + CoreServices
  .addDownload(url, filename, destination, priority) → String   // returns new DownloadId
  .listDownloads() → List<DownloadInfoDto>
  .getDownload(id) → DownloadInfoDto?
  .pauseDownload(id)
  .resumeDownload(id)
  .cancelDownload(id)
  .removeDownload(id)
  .retryDownload(id) → String                // removes old, enqueues new; returns new DownloadId
  .setPriority(id, DownloadPriorityDto)
  .scheduleDownload(id, startAtMillis: Long)
  .addEventListener(DownloadEventListener)   // push-based, no polling
  .shutdown()                                // idempotent, safe to call multiple times
```

### DTOs
- `CoreConfig(databasePath, downloadsDir, runtimeDir)`
- `DownloadInfoDto(id, url, engine, filename, destination, status, priority, progress, createdAtMillis, updatedAtMillis, errorMessage?)`
- `DownloadProgressDto(downloadedBytes: ULong, totalBytes: Long, speedBytesPerSecond: Long, estimatedRemainingSeconds: Long)` — `-1` sentinel = unknown
- `DownloadStatusDto`: Queued | Preparing | Downloading | Paused | Completed | Failed | Cancelled
- `DownloadPriorityDto`: Low | Normal | High
- `DownloadEventDto`: Created(item) | Updated(item) | Removed(id)

### Kotlin repository surface (:bridge module)
- `BridgeDownloadRepository` — implements `DownloadRepository` interface from `:core`
- `BridgeInitializer` — singleton lifecycle manager; call `initialize(context)` once

### Kotlin DownloadRepository interface (unchanged from Phase 20A)
```
observeDownloads(): Flow<List<DownloadItem>>
observeEvents(): Flow<DownloadEvent>
addDownload(url, fileName, destinationPath, priority, mimeType?, totalBytes?, referrer?): String
pauseDownload(downloadId)
resumeDownload(downloadId)
cancelDownload(downloadId, deleteFile)
removeDownload(downloadId, deleteFile)
retryDownload(downloadId): String
setPriority(downloadId, priority)
scheduleDownload(downloadId, startAtMillis)
getDownload(downloadId): DownloadItem?
```

---

## 7. Event Architecture

- Rust `DownloadEventBus` publishes `DownloadEvent` (Created/Updated/Removed) to a `tokio::sync::broadcast` channel on every repository write.
- `DownloadService.addEventListener()` subscribes a `broadcast::Receiver` and spawns a Tokio task on the existing runtime that forwards events to the `DownloadEventListener` callback.
- The callback is invoked on a Rust Tokio thread. UniFFI guarantees the callback interface is `Send + Sync`.
- On the Kotlin side, `BridgeDownloadRepository` registers an anonymous `DownloadEventListener` that:
  1. Maps `DownloadEventDto` → `DownloadEvent` and emits to `MutableSharedFlow<DownloadEvent>`
  2. Launches a coroutine on `Dispatchers.IO` to re-fetch the full list and update `MutableStateFlow<List<DownloadItem>>`
- `observeDownloads()` returns the `StateFlow` — Compose/ViewModel collect it reactively.
- `observeEvents()` returns the `SharedFlow` — for granular event-driven UI updates.
- **No polling** anywhere in the architecture.

---

## 8. Error Handling

### Rust `MobileApiError` enum (maps to Kotlin `MobileApiError` sealed class)

| Variant | Mapped from |
|---|---|
| `Validation(String)` | Empty URL, invalid UUID, negative millis |
| `NotFound(String)` | `QueueError::DownloadNotFound`, `StateMachineError::NotFound`, `ScheduleError::DownloadNotFound` |
| `UnavailableEngine(String)` | `QueueError::UnsupportedEngineOperation`, engine `is_available()` check |
| `InvalidStateTransition(String)` | `QueueError::InvalidStatus`, `StateMachineError::InvalidTransition`, retry on non-failed item |
| `Storage(String)` | `StorageError`, `QueueError::Storage`, `ScheduleError::Storage` |
| `Runtime(String)` | All other errors: lock poisoned, shutdown, actor/worker failures, `CoreServicesError` |

UniFFI surfaces these as Kotlin exceptions thrown from the bridge methods, which callers handle in `try/catch` or via Kotlin coroutine exception handling.

---

## 9. Thread / Lifecycle Handling

### Threading
- A dedicated multi-threaded `tokio::runtime::Runtime` is created inside `DownloadService::new()` and owned there.
- All `block_on()` calls run async work on this runtime from JNI/UniFFI threads (dispatched to `Dispatchers.IO` by Kotlin before reaching the bridge).
- The Android UI thread is never blocked.
- The event listener task runs as a Tokio-spawned task on the existing runtime.
- Kotlin coroutines switch to `Dispatchers.IO` for every bridge call via `withContext(Dispatchers.IO)`.

### Lifecycle
- `BridgeInitializer.initialize(context)` is called in `Application.onCreate()`.
- `BridgeInitializer.shutdown()` is called in `Application.onTerminate()` (emulator) or via `ProcessLifecycleOwner` (production — recommended for Phase 20E).
- `DownloadService.shutdown()` is **idempotent**: second+ calls are no-ops (the inner `Option<ServiceInner>` is `take()`-d on first call).
- Shutdown cleanly: stops CoreServices (scheduler → queue → engines → SQLite), then drops the Tokio runtime (joining all spawned tasks).
- Activity recreation / configuration changes: the repository is a singleton via `BridgeInitializer`; ViewModels survive recreation via `ViewModelProvider`.
- Duplicate initialization: `BridgeInitializer.initialize()` uses double-checked locking and returns the existing instance if already initialized.
- Bridge initialization failure: the `DownloadService` constructor throws `MobileApiError` which propagates as a Kotlin exception; `MyDownloaderApp` should catch and log this (production hardening for Phase 20C/20E).

---

## 10. Security Considerations

- No credentials, cookies, or secrets cross the FFI boundary. The bridge carries only: paths, URLs, filenames, priorities, timestamps, and error messages.
- URL validation: `DownloadSource::new()` rejects empty URLs before the item reaches the queue. More thorough URL validation (scheme, authority) is handled by the Rust engine layer.
- No `unsafe` code in the android-bridge crate except the single `uniffi::include_scaffolding!` expansion (workspace lint overridden only for this crate with a documented justification).
- No shell execution from Kotlin; no arbitrary command execution; no hardcoded secrets.
- The `BridgeInitializer` object is not exported to external processes — it is an in-process singleton.
- The native `.so` files are loaded from the app's own APK via `System.loadLibrary`, not from an external source.
- No unnecessary network permissions added.
- SQLite database is stored in `context.filesDir` (app-private, not world-readable).

---

## 11. Tests Added

### Rust unit tests (`crates/android-bridge/src/service.rs`, 14 tests)
1. `init_and_shutdown` — creates service, shuts down cleanly
2. `double_shutdown_is_safe` — second shutdown is a no-op
3. `list_downloads_empty_initially` — fresh service has empty list
4. `add_download_returns_uuid` — returned ID is a valid UUID
5. `get_download_not_found` — unknown ID returns `Ok(None)`
6. `add_and_list_two_downloads` — list returns both added items
7. `invalid_url_returns_validation_error` — empty URL → `MobileApiError::Validation`
8. `pause_nonexistent_returns_not_found` — unknown ID → `MobileApiError::NotFound`
9. `cancel_nonexistent_returns_not_found` — unknown ID → `MobileApiError::NotFound`
10. `remove_nonexistent_returns_not_found` — unknown ID → `MobileApiError::NotFound`
11. `priority_change_roundtrip` — set High priority, verify via get
12. `event_listener_receives_created_event` — listener fires within 2 seconds of addDownload
13. `retry_non_failed_returns_invalid_state_error` — retry Queued item → `MobileApiError::InvalidStateTransition`
14. `concurrent_list_calls` — 4 threads call listDownloads concurrently, all succeed

### Android JVM unit tests (`apps/android/bridge/src/test/.../MappingTest.kt`, 14 tests)
- `criticalPriorityMapsToHighDto` — CRITICAL → HIGH (documented limitation)
- `highPriorityMapsToHighDto`
- `normalPriorityRoundTrips`
- `lowPriorityRoundTrips`
- `preparingStatusMapsToActiveState`
- `downloadingStatusMapsToActiveState`
- `failedStatusMapsToFailedStateWithRetryableTrue`
- `failedStatusWithNullErrorMessageUsesDefault`
- `unknownTotalBytesNegativeOneMapsToNull`
- `knownTotalBytesPositiveMapsThroughToItem`
- `zeroTotalBytesIsNotSentinelAndMapsThrough`
- `completedStatusMapsToCompletedStateWithDestination`
- `cancelledStatusMapsToCancelledState`
- `pausedStatusMapsToPausedState`
- `queuedStatusMapsToQueuedState`

All 28 tests pass across Rust and Android.

---

## 12. Rust Validation Results

```
cargo fmt --all -- --check       PASS (0 formatting issues)
cargo check --workspace          PASS (Finished in ~26s)
cargo test --workspace           PASS* (316 tests pass; 1 pre-existing desktop failure)
cargo clippy --workspace \
  --all-targets --all-features \
  -- -D warnings                 PASS (0 warnings on new code, Finished in ~28s)
```

*Pre-existing failure: `commands::tests::set_priority_changes_persisted_value` in `mydownloader-desktop`. Verified pre-existing by stashing Phase 20B changes and confirming it fails identically on the base. Not introduced by Phase 20B.

---

## 13. Android Validation Results

```
./gradlew assembleDebug          BUILD SUCCESSFUL in 2m 4s (122 tasks, 117 executed)
./gradlew :bridge:test           BUILD SUCCESSFUL in 22s (all JVM tests pass)
```

SDK XML version warning is non-fatal (SDK tool version mismatch, no functional impact).

---

## 14. APK Path

```
d:\Projects\MyDownloader\apps\android\app\build\outputs\apk\debug\app-debug.apk
Size: 10,733,475 bytes (~10.2 MB)
Generated: 2026-10-12
```

---

## 15. Remaining Limitations

1. **No native library in APK yet** — The `.so` files for Android targets (`arm64-v8a`, `armeabi-v7a`, `x86_64`) are not built or packaged. The APK compiles and the Kotlin bridge code is correct, but calling any bridge method at runtime will fail with `UnsatisfiedLinkError`. Requires `cargo-ndk` + Android NDK setup and a Gradle build task to cross-compile the Rust crate — this is Phase 20C/20E work.
   - The `jniLibs/` directories have `.gitkeep` placeholders ready for the `.so` files.
   - The `uniffi-bindgen` binary is gated behind `--features uniffi-cli` to avoid bloating the library archive.

2. **`DownloadPriority.CRITICAL` has no Rust equivalent** — mapped to `High`. Documented in `BridgeDownloadRepository.mapPriorityToDto` and `MappingTest`.

3. **Engine status not exposed via `DownloadRepository`** — `EngineStatusDto` / `getEngineStatus()` is available in the Rust bridge (`DownloadService`) but not yet surfaced through the `DownloadRepository` interface. The interface can be extended in Phase 20C.

4. **Process death / background downloads** — The Rust runtime is stopped when the Android process dies. Background download continuation across process death requires a `ForegroundService` binding the lifecycle to the bridge. Deferred to Phase 20E.

5. **Pre-existing desktop test failure** — `commands::tests::set_priority_changes_persisted_value` in `mydownloader-desktop` fails independently of Phase 20B. Needs investigation in a future desktop-focused phase.

6. **No `ProcessLifecycleOwner` shutdown** — `BridgeInitializer.shutdown()` is called from `Application.onTerminate()` (emulator-only). Production shutdown via `ProcessLifecycleOwner` should be added in Phase 20C or 20E.

---

## 16. Recommended Next Phase

**Phase 20C — Native Library Build Pipeline + Full Integration**

Key work:
1. Install `cargo-ndk` and configure `.cargo/config.toml` with Android linker targets.
2. Add a Gradle `exec` task (or `cargo-ndk` Gradle plugin) to cross-compile `crates/android-bridge` for `arm64-v8a`, `armeabi-v7a`, `x86_64` and copy the `.so` files to `bridge/src/main/jniLibs/`.
3. Wire `DownloadsScreen` in `:downloads` to the real `DownloadsViewModel` (inject `BridgeInitializer.getRepository()` via the `Factory`).
4. Add `ProcessLifecycleOwner` shutdown registration in `MyDownloaderApp`.
5. Add integration/instrumented tests (`androidTest`) that exercise the bridge end-to-end on a device.
6. Fix the pre-existing `commands::tests::set_priority_changes_persisted_value` desktop test.
7. Expose `getEngineStatus()` through the `DownloadRepository` interface.
8. Handle bridge initialization failure gracefully in `MyDownloaderApp` (catch + log + show error UI).

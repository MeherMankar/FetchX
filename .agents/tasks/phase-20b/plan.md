# Implementation Plan — Phase 20B: Rust ↔ Android UniFFI Bridge

## Ground truth confirmed by reading the codebase

- `DownloadRepository::get(id: DownloadId) -> Result<Option<DownloadItem>, StorageError>` — confirmed in `crates/storage/src/repository.rs`
- `DownloadRepository::list() -> Result<Vec<DownloadItem>, StorageError>` — confirmed same file
- `DownloadEventBus::subscribe() -> broadcast::Receiver<DownloadEvent>` — confirmed in `crates/queue/src/events.rs`
- `Scheduler::schedule_download(id, scheduled_at: Timestamp)` — confirmed in `crates/scheduler/src/lib.rs`
- `QueueManager::enqueue(item: DownloadItem) -> Result<(), QueueError>` — confirmed in `crates/queue/src/manager.rs`
- `DownloadPriority` has exactly 3 variants: `Low`, `Normal`, `High` — confirmed in `crates/models/src/priority.rs`
- Retry pattern: `queue().remove_download(old_id)` + `queue().enqueue(new_item_with_new_id)` — confirmed in `crates/cli/src/commands/lifecycle.rs`
- `DownloadStatus` has: `Queued`, `Preparing`, `Downloading`, `Paused`, `Completed`, `Failed`, `Cancelled`
- `DownloadProgress` fields: `downloaded_bytes: u64`, `total_bytes: Option<u64>`, `speed_bytes_per_second: Option<u64>`, `estimated_remaining_seconds: Option<u64>`
- NDK/cargo-ndk NOT installed on host. Gradle build MUST succeed without them (use `buildRustBridge` property guard).
- Host Rust target is `x86_64-pc-windows-msvc` only; android targets not installed.
- Crate name must be `mydownloader-android-bridge` (per context.json key_patterns).
- `#![forbid(unsafe_code)]` workspace lint — android-bridge needs `#[allow(unsafe_code)]` on the specific UniFFI scaffolding call site only (not crate-wide), with a SAFETY comment.

---

## Step A — Create `crates/android-bridge` Rust crate

**Files to create:**
- `crates/android-bridge/Cargo.toml`
- `crates/android-bridge/src/lib.rs`
- `crates/android-bridge/src/dto.rs`
- `crates/android-bridge/src/error.rs`
- `crates/android-bridge/src/service.rs`
- `crates/android-bridge/uniffi/android_bridge.udl`
- `crates/android-bridge/uniffi.toml`
- `crates/android-bridge/build.rs`

**Files to modify:**
- `Cargo.toml` — add `"crates/android-bridge"` to `[workspace] members`

### A.1 — `Cargo.toml` workspace membership

Add `"crates/android-bridge"` to the `members` list in the root `Cargo.toml`. All workspace deps inherited via `workspace = true`. No second entry needed anywhere else.

### A.2 — `crates/android-bridge/Cargo.toml`

```toml
[package]
name = "mydownloader-android-bridge"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[lib]
crate-type = ["cdylib", "lib"]
name = "mydownloader_android_bridge"

[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi_bindgen.rs"

[dependencies]
mydownloader-core-services = { path = "../core-services" }
mydownloader-models        = { path = "../models" }
mydownloader-queue         = { path = "../queue" }
mydownloader-scheduler     = { path = "../scheduler" }
uniffi                     = { version = "0.29", features = ["tokio"] }
tokio                      = { workspace = true }
thiserror                  = { workspace = true }
tracing                    = { workspace = true }

[build-dependencies]
uniffi = { version = "0.29", features = ["build"] }

[lints]
workspace = true
```

Decision: `crate-type = ["cdylib", "lib"]` so the crate produces a `.so` for Android AND a `lib` that host `cargo test` can link against without requiring the Android linker.

Decision: UniFFI 0.29 is the version specified in the task prompt. The `tokio` feature on the `uniffi` dep enables the async callback machinery.

### A.3 — `crates/android-bridge/build.rs`

```rust
fn main() {
    uniffi::generate_scaffolding("uniffi/android_bridge.udl").unwrap();
}
```

This tells UniFFI to emit the Rust scaffolding from the UDL file at build time.

### A.4 — `crates/android-bridge/uniffi/android_bridge.udl`

Define the complete public API surface. Key design decisions:

- `CoreConfig` — a record (value type) carrying the three path strings.
- `DownloadInfoDto` — flat record with all fields as primitives; no nested optionals in records (UniFFI limitation pre-0.29 resolved but keep flat for clarity).
- `DownloadStatusDto` — enum mirroring Rust `DownloadStatus`; adds `Preparing` (maps to Android `Active` with 0 progress).
- `DownloadPriorityDto` — enum with `Low`, `Normal`, `High` only (CRITICAL is gone at the DTO layer).
- `DownloadProgressDto` — record with `downloaded_bytes: u64`, `total_bytes: i64` (-1 = unknown), `speed_bytes_per_second: i64` (-1 = unknown), `estimated_remaining_seconds: i64` (-1 = unknown). Use `i64` sentinels because UniFFI records cannot contain `Option<u64>` cleanly across the ABI without boxing.
- `DownloadEventDto` — enum with `Created`, `Updated`, `Removed` variants each carrying their payload as flat fields.
- `MobileApiError` — error enum used as the return type for all fallible UDL methods.
- `DownloadEventListener` — callback interface (Kotlin-implemented); `on_event(event: DownloadEventDto)`.
- `DownloadService` — the central object; constructor is `new(config: CoreConfig)` returning `Result<DownloadService, MobileApiError>`; all mutating methods are `[Throws=MobileApiError]`.

Full UDL:

```
namespace android_bridge {};

[Error]
enum MobileApiError {
    "Validation",
    "NotFound",
    "UnavailableEngine",
    "InvalidStateTransition",
    "Storage",
    "Runtime",
};

record CoreConfig {
    string database_path;
    string downloads_dir;
    string runtime_dir;
};

enum DownloadStatusDto {
    "Queued",
    "Preparing",
    "Downloading",
    "Paused",
    "Completed",
    "Failed",
    "Cancelled",
};

enum DownloadPriorityDto {
    "Low",
    "Normal",
    "High",
};

record DownloadProgressDto {
    u64 downloaded_bytes;
    i64 total_bytes;
    i64 speed_bytes_per_second;
    i64 estimated_remaining_seconds;
};

record DownloadInfoDto {
    string id;
    string url;
    string engine;
    string filename;
    string destination;
    DownloadStatusDto status;
    DownloadPriorityDto priority;
    DownloadProgressDto progress;
    i64 created_at_millis;
    i64 updated_at_millis;
    string? error_message;
};

[Enum]
interface DownloadEventDto {
    Created(DownloadInfoDto item);
    Updated(DownloadInfoDto item);
    Removed(string id);
};

callback interface DownloadEventListener {
    void on_event(DownloadEventDto event);
};

interface DownloadService {
    [Throws=MobileApiError]
    constructor(CoreConfig config);

    [Throws=MobileApiError]
    sequence<DownloadInfoDto> list_downloads();

    [Throws=MobileApiError]
    DownloadInfoDto? get_download(string id);

    [Throws=MobileApiError]
    string add_download(
        string url,
        string filename,
        string destination,
        DownloadPriorityDto priority
    );

    [Throws=MobileApiError]
    void pause_download(string id);

    [Throws=MobileApiError]
    void resume_download(string id);

    [Throws=MobileApiError]
    void cancel_download(string id);

    [Throws=MobileApiError]
    void remove_download(string id);

    [Throws=MobileApiError]
    string retry_download(string id);

    [Throws=MobileApiError]
    void set_priority(string id, DownloadPriorityDto priority);

    [Throws=MobileApiError]
    void schedule_download(string id, i64 start_at_millis);

    void add_event_listener(DownloadEventListener listener);

    void shutdown();
};
```

Decision: `add_download` takes explicit `destination` string. The Android side constructs the full path from `Context.getExternalFilesDir(null)` + filename. This keeps path logic in Kotlin where `Context` is available.

Decision: `add_event_listener` is not fallible and not `[Throws=…]` — registering a listener cannot fail. Returns void; the event delivery Tokio task is spawned inside the same method.

Decision: `retry_download` returns the new download ID as a `String`, same as `add_download`.

### A.5 — `crates/android-bridge/uniffi.toml`

```toml
[bindings.kotlin]
package_name = "dev.mydownloader.bridge.uniffi"
cdylib_name  = "mydownloader_android_bridge"
```

### A.6 — `crates/android-bridge/src/error.rs`

Define `MobileApiError` matching the UDL enum, plus conversion impls from `QueueError`, `StorageError`, `CoreServicesError`, `ScheduleError`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum MobileApiError {
    #[error("validation error: {0}")]       Validation(String),
    #[error("not found: {0}")]              NotFound(String),
    #[error("engine unavailable: {0}")]     UnavailableEngine(String),
    #[error("invalid state transition: {0}")] InvalidStateTransition(String),
    #[error("storage error: {0}")]          Storage(String),
    #[error("runtime error: {0}")]          Runtime(String),
}
```

Mapping rules (implemented as `From` or match arms):
- `QueueError::DownloadNotFound(_)` → `NotFound`
- `QueueError::InvalidStatus { .. }` → `InvalidStateTransition`
- `QueueError::InvalidConfiguration` → `Validation`
- `QueueError::Stopped` → `Runtime`
- `QueueError::Storage(_)` → `Storage`
- `QueueError::UnsupportedEngineOperation { .. }` → `UnavailableEngine`
- All other `QueueError` variants → `Runtime`
- `ScheduleError::DownloadNotFound(_)` → `NotFound`
- `ScheduleError::InvalidState { .. }` → `InvalidStateTransition`
- `ScheduleError::Stopped` → `Runtime`
- `ScheduleError::Storage(_)` → `Storage`
- `ScheduleError::Queue(e)` → delegate to QueueError mapping
- Other `ScheduleError` variants → `Runtime`
- `CoreServicesError::*` → `Runtime`

### A.7 — `crates/android-bridge/src/dto.rs`

Implement bidirectional conversion functions (not trait impls, to avoid orphan issues):
- `item_to_dto(item: &DownloadItem) -> DownloadInfoDto`
- `status_to_dto(s: DownloadStatus) -> DownloadStatusDto`
- `priority_to_dto(p: DownloadPriority) -> DownloadPriorityDto`
- `priority_from_dto(p: DownloadPriorityDto) -> DownloadPriority` — maps `High` and `Critical` (if ever added) to `DownloadPriority::High`; `Low` → `Low`; `Normal` → `Normal`
- `progress_to_dto(p: &DownloadProgress) -> DownloadProgressDto` — `None` fields become `-1i64`; cast `u64` to `i64` saturating at `i64::MAX`
- `event_to_dto(e: DownloadEvent) -> DownloadEventDto`

Pitfall: `DownloadStatus::Preparing` maps to `DownloadStatusDto::Preparing`. Android's `DownloadState.Active` is mapped from both `Preparing` and `Downloading` by the Kotlin side (`BridgeDownloadRepository`) based on the status value, not the Rust crate.

### A.8 — `crates/android-bridge/src/service.rs`

`DownloadService` struct with:
```rust
pub struct DownloadService {
    runtime: tokio::runtime::Runtime,
    core: CoreServices,
    listeners: std::sync::Mutex<Vec<Box<dyn DownloadEventListener>>>,
}
```

`DownloadService::new(config: CoreConfig) -> Result<Self, MobileApiError>`:
1. Build a `tokio::runtime::Builder::new_multi_thread().enable_all().build()` runtime — this is the ONLY Tokio runtime; all subsequent `block_on` calls use it.
2. `runtime.block_on(CoreServices::start(CoreServicesConfig { … }))` — converts path strings to `PathBuf`.
3. Returns `Self { runtime, core, listeners: Mutex::new(vec![]) }`.

Decision: `Runtime` stored by value (not `Arc`). `shutdown()` consumes it via `runtime.block_on` + then drops the runtime. Since `DownloadService` is wrapped in `Arc` by UniFFI, shutdown takes `&self`; use `OnceLock<()>` or a `AtomicBool` shutdown flag + replace `CoreServices` in an `Option<Mutex<…>>` pattern.

Revised struct:

```rust
pub struct DownloadService {
    inner: std::sync::Mutex<Option<ServiceInner>>,
}

struct ServiceInner {
    runtime: tokio::runtime::Runtime,
    core: CoreServices,
}
```

`shutdown(&self)` locks the mutex, takes the `Option` (leaving `None`), calls `runtime.block_on(core.shutdown())`, then drops `runtime` (which joins all Tokio tasks). This is clean and safe without unsafe code.

All other methods lock `inner`, get a reference to `ServiceInner`, and call `runtime.block_on(…)`.

`add_event_listener(&self, listener: Box<dyn DownloadEventListener>)`:
- Lock the mutex, get `&ServiceInner`.
- Call `core.event_bus.subscribe()` to get a `broadcast::Receiver<DownloadEvent>`.
- Spawn a `std::thread` (NOT a Tokio task — we can't `spawn` from outside the runtime without `Handle`) that calls `runtime.block_on(async { loop { recv() → on_event() } })`.
  
  Actually: use `runtime.spawn(...)` to get a Tokio `JoinHandle` and store it. Spawning on an existing runtime handle is done via `runtime.handle().spawn(…)`. This is the correct approach — no raw threads needed, no unsafe.

  ```rust
  let mut rx = self.core.event_bus.subscribe();
  let listener_arc: Arc<dyn DownloadEventListener> = Arc::from(listener);
  self.runtime.handle().spawn(async move {
      loop {
          match rx.recv().await {
              Ok(event) => listener_arc.on_event(event_to_dto(event)),
              Err(broadcast::error::RecvError::Lagged(_)) => continue,
              Err(broadcast::error::RecvError::Closed) => break,
          }
      }
  });
  ```

  Pitfall: `Box<dyn DownloadEventListener>` — UniFFI's callback interfaces are `Send + Sync`, so wrapping in `Arc` is valid.

`retry_download(&self, id: String)`:
1. Parse `DownloadId`.
2. `runtime.block_on(core.repository.get(id))` — get the item.
3. Validate `status` is `Failed` or `Cancelled`.
4. Check `core.engine_runtime.is_available(engine)`.
5. `runtime.block_on(core.queue().remove_download(old_id))`.
6. Construct new `DownloadItem` with `DownloadId::new()`, same source/filename/destination, `Timestamp::now()`.
7. `runtime.block_on(core.queue().enqueue(new_item))`.
8. Return `new_id.to_string()`.

`schedule_download(&self, id: String, start_at_millis: i64)`:
1. Parse `DownloadId`.
2. `Timestamp::from_unix_millis(start_at_millis)`.
3. `runtime.block_on(core.scheduler().schedule_download(id, ts))`.

### A.9 — `crates/android-bridge/src/lib.rs`

```rust
// UniFFI scaffolding requires one call site with allow(unsafe_code).
// SAFETY: This is the UniFFI-generated scaffolding entrypoint.
// All unsafe operations are encapsulated by the uniffi macro expansion.
#[allow(unsafe_code)]
uniffi::include_scaffolding!("android_bridge");

mod dto;
mod error;
mod service;

pub use error::MobileApiError;
pub use service::DownloadService;
```

This is the ONLY place `unsafe_code` is allowed; the rest of the crate is safe.

### A.10 — `crates/android-bridge/src/bin/uniffi_bindgen.rs`

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

Used to generate Kotlin bindings: `cargo run -p mydownloader-android-bridge --bin uniffi-bindgen generate --library target/<triple>/debug/libmydownloader_android_bridge.so --language kotlin --out-dir apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/`

For initial generation without a compiled `.so`, use the UDL source approach:
`cargo run -p mydownloader-android-bridge --bin uniffi-bindgen generate uniffi/android_bridge.udl --language kotlin --out-dir apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/`

Decision: Generate from UDL source initially and check the generated Kotlin in. This avoids requiring NDK cross-compilation on the development machine for the first pass.

### A.11 — Rust unit tests in `crates/android-bridge/src/service.rs` (cfg test block)

Tests use `tempfile::tempdir()` for path isolation. Add `tempfile = "3"` to `[dev-dependencies]`.

Test list:
1. `init_and_shutdown` — creates service, shuts down cleanly
2. `double_shutdown_is_safe` — shutdown twice does not panic
3. `add_download_returns_uuid` — add one item, verify UUID string parses
4. `list_downloads_empty_initially` — new service has empty list
5. `get_download_not_found` — get unknown ID returns Ok(None)
6. `add_and_list` — add two items, list returns both
7. `invalid_url_returns_validation_error` — empty URL maps to Validation
8. `pause_nonexistent_returns_not_found` — pause unknown ID → NotFound
9. `cancel_nonexistent_returns_not_found`
10. `remove_nonexistent_returns_not_found`
11. `priority_change` — add item, set_priority, verify via get
12. `event_listener_receives_created_event` — register listener, add download, assert on_event called
13. `retry_non_failed_returns_invalid_state` — retry Queued item → InvalidStateTransition
14. `concurrent_list_calls` — spawn 4 threads calling list_downloads, all succeed

**Verify A:** `cargo check --workspace` from repo root passes. `cargo test -p mydownloader-android-bridge` passes.

---

## Step B — Create Android `:bridge` module

**Files to create:**
- `apps/android/bridge/build.gradle.kts`
- `apps/android/bridge/src/main/AndroidManifest.xml`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/BridgeInitializer.kt`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/BridgeDownloadRepository.kt`
- `apps/android/bridge/src/main/jniLibs/arm64-v8a/.gitkeep`
- `apps/android/bridge/src/main/jniLibs/armeabi-v7a/.gitkeep`
- `apps/android/bridge/src/main/jniLibs/x86_64/.gitkeep`
- `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/` ← generated Kotlin bindings go here
- `apps/android/bridge/src/test/kotlin/dev/mydownloader/bridge/MappingTest.kt`

**Files to modify:**
- `apps/android/settings.gradle.kts` — add `include(":bridge")`
- `apps/android/gradle/libs.versions.toml` — add `junit` and `kotlin-test` entries if absent

### B.1 — `apps/android/bridge/build.gradle.kts`

```kotlin
plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace  = "dev.mydownloader.bridge"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation(project(":core"))
    implementation(libs.kotlinx.coroutines.android)

    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
}
```

The UniFFI-generated Kotlin source lives in `src/main/kotlin/dev/mydownloader/bridge/uniffi/`. No annotation processing or code-gen plugin needed — the file is checked in.

The `.so` files land in `src/main/jniLibs/<abi>/`. They are NOT checked in; the jniLibs directories contain only `.gitkeep` files. The Gradle build succeeds without them at compile time because the JVM unit tests do not load the native library.

### B.2 — `apps/android/bridge/src/main/AndroidManifest.xml`

Minimal:
```xml
<?xml version="1.0" encoding="utf-8"?>
<manifest package="dev.mydownloader.bridge" />
```

### B.3 — Generated UniFFI Kotlin bindings

Place the generated file at `apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/android_bridge.kt`.

This is generated by running (after Step A compiles):
```
cargo run -p mydownloader-android-bridge --bin uniffi-bindgen \
  generate uniffi/android_bridge.udl \
  --language kotlin \
  --out-dir apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/
```

The generated file must be committed. It is the source of truth for the Kotlin API surface.

### B.4 — `BridgeInitializer.kt`

```kotlin
package dev.mydownloader.bridge

import android.content.Context
import dev.mydownloader.bridge.uniffi.CoreConfig
import dev.mydownloader.bridge.uniffi.DownloadService
import java.io.File

object BridgeInitializer {

    @Volatile
    private var service: DownloadService? = null

    fun initialize(context: Context) {
        if (service != null) return
        synchronized(this) {
            if (service != null) return
            System.loadLibrary("mydownloader_android_bridge")
            val dataDir  = context.filesDir
            val dlDir    = context.getExternalFilesDir(null)
                ?: File(dataDir, "downloads")
            val runtimeDir = File(dataDir, "runtime")
            val config = CoreConfig(
                databasePath = File(dataDir, "mydownloader.db").absolutePath,
                downloadsDir  = dlDir.absolutePath,
                runtimeDir    = runtimeDir.absolutePath,
            )
            service = DownloadService(config)
        }
    }

    fun getService(): DownloadService =
        service ?: error("BridgeInitializer.initialize() must be called first")

    fun shutdown() {
        synchronized(this) {
            service?.shutdown()
            service = null
        }
    }
}
```

Decision: `System.loadLibrary` call is inside `initialize()`. On JVM unit test runs the test fakes bypass this object, so the library is never loaded and tests pass without the `.so`.

Pitfall: `getExternalFilesDir` can return `null` on some devices (no external storage); fall back to internal storage.

### B.5 — `BridgeDownloadRepository.kt`

Implements `dev.mydownloader.core.download.DownloadRepository`.

```kotlin
package dev.mydownloader.bridge

import dev.mydownloader.bridge.uniffi.*
import dev.mydownloader.core.download.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*

class BridgeDownloadRepository(
    private val service: DownloadService,
    externalScope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO),
) : DownloadRepository {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    private val _events    = MutableSharedFlow<DownloadEvent>(extraBufferCapacity = 64)

    init {
        // Register event listener with the Rust bridge
        service.addEventListener(object : DownloadEventListener {
            override fun onEvent(event: DownloadEventDto) {
                val mapped = mapEvent(event) ?: return
                _events.tryEmit(mapped)
                // Refresh the state snapshot on every event
                externalScope.launch {
                    val items = withContext(Dispatchers.IO) {
                        service.listDownloads().map(::mapItem)
                    }
                    _downloads.value = items
                }
            }
        })
        // Load initial state
        externalScope.launch {
            _downloads.value = withContext(Dispatchers.IO) {
                service.listDownloads().map(::mapItem)
            }
        }
    }

    override fun observeDownloads(): Flow<List<DownloadItem>> = _downloads.asStateFlow()
    override fun observeEvents(): Flow<DownloadEvent> = _events.asSharedFlow()

    override suspend fun addDownload(
        url: String, fileName: String, destinationPath: String,
        priority: DownloadPriority, mimeType: String?,
        totalBytes: Long?, referrer: String?
    ): String = withContext(Dispatchers.IO) {
        service.addDownload(url, fileName, destinationPath, mapPriorityToDto(priority))
    }

    override suspend fun pauseDownload(downloadId: String) =
        withContext(Dispatchers.IO) { service.pauseDownload(downloadId) }

    override suspend fun resumeDownload(downloadId: String) =
        withContext(Dispatchers.IO) { service.resumeDownload(downloadId) }

    override suspend fun cancelDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) { service.cancelDownload(downloadId) }

    override suspend fun removeDownload(downloadId: String, deleteFile: Boolean) =
        withContext(Dispatchers.IO) { service.removeDownload(downloadId) }

    override suspend fun retryDownload(downloadId: String): String =
        withContext(Dispatchers.IO) { service.retryDownload(downloadId) }

    override suspend fun setPriority(downloadId: String, priority: DownloadPriority) =
        withContext(Dispatchers.IO) { service.setPriority(downloadId, mapPriorityToDto(priority)) }

    override suspend fun scheduleDownload(downloadId: String, startAtMillis: Long) =
        withContext(Dispatchers.IO) { service.scheduleDownload(downloadId, startAtMillis) }

    override suspend fun getDownload(downloadId: String): DownloadItem? =
        withContext(Dispatchers.IO) { service.getDownload(downloadId)?.let(::mapItem) }

    // ── Mapping helpers ──────────────────────────────────────────────────

    private fun mapItem(dto: DownloadInfoDto): DownloadItem = DownloadItem(
        id              = dto.id,
        fileName        = dto.filename,
        url             = dto.url,
        destinationPath = dto.destination,
        state           = mapState(dto),
        priority        = mapPriorityFromDto(dto.priority),
        engine          = dto.engine,
        mimeType        = null,
        totalBytes      = dto.progress.totalBytes.takeIf { it >= 0 },
        createdAt       = dto.createdAtMillis
    )

    private fun mapState(dto: DownloadInfoDto): DownloadState = when (dto.status) {
        DownloadStatusDto.QUEUED    -> DownloadState.Queued
        DownloadStatusDto.PREPARING,
        DownloadStatusDto.DOWNLOADING -> DownloadState.Active(
            downloadedBytes = dto.progress.downloadedBytes.toLong(),
            totalBytes      = dto.progress.totalBytes.takeIf { it >= 0 },
            speedBytesPerSec = dto.progress.speedBytesPerSecond.takeIf { it >= 0 } ?: 0L,
            etaSeconds       = dto.progress.estimatedRemainingSeconds.takeIf { it >= 0 },
            segmentCount     = 1
        )
        DownloadStatusDto.PAUSED    -> DownloadState.Paused
        DownloadStatusDto.COMPLETED -> DownloadState.Completed(
            totalBytes      = dto.progress.totalBytes.takeIf { it >= 0 } ?: 0L,
            destinationPath = dto.destination
        )
        DownloadStatusDto.FAILED    -> DownloadState.Failed(
            reason    = dto.errorMessage ?: "unknown error",
            retryable = true
        )
        DownloadStatusDto.CANCELLED -> DownloadState.Cancelled
    }

    private fun mapPriorityFromDto(dto: DownloadPriorityDto): DownloadPriority = when (dto) {
        DownloadPriorityDto.LOW    -> DownloadPriority.LOW
        DownloadPriorityDto.NORMAL -> DownloadPriority.NORMAL
        DownloadPriorityDto.HIGH   -> DownloadPriority.HIGH
    }

    private fun mapPriorityToDto(priority: DownloadPriority): DownloadPriorityDto = when (priority) {
        DownloadPriority.LOW      -> DownloadPriorityDto.LOW
        DownloadPriority.NORMAL   -> DownloadPriorityDto.NORMAL
        DownloadPriority.HIGH     -> DownloadPriorityDto.HIGH
        DownloadPriority.CRITICAL -> DownloadPriorityDto.HIGH  // CRITICAL → High, per constraint 6
    }

    private fun mapEvent(dto: DownloadEventDto): DownloadEvent? = when (dto) {
        is DownloadEventDtoCreated -> {
            val item = mapItem(dto.item)
            when (item.state) {
                is DownloadState.Active  -> DownloadEvent.Started(item.id)
                else                     -> null  // Created → no distinct Started event needed; state already visible
            }
        }
        is DownloadEventDtoUpdated -> {
            val item = mapItem(dto.item)
            when (val s = item.state) {
                is DownloadState.Active    -> DownloadEvent.Progress(
                    item.id,
                    s.downloadedBytes,
                    s.totalBytes,
                    s.speedBytesPerSec,
                    s.etaSeconds
                )
                is DownloadState.Paused   -> DownloadEvent.Paused(item.id)
                is DownloadState.Completed -> DownloadEvent.Completed(item.id, item.destinationPath, s.totalBytes)
                is DownloadState.Failed    -> DownloadEvent.Failed(item.id, s.reason, s.retryable)
                is DownloadState.Cancelled -> DownloadEvent.Cancelled(item.id)
                else                       -> null
            }
        }
        is DownloadEventDtoRemoved -> null  // Removed has no matching DownloadEvent variant
        else -> null
    }
}
```

Decision: `observeDownloads()` is implemented as a `StateFlow`; every event triggers a fresh `listDownloads()` call on `Dispatchers.IO`. This is simple and correct — no in-memory delta tracking that could drift from Rust state.

Pitfall: `DownloadProgress.downloadedBytes` is `u64` in Rust but comes through UniFFI as `ULong` in Kotlin. Cast to `Long` with `toLong()` — overflow is not realistic for file sizes in practice.

### B.6 — `apps/android/bridge/src/test/kotlin/dev/mydownloader/bridge/MappingTest.kt`

Unit tests using fakes (no native library loaded):

```kotlin
class MappingTest {
    @Test fun criticalPriorityMapsToHigh() { … }
    @Test fun preparingStatusMapsToActiveState() { … }
    @Test fun downloadingStatusMapsToActiveState() { … }
    @Test fun failedStatusMapsToFailedStateWithRetryableTrue() { … }
    @Test fun unknownTotalBytesNegativeOneMapToNull() { … }
    @Test fun knownTotalBytesPositiveMapsThrough() { … }
}
```

Tests instantiate the mapping functions directly or through a fake `DownloadInfoDto`.

**Verify B:** `./gradlew :bridge:test` from `apps/android` passes all unit tests. `./gradlew assembleDebug` compiles without errors.

---

## Step C — Wire `:bridge` into `:app` and `:downloads`

**Files to modify:**
- `apps/android/app/build.gradle.kts` — add `implementation(project(":bridge"))`
- `apps/android/downloads/build.gradle.kts` — add `implementation(project(":bridge"))`
- `apps/android/app/src/main/kotlin/dev/mydownloader/app/MyDownloaderApp.kt`
- `apps/android/downloads/src/main/kotlin/dev/mydownloader/downloads/ui/DownloadsViewModel.kt`

### C.1 — `MyDownloaderApp.kt`

```kotlin
class MyDownloaderApp : Application() {
    override fun onCreate() {
        super.onCreate()
        BridgeInitializer.initialize(this)
    }
}
```

Add `ProcessLifecycleOwner` observer or override `onTerminate()` for shutdown. Decision: use `ProcessLifecycleOwner` (available via `lifecycle-process` artifact). Add `lifecycle-process` to the dependency if not present. In `onCreate()` register:

```kotlin
ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
    override fun onStop(owner: LifecycleOwner) {
        // Called when app moves to background — do NOT shutdown here; user may return
    }
})
```

For shutdown on process death: override `onTerminate()` as a best-effort (not guaranteed by Android), plus register a JVM shutdown hook:

```kotlin
Runtime.getRuntime().addShutdownHook(Thread { BridgeInitializer.shutdown() })
```

Decision: JVM shutdown hook is the most reliable way to invoke shutdown on process exit on Android.

### C.2 — Add `libs.versions.toml` entries

Add if not already present:
```toml
[versions]
junit = "4.13.2"
kotlin-test = "2.0.21"   # matches kotlin version

[libraries]
junit = { group = "junit", name = "junit", version.ref = "junit" }
kotlin-test = { group = "org.jetbrains.kotlin", name = "kotlin-test", version.ref = "kotlin-test" }
lifecycle-process = { group = "androidx.lifecycle", name = "lifecycle-process", version.ref = "lifecycle" }
```

Note: `kotlin-test` version matches the `kotlin` version already in the file (2.0.21).

### C.3 — `DownloadsViewModel.kt`

Replace the stub with a real implementation:

```kotlin
class DownloadsViewModel(
    private val repository: DownloadRepository
) : ViewModel() {

    private val _downloads = MutableStateFlow<List<DownloadItem>>(emptyList())
    val downloads: StateFlow<List<DownloadItem>> = _downloads.asStateFlow()

    val activeDownloads: StateFlow<List<DownloadItem>> = downloads.map { list ->
        list.filter { it.state is DownloadState.Active }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    val queuedDownloads: StateFlow<List<DownloadItem>> = downloads.map { list ->
        list.filter { it.state is DownloadState.Queued }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    val completedDownloads: StateFlow<List<DownloadItem>> = downloads.map { list ->
        list.filter { it.state is DownloadState.Completed }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    val failedDownloads: StateFlow<List<DownloadItem>> = downloads.map { list ->
        list.filter { it.state is DownloadState.Failed }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    init {
        viewModelScope.launch {
            repository.observeDownloads().collect { _downloads.value = it }
        }
    }

    fun pause(id: String)  = viewModelScope.launch { repository.pauseDownload(id) }
    fun resume(id: String) = viewModelScope.launch { repository.resumeDownload(id) }
    fun cancel(id: String) = viewModelScope.launch { repository.cancelDownload(id) }
    fun remove(id: String) = viewModelScope.launch { repository.removeDownload(id) }
    fun retry(id: String)  = viewModelScope.launch { repository.retryDownload(id) }

    companion object {
        fun factory(repository: DownloadRepository): ViewModelProvider.Factory =
            object : ViewModelProvider.Factory {
                @Suppress("UNCHECKED_CAST")
                override fun <T : ViewModel> create(modelClass: Class<T>): T =
                    DownloadsViewModel(repository) as T
            }
    }
}
```

Decision: `factory` companion provides a `ViewModelProvider.Factory` so `MainActivity` can wire it up without Hilt. Factory is minimal and matches the project pattern (no DI framework).

The `:downloads` module depends on `:core` (for `DownloadRepository` interface) — this dependency already exists. The ViewModel itself does NOT import from `:bridge`; the concrete `BridgeDownloadRepository` is wired in `:app`.

### C.4 — `MainActivity` or `AppNavigation` factory usage

Wire the factory in `MainActivity` or wherever `DownloadsViewModel` is created:

```kotlin
val repository = BridgeDownloadRepository(BridgeInitializer.getService())
val viewModel: DownloadsViewModel by viewModels { DownloadsViewModel.factory(repository) }
```

This goes in the `@Composable` host or `Activity`; file location depends on existing nav structure. Check `AppNavigation.kt` (if it exists) — if not, wire in `MainActivity.kt` directly.

**Verify C:** `./gradlew assembleDebug` from `apps/android` compiles cleanly. No unresolved references.

---

## Step D — UniFFI binding generation

This step is partially covered in A.10. The explicit command to run after `cargo build` for the host to produce Kotlin bindings:

```
cd d:\Projects\MyDownloader
cargo run -p mydownloader-android-bridge --bin uniffi-bindgen generate \
  crates/android-bridge/uniffi/android_bridge.udl \
  --language kotlin \
  --out-dir apps/android/bridge/src/main/kotlin/dev/mydownloader/bridge/uniffi/
```

The `--language kotlin` flag with UDL input does NOT require a compiled `.so`; uniffi-bindgen reads the UDL directly.

Decision: generate from UDL source (not from a compiled `.so`) so the host machine never needs the Android NDK for this step.

The generated file must be committed. It will reference the `uniffi.toml` settings for package name and cdylib name.

**Verify D:** The generated `android_bridge.kt` file exists at the expected path. `./gradlew :bridge:compileDebugKotlin` succeeds (Kotlin can resolve all UniFFI-generated types).

---

## Step E — Validation (final checks)

Run in order from the repo root:

1. `cargo fmt --all -- --check`
2. `cargo check --workspace`
3. `cargo test --workspace`
4. `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Then from `apps/android`:

5. `./gradlew assembleDebug`
6. `./gradlew :bridge:test`
7. Verify APK at `apps/android/app/build/outputs/apk/debug/app-debug.apk`

---

## Dependency order

```
A (Rust crate) → D (generate bindings) → B (Android :bridge) → C (wire :app/:downloads) → E (validate)
```

Steps A and D are sequenced A-then-D because D requires the UDL file created in A.
Steps B and C cannot start until D has produced the committed Kotlin bindings.
Step C depends on B because `BridgeDownloadRepository` and `BridgeInitializer` are defined in B.

---

## Key pitfalls

1. **`u64` → Kotlin `ULong`:** UniFFI 0.29 maps Rust `u64` to Kotlin `ULong`. `BridgeDownloadRepository` must call `.toLong()` (or use unsigned arithmetic) where a `Long` is expected. Mark the conversion sites explicitly.

2. **`DownloadEventDto` enum in UDL:** UniFFI `[Enum] interface` (not plain `enum`) is required for enum variants with data. The generated Kotlin uses sealed class + data class subclasses (e.g., `DownloadEventDtoCreated`). The `when` in `BridgeDownloadRepository.mapEvent` must match these generated names.

3. **`cdylib` on host:** `cargo test -p mydownloader-android-bridge` builds the `lib` crate type, not `cdylib`. The test binary links the Rust code directly. No `.so` is produced or needed for host tests.

4. **`tokio::runtime::Runtime` ownership vs `Send`:** `Runtime` is `Send` but not `Sync`. Wrapping in `Mutex<Option<ServiceInner>>` satisfies UniFFI's `Sync` requirement for the outer `DownloadService` struct (UniFFI requires `Send + Sync`).

5. **Android `Preparing` status:** The Android `DownloadState.Active` data class requires `segmentCount`. Hardcode `segmentCount = 1` for now; this field is not tracked by Rust's `DownloadProgress`.

6. **`scheduleDownload` — ScheduleError vs QueueError:** The Scheduler methods return `ScheduleError`, not `QueueError`. The error conversion in `error.rs` must handle `ScheduleError` variants separately.

7. **`add_download` destination path:** The Rust `DownloadItem::new` takes a `PathBuf` destination. The Kotlin caller constructs the full path. The bridge `add_download` method concatenates the downloads dir + filename server-side — but since the Kotlin already provides the full `destination` string, the bridge uses it directly via `PathBuf::from(destination)`.

8. **`mimeType`, `totalBytes`, `referrer` params in `addDownload`:** The `DownloadRepository` interface signature includes these but Rust `DownloadItem` has no fields for them. They are accepted and ignored at the bridge layer. Document this in a code comment.

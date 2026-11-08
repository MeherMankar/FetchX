# Download Navi Feature Analysis

## Reference review

- Repository: `TachibanaGeneralLaboratories/download-navi`
- Reviewed revision: `10c7f9e5b306f66b122782e4878446d0ff2ac9b9` (`master`, 2025-05-24)
- License: GNU GPL version 3 or later, stated in `README.md`; `LICENSE.md` contains the GNU GPL version 3 text.
- Documentation reviewed: `README.md`, including the background-download, multipart-download, connection-policy, and power-management feature descriptions.
- No Download Navi source was copied. The selected change is an independent Android service-lifecycle adaptation; FetchX remains separate from the GPL-licensed implementation.

## Relevant upstream behavior

- `app/src/main/AndroidManifest.xml` declares internet/network state, foreground service, boot completion, wake-lock, and notification permissions, and registers the service and receivers.
- `app/src/main/java/com/tachibana/downloader/service/DownloadService.java` runs long-lived work in a foreground service with an ongoing notification. A null/system-restart intent triggers `DownloadEngine.restoreDownloads()`; explicit shutdown stops the service without requesting a restart.
- `app/src/main/java/com/tachibana/downloader/core/model/DownloadEngine.java` owns download-engine lifecycle and restoration of persisted download records.
- `app/src/main/java/com/tachibana/downloader/core/model/data/entity/DownloadInfo.java` persists transfer state including partial-download support, piece count, retry count, and retry delay.
- `app/src/main/java/com/tachibana/downloader/receiver/BootReceiver.java` restores downloads after boot only when the user's autostart setting is enabled.
- `app/src/main/java/com/tachibana/downloader/receiver/ConnectionReceiver.java` asks the engine to reschedule work after connectivity changes.
- `app/src/main/java/com/tachibana/downloader/MainApplication.java` initializes notification channels and starts notification updates.
- `app/src/test/java/com/tachibana/downloader/core/model/data/entity/DownloadInfoTest.java` checks multipart piece constraints and range geometry. This test does not exercise Android service process-restart behavior.
- `app/build.gradle` includes WorkManager. `DownloadService.java` documents that the foreground service is reserved for work longer than ten minutes that cannot use WorkManager.

## FetchX current-state audit

FetchX already has a Rust-owned recovery path. `QueueManager` calls `DownloadStateMachine::recover_interrupted` on startup, which returns interrupted `Preparing`, `Downloading`, metadata-acquiring, recovering, and seeding records to the queued state while preserving partial files. The Rust queue tests cover requeue after restart and the configured active-download limit.

On Android, `FetchXApp` initializes the UniFFI `DownloadService` and Rust `CoreServices`, then observes the shared repository and starts `DownloadForegroundService` for active or queued work. The foreground service owns only Android lifecycle, notification, and network/power-policy work. It previously returned `START_NOT_STICKY` even when its repository was ready, opting out of Android's service restart after process death. A restarted service can receive a null intent; FetchX ignores the intent payload and reconnects to the repository initialized by `FetchXApp`.

The desktop, CLI, and remote hosts use the same Rust queue and persisted recovery behavior, but do not use Android foreground-service lifecycle policy.

## Capability comparison

| Capability | Download Navi | FetchX before this phase | Assessment |
|---|---|---|---|
| Continue active transfers with UI closed | Foreground service and ongoing notification | Android foreground service observes active work | Already implemented adequately while the process remains alive |
| Recover after Android process death | Service accepts a system restart and restores persisted downloads | Rust queue recovers persisted interrupted items, but Android service returned `START_NOT_STICKY` | Missing at the Android service boundary; selected |
| Recovery decisions and partial-file retention | Persisted engine state and engine restoration | Rust state machine and downloader own state recovery and partial files | Already implemented adequately in shared core |
| Boot-time autostart | Boot receiver, gated by a user preference | No Android boot receiver/autostart setting | Missing; deferred because it is a separate policy and OS-start restriction |
| Network changes and transfer constraints | Connectivity receiver reschedules work; unmetered and power settings are advertised | Foreground service reacts to network/power changes and applies Android policy; Rust owns pause/resume and retries | Partially implemented; not selected |
| Scheduled and constrained work | WorkManager is a dependency; the foreground service states it is for work over ten minutes that cannot use WorkManager; network/power settings also apply | Rust scheduler supports persistent per-download scheduling; Android applies live network/power constraints in the service | Partially implemented |
| Multi-part HTTP and retries | README advertises multipart transfers; `DownloadInfo` stores retry settings | Rust HTTP engine provides segmented transfer, recovery, and bounded retry | Already implemented adequately |
| Notifications | Download and ongoing foreground notifications | Android service and notification manager publish per-download and summary notifications | Already implemented adequately |
| Android-specific engine/business logic | Java engine is Android-local | Rust is canonical; Kotlin is limited to OS integration | Not appropriate to duplicate in FetchX |

## Selected capability: restart the Android foreground service after process recreation

When the Android service has an initialized bridge, return `Service.START_STICKY`, including for duplicate starts while observation is already running. A system recreation may call `onStartCommand` with a null intent; the service reuses the already-initialized repository and observes the snapshot. Meanwhile Rust startup recovery turns interrupted transfers back into eligible queued work and resumes them through the existing queue/engine path. If the bridge is unavailable, the service stops and remains non-sticky to avoid repeated failed restarts.

Acceptance criteria:

1. A ready Android bridge yields sticky service restart mode.
2. An unavailable bridge yields non-sticky mode and the service stops.
3. Null-intent recreation follows the same repository observation and notification path as a normal start.
4. Rust remains the sole owner of interrupted-download recovery and transfer decisions.
5. Existing queue restart and queue-order behavior remain unchanged.

## Implementation and target impact

- Android adapter: `apps/android/app/src/main/kotlin/dev/mydownloader/app/service/DownloadForegroundService.kt` selects sticky restart only while the bridge is available.
- Android unit coverage: `apps/android/app/src/test/kotlin/dev/fetchx/app/service/DownloadForegroundServiceTest.kt` tests sticky and non-sticky policy selection.
- Rust recovery is reused without code changes: `crates/core/src/state_machine.rs` and `crates/queue/src/manager.rs`.
- No UniFFI or public core API change was needed.

| Target | Status |
|---|---|
| Android | Uses the sticky foreground-service restart policy and existing Rust startup recovery. An Android emulator/device process-kill test was not run. |
| Windows/Linux/macOS desktop | Unchanged; uses the existing Rust queue recovery, not Android service behavior. |
| CLI | Unchanged; uses the existing Rust queue recovery. |
| Remote server and remote UI | Unchanged; the server's Rust CoreServices queue retains its existing recovery behavior. |
| Browser/native host | Unchanged; no Android lifecycle behavior is applicable. |

## Persistence, migration, and dependencies

No schema, migration, new dependency, or public model was added. Existing SQLite records and partial files are handled by the Rust recovery path. The Android service restart mode is not separately persisted; Android restarts a sticky service while the OS considers the service eligible.

## Validation

| Command | Result |
|---|---|
| `cargo test -p fetchx-queue --lib` | Passed: 42 tests, including interrupted-work recovery, manual queue reordering, and queue-order persistence. |
| `cargo test -p fetchx -p fetchx-queue -p fetchx-remote-protocol -p fetchx-remote-server --no-fail-fast` | Passed: CLI 25, queue 42, remote protocol 9, remote server 20, and server integration 3 tests; 0 failed. |
| `.\gradlew.bat :app:testDebugUnitTest --tests 'dev.fetchx.app.service.DownloadForegroundServiceTest'` from `apps/android` | Passed: focused Android restart-policy unit tests; Android Kotlin app and dependencies compiled. |
| `npm --prefix apps/desktop/ui run build` | Passed: TypeScript check and Vite production build; queue-position UI contract remains type-correct. |
| `cargo check -p fetchx-desktop` | Passed: desktop Tauri adapter compiled. |
| `npm --prefix apps/remote-ui run build` and `npm --prefix apps/remote-ui test` | Passed: TypeScript/Vite build and all 5 remote UI tests. |

These are Rust unit tests and an Android host-side unit/build check. No emulator/device process-kill, force-stop, reboot, or live transfer validation was performed. Android sticky restart is not guaranteed after user force-stop, and this phase does not add boot-time autostart.

## Limitations and deferred work

- Android service recreation depends on Android's sticky-service policy and remains subject to OS resource management and vendor battery restrictions.
- User force-stop prevents automatic service restart until the app is launched again.
- Reboot autostart is not added; it needs an explicit user preference, receiver/foreground-service policy review, and Android-version-specific validation.
- This change does not add WorkManager, a second transfer engine, or Android-specific download recovery logic.
- Android process recreation still requires emulator/device validation before claiming runtime reliability.

## Next research target

Per the established research order, the next repository is Aria2App, focused on remote-engine integration.

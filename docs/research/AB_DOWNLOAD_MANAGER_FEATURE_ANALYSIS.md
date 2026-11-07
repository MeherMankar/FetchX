# AB Download Manager Feature Analysis

## Reference review

- Repository: [amir1376/ab-download-manager](https://github.com/amir1376/ab-download-manager)
- Reviewed revision: `68f0ea6e8ba5cbeb02c92342cd3bf7475378ae68` (`master`)
- License: Apache License 2.0, confirmed from `LICENSE` at the reviewed revision.
- Documentation reviewed: repository `README.md` and the queue-related source/tests listed below.
- No AB Download Manager source was copied. The implementation is an independent Rust design based on observed queue behavior.

## Relevant upstream behavior

- `downloader/core/src/commonMain/kotlin/ir/amirab/downloader/queue/ManualDownloadQueue.kt` tracks active and pending IDs, enforces a global concurrent-download cap, promotes eligible pending jobs, stops excess jobs when the cap shrinks, and supports moving selected queue items up/down.
- `downloader/core/src/commonMain/kotlin/ir/amirab/downloader/queue/DownloadQueue.kt` owns a named queue model, maximum concurrency, stop-when-empty behavior, scheduled start/stop times, and the same move-up/down ordering operations.
- `downloader/core/src/commonMain/kotlin/ir/amirab/downloader/queue/QueueManager.kt` manages queue instances and their lifecycle.
- `downloader/core/src/commonMain/kotlin/ir/amirab/downloader/queue/ScheduleTimes.kt` calculates the next configured start/stop time.
- `shared/app/src/commonMain/kotlin/com/abdownloadmanager/shared/repository/BaseAppRepository.kt` forwards the configured maximum concurrent count to the manual queue.
- `android/app/src/main/kotlin/com/abdownloadmanager/android/pages/home/HomeComponent.kt` wires selected downloads to the queue's move-up action; `desktop/app/src/main/kotlin/com/abdownloadmanager/desktop/pages/queue/QueueInfoComponent.kt` wires queue-page move actions and controls.
- A dedicated test for the upstream move-up/down implementation was not found by the repository search; the behavior was verified from the implementation and UI call sites.

## Capability comparison

| Capability | AB Download Manager | FetchX before this phase | Assessment |
|---|---|---|---|
| Global active-download limit | Manual queue starts/stops jobs to honor a configurable cap | Rust queue enforces active-download and global connection limits | Already implemented adequately |
| Priority scheduling | Queue operations coexist with queue-level concurrency settings | Shared Rust queue applies priority, FIFO, and fairness | Already implemented adequately |
| Manual order for pending items | Move selected IDs up/down in a queue | Persisted FIFO and priority existed, but hosts could not manually change FIFO order | Missing; selected |
| Scheduled queue start/stop and stop-when-empty | Per-queue schedule and queue settings | Persistent one-shot per-download schedules; no recurring queue calendar | Partially implemented; recurring queue controls deferred |
| Retry/failure handling | Configurable retry policy and queue lifecycle integration | Bounded transfer-level retries and manual retry paths; no uniform queue-level auto-retry policy | Partially implemented; not selected |
| Categories/packages | Categories support automatic organization and user management | Shared naming/category helpers exist, but organization differs across hosts | Partially implemented |
| Multiple named queues | Separate queue models and queue manager | One shared global queue/scheduler | Requires further investigation; not appropriate for this scoped change |
| Persistence/restart recovery | Queue state is stored by the application database | SQLite queue order and interrupted-download recovery already exist | Already implemented adequately |

## Selected capability: persistent manual queue ordering

Provide explicit move-up and move-down operations for a queued download. Moves are restricted to other queued downloads with the same priority, keeping FetchX's existing priority-first dispatch behavior intact. Boundary moves are no-ops; attempts to move missing or non-queued records return errors.

Acceptance criteria:

1. A queued item can move one position up or down within its priority group.
2. Active, paused, terminal, and missing records cannot be reordered.
3. Priority ordering, queue concurrency, and resource allocation remain unchanged.
4. The new order is persisted and restored after a queue restart.
5. Storage rejects duplicate, incomplete, or unknown full-order updates without changing the prior order.
6. Platform adapters expose the shared Rust operation rather than implementing reorder rules themselves.

## FetchX implementation

- Queue business logic: `crates/queue/src/manager.rs` adds actor-serialized `move_up` and `move_down`.
- Persistence boundary: `crates/storage/src/repository.rs` adds whole-order replacement; `crates/storage/src/sqlite/downloads.rs` validates the supplied IDs and updates existing `queue_order` values in a transaction.
- Event repository: `crates/queue/src/events.rs` delegates the order operation without publishing per-download lifecycle events because no item metadata changes.
- Regression tests: `crates/queue/src/manager.rs` covers movement, queue boundaries, priority-group isolation, invalid state/missing IDs, and restart persistence. `crates/storage/src/tests.rs` covers persisted ordering and rejection without mutation.
- No new public model, migration, or dependency was needed. Existing `queue_order` is reused.

## Integration by target

| Target | Integration status |
|---|---|
| Android | UniFFI exposes move-up/down; `BridgeDownloadRepository`, downloads repository, ViewModel, and queued-download row controls consume it. |
| Windows/Linux/macOS desktop | Tauri exposes move-up/down; TypeScript API/hook and row controls use the operations. A queue-position sort keeps persisted order visible. |
| Local CLI | `fetchx queue-move <id> up|down` delegates directly to `QueueManager`. |
| Remote CLI | `fetchx remote queue-up <id>` and `queue-down` call the authenticated RPC methods. |
| Remote server | `queue.moveUp` and `queue.moveDown` are advertised and require `DownloadsControl` authorization. |
| Remote web UI | Uses capability discovery before displaying queued-row move controls and refreshes the list after a move. |
| Browser extension/native host | Queue mutations remain excluded from its restricted RPC allowlist; it is not a queue-management UI. |

The Android foreground service, OS lifecycle, and desktop Tauri layer remain adapters; dispatch and ordering rules stay in Rust. Compilation and unit-test success do not establish device or desktop runtime validation.

## Validation

| Command | Result |
|---|---|
| `cargo test -p fetchx-storage -p fetchx-queue` | Passed: 35 storage tests and 40 queue tests at the initial implementation checkpoint. |
| `cargo test -p fetchx -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-android-bridge -p fetchx-desktop` | Passed after correcting the authorization command-count assertion; CLI, remote protocol/server, Android bridge, and desktop adapter test targets passed. |
| `npm run build` in `apps/desktop/ui` | Passed: TypeScript check and Vite production build. |
| `npm run build` and `npm test` in `apps/remote-ui` | Passed: TypeScript/Vite build and 5 tests. |
| `.\gradlew.bat :downloads:testDebugUnitTest :app:testDebugUnitTest` in `apps/android` | Passed: Android Kotlin modules compiled, UniFFI bindings regenerated, and both selected unit-test tasks succeeded. |

The commands above are host builds/unit tests. No Android device/emulator execution or Windows/Linux/macOS desktop runtime test was performed.

## Limitations and deferred work

- Reordering is one item at a time and only within its priority group; cross-priority movement would contradict the current priority-first scheduler contract.
- Android currently offers move controls on queued rows; there is no batch/multi-selection reorder or queue editor.
- The browser extension/native-host RPC remains deliberately read-only for queue mutations.
- Recurring queue schedules, named queue groups, stop-when-empty, and a unified retry policy require separate capability assessments.
- Next repository in the established research order: Download Navi, concentrating on Android background reliability and OS integration.

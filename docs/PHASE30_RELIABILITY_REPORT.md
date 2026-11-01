# Phase 30 - Production Reliability, Resilience & Failure Recovery

## Status

**PARTIAL**

Retry diagnostics, deterministic filesystem-fault coverage, and a subprocess
crash-recovery harness are implemented and tested. The downloader library suite
passed (270 passed, 3 ignored), including retry cancellation/pause behavior and
abrupt termination of a child test process followed by SQLite recovery, range
resume, and SHA-256 verification.
The queue worker cleanup regression also passes: repeated injected storage
errors release the only active slot and connection budget, and subsequent
queued downloads complete. Changed-crate Clippy and the downloader package
check passed after the queue and retry regressions were added. The full
workspace check and test suites now pass under the reduced-memory Cargo
profile. An isolated
Android `phase30Test` package was built, installed, and launched separately
from the user's debug package; its own app-private database was created.
No actual Android download was submitted. A full workspace test
attempt previously failed during Android-bridge linking with Windows
`LNK1102` out-of-memory, but the latest reduced-memory run completed
successfully. Strict workspace Clippy remains blocked by a pre-existing
warning. Android active-download recovery and actual Windows FetchX application
process-kill recovery remain unvalidated.

## Scope

Phase 30 hardens existing lifecycle and retry behavior. It does not replace the
Phase 29 Download Intelligence scope, add a second retry mechanism, or change
the intentional non-retryable classification of mid-body HTTP decode errors.

## Changes

- Added per-download `retry_started` and `retry_exhausted` diagnostics for
  transient HTTP status and request failures.
- Added structured retry logs with engine, attempt, classification, reason,
  status when available, and delay. The logs do not include the source URL.
- Extended retry tests to assert diagnostic event codes and URL redaction.
- Added a file-backed SQLite close/reopen test that recovers an interrupted
  HTTP download, resumes from its validator-protected partial file, and checks
  completed content and SHA-256.
- Added retry-backoff cancellation and pause/resume tests.
- Added a serialized queue worker regression proving the queue drains after
  injected `StorageFull` engine errors.
- Added a feature-gated deterministic queue-stress validation using the
  existing local HTTP fixture: 10, 50, and 100 items each included one
  downloader-originated injected `StorageFull` failure, verified successful
  SHA-256 hashes, persisted terminal states, idle queue state, and released
  worker/connection resources.
- Added an isolated Android `phase30Test` debug build type with its own
  application ID and package-scoped app data.

## Environment

- Host: Windows; Rust workspace and Android Gradle build.
- Android device: physical V2135, Android 13; boot completed.
- Android package: `dev.fetchx.app.debug`.
- Native Android bridge libraries were rebuilt in release mode for
  `arm64-v8a` and `x86_64` before assembling the debug APK.
- The APK installed with `adb install -r`; the local APK and installed
  `/data/app/.../base.apk` SHA-256 values matched:
  `bc9fc6af102e965c0c1c70b3d8c0b9ed98487c59c6deb83b8ac96b3f5f17e51d`.
- During this gap-closure pass, the device reconnected as serial
  `13721393500005M`; `sys.boot_completed` returned `1`. FetchX was launched
  as PID `17180`. Read-only inspection found `files/fetchx.db` (86,016 bytes)
  and `DownloadForegroundService` active in the foreground.
- The app database already contained torrent-source files and the foreground
  service was active. Its download rows were not queried and the app was not
  stopped, cleared, or subjected to instrumentation. This preserves a
  potentially active user workload.

## Validation Evidence

| Area | Result | Evidence and limits |
|---|---|---|
| Workspace tests (latest pass) | PASS | `cargo test --workspace --jobs 1` completed successfully with 0 failed tests under `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, and `CARGO_PROFILE_TEST_DEBUG=0`; this includes the new SQLite persistence regression. |
| Downloader library tests (latest pass) | PASS | `cargo test -p fetchx-downloader --lib --jobs 1`: 270 passed, 0 failed, 3 ignored; includes filesystem-fault, missing-partial, subprocess recovery, and retry cancellation/pause tests. |
| Queue library tests | PASS | `cargo test -p fetchx-queue --lib --jobs 1`: 35 passed, 0 failed; includes the new serialized StorageFull worker-failure continuation test. |
| Storage library tests | PASS | `cargo test -p fetchx-storage --lib --jobs 1`: 35 passed, 0 failed. |
| Transient request retry | PASS | A scripted connection interruption followed by success made two requests, completed the payload, emitted `retry_started` / `HTTP_TRANSIENT_REQUEST`, and did not expose the URL in diagnostics. |
| HTTP status retry and exhaustion | PASS | Tests cover 404, 403, 429, 500, 502, and 503. Non-transient statuses made one request; transient statuses exhausted four total attempts and persisted `Failed`. The 503 case asserted three retry-start events and one exhaustion event. |
| Mid-body HTTP interruption | PASS | A truncated response made one request, returned a decode error, persisted `Failed`, preserved the seven-byte `.part` file, and did not publish the destination. |
| SQLite reopen and HTTP resume | PASS | The test closed all database/repository handles, reopened the file-backed database, recovered `Downloading` to `Queued`, resumed at byte 786432 using the saved ETag, and verified `Completed`, exact bytes, matching SHA-256, and persisted completion. This is a controlled close/reopen test, not an abrupt process kill. |
| Abrupt downloader-test-process recovery | PASS, TEST HARNESS ONLY | The parent test killed a child downloader test process during a synchronized HTTP transfer, verified persisted `Downloading` state and a nonempty partial, recovered the isolated SQLite database, resumed the transfer, and verified completed bytes and SHA-256. This does not kill or relaunch the FetchX Windows application. |
| Android regular debug build/install (previous pass) | PASS | Native release libraries were rebuilt for both supported ABIs; `:app:assembleDebug` succeeded. The installed APK hash matched the newly built APK. |
| Isolated Android Phase 30 package | LIVE VALIDATED, STARTUP ONLY | Rebuilt `:app:assemblePhase30Test`; APK package ID `dev.fetchx.app.phase30test` installed successfully alongside the existing app. PID `25606`, UID `10291`, and private `files/fetchx.db` were observed. This validates isolated package startup/storage only, not download execution. |
| Android startup and idle relaunch | PASS, LIMITED | The device reported boot complete; `MainActivity` was resumed with PID 13047. Idle force-stop left no package PID; relaunch resumed `MainActivity` with PID 13559. No download was active during this test. |
| Android bridge/core startup | LIVE VALIDATED, LIMITED | The isolated package started and its app-private database was created. Targeted logcat returned no `FetchXApp`/`FetchXBridge` lines. No new download was created or executed. |
| Queue lifecycle | EXISTING TESTS PASS | The workspace suite passed, including existing startup recovery, shutdown, worker isolation, and queue/worker race tests. This phase did not add end-to-end fault injection for every queue failure mode. |
| Filesystem and persistence faults | PASS, INJECTED TEST CASES | Test-only write failures for generic I/O, `PermissionDenied`, and `StorageFull`, missing-partial recovery, final publication assertions, and persistent failure state all passed in the downloader library suite. The feature-gated queue regression also proves slot release and continuation after an injected downloader `StorageFull`; actual OS permission denial and a genuinely full disk were not induced. |
| Android active-download recovery | BLOCKED | The isolated package was clean and safe to use, but no approved deterministic fixture route was available: production URL validation rejects local/private targets. No active download was started, and no process kill/relaunch recovery was attempted. The pre-existing user package/data were not modified. |
| Windows abrupt application-process recovery | NOT VALIDATED | The new subprocess test validates downloader recovery in an isolated test harness only. No FetchX application process was used; production URL validation rejects the local fixture and no isolated Windows application runtime was available. |
| Remote/RPC integration | TESTED, LIMITED | Three existing prebuilt `server_integration` tests passed for authenticated RPC, authorization, security limits, and restart persistence. The suite deliberately rejects local/private download targets, so it did not validate remote download execution through completion. |

## Retry Semantics

HTTP request/status retries remain bounded by the existing policy: four total
attempts, a 30-second budget, 250 ms initial backoff, an 8-second maximum
backoff, and jitter. Statuses 408, 425, 429, and 5xx are transient. Timeout,
connect, and request failures are retryable under the existing classifier.

A mid-body interruption currently surfaces as a **non-transient decode error**:

```text
response begins
-> body is interrupted
-> decode error
-> no automatic retry
-> partial file is preserved
-> persisted Failed state
```

This is intentional current retry semantics, not an unresolved retry defect.
Validator- and range-based resume is a separate recovery operation; it must not
be described as an automatic retry.

## Quality Gates

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS after gap-closure edits |
| `cargo check --workspace --all-targets --jobs 1` | PASS under `CARGO_BUILD_JOBS=1` and `CARGO_INCREMENTAL=0`. |
| `cargo test --workspace --jobs 1` | PASS under `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, and `CARGO_PROFILE_TEST_DEBUG=0`; 0 failed tests. |
| `cargo test -p fetchx-queue --lib --jobs 1` | PASS: 35 passed, 0 failed. |
| `cargo test -p fetchx-storage --lib --jobs 1` | PASS: 35 passed, 0 failed. |
| `cargo test --workspace --jobs 1` | PASS on rerun: all workspace test binaries passed with 0 failures under the reduced-memory profile. |
| `cargo check -p fetchx-downloader --all-targets --jobs 1` | PASS after the queue regression addition. |
| Changed-crate Clippy (`fetchx-queue`, `fetchx-downloader`, all targets/features; existing `collapsible_if` lint allowed) | PASS after the queue regression addition. |
| Strict package Clippy (`fetchx-downloader`, `fetchx-queue`, `fetchx-storage`, `-D warnings`) | BLOCKED by the pre-existing dependency warning in `crates/models/src/progress.rs:21`; rerun with only `-A clippy::collapsible_if` passed. |
| Strict workspace Clippy (`-D warnings`) | BLOCKED only by the pre-existing `clippy::collapsible_if` in `crates/models/src/progress.rs:21`; unrelated code was not changed. |
| Android `:app:assembleDebug` | PASS in the previous pass; no production-code changes were made in this gap-closure pass |
| `git diff --check` | PASS after gap-closure edits |

## Limitations

- The controlled SQLite recovery test models a clean handle close/reopen. The
  separate subprocess test does simulate killing a child downloader test
  process during transfer, but does not kill the FetchX desktop application or
  cover power loss or a kill during a transaction/finalization window.
- Earlier Android validation covered install, activity launch, and idle process
  relaunch. The gap-closure pass observed bridge-backed startup and the
  foreground service, but did not cover a new download, progress/completion,
  active-download force-stop recovery, database row state, or on-device file
  hash.
- The gap-closure pass observed bridge/core startup behavior and an active
  foreground service, but did not inspect download rows, submit a download, or
  verify progress/completion/file integrity.
- Windows application process-kill recovery and remote end-to-end download
  execution remain unvalidated.
- The injected write/permission/storage-full and missing-partial tests passed,
  but they model errors in a test-only hook rather than inducing OS-level
  failures.
- No actual disk-full or OS permission failure was induced. Storage-full and
  permission-denied are modeled only in the new test-only injector.
- Strict workspace Clippy is not clean because of the existing
  `clippy::collapsible_if` warning in `crates/models/src/progress.rs:21`.
- Retry-backoff and active-retry cancellation are now covered through the real
  QueueManager/EngineDispatcher/DirectHttpEngine path. Finalization
  cancellation remains unproven because publication and the subsequent
  `Completed` transition have no test synchronization boundary.

## Phase 30 Gap-Closure Validation

### Classification Summary

| Gap | Classification | Evidence |
|---|---|---|
| Android isolated test app startup | LIVE VALIDATED, LIMITED | Dedicated `dev.fetchx.app.phase30test` package was installed and launched, with isolated UID and database. |
| Android bridge download execution | NOT PROVEN | Bridge initialization code path is implemented and app-private DB exists, but no real download request/progress/completion was observed; matching startup logcat entries were absent. |
| Android active-download crash recovery | BLOCKED | Isolated test app is available and safe; no deterministic fixture can reach the app without a local/private URL rejected by production SSRF policy. The existing user's package/data were not touched. |
| Downloader test-process crash recovery | PASS, TEST HARNESS ONLY | Child test process was killed during transfer; isolated persisted state was recovered, transfer resumed, and final content/SHA-256 were verified. This is not application-process validation. |
| Windows abrupt FetchX app-process recovery | NOT VALIDATED | No isolated Windows FetchX application/database fixture was available; the safe local test server is rejected by production SSRF policy. |
| Filesystem write/permission/storage-full injection | PASS, INJECTED TEST CASES | All three injected downloader error kinds passed. The feature-gated queue regression routes injected `StorageFull` through the real HTTP body-write path, confirms failure persistence, releases the queue slot, and proves a subsequent item completes. These remain injected faults, not real filesystem/ACL/disk-full conditions. |
| Missing partial during recovery | PASS | Test removed the partial while stale identity metadata remained and verified recovery issued a full request without a Range header and completed with expected content. |
| Finalization failure | TESTED | No-overwrite publication, failure/partial-retention assertions, and crash-during-finalization recovery passed in the downloader suite. |
| Queue worker failure/races | TESTED | Existing queue lifecycle/race tests plus the real retry-backoff and active-retry cancellation regressions passed, including worker cleanup, queue continuation, fairness, and global-budget bounds. |
| Remote/RPC reliability | TESTED, LIMITED | Three existing server integration tests passed for auth, role authorization, security limits, and restart persistence. They do not execute a remote download to completion. |
| Engine failure paths | TESTED, LIMITED | Existing HLS, aria2 mock, FFmpeg, yt-dlp, and native-torrent tests passed as listed below. No general live download was performed for each engine. |

### Device Evidence and Safety

- Device: physical V2135, Android 13, serial `13721393500005M`; boot property
  was `1`.
- Package: `dev.fetchx.app.debug`; process PID `17180` after launch.
- The previously built APK hash is recorded in the Environment section. This
  pass did not reinstall or rebuild it because the new code is test-only.
- Read-only inspection observed `files/fetchx.db` at 86,016 bytes and
  `DownloadForegroundService` with `isForeground=true`.
- The existing download rows were not queried. The device had torrent-source
  files and an active foreground service, so no download was started, paused,
  cancelled, force-stopped, or resumed.
- Targeted logcat searches yielded no matching `FetchXApp`, `FetchXBridge`, or
  `AndroidRuntime` records. The database/service observations indicate that
  startup progressed through the bridge-backed repository path, but are not
  evidence of a newly created or completed download.
- Active-download crash recovery, database status, on-device payload hash, and
  a deterministic Android fixture remain NOT VALIDATED.

### Windows Process Recovery

- No normal FetchX database was opened or modified and no application process
  was killed.
- The test `tests::abrupt_subprocess_termination_recovers_and_completes_http_download`
  passed. It launches a child instance of the downloader's Rust test executable
  against a loopback fixture, kills that child after persisted `Downloading`
  state and partial data are observed, then recovers and completes the download
  with verified bytes and SHA-256.
- The deterministic Rust fixtures bind to loopback, while the production
  Direct HTTP source policy rejects local/private destinations. The loopback
  exception is test-build-only; no safe Android/Windows app runtime override
  was found.
- No live Windows process-kill/relaunch, queue reconciliation, range resume,
  or final SHA-256 validation was performed in the FetchX application. The
  test-process harness is not a substitute for application-process validation.

### Filesystem Fault Matrix

| Scenario | Implementation / test | Result |
|---|---|---|
| Write failure during body transfer | `cfg(test)` write-failure injection for a specific download ID, with generic I/O, `PermissionDenied`, and `StorageFull`. Tests assert one HTTP request, persisted `Failed`, unpublished destination, and retained partial. | PASS in the downloader library suite; actual OS-level faults are not simulated. |
| Finalization/publication failure | Concurrent-destination test prevents replacement; recovery test validates interrupted finalization. | PASS, including persisted failure and retained-partial assertions. |
| Missing partial file | Recovery test deletes partial data while keeping stale identity metadata, then expects a fresh non-Range request and correct completed content. | PASS. |
| Permission failure | Modeled as a deterministic test-only write error; no OS ACL/permission change was made. | INJECTED CASE PASS; real OS permission denial NOT PROVEN. |
| Disk full | Modeled as `ErrorKind::StorageFull`; the device/host volume was not filled. | INJECTED CASE PASS; real disk-full behavior NOT PROVEN. |

No production filesystem abstraction or broad test infrastructure was added.
The feature-gated queue regression proves worker cleanup and queue continuation
following an injected downloader write error. Actual OS permission denial and
disk exhaustion remain uninduced.

### Engine Recovery Matrix

| Engine | Startup / runtime failure | Cancel / pause / resume | Retry | Cleanup / recovery / finalization | Evidence classification |
|---|---|---|---|---|---|
| Direct HTTP | Connection/HTTP failure, injected body-write failures, missing-partial recovery, finalization failure, and subprocess crash recovery passed in the downloader suite. | Cancellation/resume covered by earlier tests; no active app process was interrupted. | Bounded retry and exhaustion passed in earlier suite. | SQLite/range recovery and finalization recovery passed; subprocess test recovered an interrupted transfer and verified content/hash. The feature-gated QueueManager path also released its slot and continued after an injected body-write failure. | TESTED, PARTIAL; Windows application-process recovery and real OS filesystem-failure behavior remain NOT VALIDATED. |
| HLS | Permanent fragment failure passed among 22 tests. | Cancellation/resume using identity-validated fragments passed; pause not established. | Transient fragment retry and permanent no-retry behavior passed. | Workspace cleanup and crash-after-publication recovery passed. | TESTED with deterministic in-process fixtures; no live external HLS runtime. |
| aria2 | RPC/status failure handling passed in 38 mocked tests; actual startup is unavailable. | Pause/resume/cancel and active GID cleanup passed in mocks. | Ambiguous submission is explicitly not resubmitted; general retry behavior not established. | GID replacement and persisted recovery/resubmission cases passed in mocks. | TESTED with mocks; `aria2c` NOT FOUND, so live startup/runtime NOT VALIDATED. |
| yt-dlp | Runner/output validation tests passed among 22 tests; live process startup failure not exercised. | Cancellation preserves resume workspace; pause behavior not established. | Automatic retry behavior not established. | Published-output recovery and workspace handling passed. | TESTED at unit/fake-runner level; executable is present, but no live deterministic download was run. |
| FFmpeg | Process-failure path passed among 13 tests; `ffmpeg` NOT FOUND for live execution. | Cancellation cleanup passed; pause/resume are not established. | Automatic retry not established. | Workspace cleanup and no-overwrite publication passed. | TESTED with controlled process fixtures; live process execution NOT VALIDATED. |
| Native Torrent | Two integration tests reached Downloading/Seeding lifecycle states, but one pre-created the payload rather than receiving it from a peer. | Pause/resume/cancel released session entries; Stop Seeding retained the file. | Retry behavior not established. | Session cleanup and file SHA-256 preservation on stop passed; restart-after-stop and crash recovery not tested. | TESTED for local lifecycle controls; peer transfer and device lifecycle NOT VALIDATED. |

The downloader library suite ran after the gap-closure test additions. Focused
queue, remote, and other engine test executables were prebuilt from the
preceding successful workspace test run; those results validate existing
regressions, not new live platform behavior.

### Queue, Remote, and Phase 28 Regression Checks

- Nine focused existing queue tests passed: interrupted startup requeue,
  shutdown pause, cancellation lifecycle, engine rejection/failure persistence,
  completion/removal race, priority/FIFO/fairness, active-download/global
  connection limits, seeding capacity release, and global-budget behavior.
- Three `server_integration` tests passed for authenticated RPC security,
  authorization roles, CORS/WebSocket tickets, and restart persistence. The
  suite rejects local/private download URLs; authenticated download creation
  through queue completion and persistence was not validated.
- No Phase 28 benchmark campaign was rerun. Focused queue resource-budget,
  fairness, lifecycle, and engine regression tests ran; throughput and
  sustained-runtime performance were not measured.

### Build/Test Constraints and Generated Artifacts

- The final downloader test suite and changed-crate Clippy passed under a
  reduced-memory Cargo profile (`CARGO_BUILD_JOBS=1`,
  `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_TEST_DEBUG=0`).
- The focused queue worker cleanup regression passed after the new queue test
  was added.
- The isolated Android `phase30Test` variant built, installed, and launched
  successfully. It created a separate app-private database and was not used to
  submit a download.
- An earlier full workspace attempt failed at Android bridge linking with
  Windows `LNK1102` out-of-memory. The latest sequential workspace check and
  test runs passed under the reduced-memory profile. No unrelated process was
  killed and system paging-file configuration was not changed.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- Generated Cargo/Gradle/APK/native artifacts remain covered by existing
  ignore rules (`/target/`, Android `build/`, `*.apk`, and generated
  `jniLibs/**/*.so`). No generated artifact was added to the worktree and no
  `.gitignore` change was needed.

## Phase 30 Final Closure Validation

### Executive Summary

**PARTIAL.** Deterministic downloader recovery/fault tests and queue worker
cleanup are tested. The subprocess crash harness is not the FetchX Windows
application. The connected Android installation has a running app process and
previously observed foreground service, so no force-stop was performed. No
actual Android download/recovery or real Windows application crash/recovery
was observed. Real OS permission/disk-full conditions and remote download
completion remain unvalidated.

### Evidence Matrix

| Capability | Implementation | Automated Test | Live Validation | Result | Evidence |
|---|---|---|---|---|---|
| Retry and exhaustion | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Bounded request/status retries, exhaustion state, retry diagnostics, and URL redaction passed the downloader tests. |
| Retry cancellation and pause | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | New controlled retry-after tests show cancellation produces Cancelled without another request; pause stays Paused until explicit resume, after which the fixture completes. |
| Mid-body HTTP interruption | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Truncated body is a non-transient decode error; no automatic retry; partial retained and state persisted as Failed. Manual/range resume is separate. |
| HTTP resume and missing partial | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Controlled SQLite reopen resumes with validator/range; missing partial triggers a full request; final bytes/hash verified. |
| Filesystem write failure | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Test-only injector cases for generic I/O, PermissionDenied, and StorageFull passed; unpublished destination, retained partial, and persisted failure asserted. |
| Finalization failure | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | No-overwrite publication, failure/partial retention, and interrupted-finalization recovery assertions passed. |
| Downloader process crash recovery | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Child downloader test process was killed during transfer; isolated DB state and partial survived; recovery resumed and SHA-256 matched. This is test harness validation only. |
| Windows FetchX application recovery | IMPLEMENTED | NOT PROVEN | NOT VALIDATED | NOT VALIDATED | Tauri setup initializes CoreServices/queue in-process using the per-user app-data DB and download directory. No isolated desktop profile/fixture path was used, localhost is rejected by production URL policy, and no actual FetchX app process was killed/restarted. |
| Android isolated startup | IMPLEMENTED | TESTED | LIVE VALIDATED — LIMITED | LIVE VALIDATED — LIMITED | `dev.fetchx.app.phase30test` built/installed/launched with PID 14002, UID 10291, and isolated private database. |
| Android bridge download execution | IMPLEMENTED | TESTED | NOT PROVEN | NOT PROVEN | `BridgeDownloadRepository.addDownload` delegates to UniFFI `DownloadService`, but no real download request/progress/completion was observed. |
| Android active-download recovery | IMPLEMENTED | NOT PROVEN | BLOCKED | BLOCKED | The isolated package exists, but production SSRF rejects the localhost/private deterministic fixture; no download or kill/relaunch was performed. |
| Queue worker cleanup after storage error | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | The feature-gated regression now uses the real `QueueManager -> EngineDispatcher -> DirectHttpEngine` path. A controlled local HTTP fixture serves HEAD/GET responses; A fails at the downloader body-write boundary with injected `StorageFull`, persists `Failed`, leaves only its `.part` file, releases the single active slot, and B starts and completes with a matching SHA-256. The queue becomes idle with no active or queued entries. |
| Real OS permission failure | IMPLEMENTED | NOT PROVEN | NOT VALIDATED | NOT VALIDATED | No ACL/permission changes were made. PermissionDenied is covered only by deterministic test injection. |
| Real disk-full condition | IMPLEMENTED | TESTED | NOT VALIDATED | NOT VALIDATED | StorageFull is injected in downloader and queue tests; no volume was filled and no OS-level exhaustion was induced. |
| Database recovery/integrity | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | File-backed close/reopen and subprocess recovery validate persisted state and resumed completion. Storage health tests call `Database::check`, which runs `PRAGMA integrity_check`; the specific crash-fixture DB was not separately checked. |
| Isolated SQLite integrity check | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | `current_schema_health_check_is_read_only_and_complete` passed in the storage suite; it checks an isolated temporary database. |
| Database operation failure during state persistence | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | An isolated SQLite database reopened read-only rejects a lifecycle transition with a storage error and preserves the prior Queued state; the database is explicitly closed and the fixture is removed. This is deterministic persistence-failure coverage, not live application fault injection. |
| Pause, resume, and cancel | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Existing focused queue lifecycle tests passed; no new application/device lifecycle was run. |
| Cancel during retry backoff | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Real QueueManager regression observes the retryable 503 and cancels before the next attempt. A persists `Cancelled`, makes no second A request, releases the worker/connection budget, B completes with the expected payload, and the queue reaches idle. |
| Cancel during active retry | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Real QueueManager regression blocks A's second HTTP attempt, cancels it, releases the fixture, and verifies A remains `Cancelled`, does not publish a destination, B completes, and the queue reaches idle. |
| Cancel during finalization | IMPLEMENTED | NOT PROVEN | NOT VALIDATED | NOT PROVEN | The HTTP engine checks cancellation after body completion and immediately before finalization. It then marks the temporary identity finalizing, publishes with an atomic hard link when supported (or copy-then-delete on filesystems without hard-link support), and transitions SQLite to `Completed`. Cancellation is not observed within that sequence, and no safe synchronization point exists between publication and the durable transition. |
| Queue continuation after cancellation | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Both retry cancellation regressions use real QueueManager A/B workloads; B completes after A cancellation, with no active or queued work remaining. |
| Persistence after cancellation | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Both QueueManager regressions verify A's persisted terminal `Cancelled` state after the worker exits. |
| Startup recovery of finalizing artifact | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Existing recovery test covers both an unpublished complete `.part` and an already-published equal destination. Recovery requires the finalizing marker, exact expected size, and byte equality when a destination exists before persisting `Completed`; a mismatched destination is not accepted. Queue startup requeues interrupted statuses, after which the HTTP engine performs this reconciliation. |
| Filesystem/DB consistency after finalization race | IMPLEMENTED | TESTED | NOT VALIDATED | NOT PROVEN | Existing finalization recovery and no-overwrite publication tests pass, but the cancellation race between publication and the `Completed` transition remains unproven because no deterministic boundary exists there. |
| Queue bounds and fairness | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Existing queue tests validate configured active/connection bounds, fairness, and scheduling. |
| Queue stress - 10 items | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Feature-gated deterministic campaign completed 10 items with exactly one injected downloader `StorageFull`; 9 successful payload hashes matched, the failed `.part` remained, SQLite was terminally consistent, and active/queued/fixture connections were zero at idle. |
| Queue stress - 50 items | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Feature-gated deterministic campaign completed 50 items with exactly one injected downloader `StorageFull`; 49 successful payload hashes matched, the failed `.part` remained, SQLite was terminally consistent, and active/queued/fixture connections were zero at idle. |
| Queue stress - 100 items | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Feature-gated deterministic campaign completed 100 items with exactly one injected downloader `StorageFull`; 99 successful payload hashes matched, the failed `.part` remained, SQLite was terminally consistent, and active/queued/fixture connections were zero at idle. |
| HLS | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Existing deterministic tests cover fragment failure, retry/cancel/resume and publication recovery; no live external HLS runtime. |
| aria2 | IMPLEMENTED | TESTED | NOT VALIDATED | NOT VALIDATED | Mock-based tests passed; `aria2c` is unavailable, so live engine startup/completion was not validated. |
| yt-dlp | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Existing runner/output tests passed; no deterministic live download was run. |
| FFmpeg | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Controlled process tests passed; `ffmpeg` is unavailable for live execution. |
| Native torrent | IMPLEMENTED | TESTED | NOT VALIDATED | TESTED | Existing integration tests cover local lifecycle controls; peer transfer and Android recovery were not validated. |
| Remote/RPC completion | IMPLEMENTED | TESTED | NOT VALIDATED | NOT VALIDATED | Auth/security/restart integration tests passed; no remote download ran through completion. Existing SSRF controls were not changed. |

### Windows

The Windows app is Tauri (`apps/desktop/src-tauri`). Its setup initializes
`CoreServices` with the app-data database and platform download directory; the
download commands use the managed queue. The downloader subprocess test does
not execute this Tauri application. An actual app recovery test would require
an isolated Tauri profile/database/download directory plus a safe fixture
transport that preserves production SSRF policy. Neither was available in
this pass; Windows FetchX application recovery is **NOT VALIDATED**.

### Android

`BridgeInitializer` creates the UniFFI `DownloadService` and
`BridgeDownloadRepository`; `addDownload` delegates to that service, and the
foreground service observes repository state. This establishes that a download
path is implemented, not that it completed on-device. ADB found the physical
device booted. The new isolated variant `dev.fetchx.app.phase30test` was built,
installed, and launched (PID 14002, UID 10291); its app-private
`files/fetchx.db` was created at 86,016 bytes. Targeted startup logcat returned
no matching bridge/app entries. No download was submitted because production
SSRF rejects the localhost/private fixture needed for a deterministic
interruption test. The existing user package/database was not modified.
Isolated startup is live validated, but bridge download execution is
**NOT PROVEN** and active-download recovery is **BLOCKED**.

### Queue Reliability

The original simulated queue test uses one active slot and one global
connection. It processes four
queued items sequentially; calls one and three return an injected
`StorageFull` engine error from a test managed-engine implementation, while
calls two and four complete. The test asserts both failures are persisted with
errors, both later items complete, and the snapshot has no active or queued
work. This proves queue continuation after an engine-reported storage error;
it does not route the downloader's filesystem injector through QueueManager
or inspect operating-system task/connection handles.

The feature-gated `real_http_filesystem_failure_releases_queue_resources`
regression exercises the actual downloader path rather than returning a fake
engine result. The dispatcher performs its normal HTTP HEAD probe and GET
request against a controlled local fixture. Download A reaches the HTTP body
write, receives an injected `StorageFull`, persists `Failed`, and leaves a
zero-byte `.part` plus metadata file without publishing the destination.
Download B then completes through the same queue with the expected SHA-256;
the queue snapshot is idle and both terminal states are persisted. The
feature is opt-in and does not change production behavior when disabled.

The feature-gated
`cancellation_during_retry_backoff_releases_queue_and_continues` and
`cancellation_during_active_retry_releases_queue_and_continues` regressions
use the same real queue/dispatcher/HTTP path. They synchronize on the first
retryable response and, for the second case, the blocked second attempt
instead of sleeping for an assumed interval. In both cases A remains
persisted as `Cancelled`, does not publish a destination, B completes with
the expected payload, and the queue snapshot has no active or queued work.

Finalization is intentionally classified separately. After body completion,
the HTTP engine checks cancellation, marks the temporary identity as
finalizing, publishes without replacement, and then transitions the durable
item to `Completed`. Publication uses a hard link when supported and falls
back to copy-then-delete otherwise. There is no safe synchronization boundary
between the publish and the durable transition; adding a production delay or
hook solely for this validation would change the architecture, so cancellation
during that narrow race remains **NOT PROVEN — architectural test boundary
unavailable**.

The existing finalization recovery test covers both recoverable orderings: a
complete marked `.part` with no destination, and a destination already present
with identical bytes. Recovery requires the finalizing marker and exact size,
compares bytes before accepting an existing destination, and only then
persists `Completed`; a mismatched destination is not accepted.

### Database Persistence Failure

The core state-machine regression creates an isolated file-backed database,
closes it, reopens it read-only, and attempts `Queued -> Preparing`. SQLite
returns a storage error, the persisted item remains `Queued`, and the database
is explicitly closed before the fixture is removed. This proves error
propagation and state preservation for a deterministic persistence failure; it
does not inject failures into every database operation or validate a live
application process.

### Environment Blockers and Remaining Limits

- Earlier full workspace check/test attempts were blocked by host memory/linker
  pressure (`memory allocation ... failed` and Android bridge `LNK1102`); the
  latest reduced-memory check and test runs passed.
- Strict workspace Clippy remains blocked by the pre-existing
  `clippy::collapsible_if` in `crates/models/src/progress.rs:21`.
- No safe isolated Windows Tauri runtime/profile and fixture transport were
  used. The Android package was isolated successfully, but its deterministic
  local fixture route is rejected by production SSRF policy.
- Real OS permission/disk-full conditions, remote completion, and live engine
  downloads remain unvalidated.

## Final Classification

**PARTIAL** - deterministic retry, filesystem-fault, database-persistence,
missing-partial, subprocess crash-recovery, and feature-gated queue stress
tests pass. The real queue path proves continuation after a downloader-
originated injected filesystem error, but this remains fault-injection
evidence rather than a real OS permission or disk-full event. The subprocess
test validates an isolated downloader test process, not the FetchX Windows
application. Android active-download recovery and bridge download execution,
remote completion, and real OS filesystem failures remain unvalidated.
Finalization cancellation and the publication-to-`Completed` persistence race
remain NOT PROVEN because no safe synchronization boundary exists there. The
latest workspace check/tests pass under the reduced-memory profile, while
strict workspace Clippy remains blocked by the pre-existing warning.

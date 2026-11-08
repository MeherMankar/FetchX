# Gopeed Feature Analysis

## Repository and license

- Reference repository: `GopeedLab/gopeed`.
- Reviewed revision: `8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b`.
- Repository license: GNU GPL-3.0.
- No Gopeed source was copied into FetchX. The Apache-2.0 FetchX implementation is an independent design based on documented behavior.

## Selected capability

FetchX selected a compact runtime-status projection inspired by Gopeed's separation between durable task metadata and frequently changing runtime state.

FetchX already exposed progress in full download records. The selected improvement adds one shared-core projection containing only:

- Download ID.
- Lifecycle status.
- Downloaded bytes.
- Optional total bytes.
- Optional speed.
- Optional estimated remaining time.
- Percentage in basis points.
- Updated timestamp.

This avoids repeatedly transferring URLs, destinations, filenames, priorities, engine configuration, and errors from Android, desktop, CLI/remote clients when UI polling only needs progress state.

## Shared implementation

- Domain model: `crates/models/src/runtime.rs`
- Core API: `CoreServices::download_runtime_statuses`
- Android UniFFI: `DownloadRuntimeStatusDto` and `list_runtime_statuses`
- Desktop Tauri: `list_runtime_statuses`
- Desktop TypeScript: `listRuntimeStatuses`
- Remote JSON-RPC: `download.runtimeStatus`

No durable storage or migration is required because runtime status is derived from current `DownloadItem` state.

## Target integration

| Target | Integration status |
|---|---|
| Android | Shared UniFFI method backed by canonical Rust projection |
| Desktop Windows/Linux/macOS | Shared Tauri command and TypeScript API backed by canonical Rust projection |
| CLI | Existing list output remains metadata-compatible; core projection is available for future compact polling |
| Remote server/UI | Added JSON-RPC `download.runtimeStatus` and a typed remote UI client helper |
| Browser extension | Read-only runtime-status method is in the existing restricted browser RPC allowlist |

## Validation

- Models, core services, Android bridge: 84 unit/integration tests passed; 4 live extraction tests remained intentionally ignored.
- Desktop Tauri Rust integration: `cargo check -p fetchx-desktop` passed.
- Remote protocol/server and desktop Rust integration: `cargo check -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-desktop` passed.

## Prompt 2 audit and next capability

### Upstream material rechecked

- Current reviewed revision: `8ef3b63e8d93f34e9f6c161823ff2bdfe50f1f0b` (2026-11-06).
- License: GPL-3.0, confirmed from the repository's `LICENSE`. No Gopeed code was copied; FetchX remains an independent Apache-2.0 implementation.
- Relevant sources and tests:
  - `README.md`: documents `resolve_task` as a pre-creation operation for resource metadata/file selection, followed by `create_task`, and separates task status/statistics from task metadata.
  - `pkg/mcpserver/server.go`: defines the public task tools and their input/output schemas, including `resolve_task`, `create_task`, and lightweight status queries.
  - `pkg/api/service.go` and `pkg/api/service_test.go`: implement and test the Gopeed service operations.
  - `pkg/rest/api.go`: exposes Resolve, CreateTask, and GetTaskStatus through REST.
- These sources demonstrate an explicit preflight-versus-create boundary. FetchX has URL normalization, link extraction, filename suggestions, and an in-memory URL comparison helper, but had no CoreServices operation to compare a candidate with persisted records.

### Capability comparison and selection

| Gopeed capability/pattern | FetchX before this phase | Assessment |
|---|---|---|
| Download lifecycle, retries, pause/resume, queue recovery | Rust queue/scheduler and shared lifecycle | Already implemented adequately |
| Compact runtime status distinct from durable task metadata | Newly added `DownloadRuntimeStatus` projection | Implemented; audited in this phase |
| Pre-creation resource inspection (`resolve_task`) | Several standalone built-ins, no one resolver contract | Partially implemented; broader resolution remains follow-up |
| Search/filter/batch task management | Queue/history APIs exist, but UX varies by host | Partially implemented |
| Persisted URL duplicate lookup before creating a task | Only a helper comparing caller-supplied URL strings | Missing at the shared service/API boundary |

Selected capability: **persisted normalized-URL duplicate lookup**. It is the smallest useful part of Gopeed's preflight-before-create pattern that fits the existing service and adapter architecture. It checks existing records but does not block creation or make user policy decisions. Full resource resolution is deferred because it would require protocol-specific network inspection beyond the scope of one capability.

### FetchX implementation and exposure

- Canonical API: `CoreServices::find_duplicate_download`; it uses the existing Rust URL normalizer and repository list.
- Result: `DuplicateDownloadMatch` returns only ID, filename, status, and update time; source URLs and paths are not returned.
- Android: exposed through UniFFI as `DownloadService.find_duplicate_download`.
- Desktop: exposed as Tauri `find_duplicate_download` and a typed TypeScript API helper.
- CLI: local `find-duplicate` and remote `remote find-duplicate` commands call the same core/JSON-RPC behavior.
- Remote: `download.findDuplicate` is a read-authorized JSON-RPC method, advertised in discovery, and permitted through the existing browser RPC allowlist; remote UI provides a typed client helper.
- Browser extension/native host: the existing restricted remote RPC route permits the new read-only method. No browser-side URL matching logic was added.
- Persistence: no schema or migration change; query derives from current persisted downloads.

### Runtime-status audit and shutdown fix

- Runtime status is constructed from persisted `DownloadItem` records, so queued, preparing, downloading, paused, completed, failed, cancelled, metadata-acquiring, seeding, recovering, and stopped values retain the domain enum unchanged. Removed downloads disappear from the projection because their persisted records are deleted.
- Android UniFFI, desktop Tauri/TypeScript, and remote JSON-RPC expose the same `id`, `status`, nested `progress` (including percentage basis points), and `updated_at` millisecond timestamp fields. Remote authorization requires `DownloadsRead`; server discovery and the restricted browser RPC allowlist include the method because dispatch is implemented. CLI can access the same CoreServices API.
- Android UniFFI exposes this projection, but no Kotlin caller/UI consumption was found in the repository; this audit therefore does not claim Android runtime UI integration.
- The shutdown failure reproduced in `authorization_roles_gate_rpc_events_and_survive_restart`: cancelling a missing download propagated `NotFound` from a queue command through the actor loop, terminating the queue. Later scheduler shutdown then failed with `queue manager has shut down`.
- Queue cancellation now returns `QueueError::DownloadNotFound` to the request without stopping the actor; queue shutdown is also idempotent. The remote integration test now expects the correct not-found response (`-32004`) rather than service-unavailable (`-32010`).

### Validation and limitations

| Command | Result |
|---|---|
| `cargo test -j 1 -p fetchx-models -p fetchx-core-services -p fetchx-android-bridge -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-queue --lib --tests --no-fail-fast` | Passed; includes persisted duplicate lookup, Android bridge tests, remote authorization/restart integration, and queue lifecycle tests |
| `cargo test -j 1 -p fetchx-models -p fetchx-remote-protocol -p fetchx-queue --lib` | Passed; includes all lifecycle status projection, duplicate RPC parameter validation/browser allowlist, and queue cancellation/shutdown regression tests |
| `cargo test -p fetchx-remote-server --test server_integration authorization_roles_gate_rpc_events_and_survive_restart -- --nocapture` | Passed after the queue actor fix |
| `cargo test -j 1 -p fetchx-android-bridge -p fetchx-remote-server --lib --tests --no-fail-fast` | Passed after final runtime-status DTO/RPC alignment; includes focused payload tests and remote integration tests |
| `cargo fmt -p fetchx-remote-server -p fetchx-android-bridge -- --check` | Passed |
| `cargo check -j 1 -p fetchx-desktop` | Passed |
| `cargo check -j 1 --manifest-path crates/cli/Cargo.toml` | Passed |
| `npm --prefix apps/desktop/ui run build` | Passed |
| `npm --prefix apps/remote-ui run build` | Passed |
| `npm --prefix apps/remote-ui run test -- --maxWorkers=1` | Passed; 1 test file, 5 tests |
| `cargo check -j 1 --target aarch64-linux-android -p fetchx-android-bridge` | Blocked: `aarch64-linux-android-clang`/Android NDK toolchain is unavailable; host bridge tests compiled, but this is not Android target or device runtime validation |

A first broad parallel Rust test/build attempt exhausted the Windows environment's paging file while linking examples and test targets. The focused suites above passed with Cargo jobs and test threads bounded.

No Gopeed functionality beyond the documented workflow pattern was copied. Prompt 16 has since verified the full Gopeed `resolve_task` source contract and compared it with FetchX's protocol flows. The broader pre-creation resolver remains a meaningful gap, but implementation is deferred pending a bounded protocol/adapter scope; see [`GOPEED_RESOLVE_TASK_ANALYSIS.md`](GOPEED_RESOLVE_TASK_ANALYSIS.md).

# Ketch Feature Analysis

## Reference review

- Repository: `linroid/Ketch`
- Reviewed revision: `c6834964b4dc46d473173a9d68c56c77197684c5` (`main`, 2026-10-31)
- License at that revision: Apache License 2.0 (`LICENSE`)
- Reviewed documentation: repository README, including the feature description for parallel ranges, pause/resume, and the shared Kotlin Multiplatform download engine.
- No Ketch source was copied. The change below is an independent Rust implementation using FetchX's existing downloader contracts.

## Relevant Ketch implementation

- `library/core/src/commonMain/kotlin/com/linroid/ketch/core/engine/HttpDownloadSource.kt`: resolves server range support and resource metadata, chooses segmented transfers when supported, validates saved ETag/Last-Modified information on continuation, and restarts when the server does not support ranges.
- `library/core/src/commonMain/kotlin/com/linroid/ketch/core/segment/SegmentDownloader.kt`: downloads one bounded range, writes at explicit offsets, checks segment boundaries, and reports incomplete transfers for retry.
- `library/api/src/commonMain/kotlin/com/linroid/ketch/api/Segment.kt`: describes a byte interval and tracks its completed byte count.
- `library/core/src/commonMain/kotlin/com/linroid/ketch/core/segment/SegmentedDownloadHelper.kt`: orchestrates concurrent segment lanes and adapts to connection-count changes.
- `library/core/src/commonTest/kotlin/com/linroid/ketch/segment/SegmentDownloaderTest.kt`, `library/core/src/commonTest/kotlin/com/linroid/ketch/segment/SegmentLedgerTest.kt`, and `library/api/src/commonTest/kotlin/com/linroid/ketch/api/SegmentTest.kt`: test range boundaries, partial lane continuation, and segment accounting.
- `app/android/src/main/kotlin/com/linroid/ketch/app/android/KetchService.kt`: owns Android foreground/service lifecycle and application wiring; the transfer engine remains in the shared KMP library.

## Capability comparison

| Capability | FetchX before this phase | Assessment |
|---|---|---|
| HTTP byte-range detection and parallel segments | Existing Rust `DirectHttpEngine` negotiates and validates byte ranges; segment planning and adaptive concurrency are shared | Already implemented adequately |
| Segment size and connection limits | Configurable ranges, capped at 64; scheduler and queue supply connection budgets | Already implemented adequately |
| Segment-level retry and progress aggregation | Bounded retry, aggregate byte progress, bandwidth control, and final exact-size verification | Already implemented adequately |
| Pause/cancel and partial-segment recovery | Segment files survive cancellation; resume reuses files when resource identity matches | Already implemented, with a validator-less recovery gap |
| Changed-resource safety when no validator is supplied | Existing predicate allowed reuse when both stored/current validators were absent, provided the URL matched | Missing; selected |
| FTP parallel range transfers | FTP/FTPS support resume through `REST`, but does not share the HTTP segmented implementation | Partially implemented; not selected |
| Android background execution | Foreground service and network/power policy are platform-owned; Rust owns transfer state and work | Already implemented at the adapter boundary |
| Ketch-specific dynamic segment rebalancing | Not evidenced as necessary because FetchX already adapts active worker concurrency; repartitioning stored files would require a wider design | Requires further investigation |

## Selected capability: validator-safe segment recovery

Reuse saved HTTP range segments only when both the previously saved identity and the current probe have the same non-empty validator (ETag or Last-Modified) and the original URL matches. When a server provides no validator, discard old segment fragments and re-fetch the ranges rather than risk publishing a file assembled from different representations.

Acceptance criteria:

1. A paused/cancelled segmented transfer with a matching non-empty validator retains and resumes existing segments.
2. Existing segments with no current validator are deleted before resumed ranged work.
3. Newly downloaded bytes assemble to exactly the current server representation.
4. Existing invalid-range fallback and single-stream resume behavior remain unchanged.

## FetchX implementation and integration

- Canonical behavior: `crates/downloader/src/http.rs`, in the resume eligibility predicate before `download_segmented`.
- Regression coverage: `crates/downloader/src/tests.rs`, using a range-capable server without ETag or Last-Modified and stale pre-existing segment files.
- No public API or model change: HTTP transfer selection and recovery are internal to the existing Rust `DirectHttpEngine`.
- No persistence schema change: existing `.part.meta` identity metadata is sufficient.
- Android, Tauri desktop, CLI, remote server, and browser/native-host flows that dispatch a `direct_http` download automatically use the same Rust engine. This is shared-core implementation status, not a claim that each UI has a dedicated setting or separate segmented-download control.
- Android's UniFFI service constructs `CoreServices`; the foreground service only keeps the process alive and manages OS notifications/network policy. The desktop Tauri adapter, CLI bootstrap, and remote server likewise compose `CoreServices`, so none needs platform-specific range/recovery logic.
- Target status:

| Target | Integration status |
|---|---|
| Android | `CoreServices`/shared dispatcher behavior is available through the existing Rust bridge; no Kotlin transfer code changed. Android ARM64 build/runtime was not verified. |
| Windows/Linux/macOS desktop | Existing Tauri CoreServices composition uses the same downloader. |
| CLI | Existing local download path uses the same CoreServices downloader. |
| Remote server | Existing `download.create` dispatch uses the same CoreServices downloader. |
| Remote UI/browser client | Requests use remote JSON-RPC; the downstream server executes the shared Rust engine. No client-side range logic was added. |
| Browser extension/native host | Existing local/remote create routes delegate to their host's shared core; no extension-side segmentation was added. |

- Existing runtime-status and duplicate-lookup reachability was rechecked: Android exposes both via UniFFI, but the Kotlin `BridgeDownloadRepository` has no caller for those methods; desktop exposes typed Tauri/UI API wrappers without a found screen-level use; CLI has duplicate lookup but no compact runtime-status command; remote JSON-RPC exposes status and duplicate lookup, the remote UI has typed helpers but no found view consuming them, and the browser server route allowlist permits them without a verified dedicated extension UI. This phase did not expand those surfaces.
- Android foreground service remains a lifecycle adapter. No Kotlin transfer behavior was added.
- No dependency was introduced.

## Validation

| Command | Result |
|---|---|
| `cargo test -j 1 -p fetchx-downloader resumed_segments_without_a_server_validator_are_restarted -- --nocapture` | Passed after the fix. The regression test failed before the fix because no new ranges were requested and stale segments were reused. |
| `cargo test -j 1 -p fetchx-downloader -p fetchx-models -p fetchx-core-services -p fetchx-android-bridge -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-queue --lib --tests --no-fail-fast` | Passed; downloader 274 passed, 3 ignored; other selected model/core/bridge/protocol/server/queue unit and integration targets passed, including duplicate lookup and queue shutdown coverage. |
| `cargo check -j 1 -p fetchx-desktop` | Passed. |
| `cargo check -j 1 --manifest-path crates/cli/Cargo.toml` | Passed. |
| `cargo check -j 1 --target aarch64-linux-android -p fetchx-android-bridge` | Blocked before crate compilation: `aws-lc-sys` could not find `aarch64-linux-android-clang` / the Android NDK toolchain. |
| `rustfmt --edition 2024 --check crates/downloader/src/tests.rs` and `git diff --check` | Passed. |
| `cargo fmt -p fetchx-downloader -- --check` | Reports existing formatting differences elsewhere in `crates/downloader/src/http.rs` (SHA-256 import ordering and checksum helper formatting); no unrelated formatting cleanup was applied. |

No Android device/runtime or platform transfer run was performed. Rust tests prove behavior in the shared downloader on the host, not Android execution.

## Limitations and deferred work

- If no validator is available, a legitimate resumed transfer restarts from zero; this trades additional bandwidth for representation integrity.
- This change does not add validator negotiation, public segment configuration, adaptive rebalancing of persisted segments, parallel FTP, or UI changes.
- A future networking feature should assess Ketch's richer segment rebalancing against FetchX's existing adaptive scheduler before adding equivalent state.
- Next repository in the established research order: AB Download Manager.

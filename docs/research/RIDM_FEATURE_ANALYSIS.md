# Ridm-Inspired Media Discovery and Batch Admission

## Source review and access limitations

Requested project: <https://gitlab.com/mremiyum/ridmapp>

The GitLab project was not accessible from this environment:

- `git ls-remote https://gitlab.com/mremiyum/ridmapp.git HEAD` returned “The project you were looking for could not be found or you don't have permission to view it.”
- `https://gitlab.com/api/v4/projects/mremiyum%2Fridmapp` returned HTTP 404.
- The project page redirected to GitLab sign-in; a subsequent page request was blocked by an access challenge.
- GitHub repository search for `mremiyum ridm` returned no repositories. Guessed `mremiyum/ridmapp` and `mremiyum/ridm` GitHub remotes were not found.

Consequently, no Ridm revision, license, source files, tests, or project documentation could be verified. The exact license is unknown. This analysis does not attribute specific behaviors to Ridm and no Ridm source was copied. The comparison below distinguishes FetchX evidence from Ridm facts that remain unverified.

## Current-state audit

The Phase 0 architecture and roadmap identify Rust `CoreServices` as the shared composition boundary, with Android UniFFI, Tauri, CLI, and remote APIs acting as adapters. Existing media extraction is exposed as a single-URL operation (`CoreServices::extract_media`); it returns one `MediaInfo`. `extract_download_links` extracts explicit HTTP(S) links from supplied text, but does not discover a media playlist or produce per-item media metadata. Before this phase, download creation was exposed as single-item host operations; there was no shared-core list-admission operation.

The Seal phase's MP3/M4A selection remains a typed `YtDlpAudioFormat` on each `DownloadSource`; it is not a batch-discovery feature and is not duplicated here. This change adds no model field or persistence behavior.

The prior Seal validation record reports that Android Gradle unit tests were attempted but the wrapper JVM crashed in native `net.dll` while enumerating network interfaces (`NetworkInterface.getAll`); Android Kotlin compilation and tests were therefore not verified. No real media-site extraction or MP3/M4A conversion was run. This phase leaves Android unchanged and does not upgrade those validation claims.

The preceding phase records also describe: Gopeed-derived runtime/duplicate projections (with the wider `resolve_task` operation still deferred); Ketch-inspired validator-safe HTTP segment recovery; AB Download Manager-inspired persisted queue ordering; Download Navi-inspired Android sticky-service restart policy (without a device restart test); and Aria2App-inspired desktop-only, non-persistent aria2 preflight. Their target and validation boundaries remain as recorded in the corresponding research documents; this Ridm change does not alter those features.

## Capability comparison

| Capability | Ridm evidence | FetchX observed behavior and classification |
|---|---|---|
| Discover media items from a page or playlist | Requires investigation; source and documentation were inaccessible | `extract_download_links` finds explicit HTTP(S) strings in supplied text, not playlist entries. `MediaExtractor` extracts one URL at a time. Page/playlist media discovery is missing. |
| Present metadata and formats for each discovered item | Requires investigation | Single-URL media extraction exists. A batch of URLs is not expanded into per-item `MediaInfo` records or per-item format choices. Partially implemented. |
| Admit multiple explicit URLs to the queue | Requires investigation | Added shared-core batch admission; the desktop Add dialog supplies links extracted from pasted text. This was missing before the phase. |
| Deduplicate and validate batch items independently | Requires investigation | Each URL is HTTP(S)-validated; duplicates are normalized within the batch; each input receives its own success or error. Implemented in the shared core. |
| Naming and organizing batch items | Requires investigation | Safe package-name normalization is shared; existing queue destinations and earlier batch destinations are reserved; colliding suggested filenames receive numbered suffixes. Implemented for batch admission. |
| Partial failure, cancellation, progress, and lifecycle | Requires investigation | A failed item does not block later items. Already-enqueued items use the ordinary persisted queue and progress lifecycle; cancelling batch admission leaves accepted items queued and stops unstarted admission. Implemented for batch admission; no separate batch-progress entity is added. |
| Separation of discovery, selection, and execution | Requires investigation | The desktop dialog extracts explicit links, passes common selection options, and delegates validation/queueing to Rust. Per-item metadata discovery and selection are not implemented. |

## Selected capability and acceptance criteria

Because the reference source could not be verified, the selected work is described as a batch-download capability in the requested problem area, not as a behavior confirmed in Ridm. The non-duplicative target is shared-core admission of an explicit list of URLs. FetchX already had single-URL extraction and text-link extraction, but neither operation independently validated and enqueued a batch with per-input outcomes.

Acceptance criteria:

1. Return one outcome, in input order, for each supplied request.
2. Reject invalid HTTP(S) URLs, duplicate normalized URLs within the batch, unavailable engines, invalid format options, and unsafe package names without preventing later valid requests.
3. Avoid overwriting an existing queue destination or an earlier item in the same batch when suggested filenames collide.
4. Persist accepted items through the existing queue lifecycle; cancellation must not roll back previously accepted work.
5. Keep business rules in Rust and avoid adding batch-specific database state.

## Implementation

### Shared core

`crates/core-services/src/lib.rs` adds public `BatchDownloadRequest`, `BatchDownloadOutcome`, and `BatchDownloadError` types and `CoreServices::enqueue_download_batch`. It validates each request independently, uses the shared URL normalizer for intra-batch duplicate detection, applies the existing yt-dlp source model validation, sanitizes optional package names, reserves existing and current-batch destinations, and enqueues valid items through `QueueManager`.

The batch is intentionally non-transactional. If the future is cancelled, accepted items remain persisted, while work not yet admitted is not started. Queue dispatch, item progress, pause/resume/cancel, and completion remain the ordinary FetchX lifecycle.

`crates/core-services/src/builtin.rs` adds `sanitize_download_package_name`; the desktop adapter now reuses that helper instead of maintaining its own package-name sanitization logic.

### Public API and data

- Additive public Rust API in `fetchx-core-services`; no persisted model changes.
- Desktop Tauri adds `add_download_batch` and a per-input result DTO. The React API and Add dialog extract explicit links from the pasted text and call the Tauri command when multiple links are present. A common package, engine, priority, and media-format choice is applied to those links.
- No migration, export/import version change, new dependency, credentials, cookies, or batch-progress storage.

## Adapter integration status

| Target | Status |
|---|---|
| Rust shared core | Implemented and covered by focused tests. |
| Windows/macOS/Linux desktop | Tauri command and React Add dialog call the shared-core operation. The same desktop implementation targets all three Tauri platforms; this environment validates only Windows-host builds. |
| Android | Not exposed through UniFFI and not wired to Android UI. Android continues to create downloads through its existing single-item service methods. |
| Local CLI | No batch command added; existing CLI add operation remains single-item. |
| FetchX remote JSON-RPC and remote UI | No batch RPC method or UI flow added; existing create operation remains single-item. |
| Browser extension/native host | No batch method added. |

The core method is available to Rust hosts, but adapters without explicit wiring do not expose this capability to their users. These gaps are deferred, not claimed as cross-platform integration.

## Persistence and security

No schema migration or new storage table is required. Each accepted item is stored by the existing queue/repository path. Batch requests contain URLs and selection options only; they add no credential or cookie fields and do not persist a batch manifest. Per-input errors are returned to the caller; no secrets are logged by this feature.

## Tests and validation

Commands and results recorded for this phase:

- `cargo test -p fetchx-core-services batch_` — passed; 4 focused tests cover mixed valid/invalid/duplicate/format requests (including an empty URL, unsafe package, and unavailable engine), existing/intra-batch filename collisions, empty batches, and cancellation preserving already accepted queue items.
- `cargo test -p fetchx-core-services` — passed: 28 unit tests, 1 duplicate-download integration test, 1 media-extraction integration test, and 1 doctest; 4 live-extraction tests were ignored.
- `cargo clippy --workspace --exclude fetchx-desktop --all-targets --all-features -- -D warnings` — passed.
- `cargo check -p fetchx-desktop` — passed after the Tauri async command was changed to return `Result`.
- `cargo test -p fetchx-desktop` — passed: 60 Tauri desktop tests.
- `npm run build --prefix "D:\Projects\FetchX\apps\desktop\ui"` — passed TypeScript checking and the Vite production build.
- `cargo fmt --all -- --check` and `git diff --check` — passed.

Tests use temporary databases and example URLs; no live media site, network service, external downloader binary, or device is required by these focused core tests. Android validation is not applicable to the current diff because the Android bridge and UI are unchanged.

## Limitations and deferred work

- Ridm source, revision, license, tests, and exact discovery/batch behavior remain unverified due to GitLab access failure; no upstream-specific design conclusion should be inferred.
- This is explicit-URL batch admission, not playlist parsing, webpage media discovery, per-item format discovery, or per-item media selection.
- Only the desktop user interface currently invokes the operation. Android UniFFI/UI, local and remote CLI, authenticated JSON-RPC, remote UI, and browser RPC integration remain deferred.
- Cancellation is admission cancellation only; it does not cancel items already queued. Those items are controlled through existing per-download lifecycle operations.

Recommended next repository/work item in the established research order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

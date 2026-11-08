# FetchX Shared-Core Roadmap

## Compatibility assessment

FetchX already has the strongest part of the architecture in place: canonical Rust models, lifecycle, storage, queue, scheduler, engines, and a composition root. This phase should therefore align and constrain hosts rather than rewrite the core.

## Current Gopeed-derived progress

- Runtime status is projected from persisted domain records and exposed through Android UniFFI, desktop Tauri/TypeScript, and remote JSON-RPC.
- A repository-backed normalized-URL duplicate lookup is exposed through CoreServices and existing Android, desktop, CLI, remote, and browser-RPC boundaries. It reports a match without enforcing duplicate policy.
- A missing-download cancellation no longer terminates the queue actor; shutdown is idempotent. Focused queue and remote restart regression tests cover this lifecycle fix.
- The repository remains schema-compatible; no migration or new dependency was introduced.
- Next: assess a full pre-creation `resolve_task` contract for protocol-specific metadata and file selection, building on Gopeed's separate resolve/create workflow. Do not equate this with the narrower duplicate lookup.

## Current Ketch-derived progress

- Verified that the Rust HTTP downloader already implements range negotiation, parallel segmented transfer, adaptive worker limits, bounded retries, bandwidth sharing, pause/cancel recovery, and safe assembly.
- Tightened HTTP segment recovery: saved fragments are reused only when the current server provides the same non-empty validator as the saved resource identity. Without a validator, segments are discarded and requested again.
- The behavior is entirely inside `DirectHttpEngine`; Android, desktop, CLI, and remote hosts continue to use their existing shared-core engine dispatch. No adapter API, schema, or dependency change is required.
- Deferred: parallel FTP, dynamic persistent-segment rebalancing, and per-segment UI remain separate investigations and are not implied by this recovery-safety change.
- Next repository in the research order: AB Download Manager.

## Current AB Download Manager–derived progress

- Reviewed AB Download Manager queue orchestration and manual ordering at revision `68f0ea6e8ba5cbeb02c92342cd3bf7475378ae68` (Apache-2.0); no upstream code was copied.
- Added Rust queue operations to move queued downloads up or down within the same priority group. The existing priority-first dispatch rule remains authoritative.
- The queue actor persists reordered IDs using the existing SQLite `queue_order` column. No schema migration or new dependency is required; restart recovery preserves the ordering.
- Exposed the operations through Android UniFFI and queued-row controls, desktop Tauri and queue-order sorting/actions, local/remote CLI, authenticated JSON-RPC, and the remote web UI. The browser extension/native-host method allowlist remains read-only.
- Focused queue/storage, adapter, frontend, and Android unit/build results are recorded in `docs/research/AB_DOWNLOAD_MANAGER_FEATURE_ANALYSIS.md`.
- Next repository in the research order: Download Navi, focusing on Android background reliability rather than another queue-ordering feature.

## Current Download Navi–derived progress

- Reviewed Download Navi at revision `10c7f9e5b306f66b122782e4878446d0ff2ac9b9`; its README states GNU GPL version 3 or later and `LICENSE.md` contains GPLv3. No upstream source was copied.
- Verified that FetchX Rust already requeues interrupted persisted downloads during queue initialization and preserves downloader partials.
- Fixed the Android foreground service restart policy: when the bridge is available, a system restart (including a null start intent) is sticky; when the bridge is unavailable, the service stops and remains non-sticky.
- The service and Android repository remain adapters. Rust continues to own recovery decisions, queue eligibility, retries, and transfer execution.
- No schema migration, UniFFI contract change, dependency, or desktop/CLI/remote change was required.
- `cargo test -p fetchx-queue --lib` passed all 42 tests. The focused Android restart-policy unit test passed with `.\gradlew.bat :app:testDebugUnitTest --tests 'dev.fetchx.app.service.DownloadForegroundServiceTest'`.
- No Android emulator/device process-recreation test was run. Force-stop and boot-time autostart are not covered.
- Aria2App remote-engine integration has now been reviewed and completed as documented below.

## Current Aria2App-derived progress

- Reviewed `devgianlu/Aria2App` revision `dd6cbc9e7b17559e59d9bc41a3fd8c0468e088d7` (5.10.0, 2025-07-13); its application `LICENSE` is GNU GPL-3.0. The pinned `aria2lib` submodule revision is `290755e646f62fb26ed4112f6ea2c0e5d6cbea82` under Apache-2.0. No upstream source or binaries were copied.
- FetchX already had a shared Rust aria2 JSON-RPC client/engine, token authentication, request correlation, and an authenticated FetchX remote server. The server and CLI remote commands control FetchX; they are not external aria2 clients.
- Desktop exposes a non-persistent remote aria2 probe through `Aria2Client`. The probe requires HTTPS for remote endpoints and restricts HTTP to IP loopback; the returned diagnostic includes version and aria2 `enabledFeatures`, without persisting endpoint or secret.
- The probe is read-only and does not configure remote aria2 as the active engine. No arbitrary-endpoint FetchX RPC, Android probe UI/UniFFI method, CLI probe command, storage change, migration, or dependency was added.
- Per-target status, upstream files, tests, and limitations are recorded in `docs/research/ARIA2APP_FEATURE_ANALYSIS.md`. Android `START_STICKY` remains covered by host-side policy tests only; no process-recreation test on a device/emulator was run.
- Recommended next repository/work item in the established research order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

## Current Seal-derived progress

- Reviewed Seal revision `7677f61a20fda4210e84215fef9e9b51d25daa47` and its root GNU GPL version 3 license, dated 29 June 2007. No Seal source was copied.
- Verified that FetchX already had Rust-owned yt-dlp extraction, video-format selection, progress/cancellation, and Android FFmpeg support for merged formats. Added the single missing capability: opt-in MP3/M4A audio extraction.
- Rust owns the typed persisted option, validation, `bestaudio/best` selection, yt-dlp conversion arguments, lifecycle, progress, and output finalization. Android Kotlin exposes the browser choice and supplies bundled FFmpeg; desktop and local CLI expose the same shared-core operation.
- Added the additive SQLite migration 12 and version-2 export handling. Remote JSON-RPC and the browser extension/native-host remain unchanged and do not expose media-specific extraction.
- Tests, build/runtime limitations, target status, and reviewed Seal source files are recorded in `docs/research/SEAL_FEATURE_ANALYSIS.md`.
- Recommended next research item: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

## Current Ridm-phase status

- The supplied GitLab project path could not be inspected: `git ls-remote` reported that the project was not found or accessible, the GitLab project API returned 404, and the web page redirected to sign-in/was blocked by an access challenge. A GitHub search for `mremiyum ridm` returned no repositories, and guessed GitHub mirrors were not found. No Ridm revision, license, source, tests, or documentation could therefore be verified; no Ridm source was copied and no project-specific implementation claims are made.
- FetchX previously extracted metadata/formats for one media URL at a time and extracted explicit HTTP(S) links from supplied text, but had no shared operation to admit a list of URLs with independent outcomes.
- Added `CoreServices::enqueue_download_batch` for independent per-URL validation/admission, normalized intra-batch duplicate detection, safe package names, collision-free suggested filenames, and partial-success/cancellation semantics. It reuses the normal queue and persistence lifecycle and adds no schema migration or dependency.
- The desktop Add dialog extracts explicit links from pasted text and calls the new Tauri batch command. The operation is not exposed through Android UniFFI/UI, local or remote CLI, authenticated JSON-RPC, remote UI, or browser RPC; these adapter surfaces remain deferred rather than being represented as integrated.
- Core test/build results, exact files, and limitations are recorded in `docs/research/RIDM_FEATURE_ANALYSIS.md`.
- Recommended next repository/work item in the established research order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

## Milestone 0 — contract freeze

1. Create one versioned core operation/error/event contract in Rust.
2. Contract-test UniFFI DTOs against Rust models.
3. Contract-test Tauri DTOs and JSON-RPC DTOs against Rust models.
4. Document stable enum and status values.
5. Add capability registry output to Android, Tauri, and JSON-RPC discovery.

Acceptance:

- Android, desktop, CLI, and remote discovery agree on operations and error codes.
- No host advertises an engine before the capability registry confirms runtime availability.

## Milestone 1 — host-neutral built-ins

1. Route Android production link extraction, filenames, packages, normalization, duplicate detection, and categories only through Rust built-ins.
2. Remove the Android unit-test/safety fallback where no native bridge is available in production.
3. Centralize package/destination path validation.
4. Add host-neutral configuration and capability tests.

Acceptance:

- Same URL produces the same filename, category, package, and duplicate result on Android and desktop.

## Milestone 2 — lifecycle consolidation

1. Reduce desktop request construction to OS/filesystem adaptation plus validation.
2. Standardize Android repository mappings and remove silent priority mapping where the core has no matching value.
3. Use one typed event enum across core, UniFFI, Tauri, and JSON-RPC.
4. Add coalescing policy at hosts, not core.

Acceptance:

- Hosts cannot advance download status outside shared transitions.
- Missed host events recover through the same snapshot API.

## Milestone 3 — capability registry

1. Replace scattered engine-status views with one core capability registry.
2. Add platform adapters and executable/runtime status.
3. Include capabilities in remote discovery and Android/desktop diagnostics.
4. Add explicit unsupported handling.

Acceptance:

- Unsupported WebDAV/archive/plugin-execution capabilities cannot appear as implemented.
- Optional binaries report disabled/unavailable through a consistent contract.

## Milestone 4 — storage compatibility

1. Keep migration versions embedded in `fetchx-storage`.
2. Add host database compatibility and migration-contract tests.
3. Define Android/desktop platform storage profiles.
4. Ensure logical schema upgrades are safe before physical database sharing is attempted.

Acceptance:

- Every host opens the latest schema with the same migration code.
- App-private and non-shared databases remain import/export compatible.

## Milestone 5 — cross-platform integration

1. Add contract integration tests with Android UniFFI.
2. Add desktop Tauri binding tests.
3. Add JSON-RPC contract tests.
4. Test lifecycle, persistence, recovery, cancellation, and engine capability in core-owned suites.

Acceptance:

- Shared core tests pass independently of all hosts.
- Host tests validate mapping, lifecycle, permissions, and platform boundaries only.

## Milestone 6 — packaging and supported targets

1. Build Windows, Linux, and macOS desktop targets from one Tauri source.
2. Build Android ARM64 and x86_64 packages from the same core.
3. Verify CLI and remote-server builds on supported host architectures.
4. Add browser extension/native-host contract compatibility tests.

## Licensing and dependencies

- Keep FetchX Apache-2.0 components compatible with included dependency licenses.
- Treat reference-manager integrations as concepts unless source permission and license compatibility are explicit.
- Any GPL-derived implementation cannot be silently mixed into Apache-2.0 FetchX distribution.
- Before introducing a dependency, record license, platform support, maintenance status, native build requirements, security implications, and distribution restrictions.
- Review aria2, yt-dlp, FFmpeg, torrent, russh, SQLx, reqwest, UniFFI, and Tauri distributions separately; current executable/helper packaging has additional binary-license obligations.
- Avoid adding plugin code execution to Android until Android capability and policy constraints are explicitly implemented.

## First recommended implementation milestone

Implement **Milestone 0 and Milestone 1 together**:

- keep the current core/runtime graph unchanged;
- add capability and operation contracts around existing APIs;
- make shared built-ins authoritative for Android and desktop;
- remove production fallback divergence;
- add tests proving same-core behavior across Android and desktop bindings.

This is low-risk and establishes the acceptance criterion before later lifecycle or capability migrations.

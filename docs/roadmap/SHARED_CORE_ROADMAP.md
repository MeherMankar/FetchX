# FetchX Shared-Core Roadmap

## Compatibility assessment

FetchX already has the strongest part of the architecture in place: canonical Rust models, lifecycle, storage, queue, scheduler, engines, and a composition root. This phase should therefore align and constrain hosts rather than rewrite the core.

## Current Gopeed-derived progress

- Runtime status is projected from persisted domain records and exposed through Android UniFFI, desktop Tauri/TypeScript, and remote JSON-RPC.
- A repository-backed normalized-URL duplicate lookup is exposed through CoreServices and existing Android, desktop, CLI, remote, and browser-RPC boundaries. It reports a match without enforcing duplicate policy.
- A missing-download cancellation no longer terminates the queue actor; shutdown is idempotent. Focused queue and remote restart regression tests cover this lifecycle fix.
- The repository remains schema-compatible; no migration or new dependency was introduced.
- Prompt 16 verified Gopeed's pinned `resolve_task` contract and compared it with FetchX protocol-by-protocol. FetchX has specialized media and local-torrent inspection, but no common pre-creation resolver; duplicate lookup is not a substitute.
- A common resolver is useful, but implementation is deferred rather than partially adding HTTP-only or adapter-specific behavior. The proposed typed contract, transient-handle constraints, compatibility requirements, and acceptance criteria are in [`GOPEED_RESOLVE_TASK_ANALYSIS.md`](../research/GOPEED_RESOLVE_TASK_ANALYSIS.md).

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

## Current XDM-derived progress

- Reviewed XDM `master` at `0b361b2badd8690b3fae482a4b5e60673977b4e5`. Its root license is GNU GPL-3.0 (29 June 2007); reviewed DASH/HLS source files also carry Apache-2.0 SPDX headers. No XDM source was copied.
- Verified that FetchX already has native HLS downloads and yt-dlp-backed DASH extraction, but direct `.mpd` downloads entered as Direct HTTP were not routed through either media path.
- Added shared-core `.mpd` URL detection and dispatcher routing to the existing yt-dlp engine, including acceptance of direct MPD sources by that engine. Queue pause handling now uses the effective source route, preserving yt-dlp's unsupported-active-pause behavior and cancel/requeue shutdown policy. No new native DASH engine, model, migration, dependency, or adapter API was added.
- Android, desktop, CLI, remote, and browser submissions continue through the shared dispatcher. The browser source already detects `.mpd` URLs; browser runtime behavior and authenticated request-header forwarding remain unverified/deferred.
- Exact source review, capability comparison, implementation, adapter status, validation, and limitations are recorded in `docs/research/XDM_FEATURE_ANALYSIS.md`. The prior Seal Android Gradle limitation remains: its wrapper JVM crashed in native `net.dll`; no Kotlin/device validation is claimed here.
- Recommended next research item in the established order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

## Current Motrix-derived progress

- Reviewed Motrix revision `057662a00598e4c7447469cabf53c6924209eb43`; its root license is MIT, with separate third-party asset licensing noted by the project. No Motrix source or assets were copied.
- Motrix provides a customizable dashboard with engine status and persisted transfer statistics. FetchX already had current queue summaries, authenticated remote authorization, queue controls, and local engine status, but did not expose shared engine availability to remote clients.
- Added the CoreServices engine-status projection and authenticated, capability-discovered `engine.runtimeStatus` RPC. The remote UI shows all nine engine kinds and refreshes them through its existing polling. Operator and ReadOnly roles may read status; the endpoint omits failure reasons, executable paths, and configuration.
- No migration, persistence, dependency, token change, desktop Tauri change, Android/UniFFI change, CLI change, or browser RPC permission was added. The new method is excluded from the restricted browser/native-host allowlist.
- Focused Rust tests, workspace Clippy, remote UI tests/build, and browser extension tests passed as recorded in `docs/research/MOTRIX_FEATURE_ANALYSIS.md`. The browser extension build itself was not rerun; its prior `TS6133` issue remains reported by the XDM phase. The Seal Android Gradle JVM limitation is also unchanged.
- Recommended next research item: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

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

## Current Persepolis-derived progress

- Reviewed Persepolis revision `109e95261fa2827bf7bf1b24173328d90235a53e` (2025-08-10). Its root license is GNU GPL version 3; reviewed Python headers specify GPL version 3 or later. No source was copied.
- The reviewed Persepolis transfer path is its own segmented downloader; no aria2 JSON-RPC frontend behavior was established in the inspected source. FetchX's existing Rust aria2 client/engine and desktop-only remote preflight were verified before selecting the feature.
- Added `Aria2Client::get_global_stat` and a typed, validated result in the shared downloader crate. The existing desktop remote aria2 probe now reports activity counts and download/upload rates after `getVersion`. A failed or unsupported statistics request does not invalidate a successful connectivity/version probe.
- Integration is intentionally limited to the existing desktop Tauri diagnostic and settings UI. No external endpoint execution, FetchX remote RPC, Android, CLI, or browser operation was added. No endpoint/token/statistic persistence, migration, or dependency was introduced.
- Focused RPC tests, the downloader crate suite, desktop diagnostic serialization test, Rust formatting/whitespace checks, and desktop UI build results are recorded in `docs/research/PERSEPOLIS_FEATURE_ANALYSIS.md`. No live aria2 daemon or device/emulator test was run.
- Recommended next research/work item in the established order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

## Current JDownloader 2–inspired progress

- The official Eclipse setup article identifies JDownloader's source as SVN, but the official trunk endpoint returned HTTP 401 and the SVN client is unavailable here. The GitHub `mirror/jdownloader` snapshot at `f274b29897aad2e0ff0d4fd148e42aadf8b622fa` maps to SVN r49881 (2024-09-27), is not authoritative, and has no root license file. The exact upstream license and current source revision remain unverified; no JDownloader source was copied.
- The official LinkGrabber documentation describes collecting clipboard links and organizing them into packages. FetchX already had text extraction, shared batch admission, host-based single-item package suggestions, per-item results, duplicate lookup, and collision-safe filenames; however, desktop batch links all used a suggestion made from the first URL.
- Added `CoreServices::enqueue_download_batch_by_source`, an additive operation that derives and sanitizes a package per URL with the existing helper, then delegates to the existing batch validation and queue lifecycle. The original `BatchDownloadRequest` shape and `enqueue_download_batch` behavior are preserved. The desktop batch dialog activates grouping when the suggested value has not been manually edited; an explicit name remains a shared override.
- At the JDownloader 2 phase, only the existing desktop Tauri/React batch surface was wired; Android, CLI, FetchX remote JSON-RPC/UI, and browser/native-host did not yet expose batch operations. No migration, dependency, page crawl, credential flow, or plugin framework was added.
- Focused test and desktop build outcomes, plus the official source/license limitation, are recorded in `docs/research/JDOWNLOADER2_FEATURE_ANALYSIS.md`.
- Recommended next work: define and review a first supported-protocol slice against the resolver acceptance criteria before implementation; keep deep page crawling and an unverified hoster-plugin architecture out of scope.

## Cross-phase shared-core integration audit and Android batch admission (Prompts 13–15)

- The verified target-by-target capability matrix and current commit/worktree state are recorded in [`CROSS_PHASE_INTEGRATION_AUDIT.md`](../research/CROSS_PHASE_INTEGRATION_AUDIT.md).
- Android now consumes the persisted Rust duplicate lookup before Add. Matching downloads produce a confirmation warning, while **Add anyway** preserves intentional duplicate creation. Rust remains responsible for URL normalization and persisted lookup; no schema or UniFFI change was needed.
- Android bulk Add now uses the additive `CoreServices::enqueue_download_batch_with_destinations` API through UniFFI. Per-item requests carry URL, filename, absolute destination, engine/media options, and optional checksum; Android preserves package paths and owns public-storage staging/publication, while Rust validates and admits items independently, resolves duplicates/collisions, and returns one outcome per input. The existing CoreServices batch methods and desktop caller remain compatible.
- Rust creates checksum sidecars before enqueuing checked batch items, avoiding a dispatch-before-sidecar race. No migration or dependency was added. Cancellation does not roll back items already accepted by the persisted queue.
- Focused Rust core/Android bridge tests and Android bridge/downloads unit tests pass. These are host tests and Kotlin compilation; no device/emulator, live media transfer, or public-storage publication test was performed.
- Prompt 15 verified that persisted `download.runtimeStatus` is a compact projection of fields already delivered through normal download-list/event paths. Android, desktop, remote UI, and CLI already consume persisted status through those paths. The separate adapters remain available for downstream clients; no duplicate UI polling was added. Focused Rust projection/RPC tests, 60 desktop Tauri tests, Android downloads host tests, and desktop/remote UI checks passed; no device/runtime validation was performed.
- Preserve the reported limitations: no Android device process-restart test; Seal's earlier Gradle `net.dll` JVM crash; this audit's Chaquopy `yt-dlp==2026.8.19` provisioning failure (Kotlin compilation succeeded when provisioning was skipped); XDM's browser `TS6133`; two untouched desktop Clippy warnings; unavailable Ridm source/license; and unverified authoritative JDownloader source/license.
- Prompt 13 introduced no Rust business logic, persistence, migration, or dependency change. The Android warning is a presentation/adapter integration of existing shared-core behavior. Its tests and the compilation/runtime distinction are in the audit report.

Prioritized next work:

1. Validate Android batch media routing, checksum verification, public-storage staging/publication, and restart behavior on an emulator/device.
2. Keep the compact persisted download-runtime-status API available to downstream clients; existing application surfaces consume the same status through their normal list/event flow, so no additional UI integration is needed.
3. Add batch operations to CLI, remote, or browser only if those product surfaces require them; they remain intentionally unexposed.
4. Fix the browser `TS6133` and desktop Clippy warnings in separately scoped changes.
5. Before implementing resource resolution, select a bounded protocol/adapter slice and settle probe security, transient-handle lifetime, and older-remote-server fallback as specified in [`GOPEED_RESOLVE_TASK_ANALYSIS.md`](../research/GOPEED_RESOLVE_TASK_ANALYSIS.md).

## Current aria2-to-native-Rust migration

- Pinned and reviewed aria2 source, manual/RPC behavior, license, and focused tests. No upstream source was copied.
- Reused the existing Rust downloader, CoreServices, queue, storage, models, and `librqbit` engine. Native magnets now acquire and persist metadata, wait for an explicit valid file selection, and transfer selected files.
- Android supports the current native fallback selection path, desktop exposes native metadata/selection through Tauri and UI, and CLI supports inspection/selection after explicit native admission. Remote now exposes authenticated, capability-advertised metadata read and selection confirmation operations with read/control role enforcement; the remote UI capability-gates the flow for older servers. These methods are excluded from the restricted browser/native-host RPC allowlist.
- Prompt 19's trackerless loopback test covered selected-file completion, partial progress, pause, new-engine/session continuation, a controlled seeder interruption, active cancellation/handle cleanup, and retry after restoring the peer. Exact piece reuse and process/device crash recovery remain unproven.
- Prompt 21 changes the shared CoreServices default for new magnet and local `.torrent` admissions to native Rust. Desktop offers an explicit aria2 fallback, CLI keeps `--engine aria2`, Android uses native without an aria2 chooser/runtime, and the current remote UI selects native when both selection RPC capabilities are advertised; older remote clients keep legacy omitted-engine routing. Browser/native-host torrent admission remains unsupported (HTTP(S)-only).
- Existing download records retain their persisted engine and aria2 settings/helpers/RPC remain unchanged. New or missing engine configuration enables native torrent by default; an existing explicit `native_torrent.enabled = false` remains disabled, and default admission reports unavailable rather than silently switching engines.
- The native migration remains partial. Exact restart-piece recovery, process/device recovery, explicit destination-collision behavior, broader aria2 configuration parity, and aria2 removal remain open. Prompt 21 automated tests and builds are host-side; no live remote network, Android device, packaged desktop, public swarm, or live aria2 daemon was tested.
- Prompt 21 validation passed the serial Rust tests across models/downloader/storage/queue/CoreServices/CLI/remote protocol/server/Android bridge/desktop Tauri, remote UI tests and build, desktop UI build, Android `:downloads:test`, formatting, targeted Clippy, and `git diff --check`. Clippy reported only two existing desktop warnings. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for exact commands, counts, and runtime limits.
- Next native-torrent slice: prove exact partial-piece reuse and process/device restart recovery, exercise explicit destination collisions, and validate current/older remote server interoperability with live runtimes before expanding or reconsidering aria2 compatibility boundaries.
- Prompt 22 audited aria2 use across Rust, platform adapters, remote/browser boundaries, packaging, storage, and diagnostics. All default new-download routes are already Rust-native: direct HTTP/HLS/DASH/yt-dlp, FTP/FTPS, SFTP, Metalink, and native BitTorrent. No behavior was changed and no aria2 code was proven dead.
- Retain `Aria2Engine`/RPC/supervision for explicit CLI/CoreServices work, desktop/remote torrent fallback, and persisted aria2 jobs/GID reconciliation. Keep the optional desktop helper/configuration and Android/desktop external-endpoint probes; they are compatibility/settings/diagnostics surfaces, not new-download defaults. Android disables local aria2 bundling, so any legacy Android aria2 record remains persisted but cannot execute locally without an injected runtime.
- The next decision is compatibility exit criteria, not another default migration: validate native crash/process recovery and exact partial-piece reuse; exercise persisted aria2 reconciliation and live old/new remote-server behavior; define rollback/migration for explicit aria2 users and handling for legacy Android rows. Do not remove aria2 RPC, settings, credentials, helper support, or storage fields until these gates pass.
- Prompt 22 was documentation-only; no Rust, adapter, or UI tests/builds were run. `git diff --check` is the documented validation. Full inventory and evidence are in [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md).
- Exact commands, test results, runtime scope, and unverified acceptance criteria are recorded in [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md).

Next: validate exact partial-piece reuse, process restart, and explicit destination-collision behavior on supported hosts, then run the remote selection flow against an older and current live server and exercise Android/desktop runtimes. Keep aria2 packaging/RPC until all migration acceptance criteria pass across supported hosts.

## Prompt 23 — legacy aria2 recovery plan

- Persisted aria2 engine identity is authoritative for dispatch, retry, and interrupted-work recovery. Queue startup requeues interrupted states without changing the engine; it does not fall back to native Rust.
- Torrent GIDs are aria2 RPC task identifiers stored alongside torrent metadata; generic protocol GIDs are runtime-only. GIDs are never native identifiers. A supervised aria2 partial with progress requires its `.aria2` control file before resubmission; its presence does not establish that librqbit can safely consume the same bytes.
- Android has no configured local aria2 executable. Its remote aria2 settings are an endpoint probe, not a persisted-download engine binding. Legacy rows remain stored; failed rows show a sanitized generic reason and retry/remove actions, and retry preserves the row if aria2 is unavailable.
- No engine conversion or schema change is justified. Keep partial files and records intact. Completed files need no engine recovery; queue removal removes only the record and preserves files.
- Next: implement a read-only CoreServices assessment of persisted aria2 recovery requirements and runtime availability; expose an actionable explanation to Android and other clients; validate same-runtime pause/resume and process-restart behavior with a real supervised aria2 daemon, covering GID validity, stale GIDs, and `.aria2` control-file presence/absence.
- Only after this evidence, design opt-in native replacement as a new task while retaining the aria2 original until source, selection, destination collision, and integrity checks pass. No automatic adoption or deletion of aria2 data is allowed.
- Prompt 23 is documentation-only. Source inspection and `git diff --check` are the validation boundary; no tests/builds, live aria2 daemon, Android device, or process-restart experiment was run. See [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md) for the state matrix and exact findings.

## Prompt 24 — read-only persisted aria2 recovery assessment

- Added a typed CoreServices assessment with evidence-limited states for runtime availability, queued work, torrent metadata/GID reconciliation, `.aria2` control-file presence, completed/non-aria2 work, and inconclusive evidence.
- The assessment peeks at an already-running aria2 client and reads persisted metadata/filesystem state only. It never starts/restarts aria2, changes saved engine/GID/status/selection, dispatches a legacy row to native Rust, or changes records/files.
- Android UniFFI and desktop Tauri expose the result. Both UIs show the sanitized explanation and next step and identify the operation as read-only. CLI, FetchX remote JSON-RPC, and browser/native-host have no assessment operation and are not claimed as supported.
- Five focused downloader assessment tests, one CoreServices preservation test, and the Android bridge DTO mapping test passed; affected Rust test modules type-check; desktop TypeScript and production build passed; Android bridge and downloads Gradle unit tests passed after UniFFI generation was retried with constrained Rust build settings. Android device validation remains outstanding. Exact commands/results are recorded in the migration report.
- Next: test aria2 reconciliation with a live supervised runtime, including process restart, stale/current GIDs, and partial-sidecar cases. Do not treat assessment as a native-adoption decision; any future conversion must be opt-in and retain the original row/files until integrity is independently verified.

## Prompt 25 — remote and CLI aria2 recovery assessment

- Added the authenticated `download.aria2Recovery` read operation, backed by CoreServices and advertised as an additive capability under the existing `fetchx.rpc.v1` protocol. The response has `assessment_version: 1` and only sanitized state/guidance fields.
- The operation uses existing `downloads.read` authorization and remains outside the browser/native-host allowlist. The CLI checks server discovery before sending the method; older servers are not called and all previous client operations remain unchanged.
- Added local and remote CLI commands. Local CLI uses a CoreServices startup mode that suppresses aria2 process startup and queue dispatch; without runtime evidence it reports `unknown`. Remote CLI reports only the remote server's assessment, not any separate aria2 endpoint profile.
- Serial Rust test suites passed for CLI, CoreServices, remote protocol, and remote server, covering versioned shape, auth, old-server behavior, and preservation of records/files. No live network/daemon or device restart test was performed.
- Remaining migration blocker: validate recovery against a real supervised aria2 daemon across process restart and partial sidecar/GID cases. The new clients do not enable native adoption and do not change aria2 persistence or defaults.

## Prompt 26 — aria2 parity audit and next gate

- Verified runtime invocation/supervision, explicit backend selection, persisted aria2 engine/GID/sidecar recovery, settings/secrets, helper installation, diagnostics, CLI and remote assessment. Native defaults remain as previously documented; no aria2 code or user settings were removed. The full inventory is in [`ARIA2_NATIVE_RUST_MIGRATION.md`](../research/ARIA2_NATIVE_RUST_MIGRATION.md).
- Torrent routing by target: Android, desktop Add, and CLI default new admissions to native; the current remote UI does so only when the server advertises both magnet metadata and selection; older remote calls with no engine retain aria2-first compatibility; browser/native-host has no torrent route.
- Android correction: the bridge passes `aria2ExecutablePath = null` and does not extract/launch aria2. An ignored, untracked developer asset named `assets/bin/aria2c-arm64-v8a` is present in this worktree, but is not wired into runtime; a separate screen controls an external aria2 RPC server. Do not treat this local file as reproducible packaging support.
- Highest priority is native torrent process-death recovery and exact partial-piece reuse. Native session persistence is disabled. Existing controlled-loopback tests pass for pausing/recreating the engine and selected-only transfer, but operate in-process with in-memory DB state and are not crash/device tests.
- Next deliver a deterministic child-process harness: local seeder, fixed multi-file metainfo, DHT/tracker disabled, file-backed SQLite and output directory, persisted explicit selection, partial checkpoint, worker kill, database reopen, verified selected-file hash/unselected-file absence, unchanged engine/info hash/selection, and measured piece reuse. Run on desktop hosts and Android device before claiming runtime parity.
- No engine code change is justified before this evidence; no dependencies, schema, defaults, adapter behavior, or existing user data changed. Prompt 26 ran the two controlled-peer tests documented in the migration report.
- Upstream aria2 source was pinned and reviewed for behavioral requirements only; no code was copied or translated. Its `COPYING` is GPL version 2 and source headers say GPL-2.0-or-later with a limited OpenSSL exception. Obtain file/binary-specific licensing review before adding source or changing binary distribution.
- Removal gates remain: native process/device recovery; protocol behavior tests/live interoperability; real-daemon recovery of existing aria2 jobs/GIDs/sidecars; consented disposition/rollback for explicit users and old records; and platform packaging/license review. Keep runtime/RPC/settings/helper/storage fields until all gates pass.

## Prompt 27 — native torrent process-death recovery

- Added a deterministic Windows child-process harness in the native downloader tests. It uses a fixed multi-file torrent, a loopback-only seeder, disabled DHT/trackers/local discovery, file-backed SQLite, a persistent selection of file index 1, and a shared output directory.
- The test kills worker A after the persisted download reaches partial progress, verifies a complete selected-file piece remains on disk, reopens the same record in a fresh worker, and confirms state-machine recovery returns the interrupted row to `Queued` before it completes.
- Verification passed: `cargo test -p fetchx-downloader native_torrent_process_death_recovers_selected_files -- --nocapture`. Output matched the expected payload and SHA-1; the engine, info hash, destination, selection and aria2 GID remained unchanged. The unselected file contained no payload (librqbit may create an empty placeholder).
- Two successful runs measured 3,620,864 and 3,653,632 seeder-uploaded bytes after restart for a 4,194,304-byte selected file. This is aggregate evidence consistent with partial reuse, not piece/block-level proof; do not claim exact piece reuse.
- The harness exercises `DownloadStateMachine::recover_interrupted`, not the QueueManager actor's dispatch loop. Android device process-death, live aria2, packaged binaries, and public swarm behavior remain unvalidated.
- No shared-engine runtime, schema, engine default, adapter or aria2 code changed. `librqbit` session persistence stays disabled; its normal restart initialization reuses validated existing output files in this exercised scenario.
- Next: add piece/block-level request evidence and test QueueManager startup/dispatch across recovery, then run the same abrupt kill/reopen scenario on Android devices. Keep all aria2 runtime, RPC, settings, helper, GID and sidecar compatibility until the existing removal gates are satisfied.

# FetchX Shared-Core Roadmap

## Compatibility assessment

FetchX already has the strongest part of the architecture in place: canonical Rust models, lifecycle, storage, queue, scheduler, engines, and a composition root. This phase should therefore align and constrain hosts rather than rewrite the core.

## Current Gopeed-derived progress

- Runtime status is projected from persisted domain records and exposed through Android UniFFI, desktop Tauri/TypeScript, and remote JSON-RPC.
- A repository-backed normalized-URL duplicate lookup is exposed through CoreServices and existing Android, desktop, CLI, remote, and browser-RPC boundaries. It reports a match without enforcing duplicate policy.
- A missing-download cancellation no longer terminates the queue actor; shutdown is idempotent. Focused queue and remote restart regression tests cover this lifecycle fix.
- The repository remains schema-compatible; no migration or new dependency was introduced.
- Next: assess a full pre-creation `resolve_task` contract for protocol-specific metadata and file selection, building on Gopeed's separate resolve/create workflow. Do not equate this with the narrower duplicate lookup.

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

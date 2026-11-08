# Motrix-Inspired Remote Engine Status

## Reference review

- Repository: <https://github.com/agalwood/Motrix>
- Reviewed revision: `057662a00598e4c7447469cabf53c6924209eb43`.
- The root `LICENSE` is MIT, copyright 2018-present Dr_rOot. The README notes that third-party assets may have separate licenses. No Motrix source or assets were copied.
- Documentation reviewed: `README.md`, including dashboard and remote/headless server descriptions.
- Relevant source reviewed:
  - `src/renderer/routes/dashboard/dashboard-page.tsx` composes dashboard tiles from engine state, speed history, global counters, transfer statistics, and saved layout.
  - `src/renderer/routes/dashboard/tiles/engine-tile.tsx` presents aria2 engine state/version and listen/RPC ports, with a diagnostics action.
  - `src/renderer/routes/dashboard/dashboard-page.test.tsx` covers dashboard tile rendering, engine states, layout customization, and save failure behavior.
  - `src/core/stats/transfer-stats-service.ts`, `src/core/stats/transfer-stats-runtime.ts`, and `src/core/stats/transfer-stats-service.test.ts` implement persisted, time-bucketed transfer totals, lifecycle sampling, engine-disconnect gaps, snapshot bounds, retention, and persistence-failure tests.
  - `src/server/http/operator-auth.ts` and `src/server/http/operator-auth.test.ts` cover the server-shell operator authorization gate, bearer-token validation, origin checks, and failed-authentication handling.
  - `src/server/operator-token.ts` and `src/server/operator-token.test.ts` provision an explicit or random operator token, store generated tokens in a restricted-permission file, and test persistence and invalid-file handling.
  - `src/server/index.ts` wires server-side engine, event, statistics, and HTTP service lifecycles. The server bridge uses MDXP pairing/bootstrap separately from the operator control-plane token.
- Motrix tests were inspected but not run. No source was reused.

## FetchX current-state audit

The Phase 0 architecture identifies Rust CoreServices as the shared business-logic boundary. The previous XDM implementation was already present and was preserved: direct `.mpd` routing to yt-dlp and effective-engine pause/cancel handling remain covered by `docs/research/XDM_FEATURE_ANALYSIS.md`. That record reports the pre-existing browser extension build failure `TS6133` for unused `classify`; this phase does not change that source.

The earlier Ridm report remains accurate: the supplied GitLab project was unavailable, so no Ridm revision, license, source, or implementation behavior was verified. The batch-admission feature is implemented in shared Rust and wired to desktop, with per-input outcomes and collision-safe destination names.

The Seal MP3/M4A selection remains typed and validated, stored by migration `0012_add_ytdlp_audio_format.sql`, and compatible with the supported export/import shapes. Android, desktop, and CLI adapters expose it. The prior Seal record reports Android Gradle tests stopped when the wrapper JVM crashed in native `net.dll` while enumerating interfaces; Kotlin compilation/tests were not verified, and no real audio-conversion run was reported.

FetchX already had:

- Shared runtime status for optional download engines and a desktop `get_engine_status` view with a limited engine set.
- Authenticated remote JSON-RPC, role-based authorization, protocol capability discovery, download queue controls, and a remote dashboard with current download counters, health, event status, and periodic refresh.
- No remote engine-runtime status operation or remote UI view. The browser/native-host RPC allowlist is intentionally restricted and is not an appropriate endpoint for this administrative status.

## Capability comparison

| Capability | Motrix at reviewed revision | FetchX before this phase | Classification |
|---|---|---|---|
| Dashboard summaries and transfer history | Customizable tiles combine engine state, active tasks, global stats, speed history, and persisted transfer-stat snapshots | Remote dashboard summarizes current active/queued/completed counts and downloaded bytes; no historical transfer-stat service | Partially implemented; historical charts are deferred |
| Engine availability/status | Dashboard engine tile exposes engine status/version and connection ports, with a diagnostics action | Shared runtime status exists; desktop settings expose a subset, but remote clients cannot inspect it | Partially implemented; remote projection/view selected |
| Remote authentication | Server operator bearer-token control plane is separate from the MDXP agent pairing/bootstrap path; token provisioning and validation are tested | Authenticated JSON-RPC with role-based permissions already exists | Already implemented adequately for this feature; no new credential or pairing system needed |
| Download list and bulk management | Dashboard task tiles and server APIs provide the Motrix task-management surface | Remote download listing, filtering, queue controls, and lifecycle actions exist | Partially implemented; bulk actions are not expanded here |
| Queue and scheduler controls | Motrix has engine-oriented queue/speed controls and scheduler settings | FetchX remote queue pause/reordering controls exist; a remote scheduler view/control surface is absent | Partially implemented; scheduling is not selected |
| Runtime events and refresh | Server shares event/statistics lifecycle wiring; dashboard hooks consume runtime state | Remote event transport and periodic snapshot refresh exist; engine status can join the existing refresh cycle | Already implemented adequately as a transport pattern |
| NAS/client pairing and credential lifecycle | Motrix has a separate MDXP bridge/pairing and operator control-plane design | FetchX uses its own authenticated remote-server credential and roles | Requires further investigation for any future pairing feature; intentionally not duplicated |

## Selected capability and acceptance criteria

Selected: a read-only, authenticated remote engine-runtime status view. This is a narrow management surface inspired by Motrix's dashboard engine tile. FetchX's local runtime status, remote authorization, queue controls, and current remote dashboard do not already expose engine availability to a remote operator.

Acceptance criteria:

1. CoreServices projects every current `EngineKind` into a stable state (`available`, `disabled`, or `unavailable`) and optional version.
2. The remote method is additive and advertised through protocol capabilities; clients must discover the capability before using it.
3. Authorization is enforced before dispatch. Operator and ReadOnly roles may read status; the endpoint cannot mutate engine or download state.
4. The response excludes local failure reasons, executable paths, configuration, and credentials.
5. The remote UI shows status after connect and refreshes it with the existing refresh loop; older servers remain supported.
6. Tests cover projection, protocol parsing, authorization, response shape, capability discovery, and the absence of sensitive runtime details.

## Implementation and API

- `crates/core-services/src/engine_status.rs` projects all nine engine kinds, treating built-in Direct HTTP/HLS as available and mapping FTP and FTPS to their shared runtime.
- `crates/core-services/src/lib.rs` exposes the projection through `CoreServices::engine_runtime_statuses()`.
- `crates/remote-protocol/src/lib.rs` adds the typed `engine.runtimeStatus` RPC and `engine.runtimeStatus` capability. It is not added to `BROWSER_RPC_METHODS`.
- `crates/remote-server/src/authorization.rs` adds `EngineRead`; Operator and ReadOnly may read status, while Admin retains existing access.
- `crates/remote-server/src/dispatch.rs` performs permission-checked dispatch and serializes only engine, normalized state, and optional version.
- `crates/remote-server/tests/server_integration.rs` exercises unauthenticated denial, ReadOnly access, fixed engine count/states, and response sanitization.
- `apps/remote-ui/src/protocol.ts` adds the RPC types; `apps/remote-ui/src/app.tsx` capability-gates the request and view, refreshes status with the existing polling, and keeps an unavailable display for older servers. `apps/remote-ui/src/styles.css` styles the list.

This is an additive Rust/CoreServices API and remote JSON-RPC method. No existing RPC shape or lifecycle was changed.

## Target integration status

| Target | Status |
|---|---|
| Rust shared core | Implemented as a projection of existing runtime status. |
| Remote server/protocol | Implemented and authenticated; browser/native-host allowlist remains unchanged. |
| Remote UI | Implemented as an Engines view, gated by capability discovery and included in existing periodic refresh. |
| Desktop Tauri dashboard | No change; its existing local engine status remains separate from this remote feature. |
| Android | No change; no UniFFI or Android UI adapter was needed for the remote-only surface. |
| CLI | No change; no CLI status command was requested or added. |
| Browser extension/native host | No change; the new method is excluded from the restricted browser RPC allowlist. |

## Persistence, security, and limitations

No database migration, persistence, export/import change, dependency, token, endpoint secret, or credential-handling change was introduced. Runtime status is read live. The wire response deliberately does not include internal failure reasons or executable/configuration details. The method is subject to existing remote authentication, TLS/transport configuration, and role authorization; it does not create a new authentication mechanism.

The current engine list is explicit and must be updated if `EngineKind` grows. FTP and FTPS share one runtime. Status is polled rather than delivered through a new engine-specific event. Android, desktop Tauri, CLI, browser runtime, and device/emulator execution were not runtime-tested for this remote-only feature.

## Validation

Commands and results:

- `cargo test -p fetchx-remote-protocol -p fetchx-remote-server -p fetchx-core-services -j 1` — passed: CoreServices 29 unit tests, 2 integration tests, 1 doctest (4 live tests ignored); protocol 10 tests; remote server 20 unit and 3 integration tests.
- `cargo clippy --workspace --exclude fetchx-desktop --all-targets --all-features -j 1 -- -D warnings` — passed.
- `cargo fmt --all -- --check` — passed.
- `git diff --check` — passed.
- `npm test --prefix "D:\Projects\FetchX\apps\remote-ui"` — passed, 5 tests.
- `npm run build --prefix "D:\Projects\FetchX\apps\remote-ui"` — passed (TypeScript and Vite production build).
- `npm test --prefix "D:\Projects\FetchX\apps\browser\extension"` — passed, 26 tests. The browser extension build was not rerun; the XDM report records its existing `TS6133` error.

These are compilation/unit/integration checks, not a live multi-device or remote-network test. The Seal Android Gradle JVM limitation above remains unchanged.

## Next research item

Continue the established research order with Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

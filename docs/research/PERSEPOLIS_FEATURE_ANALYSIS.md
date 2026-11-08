# Persepolis Feature Analysis

## Reference review

- Repository: <https://github.com/persepolisdm/persepolis>
- Reviewed revision: `109e95261fa2827bf7bf1b24173328d90235a53e`, dated 2025-08-10.
- The root `LICENSE` is GNU General Public License version 3. Reviewed Python source headers specify GPL version 3 or later. No Persepolis source was copied.
- Documentation reviewed: `README.md`, including the project's queue, scheduling, and segmented-download descriptions.
- Relevant source reviewed:
  - `persepolis/scripts/persepolis_lib_prime.py`: request-based segmented transfer and transfer orchestration.
  - `setting.py`, `mainwindow.py`, and `useful_tools.py`: application settings, UI actions, and supporting behavior.
  - `queue.py`, `queue_prime.py`, `data_base.py`, `startup.py`, and `shutdown.py`: queue/session persistence and application lifecycle.
  - `bubble.py` and the UI settings/queue modules: notifications and presentation.
  - `test/test.py`: startup/smoke coverage; no substantive unit-test suite was identified in the reviewed test path.
- The reviewed Persepolis transfer implementation is its own segmented downloader. The reviewed source did not establish an aria2 JSON-RPC frontend or reusable aria2-management behavior. No code was reused.

## FetchX current-state audit

FetchX's Rust `Aria2Client` already owns JSON-RPC transport, token placement, endpoint validation, timeouts, response bounds, and RPC error mapping. `Aria2Engine` uses that client for download submission and lifecycle operations. `CoreServices` starts a locally supervised aria2 process; it does not persist or select arbitrary remote aria2 endpoints as download engines.

The desktop settings page already offers a transient, read-only remote aria2 preflight using `aria2.getVersion`. It reports version and enabled features, and does not save the supplied endpoint or token. The FetchX authenticated remote JSON-RPC and CLI remote commands manage FetchX, not arbitrary aria2 endpoints. Android's separate remote aria2 code is not backed by a UniFFI operation for this probe. Browser/native-host RPC remains restricted and is not an appropriate arbitrary-endpoint surface.

The earlier Motrix-inspired `engine.runtimeStatus` RPC remains a sanitized status projection for FetchX's own engines; it is not an external aria2 client. The prior Aria2App record also distinguishes its FetchX desktop preflight from persistent external-engine management.

## Capability comparison

| Capability | Persepolis at reviewed revision | FetchX before this phase | Classification |
|---|---|---|---|
| aria2 JSON-RPC management | No aria2 frontend behavior was established in the reviewed transfer path; Persepolis performs its own segmented transfers | Shared `Aria2Client` and `Aria2Engine` already implement the configured engine's RPC lifecycle | Not applicable as a Persepolis-specific transfer feature; FetchX aria2 management otherwise exists |
| External aria2 endpoint verification | Not established by reviewed sources | Desktop transient `getVersion` probe reports daemon version and enabled features | Partially implemented; selected surface |
| Aggregate aria2 activity and rates | Not established as an aria2 RPC feature | Remote endpoint probe did not call `aria2.getGlobalStat` | Missing; selected |
| Persisted profiles, reconnect behavior, or multi-server selection | Persepolis has its own queue/session persistence; the reviewed evidence does not establish aria2 endpoint profiles | FetchX's aria2 configuration supervises a local process; remote endpoint probe is transient | Requires further investigation; not selected |
| Queueing and scheduling | README describes Persepolis queue/scheduling features for its downloader | FetchX has Rust queue and scheduler logic | Already implemented in FetchX's core; no duplicate scheduler added |
| UI versus transfer execution | UI and application lifecycle invoke Persepolis's own transfer implementation | FetchX keeps transfer and aria2 RPC behavior in Rust, with UI/host adapters | Already implemented adequately in the relevant architecture |

## Selected capability: read-only aria2 global-stat snapshot

The desktop remote aria2 preflight now follows `aria2.getVersion` with the read-only `aria2.getGlobalStat` method. This gives a user useful active/waiting/stopped-result counts and current transfer rates before configuring or relying on a daemon, without introducing remote-engine execution or another management endpoint.

Acceptance criteria:

1. The shared Rust client parses aria2's string-encoded counters as non-negative `u64` values and rejects malformed counters.
2. The call uses the existing endpoint and token validation/authentication; the token is not included in the returned status.
3. If global statistics are unsupported or unavailable, the successful version/connectivity check remains successful and explicitly reports that statistics could not be read.
4. The feature is confined to the existing desktop remote-endpoint preflight; it does not expose arbitrary aria2 access through FetchX remote JSON-RPC, CLI, browser, or Android.
5. No endpoint, token, or statistics are persisted.

## Implementation and target boundaries

- `crates/downloader/src/aria2.rs` adds `Aria2Client::get_global_stat`, the typed `Aria2GlobalStat` result, and parsing of the aria2 RPC response.
- `crates/downloader/src/lib.rs` re-exports the result type.
- `apps/desktop/src-tauri/src/engine_commands.rs` adds optional global statistics to the existing diagnostic result and invokes the method after `getVersion`. Failure to read statistics does not misreport a responding daemon as unavailable; RPC details are not returned in the message.
- `apps/desktop/ui/src/api/types.ts` describes the optional response fields, and `apps/desktop/ui/src/components/SettingsPage.tsx` displays activity counts and byte-per-second rates when present.
- No public FetchX remote JSON-RPC method, Rust domain model, Android UniFFI operation, CLI command, or browser/native-host operation was added. The Tauri diagnostic IPC response is an additive optional field.
- There is no storage change, migration, new dependency, or credential persistence. The existing transient aria2 token is used only by the authenticated RPC call; diagnostics do not return it.

## Validation and limitations

- `cargo test -p fetchx-downloader global_stat -j 1`: passed, 3 tests.
- `cargo test -p fetchx-downloader -j 1`: passed, 283 unit tests and 2 native-torrent lifecycle tests; 3 unit tests were ignored.
- `cargo test -p fetchx-desktop diagnostic_result_serializes_stably -j 1`: passed, 1 test.
- `cargo clippy -p fetchx-downloader --all-targets -- -D warnings`: passed.
- `cargo clippy -p fetchx-downloader -p fetchx-desktop --all-targets -- -D warnings`: did not pass because of two Clippy warnings in untouched desktop files: `apps/desktop/src-tauri/src/app.rs:297` (`collapsible_if`) and `apps/desktop/src-tauri/src/commands.rs:1032` (`too_many_arguments`). The focused downloader Clippy check passes; these unrelated warnings were not changed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- `npm run build --prefix "D:\Projects\FetchX\apps\desktop\ui"`: passed TypeScript checking and Vite production build.
- RPC behavior was tested with a local mock server. No live aria2 daemon, external network endpoint, or credentialed service was used. No Android device/emulator, CLI, remote-server, or browser runtime check was required for this desktop-only adapter change; those surfaces were not modified.
- The previously reported XDM browser extension `TS6133` issue and Ridm GitLab source/license access limitation are outside this change and remain as recorded in their respective research documents.

## Deferred work

Persistent remote aria2 profiles, external endpoint download execution, reconnect policy, and subscriptions remain separate design questions. They require explicit endpoint lifecycle, authorization, secret-storage, and SSRF-boundary decisions; this read-only diagnostic does not establish or enable them.

Recommended next research/work item in the established order: revisit Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

# Aria2App Feature Analysis

## Reference review

- The supplied `Aria2App/Aria2App` URL returned 404. The public upstream repository is [devgianlu/Aria2App](https://github.com/devgianlu/Aria2App).
- Reviewed default-branch revision: [`dd6cbc9e7b17559e59d9bc41a3fd8c0468e088d7`](https://github.com/devgianlu/Aria2App/commit/dd6cbc9e7b17559e59d9bc41a3fd8c0468e088d7), release 5.10.0, dated 2025-07-13.
- `LICENSE` at that revision is GNU General Public License v3, dated 29 June 2007. No Aria2App source was copied.
- The app pins `aria2lib` at submodule revision `290755e646f62fb26ed4112f6ea2c0e5d6cbea82`; its `LICENSE` is Apache License 2.0. Its README describes an Android library that downloads/manages aria2 executables, including architecture-specific binaries. No submodule source or binary was copied.
- Documentation reviewed: upstream `README.md`, `app/build.gradle`, and the `aria2lib` README/build configuration.
- Relevant upstream source reviewed:
  - `app/src/main/java/com/gianlu/aria2app/api/AbstractClient.java`: JSON-RPC request IDs, token parameter construction, response validation, async callbacks, and shared client contract.
  - `api/HttpClient.java` and `api/WebSocketClient.java`: HTTP POST and WebSocket transports, request/response matching, timeouts, and connection failure paths.
  - `api/AriaRequests.java` and `api/aria2/`: typed requests/results for version and method discovery, download creation, status/listing, pause, resume, removal, and related aria2 operations.
  - `profiles/MultiProfile.java`, `profiles/ProfilesManager.java`, and `api/NetInstanceHolder.java`: saved connection profiles and switching clients when network conditions change.
  - `services/NotificationService.java` and `aria2lib`'s `Aria2Ui.java`: Android service/notification integration and binding to its in-app aria2 runtime.
- The queried `app/src/test` path does not exist at the reviewed revision; no dedicated app test tree was found there. FetchX behavior is validated with its own focused Rust tests.

Aria2App is an Android remote controller for aria2, including an optional in-app aria2 runtime. Its protocol layer is separated from Android presentation, and it supports both HTTP and WebSocket clients. FetchX does not reuse this implementation; its main application remains a Rust-owned downloader with platform adapters.

## FetchX current-state audit

- Phase 0 architecture and roadmap were reviewed, along with the Gopeed, Ketch, AB Download Manager, and Download Navi research documents.
- The prior Gopeed work exposes runtime projection and persisted duplicate lookup through documented shared-core boundaries; the full Gopeed `resolve_task` contract remains deferred.
- The prior Ketch work keeps segmented HTTP transfer and validator-safe recovery in `DirectHttpEngine`.
- The prior AB work stores manual queue ordering via existing SQLite `queue_order` and exposes the same Rust queue operations through documented host adapters.
- Download Navi's Android service policy returns `START_STICKY` only when the bridge repository is available. The host-side tests check sticky/non-sticky policy selection. No Android device/emulator process-restart test was reported or run for that phase.
- FetchX's authenticated remote JSON-RPC server controls a FetchX `CoreServices` instance; it is not an aria2 remote client. The CLI `remote` commands likewise address that FetchX server. Auth is verified at the FetchX server boundary before dispatch.
- `Aria2Client` in `crates/downloader/src/aria2.rs` implements JSON-RPC request correlation, token authentication, response limits, and error mapping. Remote HTTPS is accepted; HTTP is restricted to IP loopback. URLs with embedded credentials, query strings, or fragments are rejected. Redirects and proxy use are disabled.
- `Aria2Engine` already submits downloads and polls status through `Aria2Client`, and supports lifecycle operations. `CoreServices` currently starts a locally supervised aria2 process when enabled; it does not persist or select a remote aria2 endpoint as the download engine.
- Desktop settings now provide a non-persistent remote endpoint check through the shared Rust client. The probe calls `aria2.getVersion` and reports the version plus `enabledFeatures`; it does not create, pause, resume, or remove a download, or change saved engine configuration.

## Capability comparison

| Capability | Aria2App | FetchX before this phase | Assessment |
|---|---|---|---|
| Connect to an external aria2 RPC endpoint | Saved connection profiles support HTTP and WebSocket transports | Shared client supports JSON-RPC over HTTP(S); production `CoreServices` configures a supervised local endpoint | Partially implemented; remote endpoint is now probeable but not a persisted download-engine target |
| Token authentication and connection configuration | Profiles hold endpoint, connection method, and auth settings | `Aria2Client` sends the aria2 token; local engine secrets use existing config; remote probe credentials are transient | Partially implemented |
| Download create/status/pause/resume/remove | Typed requests invoke aria2 RPC methods | `Aria2Engine` already performs these operations and polls status; standard FetchX queue/lifecycle persists local records | Already implemented for the configured engine; external endpoint selection is deferred |
| Capability/version discovery | Uses aria2 version/method requests and models | `getVersion` already parses version and enabled features, but desktop probe previously surfaced only version | Partially implemented; selected to expose the reported feature set with the probe |
| WebSocket notifications or live subscription | WebSocket RPC transport exists; request handling also discards `aria2.on*` notifications in the reviewed listener | FetchX's aria2 engine polls status; FetchX WebSocket transport is for its own authenticated remote events | Requires further investigation; not selected |
| Reconnection/network-specific profiles | Profiles may be selected as connectivity changes | aria2 supervisor restarts a local process; no persisted remote aria2 profile lifecycle exists | Requires further investigation |
| Engine packaging | `aria2lib` manages an in-app Android aria2 binary | FetchX has its own optional engine/helper and host runtime policies | Partially implemented; not needed for this remote probe |
| FetchX remote control API | Not applicable; Aria2App speaks aria2 RPC | Separate authenticated FetchX JSON-RPC API with role checks | Not appropriate to replace or duplicate |

## Selected capability: safe remote endpoint preflight and capability discovery

Before attempting a broader external-engine configuration, let a desktop user verify an aria2 RPC endpoint without persisting its endpoint or secret. Reuse `Aria2Client` validation and `aria2.getVersion`; surface the daemon version and its `enabledFeatures` as a basic capability summary.

Acceptance criteria:

1. HTTPS remote endpoints and HTTP IP-loopback endpoints are accepted; remote HTTP, embedded URL credentials, queries, and fragments are rejected.
2. The probe uses the optional aria2 token only in the RPC authentication parameter and never returns or logs it.
3. A successful response reports the aria2 version and enabled features.
4. Authentication failures, RPC errors, malformed replies, timeouts, and unavailable servers report an unavailable diagnostic without exposing the secret.
5. The probe does not mutate FetchX configuration or invoke download lifecycle methods.
6. No remote-server JSON-RPC method is added for arbitrary endpoint probing.

This is a low-risk preflight before any future remote-engine execution feature. It reuses existing Rust protocol and security rules rather than adding a second engine or duplicating the FetchX remote server.

## Implementation and target boundaries

- Shared Rust client and validation: `crates/downloader/src/aria2.rs`. Existing `Aria2Version` carries both version and `enabled_features`; regression coverage verifies RPC token placement, secret redaction, endpoint constraints, auth errors, RPC errors, malformed responses, timeout, and transport failure.
- Desktop adapter: `apps/desktop/src-tauri/src/engine_commands.rs` exposes the explicit probe command and maps success/failure to a diagnostic DTO. It does not save endpoint data or credentials.
- Desktop UI: `apps/desktop/ui/src/api/engine.ts`, `api/types.ts`, and `components/SettingsPage.tsx` provide the opt-in endpoint/secret form and display version/features. Secret input is sent as entered and is not included in the returned result.
- Android: the shared Rust client is present in the workspace, but there is no remote aria2 profile UI, UniFFI probe operation, or Android caller for this phase. The Android sticky-service restart policy remains separate; device restart behavior is not claimed.
- Local CLI: existing commands use the locally configured engine or issue commands to a FetchX remote server. No remote aria2 probe command was added.
- FetchX remote server / remote UI: unchanged. No arbitrary endpoint-probe RPC is exposed, avoiding a server-side SSRF/configuration surface and preserving its role as a FetchX control API.
- Browser extension/native host: unchanged; no remote aria2 endpoint or credential operation is added to its restricted API.

No persisted model, public core-service model, migration, schema change, or dependency was added. The desktop diagnostic DTO adds an optional `enabled_features` field; other engine diagnostics omit it.

## Validation

| Command | Result |
|---|---|
| `cargo test -j 1 -p fetchx-downloader remote_rpc -- --nocapture` | Passed: endpoint restrictions and credential-safe authentication failure tests, 2 passed. |
| `cargo test -j 1 -p fetchx-downloader version_calls_correlate_ids_and_put_token_first -- --nocapture` | Passed: verifies version/features parsing, request IDs, token placement, and debug redaction. |
| `cargo test -j 1 -p fetchx-desktop diagnostic_result_serializes_stably -- --nocapture` | Passed: desktop diagnostic DTO serialization includes the optional feature list. |
| `cargo test -j 1 -p fetchx-downloader --lib --tests --no-fail-fast` | Passed: 276 unit tests, 3 ignored; 2 integration tests passed. Includes unavailable transport, RPC, timeout, malformed response, aria2 engine lifecycle, auth, and endpoint validation coverage. |
| `cargo test -j 1 -p fetchx-queue -p fetchx-remote-server --lib --tests --no-fail-fast` | Passed: 42 queue tests, 20 remote-server tests, and 3 server integration tests. |
| `cargo check -j 1 -p fetchx-desktop` | Passed. |
| `cargo check -j 1 --manifest-path crates/cli/Cargo.toml` | Passed. |
| `cargo fmt -p fetchx-downloader -p fetchx-desktop -- --check` and `git diff --check` | Passed. |
| `npm --prefix apps/desktop/ui run build` | Passed: TypeScript check and Vite production build. |
| `.\gradlew.bat :app:testDebugUnitTest --tests 'dev.fetchx.app.service.DownloadForegroundServiceTest' --rerun-tasks` from `apps/android` | Pending final task completion. |

These are host-side tests/builds. A mocked aria2 RPC test does not establish interoperability with a live external aria2 daemon. No Android device/emulator restart test is claimed.

## Limitations and deferred work

- This feature validates connectivity and reports version/compiled features; it does not route downloads to the remote aria2 instance.
- Endpoint settings are transient and are not saved. There is no remote profile switching, WebSocket aria2 transport, or external-engine lifecycle recovery.
- The probe uses HTTP only for IP loopback and HTTPS for remote hosts. It does not support self-signed certificates or custom trust roots.
- Android has no endpoint-probe UI/UniFFI operation; CLI and remote FetchX clients do not expose this command.
- A broader remote aria2 engine configuration needs explicit credential storage, lifecycle/reconnect semantics, endpoint ownership, and cross-target UX decisions.
- Recommended next repository/work item: return to Gopeed's deferred full `resolve_task` contract—protocol-specific metadata and file selection—already listed in `docs/roadmap/SHARED_CORE_ROADMAP.md`.

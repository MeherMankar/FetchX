# Plugin Architecture (Phases 19 and 23A–23C)

## Overview

FetchX has an initial external-plugin process manager.
Plugins run as separate processes, communicate with the host through a
length-prefixed JSON protocol, and are discovered from a well-known directory.
Process separation contains plugin crashes; it is not an OS security sandbox.

For operations routed through FetchX APIs, the host remains authoritative for:
- download queueing and scheduling
- engine dispatch
- persistence
- state machine transitions
- resource limits
- security

Plugin processes are not sandboxed and can access OS resources using the host
user's permissions. The capability policy only governs operations routed through
host adapters; it does not restrict direct OS access by a trusted executable.

---

## Architecture

```
FetchX host
  └─── PluginManager
        ┌── discover()   — scans {data_dir}/plugins/ for plugin.json
        ┌── register()   — validates manifest, adds to registry
        ┌── start()      — spawns process, performs handshake
        ┌── stop()       — sends Shutdown, waits, kills if needed
        └─── list/get()   — read-only registry views

Plugin process (external binary)
  ┌── stdin  ← HostRequest  (4-byte LE length + JSON)
  └─── stdout → PluginResponse (4-byte LE length + JSON)
```

---

## Sandbox hardening (Phase 23B)

The Phase 23A trust policy remains the mandatory gate for all plugin operations.
The Phase 23B runtime layer adds supervisor-side guardrails around execution when
an actual plugin process exists, without claiming OS-level isolation that the
host does not provide.

Current enforced host controls:

- process separation: the plugin is launched as a child process, never in the
  host process;
- bounded IPC: plugin payloads are rejected above the configured payload limit;
- startup timeout: the manager waits for a bounded handshake window;
- shutdown timeout: the manager waits for a graceful exit, then terminates the
  process if needed;
- crash-loop guard: repeated failures within a short window disable the plugin
  until the host explicitly restores it.

Current limitations:

- no OS sandbox is assumed or claimed;
- Android remains deny-closed unless a real privileged plugin runtime is added;
- filesystem, network, CPU, and memory caps are only valid when the host runtime
  can actually enforce them, otherwise they are reported as not available.

## Plugin directory

Plugins are installed as subdirectories of the application data directory:

```
{data_dir}/plugins/
  my-plugin/
    plugin.json        ← required manifest
    my-plugin.exe      ← executable (absolute path in manifest)
```

| Platform | `{data_dir}` |
|----------|-------------|
| Windows  | `%APPDATA%\com.fetchx.desktop\` |
| Linux    | `$XDG_DATA_HOME/fetchx/` |
| macOS    | `~/Library/Application Support/com.fetchx.desktop/` |

---

## Plugin manifest (`plugin.json`)

Required fields:

```json
{
  "manifest_version": 1,
  "id":           "my-plugin",
  "name":         "My Plugin",
  "version":      "0.1.0",
  "api_version":  1,
  "compatibility": {
    "minimum_api_version": 1,
    "maximum_api_version": 1
  },
  "description":  "A short description (max 256 chars)",
  "publisher":    "Publisher Name",
  "executable":   "/absolute/path/to/my-plugin",
  "permissions":  ["downloads"],
  "capabilities": ["download_create"],
  "platforms":    ["windows", "linux", "macos", "android"],
  "signature":    null
}
```

### Manifest validation rules

| Field | Rule |
|-------|------|
| `id` | Matches `^[a-z0-9_-]{2,64}$` |
| `manifest_version` | Must equal current manifest schema version (1) |
| `name`, `publisher` | Non-empty |
| `version` | Valid SemVer |
| `api_version` | Must equal current plugin API version (1) |
| `compatibility` | Inclusive host API range containing API version 1 |
| `description` | At most 256 characters |
| `executable` | Absolute path, no `..` components |
| `permissions` | Known coarse consent categories, no duplicates |
| `capabilities` | At least one known operation; each requires its matching permission |
| `platforms` | Optional; empty means all supported host platforms |
| `signature` | Optional versioned Ed25519 envelope; required for manager startup |

Maximum manifest file size: **16 KiB**.

Unknown fields cause a deserialisation error (`deny_unknown_fields`).

---

## Permissions and capabilities (Phase 23A)

Permissions are broad consent categories: `network`, `filesystem_read`,
`filesystem_write`, `downloads`, `metadata`, `process`, `engine`, and `settings`.
Declaring or approving a permission alone does not grant an operation.

Capabilities are narrower operations, including HTTP requests, filesystem
read/write, individual download actions, metadata reads, process spawn, specific
engine integrations, and settings read/write. An operation is granted only when
the manifest declares its matching permission and capability, the host explicitly
approves both, the plugin is trusted, the manifest fingerprint matches the approval,
the platform/API is compatible, and the host runtime supports the capability.
Unknown values, missing trust, revocation, invalid signature status, stale approval,
and unsupported operations fail closed.

| Permission | Capabilities requiring it |
|-----------|-----------------------------|
| `network` | `network_http_request` |
| `filesystem_read` | `filesystem_read` |
| `filesystem_write` | `filesystem_write` |
| `downloads` | `download_create`, `download_list`, `download_pause`, `download_resume`, `download_cancel`, `download_remove` |
| `metadata` | `metadata_read` |
| `process` | `process_spawn` |
| `engine` | `engine_direct_http`, `engine_aria2`, `engine_yt_dlp`, `engine_ffmpeg` |
| `settings` | `settings_read`, `settings_write` |

Newly discovered plugins are `unknown` and have no approvals. `PluginManager`
approvals are currently in-memory and reset when the manager exits; there is no
user-facing approval workflow or persistent trust store yet. The plugin handshake
checks that reported identity, version, and capabilities match the approved
manifest.

Trust states are `unknown`, `trusted`, `untrusted`, and `revoked`. Only `trusted`
can be evaluated for grants, and both a permission and each individual capability
must be approved. A signature marked invalid is denied. A publisher signature is
necessary for manager startup, but it does not establish publisher trust or grant
permissions by itself.

Android exposes the same Rust policy and signing types through the UniFFI bridge.
Android does not currently have plugin execution or privileged-operation
adapters, so its supported capability set is empty and requested capabilities
are denied. Shared signing tests run on the Rust host test target; Android plugin
execution and live signing integration are not available.

## Plugin signing (Phase 23C)

The shared plugin API verifies Ed25519 signatures using the `ed25519-dalek`
implementation. Signature metadata is versioned and contains the stable
`publisher_id`, a `key_id`, the installed-artifact SHA-256, and the standard
Base64 encoding of the 64-byte signature. `key_id` is the lowercase hexadecimal
SHA-256 fingerprint of the 32-byte public key. A host-owned publisher store maps
publisher IDs to explicitly enrolled key fingerprints and trust state; an
unknown key or publisher is never trusted automatically.

The signature covers a domain-separated canonical JSON representation:

1. Prefix `FetchX plugin signature v1` and a NUL byte.
2. Serialize the fixed-field-order v1 manifest payload as UTF-8 JSON.
3. Include the signature version/algorithm, publisher ID, key fingerprint,
   artifact hash, plugin ID/name/version, API compatibility, display publisher,
   description, executable path, permissions, capabilities, and platforms.
4. Include the manifest schema version. Sort permission, capability, and
   platform arrays by their serialized values. Normalize executable path
   separators to `/`. Exclude only the signature bytes, which cannot sign
   themselves.

Manifest parsing rejects unknown fields, duplicate set-like values, malformed
signature metadata, and unsupported signature versions. Changing the version,
publisher association, executable path, permissions, or capabilities invalidates
the signature. The approval fingerprint remains separately checked by the
Phase 23A policy.

FetchX does not currently have a plugin archive/package format. Its installed
artifact is a directory containing `plugin.json`, the executable, and optional
resources. The manager computes a deterministic tree hash over every regular file
except the root `plugin.json`: sorted UTF-8 relative paths with `/` separators,
each path and file content prefixed by an unsigned 64-bit little-endian length,
under the `FetchX plugin artifact tree v1` domain prefix. Symbolic links and
non-regular entries are rejected. The parsed manifest fields and artifact hash
are both covered by the signature. This protects the current installed directory
model; it is not a signature for a future archive/distribution format.

Publisher trust and plugin revocations are enforced by the local
`PluginPublisherStore`. The manager verifies the installed artifact and signature
before entering `Starting`, and rechecks the Phase 23A policy before authorizing
each privileged operation. A valid signature still requires a trusted publisher,
no publisher/plugin revocation, exact manifest approval, explicit permission
and capability approvals, and runtime support. New keys can be added under an
existing publisher for an explicitly administered rotation; adding a key never
changes trust. There is no authenticated key-rotation chain, persistent trust
store, or remote revocation service yet. Store decisions currently last for the
manager process lifetime.

Validation evidence: the plugin API and manager Rust test suites passed on
Windows, including valid/invalid signatures, wrong/unknown/untrusted/revoked
publishers, plugin revocation, manifest and version tampering, artifact/resource
tampering, explicit key rotation enrollment, and manager startup gating. No
Windows plugin could be positively started because the host currently supports
no plugin capabilities. Android plugin execution is not available; its shared
policy remains deny-closed.

### Phase 23C security matrix

| Control | Status | Evidence / scope |
|---|---|---|
| Ed25519 verification | TESTED | Real `ed25519-dalek` verification, valid and invalid signatures |
| Canonical signed payload | TESTED | Versioned domain prefix, fixed JSON fields, sorted set-like values |
| Installed artifact tree hash | TESTED | Executable/resources hashed; root `plugin.json` fields are signed separately |
| Distributable package/archive hash | NOT AVAILABLE | No plugin archive format exists |
| Publisher identity and key fingerprint | TESTED | Signed publisher ID bound to enrolled SHA-256 public-key fingerprint |
| Trusted publisher store | TESTED | Unknown and untrusted publishers denied; no trust-on-first-use |
| Unknown/untrusted publisher rejection | TESTED | API signing tests |
| Publisher/plugin revocation | TESTED | Local in-memory store; no remote or persistent revocation |
| Version/manifest/permission/capability binding | TESTED | Tampering tests fail verification |
| Phase 23A approval/fingerprint integration | TESTED | Verified signatures still require explicit approvals and matching fingerprint |
| Phase 23B startup integration | TESTED | Manager verifies artifact/signature before `Starting`; tamper test denies |
| Windows signed plugin live startup | NOT AVAILABLE | Runtime supports no plugin capabilities; positive startup cannot pass policy |
| Windows live tamper/startup validation | NOT VALIDATED | Rust manager tests only; no live plugin process validation |
| Android shared signing implementation | IMPLEMENTED | Shared Rust API; Rust tests ran on Windows host |
| Android plugin execution/live validation | NOT AVAILABLE | No Android plugin runtime or privileged adapter |

## Plugin registry (Phase 23D)

The shared plugin API provides a strict registry-catalog model, a provider
interface, and a local in-memory provider. Catalog parsing rejects unknown
fields, oversized responses, duplicate plugin/release/target data, invalid
identifiers, inconsistent permission/capability declarations, malformed
compatibility metadata, unsafe filenames, and download URLs with non-HTTPS
schemes, user info, fragments, or traversal-like paths.
Provider errors are returned to the caller; an unavailable provider is not
replaced with an empty successful catalog.

Registry data is untrusted metadata. It cannot deserialize a computed security
assessment or grant publisher trust, plugin approval, permissions, or runtime
capabilities. Assessment compares the catalog with a validated signed manifest,
the caller-supplied artifact SHA-256, the host publisher store, and the current
compatibility environment. A signature can be cryptographically valid while
its publisher is untrusted; that state is reported separately and is not
admitted. `artifact_admission_allowed` means only that the signed artifact
metadata passes the registry admission checks. It is not a plugin installation
API and does not authorize runtime operations. Any available Phase 23A policy
decision is computed separately and still requires explicit approvals and
runtime capability support.

The artifact assessment compares the caller-supplied SHA-256 with the catalog
hash and signed manifest hash. The current API does not fetch downloads or
itself measure artifact bytes/size. There is no remote provider, marketplace
UI, download/install flow, persistent catalog, or live registry integration.
The local provider and shared assessment tests ran as part of
`cargo test -p fetchx-plugin-api`; this is host-side test evidence only.

---

## IPC protocol

### Framing

```
┌──────────────────────┬─────────────────────────────────────┐
│  4 bytes (LE u32)    │  N bytes UTF-8 JSON                  │
│  payload length = N  │  { "version": 1, "type": "...", ... }│
└───────────────────────┴─────────────────────────────────────┘
```

Maximum payload: **64 KiB**.  Frames larger than this are rejected before
any allocation occurs.

### Current protocol version: 1

### Host → Plugin request types

| `type` | Description |
|--------|-------------|
| `hello` | Initial handshake; plugin must respond with `hello_ack` |
| `ping` | Health check; plugin must respond with `pong` |
| `shutdown` | Ordered shutdown; plugin must respond with `shutdown_ack` then exit |
| `status` | Request status information |

### Plugin → Host response types

| `type` | Description |
|--------|-------------|
| `hello_ack` | Handshake acknowledgement (includes plugin ID, version, capabilities) |
| `pong` | Health check response |
| `shutdown_ack` | Shutdown acknowledged; plugin will exit |
| `status_info` | Status information response |
| `error` | Plugin-side error (code + message; no stack traces or secrets) |

---

## Lifecycle

```
Plugin registered
      ↓
Plugin starting (process spawned)
      ↓  Hello sent
      ↓  HelloAck received (within 10s timeout)
Plugin running
      ↓
Plugin stopping (Shutdown sent)
      ↓  ShutdownAck received (within 5s timeout) or process killed
Plugin stopped
```

If any step fails the plugin enters the `failed` state with a reason string.
A failed plugin does not crash the host.

---

## Security model

1. **Out-of-process, not sandboxed.** A crashing plugin cannot take down the host,
   but its process otherwise runs with the host user's OS permissions.

2. **Environment isolation.** The manager clears inherited environment variables
   before spawn; on Windows it restores only required system/temp variables.
   This does not restrict filesystem, network, or process access.

3. **Structured process invocation.** The plugin executable is launched with
   `tokio::process::Command::new(executable)` — no shell string construction.

4. **Bounded communication.** All IPC messages are bounded at 64 KiB.
   Oversized messages are rejected before allocation.

5. **Absolute executable paths.** Plugin manifests must provide absolute
   paths with no `..` traversal components.

6. **Fail-closed approvals.** Process start requires explicit trusted status, approval for every requested
   permission and capability, and a supported runtime adapter. No runtime capabilities
   are currently registered, so the manager and Android bridge deny every requested
   operation. Each future host-side privileged operation must also call the shared
   evaluator immediately before dispatch. The current IPC protocol has no
   plugin-originated privileged requests.

7. **Strict schemas.** Manifest JSON, trust context, and protocol messages use
   strict deserialization where supported.

8. **No unsafe code.** Both `plugin-api` and `plugin-manager` declare
   `#![forbid(unsafe_code)]`.

9. **No in-process dynamic loading.** Plugins are NOT Rust dylibs loaded at
   runtime. They are external processes only.

10. **Signing is not publisher authorization.** Only a signature verified
    against a key explicitly enrolled in a trusted publisher record can pass
    the signing gate. Signature presence and publisher-supplied names never
    establish trust.

---

## Resource limits

| Resource | Limit |
|---------|-------|
| Handshake timeout | 10 seconds |
| Shutdown timeout | 5 seconds |
| Maximum manifest size | 16 KiB |
| Maximum IPC message | 64 KiB |
| Plugin process stderr | discarded (not forwarded to host logs) |

---

## CLI

```
FetchX plugins [list]
FetchX plugins info <id>
FetchX plugins status <id>
```

All commands support `--json`.  Output never includes executable paths,
secrets, or internal process details.

---

## Current limitations and deferred phases

- In-process plugin loading (dylib / WASM)
- Plugin marketplace or download
- User-facing and persistent permission approval/trust management
- Android plugin execution
- Capability-specific engine dispatch (built-in engines remain first-class)
- Plugin hot-reload or restart-on-crash
- OS-level plugin sandboxing (filesystem/network/process isolation), deferred to Phase 23B
- Persistent publisher trust, authenticated key rotation, and remote revocation
  distribution
- A signed plugin archive/distribution format and its install-time verification
- Plugin registry/marketplace, deferred to Phase 23D
- DRM, CAPTCHA, or access-control bypass

These are deferred to future phases.

---

## Future extensibility

The versioned protocol (`api_version` in the manifest, `version` in every
message) ensures breaking changes can be introduced without silently
breaking existing plugins.  When the protocol needs a new message type,
increment `PROTOCOL_VERSION` and update both `plugin-api` and the host.

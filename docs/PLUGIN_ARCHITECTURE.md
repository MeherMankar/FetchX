# Plugin Architecture (Phases 19 and 23A)

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
| `signature` | Optional structurally valid Ed25519 metadata; not cryptographically checked |

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
must be approved. A signature marked invalid is denied. Unsigned or structurally
present-but-unverified signatures never establish trust or grant permissions.

Android exposes the same Rust evaluator through the UniFFI bridge. Android does
not currently have plugin execution or privileged-operation adapters, so its
supported capability set is empty and requested capabilities are denied. This is
policy evaluation parity, not an Android plugin runtime.

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

10. **Signatures are not verified.** Signature metadata is validated for shape
    only. No `verified` state is representable in Phase 23A; signatures do not
    establish trust. Cryptographic verification and publisher revocation belong
    to Phase 23C.

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
- Cryptographic signature verification and revocation, deferred to Phase 23C
- Plugin registry/marketplace, deferred to Phase 23D
- DRM, CAPTCHA, or access-control bypass

These are deferred to future phases.

---

## Future extensibility

The versioned protocol (`api_version` in the manifest, `version` in every
message) ensures breaking changes can be introduced without silently
breaking existing plugins.  When the protocol needs a new message type,
increment `PROTOCOL_VERSION` and update both `plugin-api` and the host.

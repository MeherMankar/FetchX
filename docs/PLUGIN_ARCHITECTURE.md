# Plugin Architecture (Phase 19)

## Overview

MyDownloader supports external plugins via a secure out-of-process model.
Plugins run as separate processes, communicate with the host through a
length-prefixed JSON protocol, and are discovered from a well-known directory.

The host (MyDownloader) remains authoritative for:
- download queueing and scheduling
- engine dispatch
- persistence
- state machine transitions
- resource limits
- security

Plugins are integration/extension points only.  They cannot bypass the core.

---

## Architecture

```
MyDownloader host
  └── PluginManager
        ├── discover()   — scans {data_dir}/plugins/ for plugin.json
        ├── register()   — validates manifest, adds to registry
        ├── start()      — spawns process, performs handshake
        ├── stop()       — sends Shutdown, waits, kills if needed
        └── list/get()   — read-only registry views

Plugin process (external binary)
  ├── stdin  ← HostRequest  (4-byte LE length + JSON)
  └── stdout → PluginResponse (4-byte LE length + JSON)
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
| Windows  | `%APPDATA%\com.mydownloader.desktop\` |
| Linux    | `$XDG_DATA_HOME/mydownloader/` |
| macOS    | `~/Library/Application Support/com.mydownloader.desktop/` |

---

## Plugin manifest (`plugin.json`)

Required fields:

```json
{
  "id":           "my-plugin",
  "name":         "My Plugin",
  "version":      "0.1.0",
  "api_version":  1,
  "description":  "A short description (≤ 256 chars)",
  "author":       "Author Name",
  "executable":   "/absolute/path/to/my-plugin",
  "capabilities": ["engine"],
  "platforms":    ["windows", "linux", "macos"]
}
```

### Manifest validation rules

| Field | Rule |
|-------|------|
| `id` | Matches `^[a-z0-9_-]{2,64}$` |
| `name` | Non-empty |
| `version` | Non-empty |
| `api_version` | Must equal current host API version (1) |
| `description` | ≤ 256 characters |
| `author` | Non-empty |
| `executable` | Absolute path, no `..` components |
| `capabilities` | At least one; values from the known capability set |

Maximum manifest file size: **16 KiB**.

Unknown fields cause a deserialisation error (`deny_unknown_fields`).

---

## Capabilities

| Capability | Description |
|-----------|-------------|
| `engine` | Plugin provides a download engine the queue can dispatch to |
| `post_processor` | Plugin post-processes completed downloads |
| `url_resolver` | Plugin resolves URLs into direct download links |
| `metadata` | Plugin provides metadata for a URL or file |

A plugin may only exercise capabilities it declared and the host accepted.
Unknown capability values cause the manifest to be rejected.

---

## IPC protocol

### Framing

```
┌──────────────────────┬──────────────────────────────────────┐
│  4 bytes (LE u32)    │  N bytes UTF-8 JSON                  │
│  payload length = N  │  { "version": 1, "type": "...", ... }│
└──────────────────────┴──────────────────────────────────────┘
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

1. **Out-of-process.** Plugins run as separate OS processes with no shared
   memory. A crashing plugin cannot take down the host.

2. **No secrets forwarded.** Plugin processes receive no credentials, API
   keys, aria2 RPC tokens, browser cookies, or session data by default.

3. **Structured process invocation.** The plugin executable is launched with
   `tokio::process::Command::new(executable)` — no shell string construction.

4. **Bounded communication.** All IPC messages are bounded at 64 KiB.
   Oversized messages are rejected before allocation.

5. **Absolute executable paths.** Plugin manifests must provide absolute
   paths with no `..` traversal components.

6. **`deny_unknown_fields`.** Manifest JSON and protocol messages use strict
   deserialisation where supported.

7. **No unsafe code.** Both `plugin-api` and `plugin-manager` declare
   `#![forbid(unsafe_code)]`.

8. **No in-process dynamic loading.** Plugins are NOT Rust dylibs loaded at
   runtime. They are external processes only.

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
mydownloader plugins [list]
mydownloader plugins info <id>
mydownloader plugins status <id>
```

All commands support `--json`.  Output never includes executable paths,
secrets, or internal process details.

---

## What Phase 19 does NOT implement

- In-process plugin loading (dylib / WASM)
- Plugin marketplace or download
- Plugin authentication/signing enforcement
- Persistent plugin enable/disable state (SQLite migration)
- Capability-specific engine dispatch (built-in engines remain first-class)
- Plugin hot-reload or restart-on-crash
- Plugin sandboxing (filesystem/network isolation)
- DRM, CAPTCHA, or access-control bypass

These are deferred to future phases.

---

## Future extensibility

The versioned protocol (`api_version` in the manifest, `version` in every
message) ensures breaking changes can be introduced without silently
breaking existing plugins.  When the protocol needs a new message type,
increment `PROTOCOL_VERSION` and update both `plugin-api` and the host.

# FetchX CLI

The `FetchX` command-line interface provides full access to the same
download engine, queue, scheduler, and engine configuration as the desktop
application.  It shares the exact same SQLite database and downloaded-file
directory — running the CLI while the desktop is open is supported (both use
the same persistent queue).

## Building

```sh
cargo build --release -p FetchX
```

The binary is at `target/release/FetchX` (or `FetchX.exe` on Windows).

## Usage

```
FetchX <COMMAND> [OPTIONS]
```

Global option:

| Flag | Description |
|------|-------------|
| `--json` | Output machine-readable JSON to stdout; errors go to stderr |

---

## Commands

### `version`
```
FetchX version [--json]
```
Prints the application version.

---

### `engines`
```
FetchX engines [--json]
```
Shows runtime availability of all download engines.  Never exposes RPC
secrets or authentication tokens.

Example output:
```
ENGINE       AVAILABLE  NOTE
────────────  ──────────  ──────────────
direct-http  yes        built-in
aria2        yes        1.37.0
yt-dlp       no         disabled in config
```

---

### `config`
```
FetchX config [--json]
```
Shows non-sensitive configuration.  The aria2 RPC secret is shown only as
`[configured]` or `[not set]` — never as its actual value.

---

### `add`
```
FetchX add <URL> [OPTIONS]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-e`, `--engine <ENGINE>` | `direct-http` | Engine: `direct-http`, `aria2`, `yt-dlp` |
| `-p`, `--priority <PRIORITY>` | `normal` | `low`, `normal`, `high` |
| `--schedule <DATETIME>` | — | Start time: `"YYYY-MM-DD HH:MM"` |

Examples:
```sh
FetchX add https://example.com/file.zip
FetchX add https://example.com/video --engine yt-dlp --priority high
FetchX add https://example.com/large.iso --schedule "2026-11-01 02:00"
FetchX add https://example.com/file.zip --json
```

The backend validates the URL and engine availability.  An explicitly
selected unavailable engine is **rejected** — it is never silently
downgraded to another engine.

---

### `list`
```
FetchX list [--filter <STATUS>] [--limit <N>] [--json]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-f`, `--filter` | `all` | `all`, `active`, `queued`, `paused`, `completed`, `failed`, `cancelled` |
| `-n`, `--limit` | — | Limit results |

Example output:
```
ID        STATUS       PROGRESS   SPEED      ETA    FILE
────────  ────────────  ──────────  ──────────  ──────  ──────────────────────────
abc12345  Downloading  42.5%      8.4 MB/s   00:21  movie.mp4
def67890  Paused       73.1%      —          —      archive.zip
```

---

### `status`
```
FetchX status <ID> [--json]
```
Detailed status of one download including URL, destination, engine, schedule, and error.

---

### `pause`
```
FetchX pause <ID> [--json]
```

---

### `resume`
```
FetchX resume <ID> [--json]
```

---

### `cancel`
```
FetchX cancel <ID> [--json]
```

---

### `remove`
```
FetchX remove <ID> [--json]
```
Removes the download record.  **Downloaded and partial files are preserved.**

---

### `retry`
```
FetchX retry <ID> [--json]
```
Retries a failed or cancelled download.  Creates a new download record with
the original URL and **preserves the original engine assignment**.  If the
original engine is no longer available, the retry is rejected.

---

### `priority`
```
FetchX priority <ID> <PRIORITY> [--json]
```
Valid priorities: `low`, `normal`, `high`.

---

### `schedule`
```
FetchX schedule <ID> --at "YYYY-MM-DD HH:MM" [--json]
FetchX schedule <ID> --clear [--json]
```
Sets or clears a one-shot start time.  Past timestamps are rejected.

---

### `monitor`
```
FetchX monitor
```
Live-updating terminal view of active downloads.  Uses the existing event
bus — no aggressive polling.  Press `Ctrl+C` to exit cleanly.

---

### `remote`

The remote commands control another FetchX server using the shared
`fetchx.rpc.v1` protocol. They do not start local CoreServices or access the
remote server's database directly.

Set the endpoint and bearer token in the environment. The token is not accepted
as a command-line argument, saved to disk, or printed by remote commands:

```powershell
$env:FETCHX_REMOTE_URL = "https://fetchx.example.com"
$env:FETCHX_REMOTE_TOKEN = "<token from fetchx server init-auth>"
fetchx remote connect
fetchx remote status
fetchx remote downloads
fetchx remote add https://www.iana.org/robots.txt
fetchx remote pause <ID>
fetchx remote resume <ID>
fetchx remote cancel <ID>
```

`remote connect <URL>` or `--server <URL>` can override the endpoint for one
invocation. HTTP is allowed only for loopback endpoints; other servers require
HTTPS. Redirects are not followed, and responses are size-bounded.

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Download not found |
| 4 | Invalid state transition |
| 5 | Engine unavailable |
| 6 | Configuration error |

---

## JSON output

All commands support `--json`.  Stdout contains only the JSON response.
Errors always go to stderr.

Success envelope:
```json
{ "ok": true, ... }
```

Error envelope (on stderr):
```json
{ "ok": false, "error": { "code": "error_code", "message": "..." } }
```

---

## Data directory

The CLI uses the **same database and downloads directory** as the desktop app:

| Platform | Path |
|----------|------|
| Windows  | `%APPDATA%\com.fetchx.desktop\fetchx.db` |
| Linux    | `$XDG_DATA_HOME/fetchx/fetchx.db` |
| macOS    | `~/Library/Application Support/com.fetchx.desktop/fetchx.db` |

---

## Security

- No shell execution.  All process arguments are structured.
- Secrets (aria2 RPC token) are never printed.
- `--json` output never includes credentials, tokens, or session data.
- URLs are validated by the same backend logic as the desktop UI.
- Unsupported URL schemes are rejected before any network activity.

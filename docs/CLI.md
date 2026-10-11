# MyDownloader CLI

The `mydownloader` command-line interface provides full access to the same
download engine, queue, scheduler, and engine configuration as the desktop
application.  It shares the exact same SQLite database and downloaded-file
directory — running the CLI while the desktop is open is supported (both use
the same persistent queue).

## Building

```sh
cargo build --release -p mydownloader
```

The binary is at `target/release/mydownloader` (or `mydownloader.exe` on Windows).

## Usage

```
mydownloader <COMMAND> [OPTIONS]
```

Global option:

| Flag | Description |
|------|-------------|
| `--json` | Output machine-readable JSON to stdout; errors go to stderr |

---

## Commands

### `version`
```
mydownloader version [--json]
```
Prints the application version.

---

### `engines`
```
mydownloader engines [--json]
```
Shows runtime availability of all download engines.  Never exposes RPC
secrets or authentication tokens.

Example output:
```
ENGINE       AVAILABLE  NOTE
───────────  ─────────  ──────────────
direct-http  yes        built-in
aria2        yes        1.37.0
yt-dlp       no         disabled in config
```

---

### `config`
```
mydownloader config [--json]
```
Shows non-sensitive configuration.  The aria2 RPC secret is shown only as
`[configured]` or `[not set]` — never as its actual value.

---

### `add`
```
mydownloader add <URL> [OPTIONS]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-e`, `--engine <ENGINE>` | `direct-http` | Engine: `direct-http`, `aria2`, `yt-dlp` |
| `-p`, `--priority <PRIORITY>` | `normal` | `low`, `normal`, `high` |
| `--schedule <DATETIME>` | — | Start time: `"YYYY-MM-DD HH:MM"` |

Examples:
```sh
mydownloader add https://example.com/file.zip
mydownloader add https://example.com/video --engine yt-dlp --priority high
mydownloader add https://example.com/large.iso --schedule "2026-11-01 02:00"
mydownloader add https://example.com/file.zip --json
```

The backend validates the URL and engine availability.  An explicitly
selected unavailable engine is **rejected** — it is never silently
downgraded to another engine.

---

### `list`
```
mydownloader list [--filter <STATUS>] [--limit <N>] [--json]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-f`, `--filter` | `all` | `all`, `active`, `queued`, `paused`, `completed`, `failed`, `cancelled` |
| `-n`, `--limit` | — | Limit results |

Example output:
```
ID        STATUS       PROGRESS   SPEED      ETA    FILE
────────  ───────────  ─────────  ─────────  ─────  ──────────────────────────
abc12345  Downloading  42.5%      8.4 MB/s   00:21  movie.mp4
def67890  Paused       73.1%      —          —      archive.zip
```

---

### `status`
```
mydownloader status <ID> [--json]
```
Detailed status of one download including URL, destination, engine, schedule, and error.

---

### `pause`
```
mydownloader pause <ID> [--json]
```

---

### `resume`
```
mydownloader resume <ID> [--json]
```

---

### `cancel`
```
mydownloader cancel <ID> [--json]
```

---

### `remove`
```
mydownloader remove <ID> [--json]
```
Removes the download record.  **Downloaded and partial files are preserved.**

---

### `retry`
```
mydownloader retry <ID> [--json]
```
Retries a failed or cancelled download.  Creates a new download record with
the original URL and **preserves the original engine assignment**.  If the
original engine is no longer available, the retry is rejected.

---

### `priority`
```
mydownloader priority <ID> <PRIORITY> [--json]
```
Valid priorities: `low`, `normal`, `high`.

---

### `schedule`
```
mydownloader schedule <ID> --at "YYYY-MM-DD HH:MM" [--json]
mydownloader schedule <ID> --clear [--json]
```
Sets or clears a one-shot start time.  Past timestamps are rejected.

---

### `monitor`
```
mydownloader monitor
```
Live-updating terminal view of active downloads.  Uses the existing event
bus — no aggressive polling.  Press `Ctrl+C` to exit cleanly.

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
| Windows  | `%APPDATA%\com.mydownloader.desktop\mydownloader.db` |
| Linux    | `$XDG_DATA_HOME/mydownloader/mydownloader.db` |
| macOS    | `~/Library/Application Support/com.mydownloader.desktop/mydownloader.db` |

---

## Security

- No shell execution.  All process arguments are structured.
- Secrets (aria2 RPC token) are never printed.
- `--json` output never includes credentials, tokens, or session data.
- URLs are validated by the same backend logic as the desktop UI.
- Unsupported URL schemes are rejected before any network activity.

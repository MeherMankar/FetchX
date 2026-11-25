# Backend Feature Implementation Report

## Summary

All three features were already implemented in the partial work committed to the `master` branch. This step performed a build fix for a newly added `EngineKind::Ed2k` variant that was not covered in two exhaustive match expressions in `commands.rs`, then verified all three checks pass.

---

## Feature 1 — Webhooks

**Status: Already implemented.**

### Backend (`app.rs` + `commands.rs`)
- `forward_event` fires an HTTP POST webhook on `Completed` or `Failed` status when `config.webhook.enabled`, the URL is non-empty, and the respective `on_complete`/`on_fail` flag is set.
- Body: `{ "event": "completed"|"failed", "download": { "id", "filename", "url", "destination", "size_bytes" }, "timestamp": unix_ms }`.
- HMAC-SHA256 signature computed with a pure-Rust implementation; added as `X-FetchX-Signature: sha256=<hex>` when `config.webhook.secret` is non-empty.
- Spawned in a non-blocking `tauri::async_runtime::spawn` task; failures logged with `tracing::warn!`, never fatal.
- `get_webhook_config` and `set_webhook_config` Tauri commands implemented in `commands.rs` and registered in the `invoke_handler` in `app.rs`.
- `reqwest` was already present in `crates/core-services/Cargo.toml` and `apps/desktop/src-tauri/Cargo.toml`.

### UI (`SettingsPage.tsx`)
- `WebhookSection` component fully implemented with enabled toggle, URL input, on_complete/on_fail checkboxes, and optional secret field.
- Rendered under the `notifications` settings category.

---

## Feature 2 — Post-download environment variables

**Status: Already implemented.**

### Backend (`app.rs`)
The `RunCommand` branch of the post-download action handler sets all required environment variables:
- `FETCHX_FILE` — absolute path to the downloaded file
- `FETCHX_URL` — source URL
- `FETCHX_STATUS` — `"completed"` (always, in this branch)
- `FETCHX_SIZE` — file size in bytes as a decimal string
- `FETCHX_FILENAME` — filename without directory path
- `FETCHX_ID` — download ID (UUID string)
- `FETCHX_ENGINE` — engine kind string (lowercased, spaces replaced with underscores)

Both `#[cfg(windows)]` (cmd /C) and `#[cfg(not(windows))]` (sh -c) paths set all variables.

---

## Feature 3 — Archive auto-extract

**Status: Already implemented.**

### Backend (`app.rs`)
- `extract_archive` async function dispatches to `extract_archive_blocking` via `tokio::task::spawn_blocking`.
- ZIP extraction uses `zip::ZipArchive` to extract to `{stem}/` directory next to the archive.
- tar.gz/tgz extraction uses `flate2::read::GzDecoder` + `tar::Archive::unpack`.
- Also handles `.tar.bz2` and `.tar.xz` by filename suffix detection.
- When `delete_after` is true, removes the archive after successful extraction with a non-fatal warning on failure.
- `zip = "2"`, `flate2 = "1"`, and `tar = "0.4"` were already in `apps/desktop/src-tauri/Cargo.toml`.

### UI (`SettingsPage.tsx`)
- `AfterDownloadSection` already includes `extract_archive` in the action dropdown (`ACTION_LABELS`).
- When `extract_archive` is selected, a toggle for "Delete archive after extraction" is shown with a hint listing supported formats.

---

## Build fix applied

A new `EngineKind::Ed2k` variant was added to `crates/models/src/engine.rs` but was not covered in two exhaustive `match` expressions in `apps/desktop/src-tauri/src/commands.rs`:

1. **`engine_matches_source`** — added `EngineKind::Ed2k => source.scheme() == "ed2k"`.
2. **`IpcError::engine_unavailable`** — added `EngineKind::Ed2k` arm with an appropriate error message.

---

## Verification results

| Check | Result |
|---|---|
| `cargo check -p fetchx-desktop -p fetchx-core-services` | ✅ Pass |
| `cargo fmt --all -- --check` | ✅ Pass |
| `npx tsc --noEmit` (apps/desktop/ui) | ✅ Pass |

---

## Files changed

- `apps/desktop/src-tauri/src/commands.rs` — added `EngineKind::Ed2k` to two match arms
- `.agents/tasks/gopeed-features/backend-report.md` — this report

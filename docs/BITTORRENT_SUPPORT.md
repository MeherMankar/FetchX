# BitTorrent and magnet support status

The current implementation is a limited Phase 20S foundation, not a complete
torrent client. Torrent networking remains delegated to the shared aria2
process; there is no Kotlin or React torrent engine.

## Implemented

- `DownloadSource` validates magnet URIs for the aria2 engine, requires exactly
  one valid `urn:btih` hash, bounds URI and tracker counts, and rejects tracker
  schemes other than HTTP, HTTPS, and UDP.
- Shared Rust metainfo parsing accepts canonical BitTorrent v1 `.torrent`
  files, computes the v1 info hash from the original encoded `info` dictionary,
  and exposes the name, total size, files, piece length/count, and trackers.
- Metainfo parsing rejects malformed bencode, oversized inputs/file lists,
  inconsistent piece metadata, unsafe path segments, duplicate/case-conflicting
  paths, and file/directory path conflicts. Windows device names and path
  separators are rejected before aria2 receives the metainfo.
- Shared Rust selection state supports selecting individual files, all/none,
  and directory-prefix groups, with selected count and byte totals.
- CoreServices validates and queues local `.torrent` files. CLI, desktop/Tauri,
  and Android bridge entry points route those requests to this shared method.
- The shared aria2 RPC adapter submits magnet URIs as `aria2.addUri` jobs,
  applies the configured native download rate, disallows overwrite/renaming,
  and currently limits each torrent to one peer to keep connection accounting
  conservative.
- Local metainfo files are submitted through `aria2.addTorrent` with the same
  conservative peer/rate/output options. The file is revalidated immediately
  before submission.
- Completed magnet output is checked as a directory tree. Symlinks and special
  files are rejected, and regular-file lengths must agree with aria2's final
  byte counts.
- The CLI `add` command accepts `--magnet URI` or a positional local
  `.torrent` path, defaults both to aria2, and accepts `--destination` as a
  torrent directory (rather than an output file).
- Desktop add and Android add paths accept magnet URIs and route them through
  the shared aria2 adapter and queue. Both also accept a local `.torrent` path
  and route it to CoreServices; Android exposes the operation through UniFFI.

## Not implemented or not validated

- The selection object is not persisted on `DownloadItem` and is not passed as
  aria2's `select-file` option. aria2 therefore downloads all files; UI file
  selection and per-file priorities remain unimplemented.
- Metadata acquisition state, tracker status, peer/seed/leecher counts, upload
  rate, share ratio, and seeding lifecycle are not exposed.
- Dynamic per-torrent peer allocation. The fixed one-peer ceiling is
  deliberately conservative and does not claim Turbo behavior.
- A metadata timeout or a separate metadata queue state. Metadata acquisition
  currently shares the ordinary aria2 queued/active lifecycle.
- Durable aria2 GID/task persistence and safe recovery across process restart.
  Same-process pause/resume uses the retained GID; torrent restart recovery is
  not validated.
- End-to-end testing with an aria2 binary, a controlled local BitTorrent swarm,
  or an Android device torrent transfer. The Android bridge requires an
  available aria2 executable; no transfer has been reported without one.
- A headless/server listener or dedicated CLI file-selection controls. The
  host-independent CoreServices API is available, but queue lifecycle,
  recovery, telemetry, and selection behavior lack dedicated headless tests.

Metainfo path traversal checks now run before aria2 submission. However, aria2
is an external writer and a hostile local process could race filesystem
replacement after validation; the current implementation does not provide
OS-level sandboxing or prove confinement against that race. Do not mark Phase
20S complete based only on parser and submission tests.

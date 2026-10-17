# BitTorrent and magnet support status

The current implementation is a limited Phase 20S foundation, not a complete
torrent client. Torrent networking remains delegated to the shared aria2
process; there is no Kotlin or React torrent engine.

## Implemented

- `DownloadSource` validates magnet URIs for the aria2 engine, requires exactly
  one valid `urn:btih` hash, bounds URI and tracker counts, and rejects tracker
  schemes other than HTTP, HTTPS, and UDP.
- The shared aria2 RPC adapter submits magnet URIs as `aria2.addUri` jobs,
  applies the configured native download rate, disallows overwrite/renaming,
  and currently limits each torrent to one peer to keep connection accounting
  conservative.
- Completed magnet output is checked as a directory tree. Symlinks and special
  files are rejected, and regular-file lengths must agree with aria2's final
  byte counts.
- The CLI `add` command accepts a magnet URI, defaults it to aria2, and accepts
  `--destination` as a torrent directory (rather than an output file).
- Desktop add and Android add paths accept magnet URIs and route them through
  the shared aria2 adapter and queue. Android exposes the shared operation
  through UniFFI.

## Not implemented or not validated

- Local `.torrent` files and metainfo parsing, including validation of torrent
  paths before aria2 writes them.
- Torrent metadata acquisition state, file listing/selection, per-file
  priorities, tracker status, peer/seed/leecher counts, upload rate, and
  seeding lifecycle.
- Dynamic per-torrent peer allocation. The fixed one-peer ceiling is
  deliberately conservative and does not claim Turbo behavior.
- A metadata timeout or a separate metadata queue state. Metadata acquisition
  currently shares the ordinary aria2 queued/active lifecycle.
- Durable aria2 GID/task persistence and safe recovery across process restart.
  Same-process pause/resume uses the retained GID; service/process recovery for
  torrents has not been demonstrated.
- End-to-end testing with an aria2 binary, a controlled local BitTorrent swarm,
  or an Android device torrent transfer.
- A headless/server listener or dedicated CLI file-selection controls. The
  existing host-independent `CoreServices` remains the backend foundation.

Treat torrent path traversal protection as incomplete until metainfo paths can
be inspected or otherwise confined before aria2 creates output. Do not mark
Phase 20S complete based only on the current magnet submission path.

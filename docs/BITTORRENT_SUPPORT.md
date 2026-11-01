# BitTorrent and magnet support

FetchX supports magnet links and local `.torrent` files through two engine
paths: aria2 when configured, and the native `librqbit` engine on platforms
where aria2 is unavailable (including the Android configuration).

## Shared torrent handling

- `DownloadSource` validates magnet URIs, requires one valid `urn:btih` hash,
  bounds URI and tracker counts, and accepts only HTTP, HTTPS, and UDP trackers.
- Shared metainfo parsing accepts canonical BitTorrent v1 files, computes the
  info hash from the original encoded `info` dictionary, and exposes the name,
  size, file list, piece information, and trackers.
- Metainfo validation rejects malformed bencode, oversized inputs/file lists,
  inconsistent piece metadata, unsafe path segments, duplicate/conflicting
  paths, and Windows device names before an engine receives the metainfo.
- Torrent file selection is not persisted or sent as an engine selection
  option; torrents currently download all files.

## Native engine lifecycle

- Native torrent transfers are handled by one `librqbit::Session` per engine
  instance and remain behind `EngineKind::NativeTorrent`.
- Magnet downloads persist `Preparing`, `MetadataAcquiring`, and `Downloading`
  transitions. Cancellation and pause signals interrupt metadata acquisition.
- Successful native transfers persist `Downloading` → `Completed` → `Seeding`.
  Progress checkpoints are conditional on both the status and the prior
  timestamp, preventing a stale checkpoint from overwriting a lifecycle
  transition or newer progress.
- Pausing removes the torrent from the active session with payload deletion
  disabled, allowing the queue worker and its resources to be released. Resume
  requeues the item and adds it to the session again.
- Cancellation and failure also remove the session entry without deleting
  downloaded data. Cleanup failures are returned rather than treated as
  successful cancellation.
- Stop Seeding removes the in-session torrent with payload deletion disabled.
  CoreServices persists the terminal `Stopped` state. No ratio- or time-based
  seeding policy is currently implemented.
- Restart recovery is FetchX-managed; librqbit session persistence is disabled.
  Interrupted active states are requeued, and completed files are revalidated
  by librqbit when the torrent is added again.

## aria2 engine lifecycle

- Magnets and local metainfo files can be submitted through the shared aria2
  RPC adapter with conservative peer and rate limits.
- Durable aria2 GID reconciliation and torrent-specific restart behavior remain
  limited; do not infer runtime recovery from parser or queue tests.

## Validation status and limitations

- Automated native-engine tests exercise pause → resume → cancel using a
  local private synthetic torrent without a seeder, and verify that session
  entries are removed.
- A separate native-engine test verifies completion of an already-present,
  hash-valid synthetic payload, persisted 100% progress, transition to
  seeding, and Stop Seeding while preserving the payload. This does not prove
  that a torrent payload was transferred from a peer.
- End-to-end transfer from a controlled torrent seeder and Android-device
  lifecycle validation must be reported separately and only after they are
  actually observed.

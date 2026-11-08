# XDM Feature Analysis

## Reference review

- Repository: <https://github.com/subhra74/xdm>
- Reviewed `master` HEAD: `0b361b2badd8690b3fae482a4b5e60673977b4e5`.
- The root `LICENSE` is GNU General Public License version 3, dated 29 June 2007. The reviewed DASH/HLS Kotlin files carry individual `SPDX-License-Identifier: Apache-2.0` headers. No XDM source was copied or linked into FetchX; future reuse would require a file-by-file licensing review.
- Documentation reviewed: `README.md`, including the HLS/DASH quality and audio-track choices, muxing, subtitle, browser, and CLI feature descriptions.
- Relevant source reviewed:
  - `browser-extension/chrome-extension/request-watcher.js` observes successful browser responses and identifies download/media candidates by URL extensions, configured patterns, or MIME type. It keeps short-lived request observations and filters blocked hosts and unsuccessful responses.
  - `xdm-app/src/main/java/xdm/integration/VideoHelper.kt` fetches and parses HLS/DASH manifests, supports request headers/cookies, propagates cancellation, and bounds manifest work and tracked state.
  - `xdm-app/src/main/java/xdm/integration/StreamChoices.kt` turns parsed manifests into video/audio choices; it groups HLS audio renditions with variants and pairs DASH audio and video representations.
  - `xdm-core/src/main/java/xdm/core/downloaders/web/streaming/manifest/dash/MpdParser.kt` and `RepresentationParser.kt` parse MPD periods, adaptation sets, representations, segment templates/lists and resolve media URLs.
  - `xdm-core/src/main/java/xdm/core/downloaders/web/streaming/manifest/hls/HlsParser.kt` parses HLS media/master playlists, segments, encryption, byte ranges, and rendition metadata.
  - `xdm-core/src/main/java/xdm/core/downloaders/web/streaming/downloader/dash/DashDownloaderTask.kt` and `.../hls/HlsDownloaderTask.kt` adapt selected streams to XDM's resumable chunk downloader and muxer.
- Tests reviewed: `xdm-core/src/test/java/xdm/core/TestDashParser.kt`, `TestDashSegmentLimits.kt`, and `TestDashE2E.kt`; and `xdm-app/src/test/java/xdm/StreamChoicesTest.kt`. These include parser edge cases, segment-count limits, representation/audio selection, and generated end-to-end media cases. XDM's tests were not run in this phase.

## FetchX current-state audit

The Phase 0 architecture and roadmap identify Rust `CoreServices` and its shared `EngineDispatcher` as the cross-platform download path. The worktree was clean before this phase. The preceding Ridm batch admission is committed at `00ebad8ef8d6be12f8785d623457c6815922f992`; its collision-safe destination allocation and desktop Tauri wiring are present. It admits explicit URLs and is not playlist discovery.

The Seal MP3/M4A selection remains validated in `DownloadSource`, passed through yt-dlp arguments, and constrained by migration `0012_add_ytdlp_audio_format.sql`. Storage reads and writes the option and export/import preserves it; imports of the prior export shape default it to absent. Android UniFFI, desktop Tauri, and local CLI expose the option. The prior Seal record reports Android Gradle tests stopped when the wrapper JVM crashed in native `net.dll` during `NetworkInterface.getAll`; Kotlin compilation/tests were not verified, and no real audio conversion or device test was run.

FetchX already has:

- Native HLS segment download in `crates/downloader`, including variant selection (highest bandwidth), AES-128 handling, cancellation, progress, and normal queue lifecycle.
- yt-dlp media extraction and format selection. `MediaProtocol::Dash` already maps to yt-dlp; no separate native DASH engine exists.
- Browser-extension source that classifies `.m3u8` and `.mpd` network resources and lets users submit selected resource URLs through the existing batch-add path. The shared dispatcher previously recognized HLS but did not route a direct `.mpd` URL away from Direct HTTP.
- Existing safe source validation and yt-dlp process, progress, error, and cancellation handling.

The extension's media interception source and its manifests request `webRequest` (and cookies), while the extension README says these permissions are not requested. This pre-existing documentation/manifest inconsistency and the browser runtime behavior were not changed or independently runtime-validated.

## Capability comparison

| Capability | XDM | FetchX before this phase | Classification |
|---|---|---|---|
| Detect browser video and stream responses | Browser request watcher uses response status, extensions, patterns, MIME, and blocked-host rules | Browser-extension source detects HLS/DASH and direct media; it forwards selected URLs through the generic add path | Partially implemented; runtime/permission behavior needs further investigation |
| HLS segment downloads | Native resumable chunks, encryption, audio/subtitle tracks, and muxing | Native HLS engine handles segmented media and AES-128, but automatically chooses the highest-bandwidth variant | Already implemented adequately for the existing native HLS contract; user rendition/audio selection remains partial |
| DASH manifest parsing and representation selection | MPD parser expands representations and segment addressing; UI groups video/audio pairs | Page extraction can return DASH formats through yt-dlp, but direct MPD downloads were treated as ordinary HTTP | Partially implemented; direct-MPD dispatch was missing |
| DASH media download and muxing | Native DASH chunk downloader, audio/video pairing, and muxing | yt-dlp engine already supports DASH formats; no native DASH engine | Existing yt-dlp path is appropriate; a second native engine is not selected |
| Request headers, cookies, and authenticated streams | Captured request context can be carried into manifest and chunk requests | Extractor supports filtered headers and temporary cookie input; the browser add path does not forward captured stream headers/cookies | Requires further investigation; not expanded |
| Subtitles and XDM-specific stream muxing | Stream dialog supports subtitle/track selection and XDM muxing | Existing yt-dlp/FFmpeg path handles supported media selection and merging | Not appropriate for this narrowly scoped change; avoid duplicating media tooling |

## Selected capability: dispatch direct DASH MPD URLs to yt-dlp

This closes a specific gap without adding a DASH parser or downloader. A direct `http(s)` URL whose path ends in `.mpd` is a DASH manifest, not a media file. FetchX's browser source could discover and submit one, but the shared dispatcher would otherwise treat it as a generic HTTP file and save the manifest instead of extracting its representations. FetchX already has a DASH-capable yt-dlp path, and a local synthetic MPD probe confirmed that the installed yt-dlp can extract a DASH representation from a direct manifest URL.

Acceptance criteria:

1. Match only valid HTTP(S) URLs whose URL path ends in `.mpd`, case-insensitively; query values do not determine the match.
2. Route a Direct HTTP MPD through the configured yt-dlp engine, which continues to own extraction and the existing download lifecycle.
3. If yt-dlp is unavailable, return the existing explicit `EngineNotAvailable` result rather than falling back to saving XML.
4. Leave non-MPD Direct HTTP URLs, HLS dispatch, page-level media selection, and persisted models unchanged.
5. Keep active-pause behavior accurate for the effective yt-dlp engine; shutdown should use the existing cancel-and-requeue behavior for an unpausable DASH transfer.
6. Add focused tests for URL classification, missing runtime, configured-runner dispatch, and lifecycle behavior.

## Implementation and adapter status

- `crates/downloader/src/dispatcher.rs` adds `is_dash_url` and routes direct HTTP(S) `.mpd` paths to `EngineKind::YtDlp`. The dispatcher also reports active-pause support from the effective source route so the queue rejects unsupported active pause and uses its existing cancel-and-requeue shutdown behavior for DASH.
- `crates/downloader/src/lib.rs` exports the helper. `crates/downloader/src/ytdlp.rs` permits a direct MPD source while preserving its existing URL validation, process runner, progress, error, cancellation, and finalization behavior. `ManagedDownloadEngine::supports_active_pause_for` supplies the source-aware pause query used by the queue.
- The source remains recorded as `EngineKind::DirectHttp`, following the existing direct-HLS dispatcher pattern. This is a dispatch change, not a new engine or format-selection API.

| Target | Status |
|---|---|
| Rust shared core | Implemented and covered by dispatcher and downloader tests. |
| Android | Uses the same CoreServices dispatcher when the configured yt-dlp runtime is available. Rust Android-bridge tests pass; no Kotlin or UI change was made. No Android device/emulator test was run. |
| Windows/macOS/Linux desktop | Uses the shared dispatcher. Desktop Rust tests pass; no desktop UI/API changes were required. |
| Local CLI | Uses CoreServices and the shared dispatcher for direct URL downloads. CLI tests pass. |
| Remote server/UI | Generic remote downloads use CoreServices and the same dispatcher. Remote-server tests pass; no remote protocol or UI changes were made. |
| Browser extension/native host | Existing manifest detection and batch submission can send `.mpd` URLs through the generic download path. Extension unit tests pass; browser runtime behavior was not tested. The extension build currently fails at its pre-existing TypeScript check because `classify` in `src/background/media-interception.ts` is unused (`TS6133`); that unrelated source was not changed. |

## API, storage, and credential implications

The additive downloader API includes `fetchx_downloader::is_dash_url` and the source-aware `ManagedDownloadEngine::supports_active_pause_for` query. No `EngineKind`, DTO, UniFFI, Tauri, JSON-RPC, storage, migration, or export/import schema changed. No dependency was added.

No cookies or request headers were added, copied to logs, or persisted by this change. Existing URL storage behavior is unchanged: a signed/tokenized query string on a source URL remains part of the ordinary persisted download source. Embedded URL credentials continue to be rejected by the existing yt-dlp source validator. Browser-captured headers are not forwarded into this new route.

## Validation and limitations

Commands and results:

- `cargo test -p fetchx-downloader dispatcher::tests -j 1` — passed, 5 tests.
- `cargo test -p fetchx-downloader -p fetchx-queue -p fetchx-core-services -j 1` — passed: downloader 280 unit tests and 2 integration tests (3 unit tests ignored); queue 42 tests; CoreServices 28 unit tests, 2 integration tests, and 1 doctest (4 live-extraction tests ignored).
- `cargo clippy --workspace --exclude fetchx-desktop --all-targets --all-features -j 1 -- -D warnings` — passed.
- `cargo test -p fetchx-desktop -p fetchx -p fetchx-remote-server -p fetchx-android-bridge -j 1` — passed: desktop 60, CLI 26, Android bridge 24, remote server 20 unit and 3 integration tests.
- Seal regression checks passed: `cargo test -p fetchx-models source_validates_and_round_trips_ytdlp_audio_format -j 1` (1); `cargo test -p fetchx-storage export_import_preserves_ytdlp_audio_format -j 1` (1); `cargo test -p fetchx-storage imports_previous_schema_v1_exports_with_default_audio_mode -j 1` (1); and `cargo test -p fetchx-media-extractor -j 1` (76).
- `npm test --prefix "D:\Projects\FetchX\apps\browser\extension"` — passed, 26 tests in 3 files.
- `npm run build --prefix "D:\Projects\FetchX\apps\browser\extension"` — failed at `tsc --noEmit` with `src/background/media-interception.ts(103,10): TS6133: 'classify' is declared but its value is never read`. No extension source was changed.
- `cargo fmt --all -- --check` and `git diff --check` — passed.
- A full `cargo test --workspace --exclude fetchx-desktop` attempt failed during parallel compilation because the Windows paging file was too small (`os error 1455`); focused package suites passed when rerun serially.
- The installed yt-dlp was run against a temporary local synthetic DASH MPD using `yt-dlp --no-warnings -F <local-manifest-url>` and `yt-dlp --no-warnings --simulate --dump-single-json <local-manifest-url>`; both exited 0 and returned a 320x240 DASH video representation. This validates local manifest extraction only, not a complete media download or any particular site.

No live media-site download, authenticated stream, FFmpeg mux, Android Gradle build, browser runtime, or device/emulator test was performed. `yt-dlp` was available; `ffmpeg` was not found on PATH. The adapter Rust tests and Clippy compile the shared Android/CLI/remote interfaces but do not validate Android Kotlin or platform runtime behavior. The prior Seal Gradle limitation remains as recorded above.

Known limitations: direct manifest detection is URL-path based (`.mpd`) and does not sniff MIME types or identify manifests on opaque URLs. DASH parsing, representation/audio selection, cookies/headers, encrypted streams, subtitles, and native DASH downloading remain delegated to existing yt-dlp behavior or deferred. If yt-dlp is unavailable, FetchX reports the engine unavailable. The extension build issue and permission-documentation inconsistency predate this change and should be handled separately.

Recommended next research item in the established order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

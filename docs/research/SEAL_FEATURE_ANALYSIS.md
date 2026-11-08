# Seal Feature Analysis

## Reference review

- Repository: `JunkFood02/Seal`
- Reviewed revision: [`7677f61a20fda4210e84215fef9e9b51d25daa47`](https://github.com/JunkFood02/Seal/commit/7677f61a20fda4210e84215fef9e9b51d25daa47), the default-branch revision reviewed for this phase (2026-09-25).
- Exact root license reviewed: `LICENSE`, GNU General Public License version 3, 29 June 2007.
- Documentation reviewed: `README.md` and media-download/yt-dlp setup and usage guidance.
- Relevant implementation files reviewed: `app/src/main/java/com/junkfood/seal/util/DownloadUtil.kt` (MP3/M4A audio extraction and output options), `app/src/main/java/com/junkfood/seal/download/DownloaderV2.kt` (yt-dlp execution and download lifecycle), `app/src/main/java/com/junkfood/seal/download/Task.kt` and `TaskFactory.kt` (task metadata and construction), `app/src/main/java/com/junkfood/seal/DownloadService.kt` (Android background execution/notifications), and `app/src/main/java/com/junkfood/seal/ui/page/download/DownloadSettingsDialog.kt` (media settings and user choices).
- The app test tree was inspected; no focused test for audio-format extraction behavior was identified.
- No Seal source was copied. The implementation is an independent use of documented yt-dlp behavior. The GPL license was recorded; this phase does not incorporate or link Seal code.

Seal is an Android yt-dlp front end. Its documented feature set includes media extraction, audio-only downloads and conversion, metadata/subtitle options, playlists, and custom yt-dlp options. Seal delegates media-site extraction and format availability to yt-dlp, while its service/task layer owns Android orchestration and its UI presents the choices.

## FetchX current-state audit

Before this phase, FetchX already had:

- Rust `media-extractor` format discovery and a shared `FormatSelector`, including audio-only selection.
- Rust `YtDlpEngine` for isolated yt-dlp process invocation, staged output, progress, cancellation, and finalization.
- Android yt-dlp runtime integration, including bundled FFmpeg support for merged audio/video formats.
- Desktop format discovery and yt-dlp download creation.
- A generic CLI engine option for yt-dlp, but no media-specific audio conversion option.

Thus format discovery, normal video selection, and yt-dlp execution were not reimplemented. The narrow gap was a user-selectable, persisted request to extract and convert the selected media to MP3 or M4A.

## Capability comparison

| Capability | Seal | FetchX before this phase | Assessment |
|---|---|---|---|
| URL/media extraction and format discovery | yt-dlp-backed | Shared Rust extractor and yt-dlp runtime | Already implemented adequately |
| Automatic or selected video format | yt-dlp-backed | Shared Rust format selection and platform adapters | Already implemented adequately |
| Audio-only stream selection and output conversion | MP3/M4A options in media UI/yt-dlp integration | Audio-only selection existed, but there was no persisted conversion mode or host option | Missing; selected |
| Transfer progress and cancellation | Android task/service integration around yt-dlp | Shared Rust runner/queue lifecycle | Already implemented adequately |
| Download scheduling and Android notifications | Android service/task responsibilities | Shared Rust scheduler and Android notification/service adapter | Already implemented adequately for FetchX's architecture |
| Arbitrary yt-dlp command templates | Documented Seal option | FetchX uses a restricted Rust-owned argument builder | Not appropriate; arbitrary commands would bypass safe argument and lifecycle policy |
| Cookies and authenticated site access | Supported by yt-dlp-facing workflows/settings | Existing extractor filters headers and uses temporary cookie support where configured | Requires further investigation; not expanded in this phase |
| Audio metadata/post-processing beyond MP3/M4A | Seal documents metadata and media options | Android has optional yt-dlp preferences; shared core has no general media post-processing contract | Requires further investigation |
| Playlist downloading | Supported by Seal | FetchX intentionally uses single-item policy for this flow | Not appropriate for this scoped change |

## Selected capability: opt-in MP3/M4A extraction

Users can request yt-dlp to select `bestaudio/best`, extract audio, and convert it to MP3 or M4A. This is a focused addition to the existing media path rather than a second media engine.

Acceptance criteria:

1. The source model accepts only `mp3` and `m4a`, only for yt-dlp, and never together with an explicit video format ID.
2. The selected audio mode survives serialization, SQLite persistence, and export/import.
3. Rust builds the yt-dlp arguments from the typed mode and rejects a destination whose suffix does not match.
4. Successful conversion finalizes into the selected destination through the existing staging flow; cancellation and process errors retain the current failure semantics.
5. Android, desktop, and local CLI surfaces expose the option. The Android adapter uses its bundled FFmpeg for extraction.
6. No cookies, endpoint credentials, or extra media settings are introduced or logged by this feature.
7. FetchX's remote API and browser extension/native-host remain unchanged and do not claim a media-extraction operation.

Rust owns the source option, validation, persistence, yt-dlp argument generation, process lifecycle, progress, cancellation, and finalization. Kotlin supplies the Android choice and passes the request through UniFFI; Android-only runtime code supplies the bundled FFmpeg path. Desktop and CLI are thin request adapters.

## Implementation and target status

- Shared model: `crates/models/src/download.rs` adds `YtDlpAudioFormat` and validated `DownloadSource` state.
- Shared downloader: `crates/downloader/src/ytdlp.rs` selects audio, passes `--extract-audio --audio-format mp3|m4a`, validates destination suffixes, and accommodates conversion output/progress behavior.
- Persistence: migration `migrations/0012_add_ytdlp_audio_format.sql`, storage repository reads/writes, schema version 12, and export format 2 retain the selected mode. The preceding version-1/schema-11 export shape remains importable with audio mode defaulting to absent.
- Android: `crates/android-bridge` exposes the optional mode through UniFFI; the Kotlin repository and browser picker expose MP3/M4A. `AndroidYtDlpRuntime` supplies bundled FFmpeg whenever extraction is requested.
- Desktop: Tauri media-download creation and the Add dialog expose MP3/M4A selection.
- CLI: local `fetchx add --audio-format mp3|m4a URL` selects yt-dlp and validates the output extension. Remote CLI commands continue to control FetchX downloads and have no media-specific extraction option.

| Target | Status |
|---|---|
| Android | Browser picker requests MP3/M4A through UniFFI. Rust bridge tests passed; Kotlin compilation/unit tests were blocked by the Gradle JVM crash described below. No Android runtime test was run. |
| Windows/Linux/macOS desktop | Shared Tauri/Rust media path exposes the choices. Runtime conversion was not exercised against a real media site in this phase. |
| Local CLI | `--audio-format mp3|m4a` is available for local yt-dlp queue creation. |
| Remote server and remote UI | No media-specific request extension was made; generic remote downloads are unchanged. |
| Browser extension/native host | No media extractor UI or protocol extension was made. |

## Public API and data-model changes

`YtDlpAudioFormat` is a new Rust model enum with `Mp3`/`M4a` variants and an optional serialized field on `DownloadSource`. The Android UniFFI `add_media_download` and Tauri `add_media_download` operations gained an optional `audio_format` parameter; the Android Kotlin repository forwards it. The CLI adds `--audio-format mp3|m4a`. Existing download creation and video-format selection remain unchanged when this option is absent. No FetchX remote JSON-RPC or browser extension protocol method was added.

## Storage, credentials, and compatibility

The new nullable, constrained SQLite column is additive and backward-compatible for databases upgraded through migrations. SQLite schema version is now 12. Export format version is 2; import accepts the prior version-1/schema-11 export shape (the optional audio mode defaults to `None`) and current version-2/schema-12 exports. No credential fields, site cookies, or new dependencies were introduced. The existing cookie/header isolation and yt-dlp temporary workspace behavior are unchanged.

## Validation and limitations

Validation performed:

- `cargo test -p fetchx-models -p fetchx-storage -p fetchx-downloader -p fetchx-android-bridge -p fetchx -p fetchx-desktop --lib` passed: Android bridge 24, downloader 278 passed/3 ignored, models 39, and storage 37 at that point.
- `cargo test -p fetchx -p fetchx-desktop` passed: CLI 25 and desktop Tauri 60 tests. After adding the CLI audio-option parser regression test, `cargo test -p fetchx --bin fetchx` passed all 26 CLI tests.
- `cargo test -p fetchx-storage --lib` passed all 38 tests, including migration, audio persistence, export/import, and prior-schema export compatibility.
- Final `cargo test -p fetchx-storage -p fetchx-downloader --lib` passed: storage 38; downloader 278 passed and 3 ignored, including audio-mode cancellation/workspace coverage.
- `npm run build` in `apps/desktop/ui` passed TypeScript checking and the Vite production build.
- Android Gradle unit tests were attempted with `:app:testDebugUnitTest :browser:testDebugUnitTest` but the Gradle wrapper JVM crashed in native `net.dll` while enumerating network interfaces (`NetworkInterface.getAll`); Android Kotlin compilation and unit tests therefore remain unverified.
- `cargo fmt --all -- --check` passed. `git -c core.whitespace=cr-at-eol diff --check` passed.
- No device/emulator test, real media-site extraction, audio conversion, external credential use, or Windows/Linux/macOS runtime test is claimed.

Known limitations and deferred work:

- Conversion requires FFmpeg available to the active yt-dlp runtime. Android supplies its bundled binary; desktop and CLI depend on their configured runtime.
- The feature intentionally supports only MP3 and M4A. Other codecs, audio metadata policies, playlist semantics, cookie UX, and authenticated-site integration require separate investigation.
- The remote UI/API and browser extension/native host remain generic download clients; exposing media extraction there needs an explicit protocol contract rather than duplicating yt-dlp business logic.
- Recommended next repository/work item in the established research order: Gopeed's deferred full `resolve_task` contract for protocol-specific metadata and file selection.

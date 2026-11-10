# Native yt-dlp Migration Coverage

## Pinned source and scope

- Fork: [MeherMankar/yt-dlp](https://github.com/MeherMankar/yt-dlp)
- Source revision: `d58c4588a54271fe74172e2fc2f09a4a44f70679`
- Registry source: `yt_dlp/extractor/_extractors.py`
- First-slice source: `yt_dlp/extractor/generic.py` (`GenericIE`)
- License reviewed at the pinned revision: root `LICENSE`, Unlicense (public domain dedication).
- HTML parser dependency: `scraper` 0.24.0 (ISC); the resolved dependency tree also includes `cssparser` and `selectors` under MPL-2.0. No dependency source is modified or copied.
- Reused behavior is limited to common Generic HTML declarations and the scoped YouTube behavior described below; no yt-dlp Python package or executable is added.

The [CSV ledger](NATIVE_YT-DLP_COVERAGE.csv) contains one row for every imported extractor class in the pinned registry: **1,754 classes across 928 extractor modules/families**. These counts describe the source registry, not FetchX support.

## Status meanings

- `not started`: no FetchX native implementation or parity review is claimed. Shared dependencies and edge cases remain unassessed.
- `in progress`: a bounded slice exists, but it does not cover the upstream extractor's full behavior.
- `ported`: implementation exists for the tracked class/family. This status alone is not a claim of full parity.
- `parity reviewed`: source behavior, supported cases, limits, and regression tests have been reviewed against the pinned revision.

No site should be described as fully natively supported until its implementation exists and has been reviewed. `GenericIE` and `YoutubeIE` are both `in progress`; only the bounded behavior described here is implemented. Other YouTube extractor classes remain `not started`.

## Ledger maintenance

Each migration prompt should update the existing row(s), not replace the source baseline silently. Record the FetchX module, concrete shared dependencies (for example cookies/authentication, signatures, manifests, or pagination), tests, and unsupported edge cases. Keep unreviewed behavior explicitly marked; change a row to `ported` only after a native implementation and focused tests exist, and to `parity reviewed` only after comparison with the same pinned upstream revision. A source revision change requires regenerating the registry rows and documenting the old-to-new mapping before changing statuses.

## Generic first-slice limits

The native extractor fetches bounded HTML (4 MiB maximum, 20-second default timeout capped at 30 seconds), validates each redirect and pins DNS results for each request to public addresses. It blocks private, loopback, link-local, reserved, and multicast destinations. It does not forward caller cookies or headers; requests that provide them are explicitly unsupported by the native extractor so the existing yt-dlp extractor can handle them. Other unsupported GenericIE behaviors include JavaScript players, embedded site-specific players, iframe extraction, playlists, subtitles, metadata beyond basic title/duration/thumbnail, manifests beyond URL classification, and format-specific headers. Existing yt-dlp integrations remain enabled.

## JW Platform shared-family slice

The native `JwPlatformExtractor` handles the direct JW Platform media routes plus three registered site routes: Bundesliga, Business Insider, and The Hollywood Reporter. Their route behavior follows the pinned `bundesliga.py`, `businessinsider.py`, and `hollywoodreporter.py` extractors and delegates to the shared media endpoint in `jwplatform.py` (`https://cdn.jwplayer.com/v2/media/{id}`). The result parser adapts the single-item response behavior in `common.py`'s `_parse_jwplayer_data` and `_parse_jwplayer_formats`.

Requests use FetchX's public-address DNS pinning and bounded-body helper, explicit redirect validation (including HTTPS downgrade rejection), a five-redirect limit, a 4 MiB response limit, and a 30-second maximum timeout. Cookies and caller headers are not sent; those cases return unsupported so the existing yt-dlp fallback can handle them. Only one playlist item is accepted. Direct HTTP media and HLS/DASH manifest URLs are classified; manifest variant expansion, SMIL, RTMP, subtitles, playlists, DRM, access-controlled responses, and unrecognized page markup remain unsupported. The corresponding four ledger rows remain `in progress`; this is not full JW Platform or site parity.

## Prompt 9 — Brightcove, Kaltura, and Vimeo shared-family slices

These Rust translations use the authorized [MeherMankar/yt-dlp](https://github.com/MeherMankar/yt-dlp) fork at pinned revision `d58c4588a54271fe74172e2fc2f09a4a44f70679` (root `LICENSE`: Unlicense). Upstream behavior reviewed:

- Brightcove: `yt_dlp/extractor/brightcove.py`, `ninenews.py`, `craftsy.py`, `amcnetworks.py`, and registry entries in `_extractors.py`.
- Kaltura: `yt_dlp/extractor/kaltura.py`, `azmedien.py`, `inc.py`, `unitednations.py`, and registry entries in `_extractors.py`.
- Vimeo: `yt_dlp/extractor/vimeo.py`, `aeonco.py`, `amara.py`, `croatianfilm.py`, and registry entries in `_extractors.py`.

The selected Brightcove routes are Nine News, Craftsy, and AMC Networks. They resolve a page's known player reference, retrieve public player config/policy-key data, then call Brightcove's public Playback API and map the returned direct sources. The Kaltura routes are AZ Medien, Inc., and UN Web TV. They discover the selected page's partner/entry identifiers, issue the public widget-session multirequest for entry metadata and ready flavor assets, and map `serveFlavor` sources. The Vimeo routes are Aeon, Amara, and Croatian Film. Their page/API metadata resolves to a Vimeo video ID, then the shared player config maps progressive sources and classifies supplied HLS/DASH manifest URLs.

The code is in `crates/media-extractor/src/{provider_http,brightcove,kaltura,vimeo}.rs`; all three extractors are registered in CoreServices after YouTube/JW Platform and before Generic and the existing yt-dlp fallback. The provider HTTP helper reuses FetchX public-address DNS pinning and bounded-body reading, caps responses at 4 MiB, limits GET redirects to five while rejecting HTTPS downgrades, rejects POST redirects, and applies a 30-second maximum extractor timeout. No caller cookies or arbitrary headers are sent; if supplied, these native provider extractors decline and allow the existing fallback to run.

This is not parity or a live-compatibility claim. Brightcove supports only the selected wrapper page shapes and returned sources; legacy players, authentication, DRM, playlists, and manifest expansion remain unimplemented. Kaltura supports only the selected wrappers and ready non-DRM flavor assets; direct Kaltura URLs, captions, playlists, session-bound formats, and HLS expansion remain unsupported. Vimeo supports only public single videos; login, password/private/embed-only access, playlists, subtitles, and manifest expansion remain unsupported. Region/access errors are not bypassed. The nine selected site rows and the BrightcoveNewIE, KalturaIE, and VimeoIE shared-backend rows remain `in progress`; see the CSV ledger for per-route scope.

## YouTube native scope and limits

`YoutubeIE` recognizes 11-character video IDs in YouTube watch, `youtu.be`, Shorts, embed, and live URL forms. It tries the pinned yt-dlp default unauthenticated client order: `tv_samsung`, `visionos`, then `web`. The first two do not require GVS PO tokens for HTTPS media under this pinned policy; `web` is not requested unless a caller supplies a `web.gvs` token. Successful responses map title, duration, the highest-dimension thumbnail, and at most 64 entries from `streamingData.formats` and `adaptiveFormats`.

For supported audio/video formats, the extractor accepts direct URLs and parses `signatureCipher`/`cipher` URL, signature, and parameter fields. It transforms `s` signatures and `n` query values from the current player JavaScript; already-transformed `sig`/`signature` values are inserted as-is. A supplied client-scoped GVS token is added as `pot` to that client's media URLs; a supplied player-context token is sent in `serviceIntegrityDimensions.poToken` for that client. Tokens are caller-provided base64url values, are validated and redacted in Debug output, and are never acquired automatically. The mapping retains the itag, extension, quality label, codecs, dimensions, frame rate, bitrate, and content length when present. Only resulting HTTPS `googlevideo.com` URLs are returned.

The shared `ExtractOptions.cookies` input uses Netscape cookie-jar format. The native YouTube path validates its bounds and syntax and sends only unexpired cookies scoped to `youtube.com` (and matching the player API path) to clients marked as cookie-capable by the pinned source (`tv_samsung` and `web`). Cookies are not sent to `visionos`; caller-supplied arbitrary headers remain unsupported by this path. Android exposes a separate `discover_media_formats_with_access` binding that accepts optional cookies and typed `(client, context, token)` values; desktop/shared-core callers can pass the same values through `ExtractOptions`. Cookies continue to flow to the existing yt-dlp fallback. PO tokens are consumed by the native extractor and are not currently passed to the subprocess fallback.

Player responses are classified into fixed safe reason categories (sign-in-required, verification-required, age-restricted, or unsupported). An unavailable access requirement produces `AuthenticationRequired`; the registry still advances to the existing yt-dlp fallback and returns that typed reason only when no later extractor handles the URL. No access-control, age, or DRM restriction is bypassed.

The player script URL is taken from the player response when it is a same-origin `/s/player/` URL; otherwise FetchX uses yt-dlp's pinned iframe API fallback and the `main` player variant. The EJS solver runs in-process through `rquickjs`, without Python, Node, Deno, Bun, or another runtime process. It has no filesystem or network host bindings, and uses a 4 MiB player-script bound, 128 MiB JS memory limit, 1 MiB stack limit, five-second interpreter interrupt deadline, at most 128 challenges of at most 4 KiB each, and a two-solver concurrency limit.

Malformed cipher metadata, an unrecognized player script, a solver failure, or any challenged format that cannot be transformed makes the native YouTube extractor return `UnsupportedUrl` for the whole video. Registry ordering then allows the existing yt-dlp fallback to handle it; it does not return a partial native format set in those cases. Unsafe or DRM-marked streams are still rejected, not transformed or downloaded. Restricted or region-limited playback still requires valid caller-provided access data and may remain unavailable. Unsupported clients, formats whose GVS policy requires an absent token, alternative player clients, HLS/DASH manifests, captions, playlists, and non-video YouTube extractor families remain gaps. The `YoutubeIE` ledger row remains `in progress`, not fully supported.

## Provenance

The Rust implementation is in `crates/media-extractor/src/generic.rs`. It adapts URL discovery and metadata behavior from the fork's `GenericIE` in `yt_dlp/extractor/generic.py` at the pinned commit. The Rust implementation uses HTML parsing, FetchX's existing format models and URL/header sanitization, and its own bounded HTTP/DNS/redirect policy. It is not a full GenericIE port.

The Rust JW Platform implementation is in `crates/media-extractor/src/jwplatform.rs`. At fork revision `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense), it adapts the shared API request from `yt_dlp/extractor/jwplatform.py`, the single-item media/format mapping from `yt_dlp/extractor/common.py` (`_parse_jwplayer_data`, `_parse_jwplayer_formats`), and the supported route discovery from `yt_dlp/extractor/bundesliga.py` (`BundesligaIE`), `businessinsider.py` (`BusinessInsiderIE`), and `hollywoodreporter.py` (`HollywoodReporterIE`). Their registry entries are in `yt_dlp/extractor/_extractors.py`. This is a translated, bounded slice; no upstream Python code or runtime is added.

The YouTube request and format handling are in `crates/media-extractor/src/youtube.rs`. They adapt client selection and player contexts from `yt_dlp/extractor/youtube/_video.py` (`YoutubeIE._DEFAULT_CLIENTS`, `_get_requested_clients`, `_extract_player_response`, `_extract_formats`) and `yt_dlp/extractor/youtube/_base.py` (`INNERTUBE_CLIENTS`, `WEB_PO_TOKEN_POLICIES`, `YoutubeBaseInfoExtractor._call_api`) at fork revision `d58c4588a54271fe74172e2fc2f09a4a44f70679`. PO-token scoping follows `_video.py` (`_get_config_po_token`, `fetch_po_token`, `_extract_player_response`, and GVS `pot` URL handling); FetchX accepts supplied values only and ports no provider or acquisition logic.

`crates/media-extractor/src/youtube_js.rs` executes the authorized yt-dlp challenge solver in bounded in-process QuickJS. The player-source AST matching and signature/`n` transform procedure are from `yt_dlp/extractor/youtube/jsc/_builtin/ejs.py` (`EJSBaseJCP._construct_stdin`, `_real_bulk_solve`) and `yt_dlp/extractor/youtube/jsc/_builtin/vendor/yt.solver.core.js` at fork commit `d58c4588a54271fe74172e2fc2f09a4a44f70679`. Embedded `yt.solver.core.min.js` and `yt.solver.lib.min.js` are EJS 0.8.0 assets from `yt-dlp/ejs` tag commit `4fb477f4af56880cfd324c48bd4294a2d2294e50`; their SHA3-512 digests match the pinned fork's `yt_dlp/extractor/youtube/jsc/_builtin/vendor/_info.py`. The generated assets retain their Unlicense header and bundled dependency notices (Meriyah 6.1.4, ISC; astring 1.9.0, MIT). `rquickjs` 0.14.0 is MIT-licensed, supports the workspace Rust 1.88 floor, and embeds QuickJS with a C compiler at build time. Windows host compilation and the Android ARM64 extractor target cross-check passed; Linux/macOS native builds were not run. No yt-dlp Python runtime or external JavaScript process is added.

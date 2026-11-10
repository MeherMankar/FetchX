# Native yt-dlp Migration Coverage

## Pinned source and scope

- Fork: [MeherMankar/yt-dlp](https://github.com/MeherMankar/yt-dlp)
- Source revision: `d58c4588a54271fe74172e2fc2f09a4a44f70679`
- Registry source: `yt_dlp/extractor/_extractors.py`
- First-slice source: `yt_dlp/extractor/generic.py` (`GenericIE`)
- License reviewed at the pinned revision: root `LICENSE`, Unlicense (public domain dedication).
- HTML parser dependency: `scraper` 0.24.0 (ISC); the resolved dependency tree also includes `cssparser` and `selectors` under MPL-2.0. No dependency source is modified or copied.
- Reused behavior is limited to common Generic HTML declarations; no yt-dlp Python package or executable is added, and no source code is copied verbatim.

The [CSV ledger](NATIVE_YT-DLP_COVERAGE.csv) contains one row for every imported extractor class in the pinned registry: **1,754 classes across 928 extractor modules/families**. These counts describe the source registry, not FetchX support.

## Status meanings

- `not started`: no FetchX native implementation or parity review is claimed. Shared dependencies and edge cases remain unassessed.
- `in progress`: a bounded slice exists, but it does not cover the upstream extractor's full behavior.
- `ported`: implementation exists for the tracked class/family. This status alone is not a claim of full parity.
- `parity reviewed`: source behavior, supported cases, limits, and regression tests have been reviewed against the pinned revision.

No site should be described as natively supported until its implementation exists and has been reviewed. The current Generic row remains `in progress`; only OpenGraph video URLs, Twitter player stream URLs, and HTML5 `video`/`source` URLs are implemented.

## Ledger maintenance

Each migration prompt should update the existing row(s), not replace the source baseline silently. Record the FetchX module, concrete shared dependencies (for example cookies/authentication, signatures, manifests, or pagination), tests, and unsupported edge cases. Keep unreviewed behavior explicitly marked; change a row to `ported` only after a native implementation and focused tests exist, and to `parity reviewed` only after comparison with the same pinned upstream revision. A source revision change requires regenerating the registry rows and documenting the old-to-new mapping before changing statuses.

## Generic first-slice limits

The native extractor fetches bounded HTML (4 MiB maximum, 20-second default timeout capped at 30 seconds), validates each redirect and pins DNS results for each request to public addresses. It blocks private, loopback, link-local, reserved, and multicast destinations. It does not forward caller cookies or headers; requests that provide them are explicitly unsupported by the native extractor so the existing yt-dlp extractor can handle them. Other unsupported GenericIE behaviors include JavaScript players, embedded site-specific players, iframe extraction, playlists, subtitles, metadata beyond basic title/duration/thumbnail, manifests beyond URL classification, and format-specific headers. Existing yt-dlp integrations remain enabled.

## Provenance

The Rust implementation is in `crates/media-extractor/src/generic.rs`. It adapts URL discovery and metadata behavior from the fork's `GenericIE` in `yt_dlp/extractor/generic.py` at the pinned commit. The Rust implementation uses HTML parsing, FetchX's existing format models and URL/header sanitization, and its own bounded HTTP/DNS/redirect policy. It is not a full GenericIE port.

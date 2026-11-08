# JDownloader 2 Feature Analysis

## Reference and source-access review

- The supplied official reference is the JDownloader support article [Setup IDE Eclipse](https://support.jdownloader.org/en/knowledgebase/article/setup-ide-eclipse). It identifies the source as Subversion repositories, including `svn://svn.jdownloader.org/jdownloader/trunk`, `svn://svn.jdownloader.org/jdownloader/browser`, `svn://svn.appwork.org/utils`, and `svn://svn.jdownloader.org/jdownloader/MyJDownloaderClient`.
- The official source endpoint could not be inspected from this environment: the HTTPS form of the trunk URL returned HTTP 401, and the `svn` client is not installed. Therefore no authoritative current SVN revision, source implementation, test suite, or source license could be verified.
- The public GitHub repository `mirror/jdownloader` is explicitly a mirror, not the authoritative source repository. Its reviewed `master` HEAD is `f274b29897aad2e0ff0d4fd148e42aadf8b622fa`, whose commit metadata maps to SVN revision `r49881` dated 2024-09-27. This is an archival snapshot, not evidence of the current upstream revision.
- The mirror's root listing contains no `LICENSE` file; requesting `LICENSE` returned a not-found response. Its exact source license is therefore **unknown**, not assumed to be open source or compatible with FetchX. No mirrored source code was copied or used as implementation evidence.
- The official JDownloader wiki article [Link Grabber](https://jdownloader.org/knowledge/wiki/glossary/linkgrabber) describes LinkGrabber as a place to filter, sort, and package links; says LinkCollector extracts links from clipboard text; documents an explicit deep-decryption action for unknown links; and describes package information such as download directory and passwords. The page includes older screenshots, so it is used only for the documented concepts, not as proof of current UI behavior.
- The official [MyJDownloader API developer documentation](https://my.jdownloader.org/developers/) was reviewed for its API scope and security model. It documents an encrypted REST API and warns that response structures can be extended; it is not a source-code or desktop LinkGrabber implementation reference.
- Source paths visible in GitHub code-search metadata include `src/jd/controlling/linkcollector/LinkCollector.java`, `LinkCollectorCrawler.java`, `src/jd/controlling/linkcrawler/CrawledLink.java`, and `src/org/jdownloader/api/linkcollector/LinkCollectorAPI.java`. Their contents were not relied on because the archive is non-authoritative and its license is unavailable. The authoritative checkout and JDownloader tests were not accessible, so no claims about those files' algorithms, tests, or current behavior are made.

## FetchX current-state audit

The worktree was clean on branch `master` before implementation. Phase 0 architecture and the roadmap identify `CoreServices` as the shared Rust business-logic boundary.

Verified existing behavior:

- `extract_download_links` extracts and de-duplicates explicit HTTP(S) links from supplied text; it does not fetch arbitrary pages or perform deep decryption.
- `MediaExtractor` resolves metadata/formats for one URL at a time; it does not produce batch LinkGrabber metadata.
- `CoreServices::enqueue_download_batch` validates each URL and its media options independently, de-duplicates normalized URLs within the current batch, reserves existing and current-batch destination paths, suffixes colliding filenames, and returns one ordered result per request. A failed item does not block later items; accepted items use the ordinary queue lifecycle.
- `CoreServices::find_duplicate_download` checks persisted items for one URL. That API is separate from batch admission; the batch operation does not query it to suppress duplicates already stored in the repository.
- `suggest_download_package_name` already returns a shared Rust suggestion based on the URL host (with filename/fallback behavior), and sanitization is available in shared Rust. Single-download Android and desktop flows use the helper. The desktop batch dialog previously derived one suggestion from the first extracted URL and sent that same package name for every link.
- The batch adapter is desktop-only: Tauri maps the batch request to CoreServices and React extracts pasted text links. Android UniFFI, CLI, FetchX remote JSON-RPC/UI, and browser/native-host do not expose batch admission.

Before editing the earlier Persepolis phase was revalidated:

- `Aria2Client::get_global_stat` uses the existing authenticated RPC call and parses the string counters into non-negative integer fields.
- The desktop remote probe calls it only after `getVersion`; a failed stats call leaves the endpoint reported as available and returns a generic status message rather than RPC details. The configured token is neither persisted nor included in the diagnostic result. The UI displays returned aggregate counts/rates.
- `cargo test -p fetchx-downloader global_stat -j 1` passed all 3 focused tests; `cargo test -p fetchx-desktop diagnostic_result_serializes_stably -j 1` passed its 1 test.
- `cargo clippy -p fetchx-downloader -p fetchx-desktop --all-targets -- -D warnings` was rerun and reproduced the same two warnings in untouched files: `apps/desktop/src-tauri/src/app.rs:297` (`collapsible_if`) and `apps/desktop/src-tauri/src/commands.rs:1032` (`too_many_arguments`). They are not addressed by this phase.

## Capability comparison

| Capability | JDownloader official documentation evidence | FetchX before this phase | Classification |
|---|---|---|---|
| Extract explicit links from clipboard/pasted text | Official LinkGrabber article says LinkCollector extracts URLs from clipboard text | Shared Rust text extractor and desktop paste/batch path exist | Already implemented for explicit text links |
| Filter, sort, and package collected links | Official article lists these as LinkGrabber concepts | Queue filtering/order exists elsewhere; batch applies one package to every item, while single-item package suggestions are host-derived | Partially implemented; per-source package grouping selected |
| Package metadata beyond a directory name | Official article documents a download directory, password, and additional package information | FetchX package behavior is currently destination-folder grouping only | Requires further investigation; not selected |
| Deep decryption / recursive page discovery | Official article says an explicit deep-decryption action loads an unknown URL and searches its source | Explicit URL extraction and single-URL media extraction exist; no general recursive page crawl | Missing; not selected because it introduces network-fetch and security scope |
| Duplicate detection | The reviewed official LinkGrabber article does not specify the exact duplicate algorithm | Shared normalized single-URL lookup and intra-batch duplicate detection exist; persisted duplicate suppression is not part of batch admission | Requires further investigation; no upstream-specific claim |
| Plugin-based hoster and resolver system | Official IDE page identifies source checkout locations, but inaccessible source prevents verification of implementation details or license | FetchX has a small shared URL-resolver surface, not a general hoster plugin ecosystem | Requires further investigation; a plugin framework is not appropriate for this scope |
| Batch validation and transfer to the queue | LinkGrabber documentation presents collecting and packaging before downloads; exact current source lifecycle could not be verified | CoreServices validates and independently admits explicit URL batches with partial outcomes | Already implemented for the current desktop batch workflow |

## Selected capability: per-source package grouping for batch links

JDownloader's official documentation establishes package organization as a LinkGrabber concept, while FetchX already has a shared package-name suggestion helper. The missing behavior was to apply that per-link core rule across a pasted batch: previously the desktop dialog suggested a package using only the first URL and assigned the entire batch to that one folder.

The new behavior is explicit and bounded:

1. If no package override is supplied for a batch request, CoreServices derives each item's package name from the shared `suggest_download_package_name` helper and sanitizes it before using it as a directory.
2. Items with the same suggested package share a folder; other source suggestions form separate package folders.
3. If the user edits the package field, the desktop adapter sends that value as an explicit shared package override.
4. Existing filename collision handling, validation, per-item results, queue lifecycle, and APIs outside the batch request remain unchanged.

This is an independently implemented package policy based on FetchX's existing helper and the documented LinkGrabber packaging concept; it does not reproduce an unverified JDownloader algorithm.

## Implementation and adapter status

- `crates/core-services/src/lib.rs`: adds `CoreServices::enqueue_download_batch_by_source`, which derives a suggested package for requests without an explicit name and delegates to the existing batch validator, collision handling, and queue lifecycle.
- `apps/desktop/src-tauri/src/commands.rs`: calls the new operation when the batch has no explicit package override; explicit package requests continue through the existing `enqueue_download_batch`.
- `apps/desktop/ui/src/components/AddDialog.tsx`: distinguishes the displayed automatic suggestion from a manually edited value; the hint explains how to group a pasted batch or force one shared package.
- The public `BatchDownloadRequest` shape and existing `enqueue_download_batch` behavior remain unchanged. The new Rust CoreServices method is additive; no Tauri command argument, remote RPC operation, browser RPC method, Android UniFFI contract, CLI command, stored model, migration, or dependency was added.

| Target | Status |
|---|---|
| Rust shared core | Owns the package policy, validation/sanitization, and destination selection. |
| Windows/macOS/Linux desktop | Existing Tauri batch adapter and React dialog use source grouping by default; a manually edited package overrides it. Validation in this environment is on Windows only. |
| Android | No batch adapter exists; the Android single-download package suggestion is unchanged. |
| CLI | No batch command exists; unchanged. |
| FetchX remote JSON-RPC and remote UI | No batch method exists; unchanged. |
| Browser extension/native host | No batch method exists; unchanged. |

## Persistence, safety, and limitations

Package grouping only selects a subdirectory under the configured downloads directory. It adds no database fields or migration; the existing destination path is persisted by the ordinary download record. Suggested names pass through the existing portable-name sanitization before filesystem use. The change performs no new network requests, page crawling, credential handling, or metadata persistence.

Official JDownloader source, current revision, tests, and exact license could not be verified because the official SVN endpoint returned HTTP 401. The GitHub archive's revision and lack of license metadata are documented above; it is not treated as authoritative. No JDownloader source was copied. The official LinkGrabber wiki page is unversioned and may be outdated.

The source grouping key follows FetchX's existing suggestion helper, not a verified JDownloader algorithm. It groups by the helper's suggestion (which is host-derived for valid URLs), not by registrable domain or package metadata. Rich package metadata, duplicate filtering against persisted downloads, deep decryption, and hoster plugins remain deferred.

## Validation

Validation results:

- `cargo test -p fetchx-core-services batch_ -j 1`: passed, 5 focused tests.
- `cargo test -p fetchx-core-services -j 1`: passed, 30 unit tests, 1 persisted-duplicate integration test, 1 media-extraction integration test, and 1 doctest; 4 live-extraction tests were ignored.
- `cargo test -p fetchx-desktop -j 1`: passed all 60 desktop Rust tests.
- `npm run build --prefix "D:\Projects\FetchX\apps\desktop\ui"`: passed TypeScript checking and the Vite production build.
- `cargo clippy -p fetchx-core-services --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Prior-phase verification: `cargo test -p fetchx-downloader global_stat -j 1` passed 3 tests and `cargo test -p fetchx-desktop diagnostic_result_serializes_stably -j 1` passed 1 test.
- `cargo test -p fetchx-downloader -j 1`: passed 283 unit tests and 2 native-torrent lifecycle tests; 3 unit tests were ignored. `cargo clippy -p fetchx-downloader --all-targets -- -D warnings` passed.
- `cargo clippy -p fetchx-downloader -p fetchx-desktop --all-targets -- -D warnings` reproduced the two existing Clippy warnings described above. No live JDownloader instance, official JDownloader source checkout, network page crawl, Android device, or cross-platform desktop build was run.

Recommended next step after this phase: continue the previously recorded Gopeed investigation of the full `resolve_task` contract for protocol-specific metadata and file selection; do not infer that this package grouping implements page discovery or per-link metadata resolution.

# Phase 28 — Performance, Resource Management & Scalability

## Status

**COMPLETE — SCOPE-SPECIFIC / ENVIRONMENT-LIMITED.** The desktop HTTP and
queue validation of existing shared-core behavior completed successfully.
No Phase 28 production defect was identified. The report does not claim
Android or real-torrent performance validation, and this result does not
implement or validate the proxy/routing features listed under Phase 28 in the
roadmap.

Measurement date: 2026-11-01.

## Environment

- OS: Windows 11 Home Single Language, build 26300, x86_64.
- CPU: AMD Ryzen 5 5600H with Radeon Graphics, 6 cores / 12 logical CPUs.
- RAM: 7,698,688 KiB visible (about 7.34 GiB).
- Benchmark volume: D:, NTFS, about 275 GiB total / 64 GiB free at inspection.
- Rust: `rustc 1.99.0 (b940084d7 2026-09-28)`.
- Cargo: `cargo 1.99.0 (5f94df478 2026-08-27)`.
- Android: no device or emulator connected for the performance validation.
- Torrent peer/seeder: no reproducible local peer/seeder environment was
  available.

## Benchmark Methodology

- Runner: `.\scripts\phase28-benchmark.ps1`; `-Extended` selects three
  repetitions and 1/2/3/6/12-download concurrency. `-SustainedSeconds N`
  executes repeated three-download plus 100-item queue cycles.
- Workloads use a deterministic loopback HTTP fixture with FetchX's actual
  `DirectHttpEngine`, `QueueManager`, and SQLite repository. Queue stress uses
  small 4 KiB payloads.
- SHA-256 and exact size were checked for single-download repetitions and the
  pause/resume payload. Queue-scale status and unique IDs were checked; hashes
  were not calculated for every queue-scale payload.
- Process memory, CPU time, handles, and threads are sampled from Windows
  process APIs every 250 ms. Windows `PhysicalDisk(_Total)` counters include
  all system processes. `Win32_Process` I/O transfer counters are cumulative
  process-level I/O, not guaranteed physical-disk-only bytes.
- Benchmark-only loopback access is opt-in. Production defaults continue to
  reject local/private targets, and redirects do not inherit the benchmark
  exception.

## HTTP Throughput

Each payload was downloaded three times. All nine completed with exact size
and matching deterministic SHA-256.

| Payload | Duration min / median / avg / max (ms) | Throughput min / median / avg / max (MiB/s) | SHA-256 |
|---|---:|---:|---|
| 1 MiB | 14 / 18 / 17.7 / 21 | 47.341 / 53.628 / 56.082 / 67.277 | `6078837668115dc3f7b2b24c8a8978ec534583fadd5c80b77c95a00d0cf51433` |
| 10 MiB | 23 / 23 / 29.0 / 41 | 240.037 / 416.952 / 358.286 / 417.870 | `d11f37a6ef5a8889d6b73198d9d4f246ffbb047d2ac8a0d6656a9df92ca416da` |
| 100 MiB | 164 / 173 / 174.3 / 186 | 537.273 / 577.891 / 573.835 / 606.342 | `c94e82e0c7a5be6fc8233e61e630ed010264326349ee9a629667b6e4385641bb` |

These are local loopback results, not internet or Android throughput.

## Concurrency

The global connection limit was 3; worker limits were set to each row's
`max_configured`.

| Items | Workers configured / observed | Completed | Duration | Aggregate throughput | Peak fixture connections |
|---:|---:|---:|---:|---:|---:|
| 1 | 1 / 1 | 1 | 83 ms | 96.065 MiB/s | 3 |
| 2 | 2 / 2 | 2 | 114 ms | 139.849 MiB/s | 3 |
| 3 | 3 / 3 | 3 | 142 ms | 168.572 MiB/s | 3 |
| 6 | 3 / 3 | 6 | 313 ms | 153.336 MiB/s | 3 |
| 12 | 3 / 3 | 12 | 525 ms | 182.692 MiB/s | 3 |

All items completed without failure and configured worker/connection caps were
respected. Segmented jobs can allocate multiple connection-budget units, so a
one-worker run may use all three connections.

## Queue Scalability

| Queue size | Enqueue | First request | Total completion | Avg per-item completion latency | Max sampled queued depth | Peak workers / fixture connections | Completed |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 2 ms | 3 ms | 67 ms | 46.36 ms | 7 | 3 / 3 | 10 |
| 100 | 13 ms | 3 ms | 659 ms | 348.11 ms | 97 | 3 / 3 | 100 |
| 1,000 | 119 ms | 2 ms | 7,077 ms | 3,595.64 ms | 977 | 3 / 3 | 1,000 |

All item IDs were unique; no failed/cancelled queue items were reported. Queue
snapshots showed up to 6 allocated connection-budget units, while live workers
and server-observed connections remained at or below 3. Persistence-operation
counts and durable on-disk database recovery were not measured; the harness
repository is in-memory SQLite.

## Priority and Fairness

Observed request start order:

`high-0, high-1, high-2, low-0, high-3, high-4, high-5, low-1, high-6, high-7, normal-0, normal-1, high-arrival-0..5`

- Eight HIGH requests started before the first NORMAL request.
- The first LOW request started at position 3; both LOW requests received
  service.
- No NORMAL request started before the first LOW request.
- The most delayed LOW start was 1,735 ms after queue release.
- The fairness rule under test passed. This is one workload, not a general
  starvation bound.

## Cancellation

- Three rounds each cancelled three active downloads: 9/9 reached
  `Cancelled`; request-to-return measurements were 0–1 ms.
- After each round the queue had zero active workers and the fixture had zero
  active connections.
- In a separate active-plus-queued check, both items reached `Cancelled`, the
  queued item's request was never observed, and active workers/connections
  returned to zero.
- A 200–250 ms delayed observation saw no queued restart.
- Two `.part` paths existed immediately after cancellation; harness shutdown
  removed the per-run temporary directory. Partial-file retention policy was
  not changed.
- Exact downloader task counts were not available and remain **NOT MEASURED**.

## Pause / Resume

- Two pauses reached persisted `Paused` in 0–1 ms.
- Progress was 1,048,576 bytes then 2,162,688 bytes; it did not advance during
  a 100 ms paused observation.
- Active queue workers after pause: zero.
- Resume-to-next-progress latency: 247 ms and 251 ms.
- The final 4 MiB payload completed with matching SHA-256; maximum same-download
  fixture workers: 1.

## Retry Semantics

Transient HTTP status failures are automatically retried under the existing
bounded policy. The latest direct benchmark run observed one HTTP 503 retry
completing in two requests (294 ms between attempts). Repeated HTTP 503
responses exhausted the four-attempt bound, with observed gaps of 363, 726,
and 1,194 ms, and ended `Failed`.

Safe manual/range-based resume is a separate behavior. The existing
`resumes_using_actual_partial_size_and_validates_final_content` test verifies
that a later user retry can request the actual partial offset using `Range`
and a matching ETag and validate the completed content. That test does not
mean a mid-body interruption is automatically retried.

## Mid-Body Interruption

The deterministic fixture starts an HTTP 200 response with a declared
131,072-byte body, sends 65,536 bytes, and then closes the connection. The
latest direct run observed:

```text
HTTP response begins
→ connection/body interrupted
→ decode error (non-transient)
→ no automatic retry (one request, zero retries)
→ 65,536-byte partial file preserved
→ persisted Failed state
→ workers/connections cleaned up (zero / zero)
```

This is **intentional current retry semantics**, not an unresolved retry
defect. The response stream reports a decode error after response headers
have been accepted. `RetryPolicy::is_transient_request` retries timeout,
connect, request, and body errors, but not decode errors; the send/retry loop
does not wrap response-body streaming. The focused
`truncated_response_fails_and_preserves_the_partial_file` test asserts decode
classification, non-transient policy, exactly one request, persisted
`Failed`, no published destination, and preservation of the 7-byte `.part`
file. No production retry behavior was changed.

## CPU Measurements

The final extended run sampled 37 process observations at 250 ms intervals.
Whole-run average CPU was 8.73% across 12 logical CPUs; sampled peak was
22.18%. Handles peaked at 192 and threads at 84. Per-stage windows were short
(often only two samples), so those stage-level CPU estimates are low
confidence. The idle sample was 4.89% average / 9.44% peak (two samples);
three-concurrent was 18.34% / 20.56%; and the 1,000-item queue stage was
14.51% / 22.18%. A reliable single-download-only CPU window was not captured.

## Memory Measurements

| Metric | Baseline | Peak | Final |
|---|---:|---:|---:|
| Working set (RSS) | 4.71 MiB | 33.37 MiB | 24.05 MiB |
| Private bytes | 0.94 MiB | 28.23 MiB | 21.05 MiB |

Memory after work remained above its initial sample. Repeated cycles completed,
but no per-cycle memory trend or leak detector was used: absence of a memory
leak is **NOT PROVEN**. No process crash was observed.

## Disk I/O

Windows physical-disk counters were readable and sampled 297 times in the
sustained run (0 counter errors). Whole-run system-wide average read/write
rates were approximately 7.2 MB/s and 29.6 MB/s, with peaks of approximately
23.1 MB/s and 222.1 MB/s. These include unrelated OS and process activity.

During the final extended (non-sustained) run, process-level Windows I/O
transfer deltas were about 55.5 MB read and 785.9 MB written (0 query errors).
The sustained run used an earlier sampler without usable process-level I/O
deltas. These are OS transfer counters, not verified physical media bytes.
Exact FetchX disk throughput is **NOT MEASURED**; exact per-process physical
disk I/O is not isolated.

## Sustained Stress

Requested target: **10 minutes (600 seconds)**.

Actual validated duration: **583.3 seconds** in the repeated-cycle stage;
**625.7 seconds** for the full benchmark process, including startup, baseline,
and other scenarios. The cycle stage completed 282 iterations. Each contained
three concurrent 8 MiB downloads followed by 100 queued 4 KiB downloads.

All recorded component downloads completed with worker/connection caps intact;
no crash occurred and the harness exited normally. Classification:
**PARTIAL / NOT FULL TARGET**. The requested 600 seconds of repeated cycles
was not reached. The observed workload showed no reported queue/connection
leak, but durable database integrity, OS-level task counts, per-cycle memory
growth, and every queue payload hash were not checked.

## Torrent Validation

**REAL TORRENT PERFORMANCE: NOT VALIDATED / ENVIRONMENT UNAVAILABLE.** No
reproducible local peer/seeder performance workload was available. Existing
lifecycle tests pre-populate their payload and do not prove a peer transfer;
torrent lifecycle validation is not substituted for torrent performance.

## Android Validation

**ANDROID PERFORMANCE: NOT VALIDATED / ENVIRONMENT UNAVAILABLE.** No connected
device or emulator was available for this performance validation. No APK
performance workload was run. Android startup validation is not substituted
for Android performance.

## Resource Cleanup

The benchmark observed active queue workers and fixture connections return to
zero after cancellation, pause, and the interrupted-response failure. The
interruption retained its partial file until harness cleanup; cancellation
also left partial paths until the harness removed its temporary directory.
Exact downloader task counts and formal leak absence were not measured.

## Security

- `benchmark_exception_allows_only_loopback_targets`: PASS.
- `rejects_local_private_network_targets`: PASS.
- The benchmark feature is opt-in and permits loopback only; default
  production URL checks continue to reject local/private targets.
- Redirect checks do not allow the initial loopback exception to carry
  forward.

## Quality Gates

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --all-targets --jobs 1` | PASS |
| `cargo test --workspace --jobs 1` | PASS |
| Changed-crate Clippy (downloader and queue, all targets/features), allowing only baseline `clippy::collapsible_if` | PASS |
| Strict workspace Clippy | BLOCKED by pre-existing `clippy::collapsible_if` in `crates/models/src/progress.rs:21` |
| `git diff --check` | PASS |

## Limitations

- Android performance was not validated because no device/emulator was
  available.
- Real torrent performance was not validated because no reproducible peer or
  seeder environment was available.
- The sustained cycle stage ran 583.3 seconds, short of the requested
  10-minute target; it is **PARTIAL / NOT FULL TARGET**.
- Memory-leak absence is not formally proven.
- Exact per-process physical disk I/O is not isolated; system counters include
  unrelated activity.
- Durable on-disk database recovery, exact downloader task counts, and
  per-file hashes for every queue-scale item were not tested.
- Strict workspace Clippy remains blocked by the unrelated existing lint at
  `crates/models/src/progress.rs:21`; changed-crate Clippy passed.
- This report validates desktop performance/resource behavior only. It does
  not implement or validate proxy, SOCKS5, DNS-policy, or Tor-routing roadmap
  features.

## Final Classification

**COMPLETE — SCOPE-SPECIFIC / ENVIRONMENT-LIMITED.** Desktop HTTP throughput,
concurrency, queue scalability through 1,000 items, fairness, cancellation,
pause/resume, bounded retries, mid-body interruption semantics, resource
cleanup, security isolation, and the available CPU/memory/disk measurements
were executed and documented. No unresolved Phase 28 production defect
remains. Android and real-torrent performance were unavailable, sustained
cycles did not meet the full 10-minute target, and memory-leak absence and
exact per-process physical disk I/O remain unproven. These limitations are
explicitly scoped rather than reported as passing validation.

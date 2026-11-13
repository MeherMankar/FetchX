# Resource policy

Resource profiles are persisted in the shared Rust `EngineConfig` and applied
when CoreServices starts. Profile values are ceilings and policy defaults;
they do not guarantee a download speed or worker count.

| Profile | Active downloads | Global connections | Per-download connections | Global rate | Per-download rate |
| --- | ---: | ---: | ---: | ---: | ---: |
| Unlimited | 8 | 64 | 32 | Unlimited | Unlimited |
| Normal | 3 | 16 | 8 | Unlimited | Unlimited |
| Limited | 2 | 8 | 4 | 2 MiB/s | 1 MiB/s |
| Night | 2 | 4 | 2 | 512 KiB/s | 256 KiB/s |
| Custom | User configured | User configured | User configured | User configured | User configured |

The queue accounts for connections across active downloads and does not exceed
the configured global budget. Direct HTTP and HLS use shared Rust bandwidth
limiters; segmented HTTP and HLS fragments share their download's limiter.
aria2 and yt-dlp receive native per-download rate limits. Their assigned rates
include a conservative share of the global rate so they remain accounted for
alongside built-in Rust transfers.

Changes saved in desktop settings currently take effect after restarting
FetchX. Active transfers retain their existing controls. Runtime reconfiguration
is not yet exposed as a full-policy operation; `QueueManager` currently exposes
separate connection-limit and normalized pressure-signal operations.

The queue accepts `Normal`, `Elevated`, `High`, and `Critical` pressure signals.
Elevated and High pressure reduce new starts/allocation; Critical blocks new
work. Existing downloads are not terminated by pressure signals. Automatic
Windows resource monitoring and Android lifecycle/memory-signal adapters are
not implemented; platform adapters remain future work.

Native media processing is local work, not a queued network downloader. It does
not consume resource-policy bandwidth settings. Processing concurrency is bounded
by the processor's own `FfmpegConfig` semaphore; the desktop resource profile
does not currently configure that separate processor.

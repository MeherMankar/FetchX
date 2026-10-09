# MyDownloader

MyDownloader is the foundation of a cross-platform download manager. The long-term
goal is to provide a desktop application and CLI, with a Rust core that can support
download queues, resumability, external engines, and other download-manager features.

## Development status

This repository currently contains the Rust workspace foundation, engine-independent
domain models, SQLite persistence, and a deterministic download lifecycle state
machine. It does **not** download files. No UI, download engine, or integrations with
aria2, yt-dlp, or FFmpeg are implemented yet.

## Technology stack

- Rust (stable toolchain, edition 2024)
- Tokio for asynchronous operations
- Serde and Serde JSON for serialization
- SQLx with SQLite support for the planned storage layer
- Tauri 2 and React + TypeScript are planned for the desktop application

The dependency versions and shared configuration are centralized in the root
`Cargo.toml`. Crate manifests opt into dependencies as the corresponding functionality
is implemented.

## Workspace

- `crates/models` — shared domain and data structures
- `crates/core` — application logic and persisted download state transitions
- `crates/storage` — SQLx/SQLite persistence for download records
- `crates/downloader` — downloader abstraction and engine implementations
- `migrations` — embedded, ordered SQLite schema migrations
- `tests` — workspace-level integration tests
- `docs` — architecture, roadmap, and AI contribution rules

## Build

Install the stable Rust toolchain with `rustup`, then run from the repository root:

```sh
cargo check --workspace
```

## Tests

Run the crate smoke tests and any future workspace tests with:

```sh
cargo test --workspace
```

The storage crate accepts a caller-provided SQLite URL and applies embedded
migrations during connection, for example `sqlite://mydownloader.db`. In-memory
SQLite databases are also supported for tests. The repository API is documented
through the `DownloadRepository` trait; database setup is provided by
`Database::connect`.

Before submitting changes, also run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
[docs/ROADMAP.md](docs/ROADMAP.md), and [docs/AI_RULES.md](docs/AI_RULES.md).

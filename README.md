# FetchX

**One download platform. Native apps. A shared Rust core.**

FetchX is an open-source download manager built around a simple idea: your
download queue and its business logic should not be reimplemented for every
device. A shared Rust core powers FetchX clients and services, with native
interfaces for desktop and Android, plus a command-line client and an optional
remote server.

> **In active development:** FetchX is not yet a stable, general-purpose
> release. Features and polish vary by platform. Browser extensions are
> currently installed for development rather than through browser stores.

## Download FetchX

**[Download the latest release](https://github.com/MeherMankar/FetchX/releases/latest)** ·
[Browse all releases](https://github.com/MeherMankar/FetchX/releases)

Release assets include:

- **Android:** APK
- **Windows:** `.exe` and `.msi` desktop installers
- **Linux:** `.deb` and `.AppImage` desktop packages
- **macOS:** `.dmg` desktop package
- **CLI:** binaries for Windows, Linux, and macOS

Choose the asset for your platform on the release page. SHA-256 checksum files
are published alongside packages. Android signing and macOS signing/notarization
depend on release configuration; check the selected release notes before
installing.

## Why FetchX?

- **A queue you can trust.** Persistent downloads, priority ordering, pause,
  resume and cancel controls, scheduled starts, and recovery of interrupted
  work.
- **Built for real transfers.** Direct HTTP(S) downloads support safe
  range-based resume and adaptive segmented transfers when the server supports
  them.
- **Clients that share the same engine.** Rust owns download and queue logic;
  Android, desktop, CLI, and remote controls use the shared services rather
  than maintaining separate download implementations.
- **More than a command-line tool.** Use the Android app, desktop app, CLI, or
  authenticated headless server. Browser integrations can send downloads to a
  running FetchX desktop app.
- **Useful media and browser tools.** FetchX includes native media operations
  for supported formats and an Android GeckoView browser with privacy controls
  and extension support.
- **Protective defaults.** Remote control is loopback-first, and download
  requests validate destinations, including redirect targets. Unsupported
  operations fail explicitly instead of pretending to succeed.

## Choose how to use it

| Client | What it is for |
| --- | --- |
| **Android** | Native download manager and GeckoView browser, backed by the shared Rust core. |
| **Desktop** | Tauri desktop app for managing downloads and queue activity. |
| **CLI** | Add, inspect, schedule, and control downloads from a terminal or script. |
| **Remote server** | Run FetchX headlessly and control supported operations through authenticated RPC. |
| **Browser extension** | Send links from Chrome, Edge, or Firefox to FetchX; store distribution is not available yet. |

Platform integrations are at different stages of development. See the
[roadmap](docs/ROADMAP.md) for current direction and the platform documentation
for implementation details.

## Get started

### Try the CLI

Install the [Rust stable toolchain](https://www.rust-lang.org/tools/install)
(FetchX requires Rust 1.95 or newer), then run these commands from the repository
root:

```sh
cargo run -p fetchx -- --help
cargo run -p fetchx -- add https://example.com/file.zip
cargo run -p fetchx -- list
```

The CLI uses FetchX's normal application data and downloads locations. Use a URL
you trust and have permission to download.

### Build the desktop UI

The desktop frontend uses Node.js and npm:

```sh
npm --prefix apps/desktop/ui ci
npm --prefix apps/desktop/ui run build
```

Building the full Tauri application also requires the platform's native
dependencies. For Android, set up JDK 17, the Android SDK/NDK, and `cargo-ndk`.
See the [Android architecture guide](docs/ANDROID_ARCHITECTURE.md).

### Run the remote server

Build the CLI, initialize a bearer token once, then start the server:

```sh
cargo build -p fetchx
cargo run -p fetchx -- server init-auth
cargo run -p fetchx -- server
```

The server listens on loopback by default. `init-auth` displays the token once;
store it somewhere safe. Before allowing access from another machine, read the
[remote server guide](docs/REMOTE_SERVER.md) and
[Docker deployment guide](docs/DOCKER_DEPLOYMENT.md).

## How it fits together

```text
Android / Desktop / CLI / Remote clients / Browser integrations
                              |
                              v
                Shared Rust services and protocols
                              |
             +----------------+----------------+
             v                v                v
           Queue          Scheduler          Storage
                              |
                              v
                   Download and media engines
```

The workspace is split into focused crates for models, core lifecycle logic,
storage, downloading, queueing, scheduling, protocols, and platform adapters.
SQLite stores persistent download state; ordered migrations are in `migrations/`.
Learn more in the [architecture guide](docs/ARCHITECTURE.md).

## Development

Common checks from the repository root:

```sh
cargo fmt --all -- --check
cargo check --workspace --exclude fetchx-desktop
cargo test --workspace --exclude fetchx-desktop
```

These commands cover the Rust workspace without the Tauri desktop package;
desktop builds also need platform-specific native dependencies.
The desktop UI and browser extension have their own npm build steps. See
[browser extension setup](docs/BROWSER_EXTENSION.md), the
[extension README](apps/browser/extension/README.md), and the
[CLI guide](docs/CLI.md).

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Roadmap and project status](docs/ROADMAP.md)
- [CLI reference](docs/CLI.md)
- [Remote server and security model](docs/REMOTE_SERVER.md)
- [Docker and NAS deployment](docs/DOCKER_DEPLOYMENT.md)
- [Browser integration](docs/BROWSER_EXTENSION.md)
- [Android architecture](docs/ANDROID_ARCHITECTURE.md)
- [Release process](docs/RELEASING.md)

## License

FetchX is licensed under the [Apache License 2.0](LICENSE). Third-party
dependencies and bundled assets may have their own license terms.

---

If FetchX looks useful, try it, explore the roadmap, and share feedback through
[GitHub Issues](https://github.com/MeherMankar/FetchX/issues).

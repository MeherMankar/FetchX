//! FetchX CLI — Phase 18
//!
//! Thin command-line interface over the existing Rust core services.
//! No download logic lives here; everything delegates to the shared
//! `fetchx-core-services` crate (queue, scheduler, engines, storage).

#![forbid(unsafe_code)]

use std::process;

mod args;
mod bootstrap;
mod commands;
mod exit;
mod output;

use args::{Cli, Command};

#[tokio::main]
async fn main() {
    // Diagnostics to stderr; stdout is reserved for command output / JSON.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_owned()))
        .init();

    let cli = <Cli as clap::Parser>::parse();
    let json = cli.json;

    match run(cli).await {
        Ok(code) => process::exit(code as i32),
        Err(e) => {
            output::print_error(json, &e.to_string());
            process::exit(exit::GENERAL_ERROR as i32);
        }
    }
}

async fn run(cli: Cli) -> Result<exit::ExitCode, Box<dyn std::error::Error>> {
    use Command::*;

    match cli.command {
        Version => {
            commands::version::run(cli.json);
            Ok(exit::SUCCESS)
        }
        Engines => commands::engines::run(cli.json).await,
        Config => commands::config::run(cli.json).await,
        Add(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::add::run(&svc, opts, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        List(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::list::run(&svc, opts, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Status(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::status::run(&svc, opts, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Pause(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::lifecycle::pause(&svc, opts.id, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Resume(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::lifecycle::resume(&svc, opts.id, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Cancel(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::lifecycle::cancel(&svc, opts.id, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Remove(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::lifecycle::remove(&svc, opts.id, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Retry(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::lifecycle::retry(&svc, opts.id, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Priority(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::priority::run(&svc, opts, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Schedule(opts) => {
            let svc = bootstrap::start().await?;
            let code = commands::schedule::run(&svc, opts, cli.json).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Monitor => {
            let svc = bootstrap::start().await?;
            let code = commands::monitor::run(&svc).await;
            svc.shutdown().await?;
            Ok(code)
        }
        Plugins(opts) => Ok(commands::plugins::run(opts, cli.json).await),

        Torrent(opts) => {
            let svc = bootstrap::start().await?;
            use args::TorrentCommand;
            let code = match opts.command {
                TorrentCommand::Status { id } => {
                    commands::torrent::status(&svc, id, cli.json).await
                }
                TorrentCommand::Files { id } => commands::torrent::files(&svc, id, cli.json).await,
                TorrentCommand::Select { id, files } => {
                    commands::torrent::select_files(&svc, id, files, cli.json).await
                }
                TorrentCommand::StopSeeding { id } => {
                    commands::torrent::stop_seeding(&svc, id, cli.json).await
                }
            };
            svc.shutdown().await?;
            Ok(code)
        }
    }
}

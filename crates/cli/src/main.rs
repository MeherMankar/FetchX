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
        Server(opts) => {
            let defaults = fetchx_remote_server::ServerConfig::default();
            let database_path = opts
                .database
                .unwrap_or_else(|| defaults.database_path.clone());
            if let Some(action) = opts.action {
                match action {
                    args::ServerAction::InitAuth => {
                        let token = fetchx_remote_server::initialize_auth(&database_path).await?;
                        print_auth_token(token, cli.json);
                    }
                    args::ServerAction::RotateAuth => {
                        let token = fetchx_remote_server::rotate_auth(&database_path).await?;
                        print_auth_token(token, cli.json);
                    }
                    args::ServerAction::ShowRole => {
                        let role = fetchx_remote_server::current_auth_role(&database_path).await?;
                        if cli.json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "status": "ok",
                                    "role": role.as_str()
                                })
                            );
                        } else {
                            println!("Primary credential role: {}", role.as_str());
                        }
                    }
                    args::ServerAction::SetRole { role } => {
                        let role = fetchx_remote_server::Role::parse(&role)
                            .ok_or("unsupported authorization role")?;
                        fetchx_remote_server::set_auth_role(&database_path, role).await?;
                        if cli.json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "status": "authorization_role_updated",
                                    "role": role.as_str()
                                })
                            );
                        } else {
                            println!("Primary credential role updated to {}.", role.as_str());
                        }
                    }
                }
                return Ok(exit::SUCCESS);
            }
            let config = fetchx_remote_server::ServerConfig {
                bind_ip: opts.bind,
                port: opts.port,
                database_path,
                downloads_dir: opts.downloads.unwrap_or(defaults.downloads_dir.clone()),
                runtime_dir: defaults.runtime_dir,
                max_request_bytes: defaults.max_request_bytes,
                max_connections: defaults.max_connections,
                request_timeout: defaults.request_timeout,
                event_backlog: defaults.event_backlog,
            };
            let server = fetchx_remote_server::RemoteServer::start(config).await?;
            tracing::info!(address = %server.local_addr(), "FetchX remote server listening");
            server.run_until(shutdown_signal()).await?;
            Ok(exit::SUCCESS)
        }
    }
}

fn print_auth_token(token: String, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "status": "authentication_updated",
                "token": token,
                "warning": "store this token securely; it will not be displayed again"
            })
        );
    } else {
        println!(
            "Authentication credential updated.\n\nToken (shown only once):\n{token}\n\nStore it securely. It will not be displayed again."
        );
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result {
                            tracing::error!(%error, "failed to listen for Ctrl+C");
                        }
                    }
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to install SIGTERM handler");
                if let Err(error) = tokio::signal::ctrl_c().await {
                    tracing::error!(%error, "failed to listen for Ctrl+C");
                }
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for Ctrl+C");
    }
}

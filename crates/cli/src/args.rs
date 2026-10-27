//! CLI argument definitions using clap.

use clap::{Args, Parser, Subcommand};
use std::{net::IpAddr, path::PathBuf};

/// FetchX — a fast, extensible download manager.
#[derive(Parser, Debug)]
#[command(
    name = "fetchx",
    version  = env!("CARGO_PKG_VERSION"),
    about    = "FetchX command-line interface",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Output machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Print version information.
    Version,

    /// Show engine availability.
    Engines,

    /// Show non-sensitive configuration.
    Config,

    /// Add a download to the queue.
    Add(AddOpts),

    /// List downloads.
    List(ListOpts),

    /// Show detailed status of one download.
    Status(IdOpts),

    /// Pause a download.
    Pause(IdOpts),

    /// Resume a paused download.
    Resume(IdOpts),

    /// Cancel a download.
    Cancel(IdOpts),

    /// Remove a download record (files are preserved).
    Remove(IdOpts),

    /// Retry a failed or cancelled download.
    Retry(IdOpts),

    /// Change a download's priority.
    Priority(PriorityOpts),

    /// Set a one-shot start time for a queued download.
    Schedule(ScheduleOpts),

    /// Monitor active downloads in real time (Ctrl+C to exit).
    Monitor,

    /// Plugin management commands.
    Plugins(PluginsOpts),

    /// Torrent-specific commands.
    Torrent(TorrentOpts),

    /// Run the unauthenticated local remote-control server.
    Server(ServerOpts),
}

#[derive(Args, Debug)]
pub struct ServerOpts {
    /// Bind address. Defaults to localhost; use a LAN address only on a trusted network.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: IpAddr,

    /// TCP port.
    #[arg(long, default_value_t = fetchx_remote_server::DEFAULT_PORT)]
    pub port: u16,

    /// SQLite database path (defaults to the standard FetchX data directory).
    #[arg(long)]
    pub database: Option<PathBuf>,

    /// Download directory (defaults to ~/Downloads).
    #[arg(long)]
    pub downloads: Option<PathBuf>,
}

// ── Plugins ────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PluginsOpts {
    #[command(subcommand)]
    pub command: Option<PluginsCommand>,
}

#[derive(Subcommand, Debug)]
pub enum PluginsCommand {
    /// List all registered plugins.
    List,
    /// Show information about one plugin.
    Info {
        /// Plugin ID.
        id: String,
    },
    /// Show the runtime status of one plugin.
    Status {
        /// Plugin ID.
        id: String,
    },
}

// ── Add ──────────────────────────────────────────────────────────────────

/// Options accepted by the `add` command.
#[derive(Args, Debug)]
pub struct AddOpts {
    /// HTTP(S) URL or local .torrent file path.
    #[arg(
        value_name = "URL_OR_TORRENT",
        required_unless_present = "magnet",
        conflicts_with = "magnet"
    )]
    pub url: Option<String>,

    /// Magnet URI. Magnet and .torrent downloads use aria2.
    #[arg(long, value_name = "URI", conflicts_with = "url")]
    pub magnet: Option<String>,

    /// Engine to use: direct-http | aria2 | yt-dlp. Torrent downloads default to aria2.
    #[arg(short = 'e', long, value_name = "ENGINE")]
    pub engine: Option<String>,

    /// Destination file for HTTP downloads or destination directory for torrents.
    #[arg(long, value_name = "PATH")]
    pub destination: Option<std::path::PathBuf>,

    /// Select 1-based file indexes in a local .torrent, comma-separated (for example: 1,3).
    #[arg(long, value_name = "INDEXES", requires = "url")]
    pub select_files: Option<String>,

    /// Queue priority: low | normal | high
    #[arg(short = 'p', long, value_name = "PRIORITY", default_value = "normal")]
    pub priority: String,

    /// Schedule start time (ISO-8601 or "YYYY-MM-DD HH:MM").
    #[arg(long, value_name = "DATETIME")]
    pub schedule: Option<String>,
}

// ── List ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ListOpts {
    /// Filter by status: all | active | queued | paused | completed | failed | cancelled
    #[arg(short = 'f', long, value_name = "STATUS", default_value = "all")]
    pub filter: String,

    /// Limit number of results.
    #[arg(short = 'n', long, value_name = "N")]
    pub limit: Option<usize>,
}

// ── ID-only commands ──────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct IdOpts {
    /// Download ID (UUID prefix accepted).
    pub id: String,
}

// ── Priority ──────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PriorityOpts {
    /// Download ID.
    pub id: String,

    /// Priority: low | normal | high
    pub priority: String,
}

// ── Schedule ──────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ScheduleOpts {
    /// Download ID.
    pub id: String,

    /// Start at this time (ISO-8601 or "YYYY-MM-DD HH:MM").
    #[arg(long, value_name = "DATETIME")]
    pub at: String,

    /// Remove an existing schedule.
    #[arg(long, conflicts_with = "at")]
    pub clear: bool,
}

// ── Torrent ───────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct TorrentOpts {
    #[command(subcommand)]
    pub command: TorrentCommand,
}

#[derive(Subcommand, Debug)]
pub enum TorrentCommand {
    /// Show detailed torrent status including telemetry (peers, seeds, speeds).
    Status {
        /// Download ID.
        id: String,
    },
    /// List files in a torrent download.
    Files {
        /// Download ID.
        id: String,
    },
    /// Set a magnet file selection (1-based indexes, `all`, `none`, or `dir:path`).
    Select {
        /// Download ID.
        id: String,
        /// Selection expression, for example `all`, `none`, `1,3`, or `dir:folder`.
        #[arg(long, value_name = "SELECTION")]
        files: String,
    },
    /// Stop seeding after a completed torrent download.
    StopSeeding {
        /// Download ID.
        id: String,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::net::Ipv4Addr;

    use super::{Cli, Command, TorrentCommand};

    #[test]
    fn add_accepts_explicit_magnet_option() {
        let cli = Cli::try_parse_from([
            "fetchx",
            "add",
            "--magnet",
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        ])
        .expect("--magnet should satisfy add source requirement");
        let Command::Add(options) = cli.command else {
            panic!("expected add command");
        };
        assert!(options.url.is_none());
        assert!(options.magnet.is_some());
    }

    #[test]
    fn add_accepts_positional_local_torrent_source() {
        let cli = Cli::try_parse_from(["fetchx", "add", "fixture.torrent"])
            .expect("positional torrent path should be accepted");
        let Command::Add(options) = cli.command else {
            panic!("expected add command");
        };
        assert_eq!(options.url.as_deref(), Some("fixture.torrent"));
        assert!(options.magnet.is_none());
    }

    #[test]
    fn torrent_select_accepts_file_selection_expression() {
        let cli = Cli::try_parse_from([
            "fetchx",
            "torrent",
            "select",
            "download-id",
            "--files",
            "dir:folder",
        ])
        .expect("torrent selection command should parse");
        let Command::Torrent(options) = cli.command else {
            panic!("expected torrent command");
        };
        assert!(matches!(
            options.command,
            TorrentCommand::Select { id, files }
                if id == "download-id" && files == "dir:folder"
        ));
    }

    #[test]
    fn torrent_select_accepts_one_based_file_indexes() {
        let cli = Cli::try_parse_from([
            "fetchx",
            "torrent",
            "select",
            "download-id",
            "--files",
            "1,3",
        ])
        .expect("torrent file indexes should parse");
        let Command::Torrent(options) = cli.command else {
            panic!("expected torrent command");
        };
        assert!(matches!(
            options.command,
            TorrentCommand::Select { files, .. } if files == "1,3"
        ));
    }

    #[test]
    fn server_defaults_to_loopback_and_standard_port() {
        let cli = Cli::try_parse_from(["fetchx", "server"])
            .expect("server should use safe localhost defaults");
        let Command::Server(options) = cli.command else {
            panic!("expected server command");
        };
        assert_eq!(options.bind, Ipv4Addr::LOCALHOST);
        assert_eq!(options.port, fetchx_remote_server::DEFAULT_PORT);
        assert!(options.database.is_none());
    }
}

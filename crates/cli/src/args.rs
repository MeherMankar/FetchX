//! CLI argument definitions using clap.

use clap::{Args, Parser, Subcommand};
use std::{net::IpAddr, path::PathBuf};

pub const APP_VERSION: &str = match option_env!("FETCHX_APP_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};

/// FetchX — a fast, extensible download manager.
#[derive(Parser, Debug)]
#[command(
    name = "fetchx",
    version = APP_VERSION,
    about = "FetchX command-line interface",
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

    /// Find a persisted download with the same normalized URL.
    FindDuplicate {
        /// HTTP(S) URL or source URI to check.
        url: String,
    },

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

    /// Move a queued download within its priority group.
    QueueMove(QueueMoveOpts),

    /// Set a one-shot start time for a queued download.
    Schedule(ScheduleOpts),

    /// Monitor active downloads in real time (Ctrl+C to exit).
    Monitor,

    /// Plugin management commands.
    Plugins(PluginsOpts),

    /// Torrent-specific commands.
    Torrent(TorrentOpts),

    /// Database diagnostics, backups, and data transfer.
    Db(DbOpts),

    /// Print a read-only, secret-free diagnostics snapshot.
    Diagnostics {
        /// SQLite database path (defaults to the standard FetchX data directory).
        #[arg(long)]
        database: Option<PathBuf>,
    },

    /// Run the authenticated local remote-control server.
    Server(ServerOpts),

    /// Control a FetchX server through the authenticated remote protocol.
    Remote(RemoteOpts),
}

#[derive(Args, Debug)]
pub struct RemoteOpts {
    /// Remote server origin. Defaults to FETCHX_REMOTE_URL.
    #[arg(long, global = true, value_name = "URL")]
    pub server: Option<String>,

    #[command(subcommand)]
    pub command: RemoteCommand,
}

#[derive(Args, Debug)]
pub struct DbOpts {
    #[command(subcommand)]
    pub command: DbCommand,
}

#[derive(Subcommand, Debug)]
pub enum DbCommand {
    /// Check database integrity and schema without changing it.
    Check {
        /// SQLite database path (defaults to the standard FetchX data directory).
        #[arg(long)]
        database: Option<PathBuf>,
    },
    /// Report whether a database can be safely repaired; never performs destructive recovery.
    Repair {
        /// SQLite database path (defaults to the standard FetchX data directory).
        #[arg(long)]
        database: Option<PathBuf>,
    },
    /// Create and validate a consistent SQLite snapshot.
    Backup {
        /// SQLite database path (defaults to the standard FetchX data directory).
        #[arg(long)]
        database: Option<PathBuf>,
        /// Backup file (defaults to a timestamped sibling of the database).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Export non-secret download and torrent metadata to versioned JSON.
    Export {
        /// SQLite database path (defaults to the standard FetchX data directory).
        #[arg(long)]
        database: Option<PathBuf>,
        /// Export file (defaults to a timestamped JSON sibling of the database).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Replace local download metadata from a validated versioned export.
    Import {
        /// Versioned JSON export file.
        input: PathBuf,
        /// SQLite database path (defaults to the standard FetchX data directory).
        #[arg(long)]
        database: Option<PathBuf>,
        /// Allowed root for imported download and torrent paths.
        #[arg(long)]
        downloads_root: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum RemoteCommand {
    /// Authenticate and verify the remote server connection.
    Connect {
        /// Server origin. Overrides --server and FETCHX_REMOTE_URL.
        url: Option<String>,
    },
    /// Show remote server health and protocol capabilities.
    Status,
    /// List downloads on the remote server.
    Downloads,
    /// Find a remote download with the same normalized URL.
    FindDuplicate {
        /// HTTP(S) URL or source URI to check.
        url: String,
    },
    /// Create a download on the remote server.
    Add {
        /// HTTP(S) URL or magnet URI.
        url: String,
    },
    /// Pause a remote download.
    Pause {
        /// Download ID.
        id: String,
    },
    /// Resume a remote download.
    Resume {
        /// Download ID.
        id: String,
    },
    /// Cancel a remote download.
    Cancel {
        /// Download ID.
        id: String,
    },
    /// Move a queued remote download earlier in its priority group.
    QueueUp {
        /// Download ID.
        id: String,
    },
    /// Move a queued remote download later in its priority group.
    QueueDown {
        /// Download ID.
        id: String,
    },
}

#[derive(Args, Debug)]
pub struct ServerOpts {
    #[command(subcommand)]
    pub action: Option<ServerAction>,

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

    /// TLS certificate PEM file. Must be used with --tls-private-key.
    #[arg(long, requires = "tls_private_key")]
    pub tls_certificate: Option<PathBuf>,

    /// TLS private key PEM file. Must be used with --tls-certificate.
    #[arg(long, requires = "tls_certificate")]
    pub tls_private_key: Option<PathBuf>,

    /// Additional exact browser origin allowed to access the remote UI.
    #[arg(long = "allowed-origin", value_name = "ORIGIN", action = clap::ArgAction::Append)]
    pub allowed_origins: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum ServerAction {
    /// Initialize server authentication and display the token once.
    InitAuth,
    /// Replace the server token and immediately invalidate the old token.
    RotateAuth,
    /// Show the authorization role assigned to the primary credential.
    ShowRole,
    /// Set the primary credential's authorization role using local access.
    SetRole {
        /// Role to assign: admin, operator, or read-only.
        #[arg(value_parser = ["admin", "operator", "read-only"])]
        role: String,
    },
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

#[derive(Args, Debug)]
pub struct QueueMoveOpts {
    /// Download ID.
    pub id: String,

    /// Move direction.
    #[arg(value_enum)]
    pub direction: QueueDirection,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueDirection {
    Up,
    Down,
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
    use std::{
        net::Ipv4Addr,
        path::{Path, PathBuf},
    };

    use super::{
        Cli, Command, DbCommand, QueueDirection, RemoteCommand, ServerAction, TorrentCommand,
    };

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
    fn queue_move_accepts_direction_and_remote_queue_actions() {
        let local = Cli::try_parse_from(["fetchx", "queue-move", "download-id", "up"])
            .expect("queue move command should parse");
        let Command::QueueMove(options) = local.command else {
            panic!("expected queue move command");
        };
        assert_eq!(options.direction, QueueDirection::Up);

        let remote = Cli::try_parse_from(["fetchx", "remote", "queue-down", "download-id"])
            .expect("remote queue move command should parse");
        let Command::Remote(options) = remote.command else {
            panic!("expected remote command");
        };
        assert!(matches!(
            options.command,
            RemoteCommand::QueueDown { id } if id == "download-id"
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
        assert!(options.action.is_none());
    }

    #[test]
    fn remote_commands_parse_server_override_and_operations() {
        let connect = Cli::try_parse_from([
            "fetchx",
            "remote",
            "--server",
            "https://fetchx.example.test",
            "connect",
        ])
        .expect("remote connect options should parse");
        let Command::Remote(options) = connect.command else {
            panic!("expected remote command");
        };
        assert_eq!(
            options.server.as_deref(),
            Some("https://fetchx.example.test")
        );
        assert!(matches!(
            options.command,
            RemoteCommand::Connect { url: None }
        ));

        let add = Cli::try_parse_from(["fetchx", "remote", "add", "https://example.test/file.bin"])
            .expect("remote add should parse");
        let Command::Remote(options) = add.command else {
            panic!("expected remote command");
        };
        assert!(
            matches!(options.command, RemoteCommand::Add { url } if url == "https://example.test/file.bin")
        );
    }

    #[test]
    fn server_tls_requires_certificate_and_private_key_together() {
        assert!(
            Cli::try_parse_from(["fetchx", "server", "--tls-certificate", "certificate.pem"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["fetchx", "server", "--tls-private-key", "private-key.pem"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "fetchx",
                "server",
                "--tls-certificate",
                "certificate.pem",
                "--tls-private-key",
                "private-key.pem"
            ])
            .is_ok()
        );
    }

    #[test]
    fn server_auth_admin_commands_parse() {
        let init = Cli::try_parse_from(["fetchx", "server", "init-auth"])
            .expect("server init-auth should parse");
        let Command::Server(options) = init.command else {
            panic!("expected server command");
        };
        assert!(matches!(options.action, Some(ServerAction::InitAuth)));

        let rotate =
            Cli::try_parse_from(["fetchx", "server", "--database", "fetchx.db", "rotate-auth"])
                .expect("server rotate-auth should parse");
        let Command::Server(options) = rotate.command else {
            panic!("expected server command");
        };
        assert!(matches!(options.action, Some(ServerAction::RotateAuth)));
        assert_eq!(options.database, Some(PathBuf::from("fetchx.db")));

        let set_role = Cli::try_parse_from(["fetchx", "server", "set-role", "read-only"])
            .expect("server set-role should parse");
        let Command::Server(options) = set_role.command else {
            panic!("expected server command");
        };
        assert!(matches!(
            options.action,
            Some(ServerAction::SetRole { role }) if role == "read-only"
        ));

        let show_role = Cli::try_parse_from(["fetchx", "server", "show-role"])
            .expect("server show-role should parse");
        let Command::Server(options) = show_role.command else {
            panic!("expected server command");
        };
        assert!(matches!(options.action, Some(ServerAction::ShowRole)));
    }

    #[test]
    fn database_maintenance_commands_parse_paths() {
        let cli = Cli::try_parse_from([
            "fetchx",
            "db",
            "import",
            "backup.json",
            "--database",
            "fetchx.db",
            "--downloads-root",
            "Downloads",
        ])
        .expect("database import options should parse");
        let Command::Db(options) = cli.command else {
            panic!("expected database command");
        };
        assert!(matches!(
            options.command,
            DbCommand::Import {
                input,
                database: Some(database),
                downloads_root: Some(downloads_root),
            } if input == Path::new("backup.json")
                && database == Path::new("fetchx.db")
                && downloads_root == Path::new("Downloads")
        ));
    }
}

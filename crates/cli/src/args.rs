//! CLI argument definitions using clap.

use clap::{Args, Parser, Subcommand};

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
    /// HTTP or HTTPS URL to download.
    pub url: String,

    /// Engine to use: direct-http | aria2 | yt-dlp
    #[arg(short = 'e', long, value_name = "ENGINE")]
    pub engine: Option<String>,

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


/// Runtime availability of an optional engine.
#[derive(Clone, Debug)]
pub enum EngineAvailability {
    Disabled,
    Available { version: String },
    Unavailable { reason: String },
}

/// Runtime status of all optional engines.
#[derive(Clone, Debug)]
pub struct EngineRuntimeStatus {
    pub aria2: EngineAvailability,
    pub yt_dlp: EngineAvailability,
    pub native_torrent: EngineAvailability,
    /// FTP and FTPS are always-available (pure Rust, no external binary).
    pub ftp: EngineAvailability,
    /// SFTP is always-available (russh is pure Rust, no external binary).
    pub sftp: EngineAvailability,
    /// Metalink orchestration engine availability (wired in Phase 2).
    pub metalink: EngineAvailability,
}

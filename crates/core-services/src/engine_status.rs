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
}

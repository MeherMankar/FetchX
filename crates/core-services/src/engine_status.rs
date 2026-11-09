use fetchx_models::EngineKind;

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
    pub yt_dlp: EngineAvailability,
    pub native_torrent: EngineAvailability,
    /// FTP and FTPS are always-available (pure Rust, no external binary).
    pub ftp: EngineAvailability,
    /// SFTP is always-available (russh is pure Rust, no external binary).
    pub sftp: EngineAvailability,
    /// Metalink orchestration engine availability (wired in Phase 2).
    pub metalink: EngineAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineRuntimeState {
    Available,
    Disabled,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineStatusEntry {
    pub engine: EngineKind,
    pub state: EngineRuntimeState,
    pub version: Option<String>,
    pub reason: Option<String>,
}

impl EngineRuntimeStatus {
    pub fn entries(&self) -> Vec<EngineStatusEntry> {
        vec![
            builtin_entry(EngineKind::DirectHttp),
            builtin_entry(EngineKind::Hls),
            status_entry(EngineKind::YtDlp, &self.yt_dlp),
            status_entry(EngineKind::NativeTorrent, &self.native_torrent),
            status_entry(EngineKind::Ftp, &self.ftp),
            status_entry(EngineKind::Ftps, &self.ftp),
            status_entry(EngineKind::Sftp, &self.sftp),
            status_entry(EngineKind::Metalink, &self.metalink),
        ]
    }
}

fn builtin_entry(engine: EngineKind) -> EngineStatusEntry {
    EngineStatusEntry {
        engine,
        state: EngineRuntimeState::Available,
        version: None,
        reason: None,
    }
}

fn status_entry(engine: EngineKind, availability: &EngineAvailability) -> EngineStatusEntry {
    match availability {
        EngineAvailability::Disabled => EngineStatusEntry {
            engine,
            state: EngineRuntimeState::Disabled,
            version: None,
            reason: None,
        },
        EngineAvailability::Available { version } => EngineStatusEntry {
            engine,
            state: EngineRuntimeState::Available,
            version: Some(version.clone()),
            reason: None,
        },
        EngineAvailability::Unavailable { reason } => EngineStatusEntry {
            engine,
            state: EngineRuntimeState::Unavailable,
            version: None,
            reason: Some(reason.clone()),
        },
    }
}

#[cfg(test)]
mod tests {
    use fetchx_models::EngineKind;

    use super::{EngineAvailability, EngineRuntimeState, EngineRuntimeStatus};

    #[test]
    fn entries_include_all_engines_and_preserve_runtime_availability() {
        let status = EngineRuntimeStatus {
            yt_dlp: EngineAvailability::Available {
                version: "2026.01".to_owned(),
            },
            native_torrent: EngineAvailability::Unavailable {
                reason: "runtime failed".to_owned(),
            },
            ftp: EngineAvailability::Available {
                version: "suppaftp".to_owned(),
            },
            sftp: EngineAvailability::Disabled,
            metalink: EngineAvailability::Unavailable {
                reason: "metalink runtime failed".to_owned(),
            },
        };

        let entries = status.entries();
        assert_eq!(
            entries.iter().map(|entry| entry.engine).collect::<Vec<_>>(),
            [
                EngineKind::DirectHttp,
                EngineKind::Hls,
                EngineKind::YtDlp,
                EngineKind::NativeTorrent,
                EngineKind::Ftp,
                EngineKind::Ftps,
                EngineKind::Sftp,
                EngineKind::Metalink,
            ]
        );
        assert!(
            entries
                .iter()
                .filter(|entry| matches!(entry.engine, EngineKind::DirectHttp | EngineKind::Hls))
                .all(|entry| entry.state == EngineRuntimeState::Available)
        );
        assert_eq!(entries[2].state, EngineRuntimeState::Available);
        assert_eq!(entries[2].version.as_deref(), Some("2026.01"));
        assert_eq!(entries[3].state, EngineRuntimeState::Unavailable);
        assert_eq!(entries[3].reason.as_deref(), Some("runtime failed"));
        assert_eq!(entries[4].state, EngineRuntimeState::Available);
        assert_eq!(entries[5].state, EngineRuntimeState::Available);
        assert_eq!(entries[6].state, EngineRuntimeState::Disabled);
        assert_eq!(entries[7].state, EngineRuntimeState::Unavailable);
    }
}

use serde::Serialize;

use fetchx_downloader::Aria2RecoveryState as EngineRecoveryState;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Aria2RecoveryAssessment {
    pub download_id: String,
    pub state: Aria2RecoveryState,
    pub explanation: &'static str,
    pub next_step: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aria2RecoveryState {
    NotAria2,
    NotRequired,
    RuntimeUnavailable,
    ReadyToStart,
    JobAvailable,
    MissingTorrentMetadata,
    MissingGid,
    StaleGid,
    MissingControlFile,
    ControlFilePresent,
    Unknown,
}

impl Aria2RecoveryAssessment {
    pub(crate) fn new(download_id: String, state: Aria2RecoveryState) -> Self {
        let (explanation, next_step) = match state {
            Aria2RecoveryState::NotAria2 => (
                "This download does not use aria2.",
                "No aria2 recovery action is needed.",
            ),
            Aria2RecoveryState::NotRequired => (
                "The download is complete; engine recovery is not needed.",
                "Keep or open the completed file as usual.",
            ),
            Aria2RecoveryState::RuntimeUnavailable => (
                "The aria2 runtime is unavailable here. The saved record and files are unchanged.",
                "Enable or install aria2 in Download Engine settings. FetchX will not switch this job to another engine.",
            ),
            Aria2RecoveryState::ReadyToStart => (
                "aria2 is available, and this queued download has no partial progress to recover.",
                "Resume it using the normal controls; it remains assigned to aria2.",
            ),
            Aria2RecoveryState::JobAvailable => (
                "aria2 reports that the associated task is available.",
                "Use the normal Resume or Retry controls. The task remains assigned to aria2.",
            ),
            Aria2RecoveryState::MissingTorrentMetadata => (
                "Torrent recovery metadata is missing, so FetchX cannot validate its aria2 task.",
                "Keep the record and files. Restore the aria2 runtime or inspect the task in aria2 before retrying.",
            ),
            Aria2RecoveryState::MissingGid => (
                "No aria2 task ID is saved for this torrent, so its previous task cannot be checked.",
                "Keep the record and files. Verify the task in aria2 before retrying; no native fallback was attempted.",
            ),
            Aria2RecoveryState::StaleGid => (
                "aria2 does not recognize this task ID, the task is no longer resumable, or its torrent info hash does not match.",
                "Keep the record and files. Verify the task and selected files in aria2 before retrying.",
            ),
            Aria2RecoveryState::MissingControlFile => (
                "The aria2 partial control file is missing. FetchX cannot verify safe continuation of this partial.",
                "Keep the partial file and record. Check the original aria2 task before retrying.",
            ),
            Aria2RecoveryState::ControlFilePresent => (
                "The aria2 partial control file exists, but this alone does not prove that the partial can resume.",
                "Use the normal Retry control to let the persisted aria2 engine verify its own state.",
            ),
            Aria2RecoveryState::Unknown => (
                "Recovery status is unknown because the available aria2 task or filesystem evidence was inconclusive.",
                "Keep the record and files. Check aria2 availability and its task state before retrying.",
            ),
        };
        Self {
            download_id,
            state,
            explanation,
            next_step,
        }
    }
}

impl From<EngineRecoveryState> for Aria2RecoveryState {
    fn from(state: EngineRecoveryState) -> Self {
        match state {
            EngineRecoveryState::ReadyToStart => Self::ReadyToStart,
            EngineRecoveryState::JobAvailable => Self::JobAvailable,
            EngineRecoveryState::MissingTorrentMetadata => Self::MissingTorrentMetadata,
            EngineRecoveryState::MissingGid => Self::MissingGid,
            EngineRecoveryState::StaleGid => Self::StaleGid,
            EngineRecoveryState::MissingControlFile => Self::MissingControlFile,
            EngineRecoveryState::ControlFilePresent => Self::ControlFilePresent,
            EngineRecoveryState::Unknown => Self::Unknown,
        }
    }
}

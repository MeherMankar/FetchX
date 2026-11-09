use fetchx_core_services::{Aria2RecoveryAssessment, Aria2RecoveryState, CoreServices};
use fetchx_models::DownloadId;
use serde_json::{Value, json};

use crate::{
    args::IdOpts,
    commands::parse_id,
    exit::{self, ExitCode},
    output,
};

pub async fn run(svc: &CoreServices, opts: IdOpts, json_output: bool) -> ExitCode {
    let id = match parse_id(&opts.id) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json_output, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    match svc.assess_aria2_recovery(id).await {
        Ok(Some(assessment)) => {
            print_assessment(&assessment, json_output);
            exit::SUCCESS
        }
        Ok(None) => {
            output::print_error(json_output, "download not found");
            exit::NOT_FOUND
        }
        Err(_) => {
            output::print_error(json_output, "aria2 recovery assessment failed");
            exit::GENERAL_ERROR
        }
    }
}

pub(crate) fn assessment_json(assessment: &Aria2RecoveryAssessment) -> Value {
    json!({
        "ok": true,
        "assessment_version": fetchx_remote_protocol::ARIA2_RECOVERY_ASSESSMENT_VERSION,
        "assessment": assessment,
    })
}

pub(crate) fn state_name(state: Aria2RecoveryState) -> &'static str {
    match state {
        Aria2RecoveryState::NotAria2 => "not_aria2",
        Aria2RecoveryState::NotRequired => "not_required",
        Aria2RecoveryState::RuntimeUnavailable => "runtime_unavailable",
        Aria2RecoveryState::ReadyToStart => "ready_to_start",
        Aria2RecoveryState::JobAvailable => "job_available",
        Aria2RecoveryState::MissingTorrentMetadata => "missing_torrent_metadata",
        Aria2RecoveryState::MissingGid => "missing_gid",
        Aria2RecoveryState::StaleGid => "stale_gid",
        Aria2RecoveryState::MissingControlFile => "missing_control_file",
        Aria2RecoveryState::ControlFilePresent => "control_file_present",
        Aria2RecoveryState::Unknown => "unknown",
    }
}

fn print_assessment(assessment: &Aria2RecoveryAssessment, json_output: bool) {
    if json_output {
        output::print_json_ok(&assessment_json(assessment));
    } else {
        println!("aria2 recovery assessment for {}", assessment.download_id);
        println!("  State:     {}", state_name(assessment.state));
        println!("  Assessment: {}", assessment.explanation);
        println!("  Next step: {}", assessment.next_step);
        println!("  Read-only: yes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_output_covers_every_supported_assessment_state() -> Result<(), Box<dyn std::error::Error>>
    {
        let states = [
            Aria2RecoveryState::NotAria2,
            Aria2RecoveryState::NotRequired,
            Aria2RecoveryState::RuntimeUnavailable,
            Aria2RecoveryState::ReadyToStart,
            Aria2RecoveryState::JobAvailable,
            Aria2RecoveryState::MissingTorrentMetadata,
            Aria2RecoveryState::MissingGid,
            Aria2RecoveryState::StaleGid,
            Aria2RecoveryState::MissingControlFile,
            Aria2RecoveryState::ControlFilePresent,
            Aria2RecoveryState::Unknown,
        ];
        for state in states {
            let assessment = Aria2RecoveryAssessment {
                download_id: DownloadId::new().to_string(),
                state,
                explanation: "sanitized explanation",
                next_step: "safe next step",
            };
            let value = assessment_json(&assessment);
            assert_eq!(value["assessment_version"], 1);
            assert_eq!(value["assessment"]["state"], state_name(state));
            assert_eq!(value["assessment"]["explanation"], "sanitized explanation");
            assert_eq!(value["assessment"]["next_step"], "safe next step");
            assert!(!value.to_string().contains("destination"));
            assert!(!value.to_string().contains("aria2_gid"));
        }
        Ok(())
    }
}

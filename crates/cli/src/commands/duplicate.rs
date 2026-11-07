use fetchx_core_services::CoreServices;

use crate::{exit, exit::ExitCode, output};

pub async fn run(svc: &CoreServices, url: &str, json: bool) -> ExitCode {
    match svc.find_duplicate_download(url).await {
        Ok(Some(duplicate)) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "duplicate": true,
                    "download": {
                        "id": duplicate.id(),
                        "filename": duplicate.filename(),
                        "status": duplicate.status(),
                        "updated_at": duplicate.updated_at().as_unix_millis(),
                    }
                }));
            } else {
                output::print_human(&format!(
                    "Duplicate download found: {} ({})",
                    duplicate.filename(),
                    duplicate.id()
                ));
            }
            exit::SUCCESS
        }
        Ok(None) => {
            if json {
                output::print_json_ok(&serde_json::json!({ "duplicate": false }));
            } else {
                output::print_human("No matching download found.");
            }
            exit::SUCCESS
        }
        Err(error) => {
            output::print_error(json, &error.to_string());
            exit::GENERAL_ERROR
        }
    }
}

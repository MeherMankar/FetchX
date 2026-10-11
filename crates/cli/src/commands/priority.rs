use mydownloader_core_services::CoreServices;

use crate::{
    args::PriorityOpts,
    commands::{add::parse_priority, parse_id},
    exit::{self, ExitCode},
    output,
};

pub async fn run(svc: &CoreServices, opts: PriorityOpts, json: bool) -> ExitCode {
    let id = match parse_id(&opts.id) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    let priority = match parse_priority(&opts.priority) {
        Ok(p) => p,
        Err(msg) => {
            output::print_error(json, &msg);
            return exit::INVALID_ARGS;
        }
    };

    match svc.queue().set_priority(id, priority).await {
        Ok(()) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true, "id": id.to_string(), "priority": format!("{priority:?}")
                }));
            } else {
                println!("Priority set to {priority:?} for {id}");
            }
            exit::SUCCESS
        }
        Err(e) => {
            output::print_error(json, &format!("could not set priority: {e}"));
            exit::GENERAL_ERROR
        }
    }
}

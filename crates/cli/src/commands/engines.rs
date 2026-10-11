use mydownloader_core_services::EngineAvailability;

use crate::{bootstrap, exit::ExitCode, output};

pub async fn run(json: bool) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let svc = bootstrap::start().await?;
    let status = &svc.engine_runtime.status;

    if json {
        output::print_json_ok(&serde_json::json!({
            "ok": true,
            "engines": {
                "direct_http": {
                    "available": true,
                    "version": null,
                    "note": "built-in"
                },
                "aria2": avail_json(&status.aria2),
                "yt_dlp": avail_json(&status.yt_dlp),
            }
        }));
    } else {
        let mut table = output::Table::new(vec!["ENGINE", "AVAILABLE", "NOTE"]);
        table.add_row(vec![
            "direct-http".to_owned(),
            "yes".to_owned(),
            "built-in".to_owned(),
        ]);
        table.add_row(avail_row("aria2", &status.aria2));
        table.add_row(avail_row("yt-dlp", &status.yt_dlp));
        table.print();
    }

    svc.shutdown().await?;
    Ok(crate::exit::SUCCESS)
}

fn avail_row(name: &str, avail: &EngineAvailability) -> Vec<String> {
    match avail {
        EngineAvailability::Available { version } => {
            vec![name.to_owned(), "yes".to_owned(), version.clone()]
        }
        EngineAvailability::Unavailable { reason } => vec![
            name.to_owned(),
            "no".to_owned(),
            format!("unavailable: {reason}"),
        ],
        EngineAvailability::Disabled => vec![
            name.to_owned(),
            "no".to_owned(),
            "disabled in config".to_owned(),
        ],
    }
}

fn avail_json(avail: &EngineAvailability) -> serde_json::Value {
    match avail {
        EngineAvailability::Available { version } => {
            serde_json::json!({ "available": true, "version": version })
        }
        EngineAvailability::Unavailable { reason } => {
            serde_json::json!({ "available": false, "reason": reason })
        }
        EngineAvailability::Disabled => {
            serde_json::json!({ "available": false, "reason": "disabled" })
        }
    }
}

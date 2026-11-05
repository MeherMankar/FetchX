use fetchx_core_services::CoreServices;
use fetchx_storage::DownloadRepository;

use crate::{
    args::IdOpts,
    commands::parse_id,
    exit::{self, ExitCode},
    output,
};

pub async fn run(svc: &CoreServices, opts: IdOpts, json: bool) -> ExitCode {
    let id = match parse_id(&opts.id) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    let item = match svc.repository.get(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            output::print_error(json, &format!("download {id} not found"));
            return exit::NOT_FOUND;
        }
        Err(e) => {
            output::print_error(json, &format!("storage error: {e}"));
            return exit::GENERAL_ERROR;
        }
    };

    let p = item.progress();
    let sched = svc.scheduler().get_schedule(id).await.ok().flatten();

    if json {
        output::print_json_ok(&serde_json::json!({
            "ok": true,
            "download": {
                "id":         item.id().to_string(),
                "url":        item.source().url(),
                "filename":   item.filename().as_str(),
                "destination":item.destination().to_string_lossy(),
                "status":     format!("{:?}", item.status()),
                "engine":     format!("{:?}", item.source().engine()),
                "priority":   format!("{:?}", item.priority()),
                "downloaded": p.downloaded_bytes(),
                "downloaded_bytes": p.downloaded_bytes(),
                "total":      p.total_bytes(),
                "total_bytes": p.total_bytes(),
                "speed":      p.speed_bytes_per_second(),
                "speed_bytes_per_second": p.speed_bytes_per_second(),
                "eta_seconds":p.estimated_remaining_seconds(),
                "estimated_remaining_seconds": p.estimated_remaining_seconds(),
                "percentage": p.percentage_basis_points().map(|b| b as f64 / 100.0),
                "error":      item.error().map(|e| e.message()),
                "error_message": item.error().map(|e| e.message()),
                "error_code": item.error().map(|e| format!("{:?}", e.code()).to_ascii_lowercase()),
                "created_at": item.created_at().as_unix_millis(),
                "updated_at": item.updated_at().as_unix_millis(),
                "schedule":   sched.map(|s| serde_json::json!({
                    "scheduled_at_ms": s.scheduled_at().as_unix_millis(),
                    "enabled": s.enabled(),
                })),
            }
        }));
    } else {
        println!("Download: {}", item.id());
        println!("  Status:      {:?}", item.status());
        println!("  File:        {}", item.filename().as_str());
        println!("  URL:         {}", item.source().url());
        println!("  Destination: {}", item.destination().display());
        println!("  Engine:      {:?}", item.source().engine());
        println!("  Priority:    {:?}", item.priority());
        println!("  Downloaded:  {}", output::fmt_bytes(p.downloaded_bytes()));
        println!(
            "  Total:       {}",
            p.total_bytes()
                .map(output::fmt_bytes)
                .unwrap_or_else(|| "unknown".to_owned())
        );
        println!(
            "  Progress:    {}",
            output::fmt_pct(p.percentage_basis_points())
        );
        println!(
            "  Speed:       {}",
            output::fmt_speed(p.speed_bytes_per_second())
        );
        println!(
            "  ETA:         {}",
            output::fmt_eta(p.estimated_remaining_seconds())
        );
        if let Some(e) = item.error() {
            println!("  Error:       {:?} ({:?})", e.message(), e.code());
        }
        if let Some(s) = sched {
            println!(
                "  Schedule:    {} (enabled: {})",
                chrono_fmt(s.scheduled_at().as_unix_millis()),
                s.enabled()
            );
        }
    }

    exit::SUCCESS
}

fn chrono_fmt(ms: u64) -> String {
    // Simple Unix-ms → human string without a chrono dependency.
    let secs = ms / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days_approx = hours / 24;
    if days_approx > 0 {
        format!("~{days_approx}d from now")
    } else {
        format!(
            "{:02}:{:02}:{:02} from epoch",
            hours % 24,
            mins % 60,
            secs % 60
        )
    }
}

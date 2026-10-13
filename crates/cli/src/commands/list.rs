use fetchx_core_services::CoreServices;
use fetchx_models::DownloadStatus;
use fetchx_storage::DownloadRepository;
use serde_json::json;

use crate::{
    args::ListOpts,
    exit::{self, ExitCode},
    output,
};

pub async fn run(svc: &CoreServices, opts: ListOpts, json: bool) -> ExitCode {
    let all = match svc.repository.list().await {
        Ok(items) => items,
        Err(e) => {
            output::print_error(json, &format!("could not list downloads: {e}"));
            return exit::GENERAL_ERROR;
        }
    };

    // Filter by status
    let filter = opts.filter.to_ascii_lowercase();
    let mut items: Vec<_> = all
        .into_iter()
        .filter(|d| match filter.as_str() {
            "all" => true,
            "active" => matches!(
                d.status(),
                DownloadStatus::Downloading | DownloadStatus::Preparing
            ),
            "queued" => d.status() == DownloadStatus::Queued,
            "paused" => d.status() == DownloadStatus::Paused,
            "completed" => d.status() == DownloadStatus::Completed,
            "failed" => d.status() == DownloadStatus::Failed,
            "cancelled" => d.status() == DownloadStatus::Cancelled,
            _ => true,
        })
        .collect();

    // Limit
    if let Some(n) = opts.limit {
        items.truncate(n);
    }

    if json {
        let rows: Vec<_> = items
            .iter()
            .map(|d| {
                let p = d.progress();
                json!({
                    "id":          d.id().to_string(),
                    "filename":    d.filename().as_str(),
                    "status":      format!("{:?}", d.status()),
                    "engine":      format!("{:?}", d.source().engine()),
                    "downloaded":  p.downloaded_bytes(),
                    "total":       p.total_bytes(),
                    "speed":       p.speed_bytes_per_second(),
                    "eta_seconds": p.estimated_remaining_seconds(),
                    "percentage":  p.percentage_basis_points().map(|b| b as f64 / 100.0),
                })
            })
            .collect();
        output::print_json_ok(&json!({ "ok": true, "count": rows.len(), "downloads": rows }));
    } else {
        if items.is_empty() {
            println!("No downloads.");
            return exit::SUCCESS;
        }
        let mut table =
            output::Table::new(vec!["ID", "STATUS", "PROGRESS", "SPEED", "ETA", "FILE"]);
        for d in &items {
            let p = d.progress();
            let id_short = &d.id().to_string()[..8];
            table.add_row(vec![
                id_short.to_owned(),
                format!("{:?}", d.status()),
                output::fmt_pct(p.percentage_basis_points()),
                output::fmt_speed(p.speed_bytes_per_second()),
                output::fmt_eta(p.estimated_remaining_seconds()),
                output::truncate(d.filename().as_str(), 28),
            ]);
        }
        table.print();
        println!();
        println!("{} download(s)", items.len());
    }

    exit::SUCCESS
}

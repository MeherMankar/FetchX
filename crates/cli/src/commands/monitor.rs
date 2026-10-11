//! `mydownloader monitor` — live updating terminal view.
//!
//! Subscribes to the existing DownloadEvent broadcast bus so updates
//! arrive immediately without polling SQLite aggressively.
//! A fallback refresh every 2 seconds handles connection recovery.
//!
//! Ctrl+C shuts down cleanly.

use mydownloader_core_services::CoreServices;
use mydownloader_models::DownloadStatus;
use mydownloader_queue::DownloadEvent;
use mydownloader_storage::DownloadRepository;

use crate::{exit::ExitCode, output};

pub async fn run(svc: &CoreServices) -> ExitCode {
    let mut rx = svc.event_bus.subscribe();

    eprintln!("Monitoring downloads — press Ctrl+C to exit.");
    eprintln!();

    // Initial display
    print_active(svc).await;

    // Event loop — update on every event, with a 2s fallback timeout.
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(DownloadEvent::Created { .. }
                     | DownloadEvent::Updated { .. }
                     | DownloadEvent::Removed { .. }) => {
                        print_active(svc).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Fell behind; refresh anyway.
                        print_active(svc).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                // Fallback refresh.
                print_active(svc).await;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nInterrupted.");
                break;
            }
        }
    }

    crate::exit::SUCCESS
}

async fn print_active(svc: &CoreServices) {
    let all = match svc.repository.list().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error reading downloads: {e}");
            return;
        }
    };

    let active: Vec<_> = all
        .iter()
        .filter(|d| {
            matches!(
                d.status(),
                DownloadStatus::Downloading | DownloadStatus::Preparing | DownloadStatus::Queued
            )
        })
        .collect();

    let completed = all
        .iter()
        .filter(|d| d.status() == DownloadStatus::Completed)
        .count();
    let failed = all
        .iter()
        .filter(|d| d.status() == DownloadStatus::Failed)
        .count();

    // Clear previous output with ANSI escape (best-effort — ignored on non-ANSI terminals).
    print!("\x1B[2J\x1B[H");

    println!("MyDownloader — Monitor  (Ctrl+C to exit)");
    println!(
        "Active: {}  Completed: {}  Failed: {}",
        active.len(),
        completed,
        failed
    );
    println!();

    if active.is_empty() {
        println!("  No active downloads.");
    } else {
        let mut table =
            output::Table::new(vec!["ID", "STATUS", "PROGRESS", "SPEED", "ETA", "FILE"]);
        for d in &active {
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
    }
}

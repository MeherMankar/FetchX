//! Torrent-specific CLI commands: status, files, stop-seeding.

use fetchx_core_services::CoreServices;
use fetchx_models::DownloadStatus;
use fetchx_storage::TorrentMetadataRepository;

use crate::{
    commands::parse_id,
    exit::{self, ExitCode},
    output,
};

/// Show detailed torrent status with telemetry.
pub async fn status(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    let telemetry = match svc.torrent_telemetry(id).await {
        Ok(telemetry) => telemetry,
        Err(error) => {
            output::print_error(json, &format!("torrent telemetry error: {error}"));
            return exit::GENERAL_ERROR;
        }
    };
    let meta = svc
        .database
        .torrent_metadata()
        .get_torrent_metadata(id)
        .await;

    if json {
        match meta {
            Ok(Some(row)) => {
                output::print_json_ok(&serde_json::json!({
                    "ok": true,
                    "torrent": {
                        "download_id": row.download_id.to_string(),
                        "info_hash": row.info_hash,
                        "torrent_name": row.torrent_name,
                        "destination": row.destination.to_string_lossy(),
                        "metadata_state": row.metadata_state,
                        "seed_state": format!("{:?}", row.seed_state),
                        "selected_files": row.selected_file_indexes,
                        "selected_total_bytes": row.selected_total_bytes,
                        "aria2_gid": row.aria2_gid,
                        "telemetry": telemetry.as_ref().map(|value| serde_json::json!({
                            "download_speed_bps": value.download_speed_bps,
                            "upload_speed_bps": value.upload_speed_bps,
                            "downloaded_bytes": value.downloaded_bytes,
                            "uploaded_bytes": value.uploaded_bytes,
                            "peer_count": value.peer_count,
                            "seed_count": value.seed_count,
                            "share_ratio": value.share_ratio,
                            "seeding": value.seeding,
                            "tracker_uris": value.tracker_uris,
                        })),
                    }
                }));
                exit::SUCCESS
            }
            Ok(None) => {
                output::print_error(json, &format!("no torrent metadata for {id}"));
                exit::NOT_FOUND
            }
            Err(e) => {
                output::print_error(json, &format!("storage error: {e}"));
                exit::GENERAL_ERROR
            }
        }
    } else {
        match meta {
            Ok(Some(row)) => {
                println!("Torrent: {}", row.download_id);
                println!("  Name:         {}", row.torrent_name);
                println!("  Info hash:    {}", row.info_hash);
                println!("  Destination:  {}", row.destination.display());
                println!("  Metadata:     {}", row.metadata_state);
                println!("  Seed state:   {:?}", row.seed_state);
                println!(
                    "  Selected:     {} file(s)",
                    row.selected_file_indexes.len()
                );
                println!(
                    "  Selected size:{} ",
                    output::fmt_bytes(row.selected_total_bytes)
                );
                if let Some(telemetry) = telemetry {
                    println!(
                        "  Peers:        {}",
                        telemetry
                            .peer_count
                            .map(|count| count.to_string())
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                    println!(
                        "  Seeds:        {}",
                        telemetry
                            .seed_count
                            .map(|count| count.to_string())
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                    println!(
                        "  Download:     {}",
                        output::fmt_speed(telemetry.download_speed_bps)
                    );
                    println!(
                        "  Upload speed: {}",
                        output::fmt_speed(telemetry.upload_speed_bps)
                    );
                    println!(
                        "  Downloaded:   {}",
                        telemetry
                            .downloaded_bytes
                            .map(output::fmt_bytes)
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                    println!(
                        "  Uploaded:     {}",
                        telemetry
                            .uploaded_bytes
                            .map(output::fmt_bytes)
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                    println!(
                        "  Share ratio:  {}",
                        telemetry
                            .share_ratio
                            .map(|ratio| format!("{ratio:.3}"))
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                    println!(
                        "  Seeding:      {}",
                        telemetry
                            .seeding
                            .map(|seeding| seeding.to_string())
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                    if let Some(trackers) = telemetry.tracker_uris {
                        println!("  Trackers:     {}", trackers.join(", "));
                    } else {
                        println!("  Trackers:     unavailable");
                    }
                } else {
                    println!("  Telemetry:    unavailable");
                }
                if let Some(gid) = &row.aria2_gid {
                    println!("  aria2 GID:    {gid}");
                }
                exit::SUCCESS
            }
            Ok(None) => {
                eprintln!("No torrent metadata found for {id}");
                exit::NOT_FOUND
            }
            Err(e) => {
                output::print_error(json, &format!("storage error: {e}"));
                exit::GENERAL_ERROR
            }
        }
    }
}

/// List files in a torrent download.
pub async fn files(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    let meta = svc
        .database
        .torrent_metadata()
        .get_torrent_metadata(id)
        .await;

    match meta {
        Ok(Some(row)) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true,
                    "download_id": id.to_string(),
                    "selected_file_indexes": row.selected_file_indexes,
                    "selected_total_bytes": row.selected_total_bytes,
                }));
            } else {
                println!("Torrent files for {id}:");
                if row.selected_file_indexes.is_empty() {
                    println!("  (no file selection recorded — all files selected)");
                } else {
                    for idx in &row.selected_file_indexes {
                        println!("  [{idx}]");
                    }
                }
                println!(
                    "  Selected size: {}",
                    output::fmt_bytes(row.selected_total_bytes)
                );
            }
            exit::SUCCESS
        }
        Ok(None) => {
            output::print_error(json, &format!("no torrent metadata for {id}"));
            exit::NOT_FOUND
        }
        Err(e) => {
            output::print_error(json, &format!("storage error: {e}"));
            exit::GENERAL_ERROR
        }
    }
}

/// Stop seeding a completed torrent.
pub async fn stop_seeding(svc: &CoreServices, id_str: String, json: bool) -> ExitCode {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    // Check the download is actually seeding.
    use fetchx_storage::DownloadRepository;
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
    if item.status() != DownloadStatus::Seeding {
        output::print_error(
            json,
            &format!("download {id} is not seeding (status: {:?})", item.status()),
        );
        return exit::INVALID_ARGS;
    }

    match svc.stop_torrent_seeding(id).await {
        Ok(()) => {
            if json {
                output::print_json_ok(
                    &serde_json::json!({"ok": true, "id": id.to_string(), "action": "stop_seeding"}),
                );
            } else {
                println!("Seeding stopped for {id}");
            }
            exit::SUCCESS
        }
        Err(e) => {
            output::print_error(json, &format!("failed to stop seeding: {e}"));
            exit::GENERAL_ERROR
        }
    }
}

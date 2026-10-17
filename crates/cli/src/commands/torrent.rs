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
                            "completed_peer_count": value.completed_peer_count,
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
                        "  Complete peers:{}",
                        telemetry
                            .completed_peer_count
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

    match svc.torrent_download_metadata(id).await {
        Ok(Some(value)) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true,
                    "download_id": id.to_string(),
                    "metadata_state": value.metadata_state,
                    "metadata": value.metadata,
                    "selected_file_indexes": value.selected_file_indexes,
                    "selected_file_count": value.selected_file_indexes.len(),
                    "selected_total_bytes": value.selected_total_bytes,
                    "selection_confirmed": value.selection_confirmed,
                }));
            } else {
                println!("Torrent files for {id}:");
                println!("  Metadata: {}", value.metadata_state);
                if let Some(metadata) = value.metadata {
                    println!("  Name: {}", metadata.name);
                    println!("  Total size: {}", output::fmt_bytes(metadata.total_size));
                    for (index, file) in metadata.files.iter().enumerate() {
                        let selected = value.selected_file_indexes.contains(&(index as u32));
                        println!(
                            "  [{}] {} · {}{}",
                            index + 1,
                            file.path.join("/"),
                            output::fmt_bytes(file.size),
                            if selected { " · selected" } else { "" },
                        );
                    }
                } else {
                    println!("  File list is not available yet.");
                }
                println!("  Selected files: {}", value.selected_file_indexes.len());
                println!(
                    "  Selected size: {}",
                    output::fmt_bytes(value.selected_total_bytes)
                );
                println!("  Selection confirmed: {}", value.selection_confirmed);
            }
            exit::SUCCESS
        }
        Ok(None) => {
            output::print_error(json, &format!("no torrent metadata for {id}"));
            exit::NOT_FOUND
        }
        Err(e) => {
            output::print_error(json, &format!("torrent metadata error: {e}"));
            exit::GENERAL_ERROR
        }
    }
}

pub async fn select_files(
    svc: &CoreServices,
    id_str: String,
    expression: String,
    json: bool,
) -> ExitCode {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };
    let Some(current) = (match svc.torrent_download_metadata(id).await {
        Ok(metadata) => metadata,
        Err(error) => {
            output::print_error(json, &format!("torrent metadata error: {error}"));
            return exit::GENERAL_ERROR;
        }
    }) else {
        output::print_error(json, &format!("no torrent metadata for {id}"));
        return exit::NOT_FOUND;
    };
    let Some(metadata) = current.metadata else {
        output::print_error(json, "torrent metadata is not ready yet");
        return exit::INVALID_ARGS;
    };
    let selection_result = if expression == "none" {
        svc.set_torrent_selection_all(id, false).await
    } else {
        let indexes = if expression == "all" {
            (0..metadata.files.len())
                .map(u32::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "torrent has too many files".to_owned())
        } else if let Some(directory) = expression.strip_prefix("dir:") {
            match svc
                .set_torrent_directory_selected(id, directory.to_owned(), true)
                .await
            {
                Ok(()) => svc
                    .torrent_download_metadata(id)
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|value| {
                        value
                            .ok_or_else(|| "torrent metadata not found".to_owned())
                            .map(|value| value.selected_file_indexes)
                    }),
                Err(error) => Err(error.to_string()),
            }
        } else {
            expression
                .split(',')
                .map(|part| {
                    let one_based = part.trim().parse::<u32>().map_err(|_| {
                        "indexes must be positive comma-separated integers".to_owned()
                    })?;
                    one_based
                        .checked_sub(1)
                        .ok_or_else(|| "indexes are 1-based and must be positive".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()
        };
        match indexes {
            Ok(indexes) => svc.select_torrent_files(id, indexes).await,
            Err(error) => Err(fetchx_core_services::CoreServicesError::InvalidTorrent(
                error,
            )),
        }
    };
    match selection_result {
        Ok(()) => {
            if json {
                output::print_json_ok(
                    &serde_json::json!({"ok": true, "download_id": id.to_string()}),
                );
            } else {
                println!("Torrent selection saved for {id}");
            }
            exit::SUCCESS
        }
        Err(error) => {
            output::print_error(json, &format!("invalid torrent selection: {error}"));
            exit::INVALID_ARGS
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

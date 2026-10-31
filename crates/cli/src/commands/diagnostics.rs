use std::path::PathBuf;

use fetchx_models::{
    ApplicationDiagnostics, DIAGNOSTICS_SCHEMA_VERSION, DiagnosticComponentStatus,
    DiagnosticHealth, DiagnosticsSnapshot, PluginDiagnostics, QueueDiagnostics, RpcDiagnostics,
    global_diagnostics,
};
use fetchx_storage::{Database, DatabaseSchemaState};

use crate::{bootstrap, exit, output};

pub async fn run(
    database_path: Option<PathBuf>,
    json: bool,
) -> Result<exit::ExitCode, Box<dyn std::error::Error>> {
    let operation_id = uuid::Uuid::new_v4().to_string();
    global_diagnostics().record(
        fetchx_models::DiagnosticEvent::new(
            fetchx_models::DiagnosticComponent::Cli,
            "diagnostics_requested",
            fetchx_models::DiagnosticLevel::Info,
        )
        .with_operation_id(operation_id.clone()),
    );
    let path = database_path.unwrap_or_else(bootstrap::database_path);
    let check_started = std::time::Instant::now();
    let check = Database::check_path(path).await;
    let database_check_latency_ms =
        u64::try_from(check_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let storage_health = if check.is_healthy() {
        DiagnosticHealth::Healthy
    } else {
        match check.schema.state {
            DatabaseSchemaState::EmptyDatabase => DiagnosticHealth::Unavailable,
            _ => DiagnosticHealth::Unhealthy,
        }
    };
    let metrics = global_diagnostics().metrics();
    let snapshot = DiagnosticsSnapshot {
        schema_version: DIAGNOSTICS_SCHEMA_VERSION,
        generated_at_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .try_into()?,
        health: if storage_health == DiagnosticHealth::Unhealthy {
            DiagnosticHealth::Unhealthy
        } else {
            DiagnosticHealth::Degraded
        },
        application: ApplicationDiagnostics {
            name: "FetchX".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: fetchx_remote_protocol::PROTOCOL_VERSION.to_owned(),
            target_os: std::env::consts::OS.to_owned(),
            target_arch: std::env::consts::ARCH.to_owned(),
        },
        components: vec![
            DiagnosticComponentStatus {
                health: DiagnosticHealth::Unavailable,
                error_code: Some("RUNTIME_NOT_INITIALIZED".to_owned()),
            },
            DiagnosticComponentStatus {
                health: DiagnosticHealth::Unavailable,
                error_code: Some("QUEUE_NOT_INITIALIZED".to_owned()),
            },
            DiagnosticComponentStatus {
                health: DiagnosticHealth::Unavailable,
                error_code: Some("DOWNLOADER_NOT_INITIALIZED".to_owned()),
            },
            DiagnosticComponentStatus {
                health: storage_health,
                error_code: (storage_health != DiagnosticHealth::Healthy)
                    .then(|| "STORAGE_HEALTH_CHECK_FAILED".to_owned()),
            },
        ],
        queue: QueueDiagnostics {
            health: DiagnosticHealth::Unavailable,
            total_downloads: check
                .table_counts
                .get("downloads")
                .copied()
                .unwrap_or_default(),
            active_downloads: 0,
            queued_downloads: 0,
            paused_downloads: 0,
            failed_downloads: 0,
            completed_downloads: 0,
            cancelled_downloads: 0,
            max_active_downloads: None,
            global_connection_limit: None,
            bytes_downloaded: 0,
            bytes_uploaded: None,
            current_speed_bytes_per_second: 0,
            average_speed_bytes_per_second: 0,
        },
        engines: Vec::new(),
        storage: fetchx_models::StorageDiagnostics {
            health: storage_health,
            schema_version: check.schema.database_schema_version,
            database_check_latency_ms: Some(database_check_latency_ms),
            migration_state: Some(format!("{:?}", check.schema.state)),
            integrity: Some(format!("{:?}", check.sqlite_integrity)),
            database_size_bytes: check.database_size_bytes,
            error_code: (storage_health != DiagnosticHealth::Healthy)
                .then(|| "STORAGE_HEALTH_CHECK_FAILED".to_owned()),
        },
        rpc: RpcDiagnostics {
            health: DiagnosticHealth::Unavailable,
            active_connections: None,
            metrics,
        },
        plugins: PluginDiagnostics {
            health: DiagnosticHealth::Unavailable,
            discovered_plugins: None,
            running_plugins: None,
            metrics,
        },
        metrics,
        recent_events: global_diagnostics().recent_events(),
    };
    if json {
        output::print_json_ok(&snapshot);
    } else {
        print_human(&snapshot);
    }
    global_diagnostics().record(
        fetchx_models::DiagnosticEvent::new(
            fetchx_models::DiagnosticComponent::Cli,
            "diagnostics_completed",
            fetchx_models::DiagnosticLevel::Info,
        )
        .with_operation_id(operation_id),
    );
    Ok(if storage_health == DiagnosticHealth::Healthy {
        exit::SUCCESS
    } else {
        exit::GENERAL_ERROR
    })
}

fn print_human(snapshot: &DiagnosticsSnapshot) {
    println!("FetchX diagnostics (schema {})", snapshot.schema_version);
    println!(
        "Application: {} {} ({})",
        snapshot.application.name,
        snapshot.application.version,
        snapshot.application.protocol_version
    );
    println!("Overall health: {:?}", snapshot.health);
    println!("Storage health: {:?}", snapshot.storage.health);
    println!(
        "Storage schema: {:?}; integrity: {:?}; size: {:?} bytes",
        snapshot.storage.schema_version,
        snapshot.storage.integrity,
        snapshot.storage.database_size_bytes
    );
    println!(
        "Storage check latency: {:?} ms",
        snapshot.storage.database_check_latency_ms
    );
    println!(
        "Queue health: {:?}; total: {}; active: {}; queued: {}; average speed: {} B/s; uploaded: {:?} bytes",
        snapshot.queue.health,
        snapshot.queue.total_downloads,
        snapshot.queue.active_downloads,
        snapshot.queue.queued_downloads,
        snapshot.queue.average_speed_bytes_per_second,
        snapshot.queue.bytes_uploaded
    );
    println!(
        "RPC requests: {}; plugin starts: {}; download completions: {}; engine failures: {}",
        snapshot.metrics.rpc_requests,
        snapshot.metrics.plugin_starts,
        snapshot.metrics.downloads_completed,
        snapshot.metrics.engine_failures
    );
    println!("Runtime and queue state are not started by this read-only command.");
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use clap::Parser;

    use super::run;
    use crate::{args::Cli, args::Command};

    #[test]
    fn diagnostics_command_accepts_json_and_database_override() {
        let cli =
            Cli::try_parse_from(["fetchx", "diagnostics", "--json", "--database", "fetchx.db"])
                .expect("diagnostics options should parse");
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Command::Diagnostics {
                database: Some(path)
            } if path == std::path::Path::new("fetchx.db")
        ));
    }

    #[tokio::test]
    async fn missing_database_diagnostics_does_not_create_a_file() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let database = directory.path().join("missing.db");

        run(Some(database.clone()), true).await?;

        assert!(!database.exists());
        Ok(())
    }
}

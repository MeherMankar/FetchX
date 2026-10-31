use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use fetchx_storage::{Database, DatabaseCheck};

use crate::{
    args::{DbCommand, DbOpts},
    exit::{self, ExitCode},
    output,
};

pub async fn run(options: DbOpts, json: bool) -> Result<ExitCode, Box<dyn std::error::Error>> {
    match options.command {
        DbCommand::Check { database } => {
            let path = database.unwrap_or_else(crate::bootstrap::database_path);
            let check = Database::check_path(&path).await;
            print_check(&check, json, "check");
            Ok(if check.is_healthy() {
                exit::SUCCESS
            } else {
                exit::GENERAL_ERROR
            })
        }
        DbCommand::Repair { database } => {
            let path = database.unwrap_or_else(crate::bootstrap::database_path);
            let check = Database::check_path(&path).await;
            let safe = check.is_healthy();
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": safe,
                    "action": if safe { "none_needed" } else { "unsafe_to_repair" },
                    "message": if safe {
                        "database is healthy; no repair is needed"
                    } else {
                        "automatic repair is not safe; restore a validated backup or use the recovery guidance"
                    },
                    "check": check,
                }));
            } else if safe {
                output::print_human("Database is healthy; no repair is needed.");
            } else {
                output::print_human(
                    "Automatic repair is not safe. Review the diagnostics and restore a validated backup if needed.",
                );
                print_check(&check, false, "diagnostics");
            }
            Ok(if safe {
                exit::SUCCESS
            } else {
                exit::GENERAL_ERROR
            })
        }
        DbCommand::Backup {
            database,
            output: destination,
        } => {
            let database_path = database.unwrap_or_else(crate::bootstrap::database_path);
            let destination = match destination {
                Some(path) => path,
                None => timestamped_sibling(&database_path, "backup", "bak")?,
            };
            let database = Database::open_existing_path(&database_path).await?;
            let result = database.backup_to(&destination).await?;
            if json {
                output::print_json_ok(&serde_json::json!({ "ok": true, "backup": result }));
            } else {
                output::print_human(&format!(
                    "Validated database backup created at {} ({} bytes; schema {:?}).",
                    result.path.display(),
                    result.size_bytes,
                    result.schema_version
                ));
            }
            Ok(exit::SUCCESS)
        }
        DbCommand::Export {
            database,
            output: destination,
        } => {
            let database_path = database.unwrap_or_else(crate::bootstrap::database_path);
            let destination = match destination {
                Some(path) => path,
                None => timestamped_sibling(&database_path, "export", "json")?,
            };
            let database = Database::open_read_only_path(&database_path).await?;
            let result = database.export_to(&destination).await?;
            if json {
                output::print_json_ok(&serde_json::json!({ "ok": true, "export": result }));
            } else {
                output::print_human(&format!(
                    "Database export created at {} ({} downloads, {} schedules, {} torrent records; {} bytes).",
                    result.path.display(),
                    result.downloads,
                    result.schedules,
                    result.torrent_metadata,
                    result.size_bytes
                ));
            }
            Ok(exit::SUCCESS)
        }
        DbCommand::Import {
            input,
            database,
            downloads_root,
        } => {
            let database_path = database.unwrap_or_else(crate::bootstrap::database_path);
            let downloads_root = downloads_root.unwrap_or_else(crate::bootstrap::downloads_dir);
            let database = Database::open_existing_path(&database_path).await?;
            let result = database.import_from(&input, &downloads_root).await?;
            if json {
                output::print_json_ok(&serde_json::json!({ "ok": true, "import": result }));
            } else {
                output::print_human(&format!(
                    "Imported {} downloads, {} schedules, and {} torrent records. Requeued {} interrupted downloads. Recovery backup: {}",
                    result.downloads,
                    result.schedules,
                    result.torrent_metadata,
                    result.interrupted_downloads_requeued,
                    result.recovery_backup.display()
                ));
            }
            Ok(exit::SUCCESS)
        }
    }
}

fn print_check(check: &DatabaseCheck, json: bool, label: &str) {
    if json {
        output::print_json_ok(&serde_json::json!({
            "ok": check.is_healthy(),
            "operation": label,
            "check": check,
        }));
        return;
    }
    output::print_human(&format!(
        "Database: {}\nSchema: {:?} (version {:?}/{})\nSQLite integrity: {:?}\nForeign keys: {:?} ({} violations)\nRequired tables: {:?}{}\nRequired indexes: {:?}{}\nTable counts: {:?}",
        check.database_path.as_deref().unwrap_or("<unknown>"),
        check.schema.state,
        check.schema.database_schema_version,
        check.schema.current_schema_version,
        check.sqlite_integrity,
        check.foreign_keys,
        check.foreign_key_violations,
        check.required_tables,
        display_missing(&check.missing_tables),
        check.required_indexes,
        display_missing(&check.missing_indexes),
        check.table_counts
    ));
    for error in &check.errors {
        eprintln!("error: {error}");
    }
    for warning in &check.warnings {
        eprintln!("warning: {warning}");
    }
}

fn display_missing(values: &[String]) -> String {
    if values.is_empty() {
        String::new()
    } else {
        format!(" (missing: {})", values.join(", "))
    }
}

fn timestamped_sibling(
    database_path: &Path,
    kind: &str,
    extension: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let filename = database_path
        .file_name()
        .ok_or("database path has no filename")?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut name = filename.to_os_string();
    name.push(format!(".{kind}-{timestamp}.{extension}"));
    Ok(database_path.with_file_name(name))
}

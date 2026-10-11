//! `mydownloader plugins` command.
//!
//! Lists, inspects, and shows status of registered plugins.
//! Uses the shared PluginManager; does NOT add download logic here.

use std::sync::Arc;

use mydownloader_plugin_manager::PluginManager;

use crate::{
    args::{PluginsCommand, PluginsOpts},
    bootstrap,
    exit::{self, ExitCode},
    output,
};

pub async fn run(opts: PluginsOpts, json: bool) -> ExitCode {
    // Resolve plugin directory from the same base as the database.
    let plugin_dir = match bootstrap::plugin_dir() {
        Some(p) => p,
        None => {
            output::print_error(json, "could not resolve plugin directory");
            return exit::GENERAL_ERROR;
        }
    };

    let manager = PluginManager::new(plugin_dir);
    let discovered = manager.discover().await;

    if discovered.invalid > 0 {
        // Non-fatal: just report
        if !json {
            eprintln!(
                "warning: {} plugin manifest(s) failed validation",
                discovered.invalid
            );
        }
    }

    match opts.command.unwrap_or(PluginsCommand::List) {
        PluginsCommand::List => list(&manager, json).await,
        PluginsCommand::Info { id } => info(&manager, &id, json).await,
        PluginsCommand::Status { id } => status(&manager, &id, json).await,
    }
}

async fn list(manager: &Arc<PluginManager>, json: bool) -> ExitCode {
    let plugins = manager.list().await;

    if json {
        let rows: Vec<_> = plugins
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id":           p.id,
                    "name":         p.name,
                    "version":      p.version,
                    "state":        p.state.as_str(),
                    "capabilities": p.capabilities,
                })
            })
            .collect();
        output::print_json_ok(&serde_json::json!({
            "ok": true,
            "count": rows.len(),
            "plugins": rows,
        }));
    } else if plugins.is_empty() {
        println!("No plugins registered.");
        println!("Install plugins in the plugins/ directory alongside the database.");
    } else {
        let mut table = output::Table::new(vec!["ID", "NAME", "VERSION", "STATE", "CAPABILITIES"]);
        for p in &plugins {
            table.add_row(vec![
                p.id.clone(),
                p.name.clone(),
                p.version.clone(),
                p.state.as_str().to_owned(),
                p.capabilities.join(", "),
            ]);
        }
        table.print();
        println!();
        println!("{} plugin(s) registered", plugins.len());
    }

    exit::SUCCESS
}

async fn info(manager: &Arc<PluginManager>, id: &str, json: bool) -> ExitCode {
    match manager.get(id).await {
        Some(p) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true,
                    "plugin": {
                        "id":           p.id,
                        "name":         p.name,
                        "version":      p.version,
                        "state":        p.state.as_str(),
                        "capabilities": p.capabilities,
                    }
                }));
            } else {
                println!("Plugin: {}", p.id);
                println!("  Name:         {}", p.name);
                println!("  Version:      {}", p.version);
                println!("  State:        {}", p.state.as_str());
                println!("  Capabilities: {}", p.capabilities.join(", "));
            }
            exit::SUCCESS
        }
        None => {
            output::print_error(json, &format!("plugin `{id}` not found"));
            exit::NOT_FOUND
        }
    }
}

async fn status(manager: &Arc<PluginManager>, id: &str, json: bool) -> ExitCode {
    match manager.get(id).await {
        Some(p) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true,
                    "id":    p.id,
                    "state": p.state.as_str(),
                }));
            } else {
                println!("{}: {}", p.id, p.state.as_str());
            }
            exit::SUCCESS
        }
        None => {
            output::print_error(json, &format!("plugin `{id}` not found"));
            exit::NOT_FOUND
        }
    }
}

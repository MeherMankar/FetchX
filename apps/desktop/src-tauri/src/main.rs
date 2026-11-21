#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
pub mod browser_extension_installer;
mod browser_ipc;
mod clipboard_monitor;
mod commands;
mod engine_commands;
mod events;
mod media_processing;

/// Shared state that maps a notification ID to the parent directory of the
/// completed download. Used by the OS notification action handler to open the
/// containing folder when the user clicks "Show file".
#[derive(Default)]
pub struct PendingNotificationPaths(
    std::sync::Mutex<std::collections::HashMap<i32, std::path::PathBuf>>,
);

impl PendingNotificationPaths {
    pub fn insert(&self, id: i32, path: std::path::PathBuf) {
        if let Ok(mut map) = self.0.lock() {
            // Keep map small: evict entries beyond a reasonable cap.
            if map.len() >= 128 {
                if let Some(oldest) = map.keys().next().copied() {
                    map.remove(&oldest);
                }
            }
            map.insert(id, path);
        }
    }

    pub fn get(&self, id: i32) -> Option<std::path::PathBuf> {
        self.0.lock().ok()?.get(&id).cloned()
    }
}

fn main() {
    std::panic::set_hook(Box::new(|panic_info| {
        let crash_log = std::env::temp_dir().join("fetchx-crash.log");
        let payload = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let backtrace = std::backtrace::Backtrace::capture();
        let report = format!(
            "--- FetchX Crash Report ---\nLocation: {}\nMessage: {}\nBacktrace:\n{}\n",
            location, payload, backtrace
        );

        let _ = std::fs::write(&crash_log, &report);
        eprintln!("{report}");
    }));

    if let Err(error) = app::run() {
        let crash_log = std::env::temp_dir().join("fetchx-crash.log");
        let report = format!("--- FetchX Startup Error ---\nError: {error}\n");
        let _ = std::fs::write(&crash_log, &report);
        eprintln!("FetchX desktop failed to start: {error}");
        std::process::exit(1);
    }
}

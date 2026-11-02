#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
mod browser_ipc;
mod commands;
mod engine_commands;
mod events;
mod helper_manager;

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

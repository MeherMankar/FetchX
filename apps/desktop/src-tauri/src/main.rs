#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
mod browser_ipc;
mod commands;
mod config;
mod engine_commands;
mod engines;
mod events;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("FetchX desktop failed to start: {error}");
        std::process::exit(1);
    }
}


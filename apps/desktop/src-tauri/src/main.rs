#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod app;
mod commands;
mod config;
mod engine_commands;
mod engines;
mod events;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("MyDownloader desktop failed to start: {error}");
        std::process::exit(1);
    }
}

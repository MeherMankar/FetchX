use std::path::{Path, PathBuf};

fn main() {
    let engines_dir = std::path::Path::new("engines");
    std::fs::create_dir_all(engines_dir).unwrap();

    let helper_bins = [
        ("aria2c", "aria2c"),
        ("yt-dlp", "yt-dlp"),
        ("ffmpeg", "ffmpeg"),
        ("ffprobe", "ffprobe"),
    ];

    let mut missing_helpers = Vec::new();
    for (lookup_name, final_name) in helper_bins {
        if let Some(source) = resolve_helper_binary(lookup_name) {
            let destination = engines_dir.join(final_name);
            if source != destination {
                let _ = std::fs::copy(&source, &destination);
            }
        } else {
            missing_helpers.push(final_name);
        }
    }

    if !missing_helpers.is_empty() {
        println!(
            "cargo:warning=Optional helper binaries are not installed on this build machine: {}. Install them before packaging the desktop installer or use the in-app install links.",
            missing_helpers.join(", ")
        );
        println!(
            "cargo:warning=Suggested downloads: aria2c -> https://github.com/aria2/aria2/releases, yt-dlp -> https://github.com/yt-dlp/yt-dlp/releases, FFmpeg/FFprobe -> https://www.ffmpeg.org/download.html"
        );
    }

    tauri_build::build()
}

fn resolve_helper_binary(name: &str) -> Option<std::path::PathBuf> {
    let candidate_names = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };

    for dir in executable_search_dirs() {
        for candidate_name in &candidate_names {
            let candidate = dir.join(candidate_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn executable_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }

    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        dirs.push(PathBuf::from(program_files));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        dirs.push(PathBuf::from(program_files_x86));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local_app_data));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let root = PathBuf::from(user_profile);
        dirs.push(
            root.join("AppData")
                .join("Local")
                .join("Microsoft")
                .join("WindowsApps"),
        );
        dirs.push(root.join("Downloads"));
    }

    if let Some(app_dir) = std::env::var_os("APPDIR") {
        let app_dir = PathBuf::from(app_dir);
        dirs.push(app_dir.join("Contents").join("Resources"));
        dirs.push(app_dir.join("Resources"));
    }

    dirs.push(std::env::current_dir().unwrap_or_default());
    dirs.push(
        std::env::current_exe()
            .unwrap_or_default()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    );

    dirs.dedup();
    dirs
}

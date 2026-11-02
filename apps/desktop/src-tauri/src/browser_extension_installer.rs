//! Browser Extension Auto-Installer & Native Messaging Host Registrar
//!
//! Automatically extracts the FetchX browser extension files and registers the
//! extension and Native Messaging Host into all available Chromium and Gecko browsers
//! (Google Chrome, Microsoft Edge, Brave, Vivaldi, Opera, Opera GX, Arc, Yandex,
//! Thorium, Chromium, Mozilla Firefox, LibreWolf, Waterfox, Floorp) upon app
//! startup and installer setup.
//!
//! # Deterministic Extension IDs
//! - Chromium: `iemejekblepcfhklcadedfdmgnhjifio` (derived from fixed RSA public key in `manifest.json`)
//! - Firefox: `FetchX@FetchX.app` (set in `manifest.json` under `browser_specific_settings.gecko.id`)

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CHROMIUM_EXTENSION_ID: &str = "iemejekblepcfhklcadedfdmgnhjifio";
pub const FIREFOX_EXTENSION_ID: &str = "FetchX@FetchX.app";
pub const NATIVE_HOST_NAME: &str = "com.fetchx.native_host";
pub const EXTENSION_VERSION: &str = "0.2.0";

struct EmbeddedFile {
    relative_path: &'static str,
    bytes: &'static [u8],
}

const CHROMIUM_FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        relative_path: "manifest.json",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/manifest.json"),
    },
    EmbeddedFile {
        relative_path: "popup.html",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/popup.html"),
    },
    EmbeddedFile {
        relative_path: "dist/background.js",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/dist/background.js"),
    },
    EmbeddedFile {
        relative_path: "dist/content.js",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/dist/content.js"),
    },
    EmbeddedFile {
        relative_path: "dist/fetchx-extension.css",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/dist/fetchx-extension.css"),
    },
    EmbeddedFile {
        relative_path: "dist/popup.js",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/dist/popup.js"),
    },
    EmbeddedFile {
        relative_path: "icons/icon16.png",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/icons/icon16.png"),
    },
    EmbeddedFile {
        relative_path: "icons/icon32.png",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/icons/icon32.png"),
    },
    EmbeddedFile {
        relative_path: "icons/icon48.png",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/icons/icon48.png"),
    },
    EmbeddedFile {
        relative_path: "icons/icon128.png",
        bytes: include_bytes!("../../../browser/extension/dist-chromium/icons/icon128.png"),
    },
];

const FIREFOX_FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        relative_path: "manifest.json",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/manifest.json"),
    },
    EmbeddedFile {
        relative_path: "popup.html",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/popup.html"),
    },
    EmbeddedFile {
        relative_path: "dist/background.js",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/dist/background.js"),
    },
    EmbeddedFile {
        relative_path: "dist/content.js",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/dist/content.js"),
    },
    EmbeddedFile {
        relative_path: "dist/fetchx-extension.css",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/dist/fetchx-extension.css"),
    },
    EmbeddedFile {
        relative_path: "dist/popup.js",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/dist/popup.js"),
    },
    EmbeddedFile {
        relative_path: "icons/icon16.png",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/icons/icon16.png"),
    },
    EmbeddedFile {
        relative_path: "icons/icon32.png",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/icons/icon32.png"),
    },
    EmbeddedFile {
        relative_path: "icons/icon48.png",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/icons/icon48.png"),
    },
    EmbeddedFile {
        relative_path: "icons/icon128.png",
        bytes: include_bytes!("../../../browser/extension/dist-firefox/icons/icon128.png"),
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRegistrationResult {
    pub browser: String,
    pub detected: bool,
    pub native_host_registered: bool,
    pub extension_registered: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInstallSummary {
    pub success: bool,
    pub chromium_extension_dir: String,
    pub firefox_extension_dir: String,
    pub native_host_binary: Option<String>,
    pub chromium_manifest: Option<String>,
    pub firefox_manifest: Option<String>,
    pub registrations: Vec<BrowserRegistrationResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInstallStatus {
    pub extension_id_chromium: String,
    pub extension_id_firefox: String,
    pub chromium_extension_path: String,
    pub firefox_extension_path: String,
    pub chromium_extension_installed: bool,
    pub firefox_extension_installed: bool,
    pub native_host_binary_path: Option<String>,
    pub browsers: Vec<BrowserRegistrationResult>,
}

/// Extracts embedded extension files into the target directory if needed.
pub fn extract_extension_files(base_dir: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    let chromium_dir = base_dir.join("chromium");
    let firefox_dir = base_dir.join("firefox");

    write_embedded_files(&chromium_dir, CHROMIUM_FILES)?;
    write_embedded_files(&firefox_dir, FIREFOX_FILES)?;

    Ok((chromium_dir, firefox_dir))
}

fn write_embedded_files(target_dir: &Path, files: &[EmbeddedFile]) -> std::io::Result<()> {
    for file in files {
        let dest = target_dir.join(file.relative_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Only overwrite if file doesn't exist or content is different to avoid needless disk writes.
        let needs_write = match std::fs::read(&dest) {
            Ok(existing) => existing != file.bytes,
            Err(_) => true,
        };

        if needs_write {
            std::fs::write(&dest, file.bytes)?;
        }
    }
    Ok(())
}

/// Locates the native host binary (`fetchx-native-host.exe`).
pub fn locate_native_host_binary(
    app_data_dir: &Path,
    bundled_engines_dir: &Path,
) -> Option<PathBuf> {
    let binary_name = if cfg!(windows) {
        "fetchx-native-host.exe"
    } else {
        "fetchx-native-host"
    };

    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. Next to the current executable (standard release / installed location)
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        candidates.push(dir.join(binary_name));
    }

    // 2. In app data bin directory
    candidates.push(app_data_dir.join("bin").join(binary_name));

    // 3. In bundled engines directory
    candidates.push(bundled_engines_dir.join(binary_name));

    // 4. In development target directories (relative to current working directory)
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("target").join("release").join(binary_name));
        candidates.push(current_dir.join("target").join("debug").join(binary_name));
        candidates.push(
            current_dir
                .join("..")
                .join("target")
                .join("release")
                .join(binary_name),
        );
        candidates.push(
            current_dir
                .join("..")
                .join("..")
                .join("target")
                .join("release")
                .join(binary_name),
        );
        candidates.push(
            current_dir
                .join("..")
                .join("..")
                .join("target")
                .join("debug")
                .join(binary_name),
        );
    }

    // 5. Check PATH
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            candidates.push(path.join(binary_name));
        }
    }

    for candidate in candidates {
        if candidate.is_file() {
            if let Ok(canonical) = candidate.canonicalize() {
                return Some(canonical);
            }
            return Some(candidate);
        }
    }

    None
}

/// Writes the Native Messaging Host JSON manifests for Chromium browsers and Firefox.
pub fn generate_native_host_manifests(
    manifest_dir: &Path,
    host_exe_path: &Path,
) -> std::io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(manifest_dir)?;

    let exe_str = host_exe_path.to_string_lossy().to_string();

    let chromium_manifest = serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "FetchX native messaging host",
        "path": exe_str,
        "type": "stdio",
        "allowed_origins": [
            format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")
        ]
    });

    let firefox_manifest = serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "FetchX native messaging host",
        "path": exe_str,
        "type": "stdio",
        "allowed_extensions": [
            FIREFOX_EXTENSION_ID
        ]
    });

    let chromium_manifest_path = manifest_dir.join(format!("{NATIVE_HOST_NAME}.json"));
    let firefox_manifest_path = manifest_dir.join(format!("{NATIVE_HOST_NAME}.firefox.json"));

    let chromium_json =
        serde_json::to_string_pretty(&chromium_manifest).map_err(std::io::Error::other)?;
    let firefox_json =
        serde_json::to_string_pretty(&firefox_manifest).map_err(std::io::Error::other)?;

    std::fs::write(&chromium_manifest_path, chromium_json.as_bytes())?;
    std::fs::write(&firefox_manifest_path, firefox_json.as_bytes())?;

    Ok((chromium_manifest_path, firefox_manifest_path))
}

struct BrowserTarget {
    display_name: &'static str,
    native_host_key: &'static str,
    extension_key: &'static str,
    is_chromium: bool,
    detect_paths: &'static [&'static str],
    detect_reg_keys: &'static [&'static str],
}

const BROWSER_TARGETS: &[BrowserTarget] = &[
    // --- Google Chrome Family ---
    BrowserTarget {
        display_name: "Google Chrome",
        native_host_key: r"Software\Google\Chrome\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Google\Chrome\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Google\\Chrome\\Application\\chrome.exe"],
        detect_reg_keys: &["Software\\Google\\Chrome"],
    },
    BrowserTarget {
        display_name: "Google Chrome Beta",
        native_host_key: r"Software\Google\Chrome Beta\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Google\Chrome Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Google\\Chrome Beta\\Application\\chrome.exe"],
        detect_reg_keys: &["Software\\Google\\Chrome Beta"],
    },
    BrowserTarget {
        display_name: "Google Chrome Dev",
        native_host_key: r"Software\Google\Chrome Dev\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Google\Chrome Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Google\\Chrome Dev\\Application\\chrome.exe"],
        detect_reg_keys: &["Software\\Google\\Chrome Dev"],
    },
    BrowserTarget {
        display_name: "Google Chrome Canary",
        native_host_key: r"Software\Google\Chrome SxS\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Google\Chrome SxS\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Google\\Chrome SxS\\Application\\chrome.exe"],
        detect_reg_keys: &["Software\\Google\\Chrome SxS"],
    },
    BrowserTarget {
        display_name: "Chromium",
        native_host_key: r"Software\Chromium\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Chromium\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Chromium\\Application\\chrome.exe"],
        detect_reg_keys: &["Software\\Chromium"],
    },
    // --- Microsoft Edge Family ---
    BrowserTarget {
        display_name: "Microsoft Edge",
        native_host_key: r"Software\Microsoft\Edge\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Microsoft\Edge\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Microsoft\\Edge\\Application\\msedge.exe"],
        detect_reg_keys: &["Software\\Microsoft\\Edge"],
    },
    BrowserTarget {
        display_name: "Microsoft Edge Beta",
        native_host_key: r"Software\Microsoft\Edge Beta\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Microsoft\Edge Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Microsoft\\Edge Beta\\Application\\msedge.exe"],
        detect_reg_keys: &["Software\\Microsoft\\Edge Beta"],
    },
    BrowserTarget {
        display_name: "Microsoft Edge Dev",
        native_host_key: r"Software\Microsoft\Edge Dev\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Microsoft\Edge Dev\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Microsoft\\Edge Dev\\Application\\msedge.exe"],
        detect_reg_keys: &["Software\\Microsoft\\Edge Dev"],
    },
    BrowserTarget {
        display_name: "Microsoft Edge Canary",
        native_host_key: {
            r"Software\Microsoft\Edge Canary\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\Microsoft\Edge Canary\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Microsoft\\Edge Canary\\Application\\msedge.exe"],
        detect_reg_keys: &["Software\\Microsoft\\Edge Canary"],
    },
    // --- Brave Browser Family ---
    BrowserTarget {
        display_name: "Brave Browser",
        native_host_key: {
            r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\BraveSoftware\Brave-Browser\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["BraveSoftware\\Brave-Browser\\Application\\brave.exe"],
        detect_reg_keys: &["Software\\BraveSoftware\\Brave-Browser"],
    },
    BrowserTarget {
        display_name: "Brave Browser Beta",
        native_host_key: {
            r"Software\BraveSoftware\Brave-Browser-Beta\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\BraveSoftware\Brave-Browser-Beta\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["BraveSoftware\\Brave-Browser-Beta\\Application\\brave.exe"],
        detect_reg_keys: &["Software\\BraveSoftware\\Brave-Browser-Beta"],
    },
    BrowserTarget {
        display_name: "Brave Browser Nightly",
        native_host_key: {
            r"Software\BraveSoftware\Brave-Browser-Nightly\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\BraveSoftware\Brave-Browser-Nightly\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["BraveSoftware\\Brave-Browser-Nightly\\Application\\brave.exe"],
        detect_reg_keys: &["Software\\BraveSoftware\\Brave-Browser-Nightly"],
    },
    // --- Vivaldi ---
    BrowserTarget {
        display_name: "Vivaldi",
        native_host_key: r"Software\Vivaldi\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Vivaldi\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Vivaldi\\Application\\vivaldi.exe"],
        detect_reg_keys: &["Software\\Vivaldi"],
    },
    // --- Opera Family ---
    BrowserTarget {
        display_name: "Opera",
        native_host_key: {
            r"Software\Opera Software\Opera Stable\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\Opera Software\Opera Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Programs\\Opera\\opera.exe", "Opera\\opera.exe"],
        detect_reg_keys: &["Software\\Opera Software\\Opera Stable"],
    },
    BrowserTarget {
        display_name: "Opera GX",
        native_host_key: {
            r"Software\Opera Software\Opera GX Stable\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\Opera Software\Opera GX Stable\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Programs\\Opera GX\\opera.exe", "Opera GX\\opera.exe"],
        detect_reg_keys: &["Software\\Opera Software\\Opera GX Stable"],
    },
    BrowserTarget {
        display_name: "Opera Next",
        native_host_key: {
            r"Software\Opera Software\Opera Next\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\Opera Software\Opera Next\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Programs\\Opera Next\\opera.exe"],
        detect_reg_keys: &["Software\\Opera Software\\Opera Next"],
    },
    BrowserTarget {
        display_name: "Opera Developer",
        native_host_key: {
            r"Software\Opera Software\Opera Developer\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\Opera Software\Opera Developer\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Programs\\Opera Developer\\opera.exe"],
        detect_reg_keys: &["Software\\Opera Software\\Opera Developer"],
    },
    // --- Other Popular Chromium Browsers ---
    BrowserTarget {
        display_name: "Arc Browser",
        native_host_key: {
            r"Software\The Browser Company\Arc\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\The Browser Company\Arc\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Arc\\Arc.exe"],
        detect_reg_keys: &["Software\\The Browser Company\\Arc"],
    },
    BrowserTarget {
        display_name: "Yandex Browser",
        native_host_key: {
            r"Software\Yandex\YandexBrowser\NativeMessagingHosts\com.fetchx.native_host"
        },
        extension_key: {
            r"Software\Yandex\YandexBrowser\Extensions\iemejekblepcfhklcadedfdmgnhjifio"
        },
        is_chromium: true,
        detect_paths: &["Yandex\\YandexBrowser\\Application\\browser.exe"],
        detect_reg_keys: &["Software\\Yandex\\YandexBrowser"],
    },
    BrowserTarget {
        display_name: "Thorium",
        native_host_key: r"Software\Thorium\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Thorium\Extensions\iemejekblepcfhklcadedfdmgnhjifio",
        is_chromium: true,
        detect_paths: &["Thorium\\Application\\thorium.exe"],
        detect_reg_keys: &["Software\\Thorium"],
    },
    // --- Mozilla Firefox & Gecko Family ---
    BrowserTarget {
        display_name: "Mozilla Firefox",
        native_host_key: r"Software\Mozilla\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Mozilla\Firefox\Extensions",
        is_chromium: false,
        detect_paths: &["Mozilla Firefox\\firefox.exe"],
        detect_reg_keys: &[
            "Software\\Mozilla\\Mozilla Firefox",
            "Software\\Mozilla\\Firefox",
        ],
    },
    BrowserTarget {
        display_name: "LibreWolf",
        native_host_key: r"Software\LibreWolf\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\LibreWolf\Extensions",
        is_chromium: false,
        detect_paths: &["LibreWolf\\librewolf.exe"],
        detect_reg_keys: &["Software\\LibreWolf"],
    },
    BrowserTarget {
        display_name: "Waterfox",
        native_host_key: r"Software\Waterfox\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Waterfox\Extensions",
        is_chromium: false,
        detect_paths: &["Waterfox\\waterfox.exe"],
        detect_reg_keys: &["Software\\Waterfox"],
    },
    BrowserTarget {
        display_name: "Floorp",
        native_host_key: r"Software\Floorp\NativeMessagingHosts\com.fetchx.native_host",
        extension_key: r"Software\Floorp\Extensions",
        is_chromium: false,
        detect_paths: &["Floorp\\floorp.exe"],
        detect_reg_keys: &["Software\\Floorp"],
    },
];

#[cfg(windows)]
fn set_reg_value(key: &str, value_name: Option<&str>, value_data: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let full_key = format!("HKCU\\{key}");
    let mut cmd = std::process::Command::new("reg");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg("add").arg(&full_key);
    if let Some(name) = value_name {
        cmd.arg("/v").arg(name);
    } else {
        cmd.arg("/ve");
    }
    cmd.arg("/t")
        .arg("REG_SZ")
        .arg("/d")
        .arg(value_data)
        .arg("/f");

    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::other(format!(
            "reg.exe failed for {full_key}: {stderr}"
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_reg_value(_key: &str, _value_name: Option<&str>, _value_data: &str) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn query_reg_value(key: &str, value_name: Option<&str>) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let full_key = format!("HKCU\\{key}");
    let mut cmd = std::process::Command::new("reg");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg("query").arg(&full_key);
    if let Some(name) = value_name {
        cmd.arg("/v").arg(name);
    } else {
        cmd.arg("/ve");
    }
    match cmd.output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn query_reg_value(_key: &str, _value_name: Option<&str>) -> bool {
    false
}

#[cfg(windows)]
fn query_reg_key_exists(full_key: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = std::process::Command::new("reg");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.arg("query").arg(full_key);
    match cmd.output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn query_reg_key_exists(_full_key: &str) -> bool {
    false
}

fn is_browser_detected(target: &BrowserTarget) -> bool {
    #[cfg(windows)]
    {
        for reg in target.detect_reg_keys {
            if query_reg_key_exists(&format!("HKCU\\{reg}"))
                || query_reg_key_exists(&format!("HKLM\\{reg}"))
            {
                return true;
            }
        }

        let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let program_files_x86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from);

        for rel in target.detect_paths {
            if let Some(ref base) = local_app_data
                && base.join(rel).exists()
            {
                return true;
            }
            if let Some(ref base) = program_files
                && base.join(rel).exists()
            {
                return true;
            }
            if let Some(ref base) = program_files_x86
                && base.join(rel).exists()
            {
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Registers the native host and external extensions into the Windows Registry for all supported browsers.
pub fn register_browser_keys(
    chromium_manifest: Option<&Path>,
    firefox_manifest: Option<&Path>,
    chromium_ext_dir: &Path,
    firefox_ext_dir: &Path,
) -> Vec<BrowserRegistrationResult> {
    let mut results = Vec::new();

    for target in BROWSER_TARGETS {
        let detected = is_browser_detected(target);
        let mut result = BrowserRegistrationResult {
            browser: target.display_name.to_string(),
            detected,
            native_host_registered: false,
            extension_registered: false,
            error: None,
        };

        // 1. Native host registration
        let manifest_path = if target.is_chromium {
            chromium_manifest
        } else {
            firefox_manifest
        };

        if let Some(manifest) = manifest_path {
            let manifest_str = manifest.to_string_lossy().to_string();
            match set_reg_value(target.native_host_key, None, &manifest_str) {
                Ok(_) => result.native_host_registered = true,
                Err(e) => {
                    result.error = Some(format!("Native host reg failed: {e}"));
                }
            }
        }

        // 2. Extension registration
        if target.is_chromium {
            let ext_dir_str = chromium_ext_dir.to_string_lossy().to_string();
            let p_res = set_reg_value(target.extension_key, Some("path"), &ext_dir_str);
            let v_res = set_reg_value(target.extension_key, Some("version"), EXTENSION_VERSION);
            if p_res.is_ok() && v_res.is_ok() {
                result.extension_registered = true;
            } else if result.error.is_none()
                && let Some(e) = p_res.err().or(v_res.err())
            {
                result.error = Some(format!("Extension reg failed: {e}"));
            }
        } else {
            let ext_dir_str = firefox_ext_dir.to_string_lossy().to_string();
            match set_reg_value(
                target.extension_key,
                Some(FIREFOX_EXTENSION_ID),
                &ext_dir_str,
            ) {
                Ok(_) => result.extension_registered = true,
                Err(e) => {
                    if result.error.is_none() {
                        result.error = Some(format!("Firefox extension reg failed: {e}"));
                    }
                }
            }
        }

        results.push(result);
    }

    results
}

/// Ensures that the browser extension files and registry settings are fully installed and up to date.
pub fn ensure_registered(
    app_data_dir: &Path,
    bundled_engines_dir: &Path,
) -> Result<ExtensionInstallSummary, String> {
    let extension_base_dir = app_data_dir.join("browser-extension");
    let (chromium_dir, firefox_dir) = extract_extension_files(&extension_base_dir)
        .map_err(|e| format!("Failed to extract extension files: {e}"))?;

    let native_host_bin = locate_native_host_binary(app_data_dir, bundled_engines_dir);

    let mut chromium_manifest_path = None;
    let mut firefox_manifest_path = None;

    if let Some(ref host_exe) = native_host_bin {
        let manifest_dir = app_data_dir.join("native-host");
        match generate_native_host_manifests(&manifest_dir, host_exe) {
            Ok((c_man, f_man)) => {
                chromium_manifest_path = Some(c_man);
                firefox_manifest_path = Some(f_man);
            }
            Err(e) => {
                tracing::warn!(%e, "Failed to write native messaging host manifests");
            }
        }
    } else {
        tracing::info!("Native host executable not found yet; skipping manifest generation");
    }

    let registrations = register_browser_keys(
        chromium_manifest_path.as_deref(),
        firefox_manifest_path.as_deref(),
        &chromium_dir,
        &firefox_dir,
    );

    let summary = ExtensionInstallSummary {
        success: registrations.iter().all(|r| r.error.is_none()),
        chromium_extension_dir: chromium_dir.to_string_lossy().to_string(),
        firefox_extension_dir: firefox_dir.to_string_lossy().to_string(),
        native_host_binary: native_host_bin.map(|p| p.to_string_lossy().to_string()),
        chromium_manifest: chromium_manifest_path.map(|p| p.to_string_lossy().to_string()),
        firefox_manifest: firefox_manifest_path.map(|p| p.to_string_lossy().to_string()),
        registrations,
    };

    Ok(summary)
}

/// Queries current status of browser extensions and registry keys.
pub fn get_extension_status(
    app_data_dir: &Path,
    bundled_engines_dir: &Path,
) -> ExtensionInstallStatus {
    let chromium_dir = app_data_dir.join("browser-extension").join("chromium");
    let firefox_dir = app_data_dir.join("browser-extension").join("firefox");

    let chromium_installed = chromium_dir.join("manifest.json").is_file();
    let firefox_installed = firefox_dir.join("manifest.json").is_file();

    let native_host_binary = locate_native_host_binary(app_data_dir, bundled_engines_dir)
        .map(|p| p.to_string_lossy().to_string());

    let mut browsers = Vec::new();

    for target in BROWSER_TARGETS {
        let detected = is_browser_detected(target);
        let native_host_registered = query_reg_value(target.native_host_key, None);
        let extension_registered = if target.is_chromium {
            query_reg_value(target.extension_key, Some("path"))
        } else {
            query_reg_value(target.extension_key, Some(FIREFOX_EXTENSION_ID))
        };

        browsers.push(BrowserRegistrationResult {
            browser: target.display_name.to_string(),
            detected,
            native_host_registered,
            extension_registered,
            error: None,
        });
    }

    ExtensionInstallStatus {
        extension_id_chromium: CHROMIUM_EXTENSION_ID.to_string(),
        extension_id_firefox: FIREFOX_EXTENSION_ID.to_string(),
        chromium_extension_path: chromium_dir.to_string_lossy().to_string(),
        firefox_extension_path: firefox_dir.to_string_lossy().to_string(),
        chromium_extension_installed: chromium_installed,
        firefox_extension_installed: firefox_installed,
        native_host_binary_path: native_host_binary,
        browsers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_files_present_and_valid() {
        assert!(!CHROMIUM_FILES.is_empty());
        assert!(!FIREFOX_FILES.is_empty());

        let chromium_manifest = CHROMIUM_FILES
            .iter()
            .find(|f| f.relative_path == "manifest.json")
            .expect("chromium manifest present");
        let parsed: serde_json::Value =
            serde_json::from_slice(chromium_manifest.bytes).expect("valid chromium manifest json");
        assert_eq!(parsed["name"], "FetchX");
        assert!(
            parsed["key"]
                .as_str()
                .expect("key is string")
                .starts_with("MIIBIjAN")
        );

        let firefox_manifest = FIREFOX_FILES
            .iter()
            .find(|f| f.relative_path == "manifest.json")
            .expect("firefox manifest present");
        let parsed_ff: serde_json::Value =
            serde_json::from_slice(firefox_manifest.bytes).expect("valid firefox manifest json");
        assert_eq!(parsed_ff["name"], "FetchX");
        assert_eq!(
            parsed_ff["browser_specific_settings"]["gecko"]["id"],
            FIREFOX_EXTENSION_ID
        );
    }

    #[test]
    fn test_extract_extension_files_creates_structure() {
        let temp = tempfile::tempdir().expect("temp dir");
        let (chromium_dir, firefox_dir) = extract_extension_files(temp.path()).expect("extract ok");

        assert!(chromium_dir.join("manifest.json").exists());
        assert!(chromium_dir.join("popup.html").exists());
        assert!(chromium_dir.join("dist/background.js").exists());
        assert!(chromium_dir.join("icons/icon16.png").exists());

        assert!(firefox_dir.join("manifest.json").exists());
        assert!(firefox_dir.join("popup.html").exists());
        assert!(firefox_dir.join("dist/background.js").exists());
        assert!(firefox_dir.join("icons/icon16.png").exists());
    }

    #[test]
    fn test_manifest_generation() {
        let temp = tempfile::tempdir().expect("temp dir");
        let fake_exe = temp.path().join("fetchx-native-host.exe");
        std::fs::write(&fake_exe, b"MZ").expect("fake exe");

        let (c_man, f_man) =
            generate_native_host_manifests(temp.path(), &fake_exe).expect("generate ok");

        let c_content: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&c_man).unwrap()).unwrap();
        assert_eq!(c_content["name"], NATIVE_HOST_NAME);
        assert_eq!(
            c_content["allowed_origins"][0],
            format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")
        );

        let f_content: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&f_man).unwrap()).unwrap();
        assert_eq!(f_content["name"], NATIVE_HOST_NAME);
        assert_eq!(f_content["allowed_extensions"][0], FIREFOX_EXTENSION_ID);
    }

    #[test]
    fn test_browser_targets_comprehensiveness() {
        assert!(BROWSER_TARGETS.len() >= 20);

        let names: Vec<&str> = BROWSER_TARGETS.iter().map(|b| b.display_name).collect();
        assert!(names.contains(&"Google Chrome"));
        assert!(names.contains(&"Microsoft Edge"));
        assert!(names.contains(&"Brave Browser"));
        assert!(names.contains(&"Vivaldi"));
        assert!(names.contains(&"Opera"));
        assert!(names.contains(&"Opera GX"));
        assert!(names.contains(&"Mozilla Firefox"));
        assert!(names.contains(&"LibreWolf"));
        assert!(names.contains(&"Waterfox"));
    }
}

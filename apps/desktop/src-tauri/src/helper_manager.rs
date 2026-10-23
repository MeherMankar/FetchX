use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use reqwest::{Client, Response, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex, time::timeout};
use uuid::Uuid;

use crate::commands::AppState;

const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ARIA2_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_YTDLP_BYTES: u64 = 100 * 1024 * 1024;
const MAX_FFMPEG_ARCHIVE_BYTES: u64 = 400 * 1024 * 1024;
const MAX_EXTRACTED_EXE_BYTES: u64 = 300 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperId {
    Aria2c,
    YtDlp,
    Ffmpeg,
    Ffprobe,
}

fn verify_sha256(actual: &str, expected: &str) -> Result<(), String> {
    validate_sha256(expected)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("The downloaded archive checksum did not match the trusted checksum.".to_owned())
    }
}

fn is_safe_archive_entry(name: &str, expected: &str) -> bool {
    if name != expected && !name.ends_with(&format!("/{expected}")) {
        return false;
    }
    Path::new(name)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

impl HelperId {
    fn executable_name(self) -> &'static str {
        match self {
            Self::Aria2c => "aria2c.exe",
            Self::YtDlp => "yt-dlp.exe",
            Self::Ffmpeg => "ffmpeg.exe",
            Self::Ffprobe => "ffprobe.exe",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Aria2c => "aria2c",
            Self::YtDlp => "yt-dlp",
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }

    fn install_group(self) -> u8 {
        match self {
            Self::Aria2c => 0,
            Self::YtDlp => 1,
            Self::Ffmpeg | Self::Ffprobe => 2,
        }
    }

    fn supported_install_architecture(self, architecture: &str) -> bool {
        architecture == "x86_64"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HelperStatus {
    pub helper: HelperId,
    pub installed: bool,
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub architecture: Option<String>,
    pub available: bool,
    pub validation: String,
    pub install_supported: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct HelperInstallProgress {
    pub helper: HelperId,
    pub phase: &'static str,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: Option<String>,
}

#[derive(Clone, Debug)]
struct InstallFile {
    helper: HelperId,
    archive_entry: &'static str,
    staged_name: &'static str,
}

#[derive(Clone, Debug)]
struct InstallPackage {
    url: Url,
    sha256: String,
    archive_name: &'static str,
    max_bytes: u64,
    archive_format: ArchiveFormat,
    files: Vec<InstallFile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArchiveFormat {
    DirectExecutable,
    Zip,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub struct HelperManager {
    bin_dir: PathBuf,
    bundled_dir: PathBuf,
    active_installs: Mutex<HashMap<u8, Arc<AtomicBool>>>,
    client: Client,
}

impl HelperManager {
    pub fn new(bin_dir: PathBuf, bundled_dir: PathBuf) -> Self {
        let client = Client::builder()
            .connect_timeout(REQUEST_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .user_agent(concat!("FetchX/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("static HTTP client configuration is valid");
        Self {
            bin_dir,
            bundled_dir,
            active_installs: Mutex::new(HashMap::new()),
            client,
        }
    }

    async fn get_statuses(
        &self,
        configured_paths: &HashMap<HelperId, PathBuf>,
    ) -> Vec<HelperStatus> {
        let mut statuses = Vec::with_capacity(4);
        for helper in [
            HelperId::Aria2c,
            HelperId::YtDlp,
            HelperId::Ffmpeg,
            HelperId::Ffprobe,
        ] {
            statuses.push(self.get_status(helper, configured_paths.get(&helper)).await);
        }
        statuses
    }

    async fn get_status(
        &self,
        helper: HelperId,
        configured_path: Option<&PathBuf>,
    ) -> HelperStatus {
        let executable = self.find_executable(helper, configured_path);
        let install_supported = cfg!(target_os = "windows")
            && helper.supported_install_architecture(std::env::consts::ARCH);
        let Some(executable) = executable else {
            return HelperStatus {
                helper,
                installed: false,
                executable_path: None,
                version: None,
                architecture: None,
                available: false,
                validation: "Executable not found.".to_owned(),
                install_supported,
            };
        };

        let executable_path = executable.to_string_lossy().into_owned();
        match validate_pe_architecture(&executable) {
            Ok(architecture) => match detect_version(helper, &executable).await {
                Ok(version) => HelperStatus {
                    helper,
                    installed: true,
                    executable_path: Some(executable_path),
                    version: Some(version),
                    architecture: Some(architecture.to_owned()),
                    available: true,
                    validation: "Executable architecture and version validated.".to_owned(),
                    install_supported,
                },
                Err(error) => HelperStatus {
                    helper,
                    installed: true,
                    executable_path: Some(executable_path),
                    version: None,
                    architecture: Some(architecture.to_owned()),
                    available: false,
                    validation: error,
                    install_supported,
                },
            },
            Err(error) => HelperStatus {
                helper,
                installed: true,
                executable_path: Some(executable_path),
                version: None,
                architecture: None,
                available: false,
                validation: error,
                install_supported,
            },
        }
    }

    fn find_executable(
        &self,
        helper: HelperId,
        configured_path: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        let filename = helper.executable_name();
        let mut candidates = Vec::new();
        if let Some(path) = configured_path {
            candidates.push(path.clone());
        }
        candidates.push(self.bin_dir.join(filename));
        candidates.push(self.bundled_dir.join(filename));
        if let Some(path) = std::env::var_os("PATH") {
            candidates
                .extend(std::env::split_paths(&path).map(|directory| directory.join(filename)));
        }
        candidates
            .into_iter()
            .find(|candidate| fs::metadata(candidate).is_ok_and(|metadata| metadata.is_file()))
    }

    async fn install(
        &self,
        app: &AppHandle,
        helper: HelperId,
    ) -> Result<Vec<(HelperId, PathBuf)>, String> {
        if !cfg!(target_os = "windows") {
            return Err("Helper installation is currently supported only on Windows.".to_owned());
        }
        if !helper.supported_install_architecture(std::env::consts::ARCH) {
            return Err(format!(
                "Helper installation is not supported for Windows architecture '{}'.",
                std::env::consts::ARCH
            ));
        }

        let group = helper.install_group();
        let cancellation = self.begin_install(group).await?;

        let result = self
            .install_inner(app, helper, Arc::clone(&cancellation))
            .await;
        self.end_install(group).await;
        match &result {
            Ok(_) => {}
            Err(error) => {
                let phase = if cancellation.load(Ordering::Acquire) {
                    "cancelled"
                } else {
                    "failed"
                };
                self.emit(app, helper, phase, 0, None, Some(error.clone()));
            }
        }
        result
    }

    async fn begin_install(&self, group: u8) -> Result<Arc<AtomicBool>, String> {
        let mut active = self.active_installs.lock().await;
        if active.contains_key(&group) {
            return Err("An installation for this helper is already in progress.".to_owned());
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        active.insert(group, Arc::clone(&cancellation));
        Ok(cancellation)
    }

    async fn end_install(&self, group: u8) {
        self.active_installs.lock().await.remove(&group);
    }

    async fn install_inner(
        &self,
        app: &AppHandle,
        helper: HelperId,
        cancellation: Arc<AtomicBool>,
    ) -> Result<Vec<(HelperId, PathBuf)>, String> {
        ensure_not_cancelled(&cancellation)?;
        let package = self.resolve_package(helper, &cancellation).await?;
        ensure_not_cancelled(&cancellation)?;
        let staging = self.create_staging_dir()?;
        let archive_path = staging.join(package.archive_name);
        let result = async {
            self.emit(app, helper, "downloading", 0, None, None);
            self.download_archive(app, helper, &package, &archive_path, &cancellation)
                .await?;

            self.emit(
                app,
                helper,
                "verifying",
                0,
                Some(
                    fs::metadata(&archive_path)
                        .map_err(|e| e.to_string())?
                        .len(),
                ),
                None,
            );
            ensure_not_cancelled(&cancellation)?;

            let files = package.files.clone();
            let archive_format = package.archive_format;
            let archive_path_for_extract = archive_path.clone();
            let staging_for_extract = staging.clone();
            let expected_arch = expected_pe_machine(std::env::consts::ARCH)
                .ok_or_else(|| "This Windows architecture is not supported.".to_owned())?;
            tokio::task::spawn_blocking(move || {
                extract_executables(
                    &archive_path_for_extract,
                    &staging_for_extract,
                    &files,
                    archive_format,
                    expected_arch,
                )
            })
            .await
            .map_err(|error| format!("Executable extraction task failed: {error}"))??;

            ensure_not_cancelled(&cancellation)?;
            let mut staged = Vec::new();
            for file in &package.files {
                let path = staging.join(file.staged_name);
                let version = detect_version(file.helper, &path).await?;
                tracing::info!(
                    helper = file.helper.display_name(),
                    %version,
                    "downloaded helper passed runtime validation"
                );
                staged.push((file.helper, path));
            }

            self.emit(app, helper, "installing", 0, None, None);
            ensure_not_cancelled(&cancellation)?;
            self.commit_install(&staging, &staged)
        }
        .await;

        match fs::remove_dir_all(&staging) {
            Ok(()) => result,
            Err(cleanup_error) if result.is_ok() => Err(format!(
                "Installation completed, but temporary files could not be cleaned up: {cleanup_error}"
            )),
            Err(cleanup_error) => Err(format!(
                "{} Temporary files could not be cleaned up: {cleanup_error}",
                result.expect_err("the result is an error")
            )),
        }
    }

    async fn resolve_package(
        &self,
        helper: HelperId,
        cancellation: &AtomicBool,
    ) -> Result<InstallPackage, String> {
        match helper {
            HelperId::Aria2c => aria2_package(),
            HelperId::YtDlp => self.ytdlp_package(cancellation).await,
            HelperId::Ffmpeg | HelperId::Ffprobe => self.ffmpeg_package(cancellation).await,
        }
    }

    async fn ytdlp_package(&self, cancellation: &AtomicBool) -> Result<InstallPackage, String> {
        let metadata = self
            .get_allowlisted_bytes(
                "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
                MAX_METADATA_BYTES,
                Some(cancellation),
            )
            .await?;
        let release: GithubRelease = serde_json::from_slice(&metadata)
            .map_err(|error| format!("Invalid yt-dlp release metadata: {error}"))?;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == "yt-dlp.exe")
            .ok_or_else(|| {
                "The official yt-dlp release does not publish a Windows x64 executable.".to_owned()
            })?;
        let sha256 = github_digest(asset)?;
        let url = validate_allowlisted_url(&asset.browser_download_url)?;
        if !url.path().ends_with("/yt-dlp.exe") {
            return Err(
                "The yt-dlp release asset URL did not match the expected executable.".to_owned(),
            );
        }
        Ok(InstallPackage {
            url,
            sha256,
            archive_name: "yt-dlp.exe",
            max_bytes: MAX_YTDLP_BYTES,
            archive_format: ArchiveFormat::DirectExecutable,
            files: vec![InstallFile {
                helper: HelperId::YtDlp,
                archive_entry: "yt-dlp.exe",
                staged_name: "yt-dlp.exe",
            }],
        })
    }

    async fn ffmpeg_package(&self, cancellation: &AtomicBool) -> Result<InstallPackage, String> {
        let checksum = self
            .get_allowlisted_bytes(
                "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256",
                4096,
                Some(cancellation),
            )
            .await?;
        let sha256 = parse_checksum_file(&checksum)?;
        let url = validate_allowlisted_url(
            "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
        )?;
        Ok(InstallPackage {
            url,
            sha256,
            archive_name: "ffmpeg.zip",
            max_bytes: MAX_FFMPEG_ARCHIVE_BYTES,
            archive_format: ArchiveFormat::Zip,
            files: vec![
                InstallFile {
                    helper: HelperId::Ffmpeg,
                    archive_entry: "bin/ffmpeg.exe",
                    staged_name: "ffmpeg.exe",
                },
                InstallFile {
                    helper: HelperId::Ffprobe,
                    archive_entry: "bin/ffprobe.exe",
                    staged_name: "ffprobe.exe",
                },
            ],
        })
    }

    async fn get_allowlisted_bytes(
        &self,
        url: &str,
        max_bytes: u64,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, String> {
        let mut response = self.open_allowlisted(url, cancellation).await?;
        let status = response.status();
        if !status.is_success() && (status.as_u16() >= 500 || status.as_u16() == 429) {
            tokio::time::sleep(Duration::from_millis(300)).await;
            response = self.open_allowlisted(url, cancellation).await?;
        }
        if !response.status().is_success() {
            let status = response.status();
            return Err(format!(
                "The official download source returned HTTP {status}."
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("Could not read the official response: {error}"))?
        {
            if cancellation.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Err("Installation was cancelled.".to_owned());
            }
            if body.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err("The official response exceeded its permitted size.".to_owned());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn open_allowlisted(
        &self,
        initial_url: &str,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Response, String> {
        let mut url = validate_allowlisted_url(initial_url)?;
        for _ in 0..=5 {
            let mut retries = 0;
            let response = loop {
                let request = self.client.get(url.clone()).timeout(REQUEST_TIMEOUT).send();
                let result = if let Some(token) = cancellation {
                    tokio::select! {
                        response = request => response,
                        _ = wait_until_cancelled(token) => {
                            return Err("Installation was cancelled.".to_owned());
                        }
                    }
                } else {
                    request.await
                };
                match result {
                    Ok(response) => break response,
                    Err(error) if (error.is_timeout() || error.is_connect()) && retries < 2 => {
                        if cancellation.is_some_and(|token| token.load(Ordering::Acquire)) {
                            return Err("Installation was cancelled.".to_owned());
                        }
                        retries += 1;
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    Err(error) => {
                        return Err(format!(
                            "Could not contact the official download source: {error}"
                        ));
                    }
                }
            };
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| "Official source returned an invalid redirect.".to_owned())?;
                let next = url
                    .join(location)
                    .map_err(|error| format!("Invalid redirect URL: {error}"))?;
                url = validate_redirect_url(&next)?;
                continue;
            }
            return Ok(response);
        }
        Err("The official source exceeded the redirect limit.".to_owned())
    }

    async fn download_archive(
        &self,
        app: &AppHandle,
        helper: HelperId,
        package: &InstallPackage,
        path: &Path,
        cancellation: &AtomicBool,
    ) -> Result<(), String> {
        let mut last_error = "Download failed.".to_owned();
        for attempt in 0..3 {
            ensure_not_cancelled(cancellation)?;
            let response = match self
                .open_allowlisted(package.url.as_str(), Some(cancellation))
                .await
            {
                Ok(response) if response.status().is_success() => response,
                Ok(response) => {
                    let status = response.status();
                    last_error = format!("Official source returned HTTP {status}.");
                    if status.as_u16() < 500 && status.as_u16() != 429 {
                        return Err(last_error);
                    }
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_millis(300 * (attempt + 1))).await;
                        continue;
                    }
                    return Err(last_error);
                }
                Err(error) => {
                    last_error = error;
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_millis(300 * (attempt + 1))).await;
                        continue;
                    }
                    return Err(last_error);
                }
            };
            let total = response.content_length();
            if total.is_some_and(|length| length > package.max_bytes) {
                return Err("The archive exceeds the helper's maximum download size.".to_owned());
            }
            let mut output = tokio::fs::File::create(path)
                .await
                .map_err(|error| format!("Could not create a temporary download: {error}"))?;
            let mut response = response;
            let mut hasher = Sha256::new();
            let mut received = 0_u64;
            let mut failure = None;
            loop {
                let chunk = match tokio::select! {
                    chunk = response.chunk() => chunk,
                    _ = wait_until_cancelled(cancellation) => {
                        failure = Some("Installation was cancelled.".to_owned());
                        break;
                    }
                } {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(error) => {
                        failure = Some(format!("The archive download was interrupted: {error}"));
                        break;
                    }
                };
                if cancellation.load(Ordering::Acquire) {
                    failure = Some("Installation was cancelled.".to_owned());
                    break;
                }
                received += chunk.len() as u64;
                if received > package.max_bytes {
                    failure =
                        Some("The archive exceeds the helper's maximum download size.".to_owned());
                    break;
                }
                hasher.update(&chunk);
                if let Err(error) = output.write_all(&chunk).await {
                    failure = Some(format!("Could not write the temporary download: {error}"));
                    break;
                }
                self.emit(app, helper, "downloading", received, total, None);
            }
            if let Some(error) = failure {
                let _ = tokio::fs::remove_file(path).await;
                if error == "Installation was cancelled."
                    || error.contains("maximum download size")
                    || error.contains("write the temporary")
                {
                    return Err(error);
                }
                last_error = error;
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(300 * (attempt + 1))).await;
                    continue;
                }
                return Err(last_error);
            }
            output
                .flush()
                .await
                .map_err(|error| format!("Could not flush the temporary download: {error}"))?;
            let actual = hex_digest(hasher.finalize().as_slice());
            if let Err(error) = verify_sha256(&actual, &package.sha256) {
                let _ = tokio::fs::remove_file(path).await;
                return Err(error);
            }
            return Ok(());
        }
        Err(last_error)
    }

    fn create_staging_dir(&self) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.bin_dir)
            .map_err(|error| format!("Could not create the FetchX helper directory: {error}"))?;
        reject_reparse_point(&self.bin_dir)?;
        let canonical_bin = fs::canonicalize(&self.bin_dir)
            .map_err(|error| format!("Could not resolve the helper directory: {error}"))?;
        let staging = canonical_bin.join(format!(".install-{}", Uuid::new_v4()));
        fs::create_dir(&staging)
            .map_err(|error| format!("Could not create a temporary install directory: {error}"))?;
        Ok(staging)
    }

    fn commit_install(
        &self,
        staging: &Path,
        files: &[(HelperId, PathBuf)],
    ) -> Result<Vec<(HelperId, PathBuf)>, String> {
        reject_reparse_point(&self.bin_dir)?;
        let canonical_bin = fs::canonicalize(&self.bin_dir)
            .map_err(|error| format!("Could not resolve the helper directory: {error}"))?;
        if staging.parent() != Some(canonical_bin.as_path()) {
            return Err(
                "The temporary install directory is outside the managed helper directory."
                    .to_owned(),
            );
        }
        let mut targets = Vec::new();
        for (helper, source) in files {
            let target = canonical_bin.join(helper.executable_name());
            reject_existing_reparse_point(&target)?;
            if source.parent() != Some(staging) {
                return Err(
                    "A staged executable escaped the temporary install directory.".to_owned(),
                );
            }
            targets.push((*helper, source.clone(), target));
        }
        replace_transactionally(staging, &targets)
    }

    fn emit(
        &self,
        app: &AppHandle,
        helper: HelperId,
        phase: &'static str,
        received_bytes: u64,
        total_bytes: Option<u64>,
        message: Option<String>,
    ) {
        if let Err(error) = app.emit(
            "helper-install-progress",
            HelperInstallProgress {
                helper,
                phase,
                received_bytes,
                total_bytes,
                message,
            },
        ) {
            tracing::warn!(%error, "could not emit helper installation progress");
        }
    }
}

#[tauri::command]
pub async fn get_helper_status(
    manager: State<'_, HelperManager>,
    state: State<'_, AppState>,
) -> Result<Vec<HelperStatus>, String> {
    let config = state
        .config_service
        .load()
        .await
        .map_err(|error| format!("Could not load engine configuration: {error}"))?;
    let mut configured = HashMap::new();
    if let Some(path) = &config.aria2.executable_path {
        configured.insert(HelperId::Aria2c, path.clone());
    }
    if let Some(path) = &config.yt_dlp.executable_path {
        configured.insert(HelperId::YtDlp, path.clone());
    }
    Ok(manager.get_statuses(&configured).await)
}

#[tauri::command]
pub async fn install_helper(
    app: AppHandle,
    manager: State<'_, HelperManager>,
    state: State<'_, AppState>,
    helper: HelperId,
) -> Result<(), String> {
    let installed = manager.install(&app, helper).await?;
    let mut config = state.config_service.load().await.map_err(|error| {
        format!("Helper installed, but engine settings could not be loaded: {error}")
    })?;
    for (installed_helper, path) in installed {
        match installed_helper {
            HelperId::Aria2c => config.aria2.executable_path = Some(path),
            HelperId::YtDlp => config.yt_dlp.executable_path = Some(path),
            HelperId::Ffmpeg | HelperId::Ffprobe => {}
        }
    }
    state.config_service.save(&config).await.map_err(|error| {
        format!("Helper installed, but its engine path could not be saved: {error}")
    })?;
    manager.emit(&app, helper, "completed", 0, None, None);
    Ok(())
}

#[tauri::command]
pub async fn cancel_helper_install(
    manager: State<'_, HelperManager>,
    helper: HelperId,
) -> Result<bool, String> {
    let active = manager.active_installs.lock().await;
    if let Some(token) = active.get(&helper.install_group()) {
        token.store(true, Ordering::Release);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn aria2_package() -> Result<InstallPackage, String> {
    if std::env::consts::ARCH != "x86_64" {
        return Err("The trusted aria2 Windows package is available for x64 only.".to_owned());
    }
    // SHA256 from the Microsoft WinGet installer manifest for aria2 1.37.0.
    Ok(InstallPackage {
        url: validate_allowlisted_url(
            "https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip",
        )?,
        sha256: "67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288".to_owned(),
        archive_name: "aria2.zip",
        max_bytes: MAX_ARIA2_ARCHIVE_BYTES,
        archive_format: ArchiveFormat::Zip,
        files: vec![InstallFile {
            helper: HelperId::Aria2c,
            archive_entry: "aria2-1.37.0-win-64bit-build1/aria2c.exe",
            staged_name: "aria2c.exe",
        }],
    })
}

fn github_digest(asset: &GithubAsset) -> Result<String, String> {
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or_else(|| "The official release does not provide a SHA-256 digest.".to_owned())?;
    validate_sha256(digest)?;
    Ok(digest.to_ascii_lowercase())
}

fn parse_checksum_file(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "The official checksum file was not valid UTF-8.".to_owned())?;
    let checksum = text
        .split_whitespace()
        .next()
        .ok_or_else(|| "The official checksum file was empty.".to_owned())?;
    validate_sha256(checksum)?;
    Ok(checksum.to_ascii_lowercase())
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The official source provided an invalid SHA-256 checksum.".to_owned());
    }
    Ok(())
}

fn validate_allowlisted_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "The official source URL is invalid.".to_owned())?;
    let host = url
        .host_str()
        .ok_or_else(|| "The official source URL has no host.".to_owned())?
        .to_ascii_lowercase();
    let path = url.path();
    let allowed = match host.as_str() {
        "api.github.com" => path == "/repos/yt-dlp/yt-dlp/releases/latest",
        "github.com" => {
            path.starts_with("/aria2/aria2/releases/download/release-1.37.0/")
                || path.starts_with("/yt-dlp/yt-dlp/releases/download/")
        }
        "www.gyan.dev" => {
            path.starts_with("/ffmpeg/builds/ffmpeg-release-essentials.zip")
                || path.starts_with("/ffmpeg/builds/packages/")
        }
        _ => false,
    };
    if url.scheme() != "https"
        || !allowed
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("The URL is not an allowlisted HTTPS helper source.".to_owned());
    }
    Ok(url)
}

fn validate_redirect_url(url: &Url) -> Result<Url, String> {
    if validate_allowlisted_url(url.as_str()).is_ok() {
        return Ok(url.clone());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if url.scheme() == "https"
        && (host == "release-assets.githubusercontent.com"
            || host == "objects.githubusercontent.com"
            || host.ends_with(".githubusercontent.com"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
    {
        return Ok(url.clone());
    }
    Err("The download redirected to a non-allowlisted host.".to_owned())
}

async fn wait_until_cancelled(cancellation: &AtomicBool) {
    loop {
        if cancellation.load(Ordering::Acquire) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn expected_pe_machine(architecture: &str) -> Option<u16> {
    match architecture {
        "x86_64" => Some(0x8664),
        "aarch64" => Some(0xaa64),
        _ => None,
    }
}

fn validate_pe_architecture(path: &Path) -> Result<&'static str, String> {
    let mut file =
        File::open(path).map_err(|error| format!("Could not open executable: {error}"))?;
    let mut header = [0_u8; 64];
    file.read_exact(&mut header)
        .map_err(|_| "The file is not a valid Windows executable.".to_owned())?;
    if &header[..2] != b"MZ" {
        return Err("The file is not a valid Windows executable.".to_owned());
    }
    let pe_offset =
        u32::from_le_bytes(header[0x3c..0x40].try_into().expect("fixed header range")) as u64;
    if pe_offset > 1024 * 1024 {
        return Err("The Windows executable header offset is invalid.".to_owned());
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(pe_offset))
        .map_err(|error| format!("Could not seek to executable header: {error}"))?;
    let mut pe_header = [0_u8; 6];
    file.read_exact(&mut pe_header)
        .map_err(|_| "The Windows executable header is incomplete.".to_owned())?;
    if &pe_header[..4] != b"PE\0\0" {
        return Err("The file is not a valid Windows PE executable.".to_owned());
    }
    match u16::from_le_bytes([pe_header[4], pe_header[5]]) {
        0x8664 => Ok("x86_64"),
        0xaa64 => Ok("aarch64"),
        _ => Err("The executable architecture is unsupported.".to_owned()),
    }
}

async fn detect_version(helper: HelperId, executable: &Path) -> Result<String, String> {
    let args = version_arguments(helper);
    let output = timeout(
        VERSION_TIMEOUT,
        Command::new(executable)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "The executable did not respond to its version check in time.".to_owned())?
    .map_err(|error| format!("The executable could not be started: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "The executable version check failed with exit code {:?}.",
            output.status.code()
        ));
    }
    parse_version_output(helper, &output.stdout, &output.stderr)
}

fn version_arguments(helper: HelperId) -> &'static [&'static str] {
    match helper {
        HelperId::Aria2c => &["--version"],
        HelperId::YtDlp => &["--version"],
        HelperId::Ffmpeg | HelperId::Ffprobe => &["-version"],
    }
}

fn parse_version_output(helper: HelperId, stdout: &[u8], stderr: &[u8]) -> Result<String, String> {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let lines = stdout.lines().chain(stderr.lines()).map(str::trim);
    let Some(line) = lines.clone().find(|line| !line.is_empty()) else {
        return Err("The executable returned no parseable version output.".to_owned());
    };
    let normalized = line.to_ascii_lowercase();
    let expected_label = match helper {
        HelperId::Aria2c => "aria2 version",
        HelperId::YtDlp => "",
        HelperId::Ffmpeg => "ffmpeg version",
        HelperId::Ffprobe => "ffprobe version",
    };
    if !expected_label.is_empty() && !normalized.contains(expected_label) {
        return Err("The executable returned an unexpected version response.".to_owned());
    }
    let version = line
        .split_whitespace()
        .find(|part| {
            let trimmed = part.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '.'
            });
            trimmed
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_digit())
                && trimmed.chars().any(|character| character.is_ascii_digit())
        })
        .ok_or_else(|| "The executable returned no parseable version.".to_owned())?;
    Ok(version
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '.')
        .to_owned())
}

fn extract_executables(
    archive_path: &Path,
    staging: &Path,
    files: &[InstallFile],
    archive_format: ArchiveFormat,
    expected_machine: u16,
) -> Result<(), String> {
    reject_reparse_point(staging)?;
    if archive_format == ArchiveFormat::DirectExecutable {
        for install_file in files {
            if install_file.archive_entry != install_file.staged_name {
                return Err("Invalid direct executable catalog entry.".to_owned());
            }
            let output_path = staging.join(install_file.staged_name);
            let mut source = File::open(archive_path)
                .map_err(|error| format!("Could not open verified executable: {error}"))?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output_path)
                .map_err(|error| format!("Could not stage helper executable: {error}"))?;
            std::io::copy(&mut source, &mut output)
                .map_err(|error| format!("Could not copy verified executable: {error}"))?;
            output
                .flush()
                .map_err(|error| format!("Could not flush staged executable: {error}"))?;
            validate_staged_executable(&output_path, expected_machine)?;
        }
        return Ok(());
    }
    let archive_file = File::open(archive_path)
        .map_err(|error| format!("Could not open verified archive: {error}"))?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| format!("The verified helper archive is invalid: {error}"))?;
    for install_file in files {
        let mut matched = None;
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|error| format!("Could not inspect helper archive entry: {error}"))?;
            if is_safe_archive_entry(entry.name(), install_file.archive_entry) {
                if entry.is_dir()
                    || entry.size() == 0
                    || entry.size() > MAX_EXTRACTED_EXE_BYTES
                    || entry
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(
                        "The helper archive contains an unsafe executable entry.".to_owned()
                    );
                }
                matched = Some(index);
                break;
            }
        }
        let index = matched.ok_or_else(|| {
            format!(
                "The official archive does not contain {}.",
                install_file.helper.display_name()
            )
        })?;
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read helper archive entry: {error}"))?;
        let output_path = staging.join(install_file.staged_name);
        if output_path.parent() != Some(staging) {
            return Err("The helper archive attempted to escape the staging directory.".to_owned());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|error| format!("Could not stage helper executable: {error}"))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut copied = 0_u64;
        loop {
            let count = entry
                .read(&mut buffer)
                .map_err(|error| format!("Could not extract helper executable: {error}"))?;
            if count == 0 {
                break;
            }
            copied += count as u64;
            if copied > MAX_EXTRACTED_EXE_BYTES {
                return Err("The extracted helper exceeded its maximum size.".to_owned());
            }
            hasher.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|error| format!("Could not write staged helper executable: {error}"))?;
        }
        output
            .flush()
            .map_err(|error| format!("Could not flush staged helper executable: {error}"))?;
        drop(output);
        validate_staged_executable(&output_path, expected_machine)?;
    }
    Ok(())
}

fn validate_staged_executable(path: &Path, expected_machine: u16) -> Result<(), String> {
    let architecture = validate_pe_architecture(path)?;
    if expected_pe_machine(architecture) != Some(expected_machine) {
        return Err("The downloaded helper has an incompatible architecture.".to_owned());
    }
    Ok(())
}

fn replace_transactionally(
    staging: &Path,
    files: &[(HelperId, PathBuf, PathBuf)],
) -> Result<Vec<(HelperId, PathBuf)>, String> {
    let mut backed_up: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut installed: Vec<PathBuf> = Vec::new();
    for (helper, staged, target) in files {
        let backup = staging.join(format!(".backup-{}", helper.executable_name()));
        if target.exists() {
            if let Err(error) = fs::rename(target, &backup) {
                restore_install(&backed_up, &installed);
                return Err(format!("Could not preserve the existing helper: {error}"));
            }
            backed_up.push((backup.clone(), target.clone()));
        }
        if let Err(error) = fs::rename(staged, target) {
            restore_install(&backed_up, &installed);
            return Err(format!(
                "Could not atomically install the verified helper: {error}"
            ));
        }
        installed.push(target.clone());
    }
    for (backup, _) in &backed_up {
        if let Err(error) = fs::remove_file(backup) {
            tracing::warn!(%error, path = %backup.display(), "could not remove helper backup");
        }
    }
    Ok(files
        .iter()
        .map(|(helper, _, target)| (*helper, target.clone()))
        .collect())
}

fn restore_install(backed_up: &[(PathBuf, PathBuf)], installed: &[PathBuf]) {
    for path in installed.iter().rev() {
        let _ = fs::remove_file(path);
    }
    for (backup, target) in backed_up.iter().rev() {
        let _ = fs::rename(backup, target);
    }
}

fn reject_reparse_point(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect managed helper directory: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("The managed helper directory must not be a symlink or junction.".to_owned());
    }
    Ok(())
}

fn reject_existing_reparse_point(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("Refusing to replace a symlink or reparse-point helper.".to_owned())
        }
        Ok(metadata) if !metadata.is_file() => {
            Err("Refusing to replace a non-file helper path.".to_owned())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not inspect existing helper path: {error}")),
    }
}

fn ensure_not_cancelled(cancellation: &AtomicBool) -> Result<(), String> {
    if cancellation.load(Ordering::Acquire) {
        Err("Installation was cancelled.".to_owned())
    } else {
        Ok(())
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0x0f) as usize] as char);
    }
    text
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        path::{Component, Path},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use super::{
        HelperId, InstallFile, aria2_package, ensure_not_cancelled, expected_pe_machine,
        hex_digest, parse_checksum_file, replace_transactionally, validate_allowlisted_url,
        validate_pe_architecture, validate_sha256, verify_sha256,
    };

    fn sample_pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0_u8; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[test]
    fn helper_catalog_contains_only_the_four_supported_helpers() {
        assert_eq!(HelperId::Aria2c.display_name(), "aria2c");
        assert_eq!(HelperId::YtDlp.display_name(), "yt-dlp");
        assert_eq!(HelperId::Ffmpeg.display_name(), "ffmpeg");
        assert_eq!(HelperId::Ffprobe.display_name(), "ffprobe");
        assert_eq!(aria2_package().expect("catalog URL valid").files.len(), 1);
        assert_eq!(
            aria2_package().expect("aria2 package").sha256,
            "67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288"
        );
    }

    #[test]
    fn rejects_untrusted_urls_and_non_https_sources() {
        assert!(validate_allowlisted_url("https://example.com/helper.exe").is_err());
        assert!(validate_allowlisted_url("http://github.com/helper.exe").is_err());
        assert!(validate_allowlisted_url("https://github.com.example.com/file").is_err());
        assert!(validate_allowlisted_url("https://user@github.com/file").is_err());
        assert!(validate_allowlisted_url("https://github.com:8443/file").is_err());
        assert!(
            validate_allowlisted_url("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
                .is_ok()
        );
        assert!(
            validate_allowlisted_url(
                "https://github.com/other/repo/releases/download/tag/file.exe"
            )
            .is_err()
        );
        assert!(
            validate_allowlisted_url(
                "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
            )
            .is_ok()
        );
        assert!(serde_json::from_str::<HelperId>(r#""../../evil.exe""#).is_err());
    }

    #[test]
    fn restricts_redirects_to_official_hosts_or_github_asset_hosts() {
        assert!(
            super::validate_redirect_url(
                &super::Url::parse("https://release-assets.githubusercontent.com/file").unwrap()
            )
            .is_ok()
        );
        assert!(
            super::validate_redirect_url(&super::Url::parse("https://evil.example/file").unwrap())
                .is_err()
        );
        assert!(
            super::validate_redirect_url(
                &super::Url::parse("http://release-assets.githubusercontent.com/file").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn parses_only_expected_version_responses() {
        assert_eq!(
            super::parse_version_output(HelperId::Aria2c, b"aria2 version 1.37.0\n", b"")
                .expect("aria2 version"),
            "1.37.0"
        );
        assert_eq!(
            super::parse_version_output(HelperId::YtDlp, b"2026.10.21\n", b"")
                .expect("yt-dlp version"),
            "2026.10.21"
        );
        assert!(super::parse_version_output(HelperId::Ffmpeg, b"garbage 1.0", b"").is_err());
        assert!(super::parse_version_output(HelperId::Ffprobe, b"", b"").is_err());
    }

    #[test]
    fn selects_only_supported_windows_architecture() {
        let helper = HelperId::Ffmpeg;
        assert!(helper.supported_install_architecture("x86_64"));
        assert!(!helper.supported_install_architecture("aarch64"));
        assert!(!helper.supported_install_architecture("x86"));
        assert_eq!(expected_pe_machine("x86_64"), Some(0x8664));
        assert_eq!(expected_pe_machine("aarch64"), Some(0xaa64));
        assert_eq!(expected_pe_machine("x86"), None);
    }

    #[test]
    fn validates_pe_architecture_and_rejects_malformed_or_wrong_machine_files() {
        let directory = tempdir().expect("temp directory");
        let x64 = directory.path().join("x64.exe");
        fs::write(&x64, sample_pe(0x8664)).expect("write PE fixture");
        assert_eq!(validate_pe_architecture(&x64).expect("x64 PE"), "x86_64");

        let arm = directory.path().join("arm.exe");
        fs::write(&arm, sample_pe(0xaa64)).expect("write PE fixture");
        assert_eq!(validate_pe_architecture(&arm).expect("ARM PE"), "aarch64");

        let malformed = directory.path().join("bad.exe");
        fs::write(&malformed, b"not executable").expect("write invalid fixture");
        assert!(validate_pe_architecture(&malformed).is_err());
    }

    #[test]
    fn validates_checksum_format_and_parses_expected_hash() {
        let expected = "a".repeat(64);
        assert!(validate_sha256(&expected).is_ok());
        assert!(validate_sha256("not-a-hash").is_err());
        assert_eq!(
            parse_checksum_file(format!("{expected}  ffmpeg.zip\n").as_bytes())
                .expect("checksum parses"),
            expected
        );
        assert!(parse_checksum_file(b"").is_err());
        assert!(parse_checksum_file(b"xyz ffmpeg.zip").is_err());
        assert!(verify_sha256(&expected, &expected).is_ok());
        assert!(verify_sha256(&expected, &"b".repeat(64)).is_err());
    }

    #[test]
    fn detects_checksum_mismatch_before_installation() {
        let content = b"verified test fixture";
        let actual = hex_digest(&Sha256::digest(content));
        let wrong = "0".repeat(64);
        assert_ne!(actual, wrong);
        assert!(verify_sha256(&actual, &wrong).is_err());
    }

    #[test]
    fn discovery_uses_fixed_executable_names() {
        let directory = tempdir().expect("temp directory");
        let manager = super::HelperManager::new(
            directory.path().join("managed").join("bin"),
            directory.path().join("resources").join("engines"),
        );
        if let Some(path) = manager.find_executable(HelperId::YtDlp, None) {
            assert_eq!(path.file_name().unwrap(), "yt-dlp.exe");
        }
        assert_eq!(HelperId::Aria2c.executable_name(), "aria2c.exe");
    }

    #[test]
    fn existing_version_is_preserved_when_replacement_fails() {
        let directory = tempdir().expect("temp directory");
        let managed = directory.path().join("bin");
        fs::create_dir_all(&managed).expect("managed directory");
        let existing = managed.join("aria2c.exe");
        fs::write(&existing, b"working old version").expect("existing executable");
        let staging = managed.join(".stage");
        fs::create_dir(&staging).expect("stage directory");
        let staged_missing = staging.join("missing.exe");
        let error = replace_transactionally(
            &staging,
            &[(HelperId::Aria2c, staged_missing, existing.clone())],
        )
        .expect_err("missing staged file must fail");
        assert!(error.contains("atomically install"));
        assert_eq!(
            fs::read(existing).expect("old file retained"),
            b"working old version"
        );
    }

    #[test]
    fn failed_download_staging_directory_can_be_removed_without_touching_installed_files() {
        let directory = tempdir().expect("temp directory");
        let manager = super::HelperManager::new(
            directory.path().join("managed").join("bin"),
            directory.path().join("resources").join("engines"),
        );
        let installed = manager.bin_dir.join("yt-dlp.exe");
        fs::create_dir_all(&manager.bin_dir).expect("managed directory");
        fs::write(&installed, b"existing binary").expect("existing helper");
        let staging = manager.create_staging_dir().expect("staging directory");
        fs::write(staging.join("partial.download"), b"partial").expect("partial download");
        fs::remove_dir_all(staging).expect("clean staging after failure");
        assert_eq!(
            fs::read(installed).expect("preserved helper"),
            b"existing binary"
        );
    }

    #[test]
    fn atomically_replaces_existing_executable_after_validation() {
        let directory = tempdir().expect("temp directory");
        let managed = directory.path().join("bin");
        fs::create_dir_all(&managed).expect("managed directory");
        let staging = managed.join(".stage");
        fs::create_dir(&staging).expect("stage directory");
        let existing = managed.join("aria2c.exe");
        fs::write(&existing, b"old").expect("old executable");
        let staged = staging.join("aria2c.exe");
        fs::File::create(&staged)
            .expect("staged file")
            .write_all(b"verified new")
            .expect("write staged file");
        replace_transactionally(&staging, &[(HelperId::Aria2c, staged, existing.clone())])
            .expect("atomic replacement");
        assert_eq!(
            fs::read(existing).expect("installed executable"),
            b"verified new"
        );
    }

    #[test]
    fn cancellation_is_observed_and_does_not_become_success() {
        let cancelled = Arc::new(AtomicBool::new(false));
        assert!(ensure_not_cancelled(&cancelled).is_ok());
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            ensure_not_cancelled(&cancelled).expect_err("cancelled"),
            "Installation was cancelled."
        );
    }

    #[tokio::test]
    async fn concurrent_install_requests_for_a_helper_group_are_rejected() {
        let directory = tempdir().expect("temp directory");
        let manager = super::HelperManager::new(
            directory.path().join("managed").join("bin"),
            directory.path().join("resources").join("engines"),
        );
        let cancellation = manager.begin_install(2).await.expect("first install lock");
        assert!(
            manager.begin_install(2).await.is_err(),
            "a second install for the same package must be rejected"
        );
        cancellation.store(true, Ordering::Release);
        manager.end_install(2).await;
        assert!(manager.begin_install(2).await.is_ok());
    }

    #[test]
    fn installation_rejects_staged_executables_outside_managed_directory() {
        let directory = tempdir().expect("temp directory");
        let managed = directory.path().join("bin");
        fs::create_dir_all(&managed).expect("managed directory");
        let staging = managed.join(".stage");
        fs::create_dir(&staging).expect("stage directory");
        let outside = directory.path().join("outside.exe");
        fs::write(&outside, b"outside").expect("outside file");
        let manager =
            super::HelperManager::new(managed, directory.path().join("resources").join("engines"));
        assert!(
            manager
                .commit_install(&staging, &[(HelperId::Aria2c, outside)])
                .is_err()
        );
    }

    #[test]
    fn rollback_restores_files_if_a_later_pair_install_fails() {
        let directory = tempdir().expect("temp directory");
        let managed = directory.path().join("bin");
        fs::create_dir_all(&managed).expect("managed directory");
        let staging = managed.join(".stage");
        fs::create_dir(&staging).expect("stage directory");
        let ffmpeg = managed.join("ffmpeg.exe");
        let ffprobe = managed.join("ffprobe.exe");
        fs::write(&ffmpeg, b"old ffmpeg").expect("old ffmpeg");
        fs::write(&ffprobe, b"old ffprobe").expect("old ffprobe");
        let staged_ffmpeg = staging.join("ffmpeg.exe");
        fs::write(&staged_ffmpeg, b"new ffmpeg").expect("new ffmpeg");
        let missing_ffprobe = staging.join("missing-ffprobe.exe");
        let error = replace_transactionally(
            &staging,
            &[
                (HelperId::Ffmpeg, staged_ffmpeg, ffmpeg.clone()),
                (HelperId::Ffprobe, missing_ffprobe, ffprobe.clone()),
            ],
        )
        .expect_err("second install should fail");
        assert!(error.contains("atomically install"));
        assert_eq!(fs::read(ffmpeg).expect("restored ffmpeg"), b"old ffmpeg");
        assert_eq!(fs::read(ffprobe).expect("restored ffprobe"), b"old ffprobe");
    }

    #[test]
    fn installed_helper_is_detected_after_atomic_replacement() {
        let directory = tempdir().expect("temp directory");
        let managed = directory.path().join("bin");
        fs::create_dir_all(&managed).expect("managed directory");
        let staging = managed.join(".stage");
        fs::create_dir(&staging).expect("stage directory");
        let staged = staging.join("yt-dlp.exe");
        fs::write(&staged, sample_pe(0x8664)).expect("staged PE");
        let target = managed.join("yt-dlp.exe");
        replace_transactionally(&staging, &[(HelperId::YtDlp, staged, target.clone())])
            .expect("install helper");
        let manager = super::HelperManager::new(
            managed.clone(),
            directory.path().join("resources").join("engines"),
        );
        assert_eq!(
            manager.find_executable(HelperId::YtDlp, None).as_deref(),
            Some(target.as_path())
        );
    }

    #[test]
    fn executable_validation_fails_for_a_missing_file_and_wrong_version_label() {
        let directory = tempdir().expect("temp directory");
        assert!(validate_pe_architecture(&directory.path().join("missing.exe")).is_err());
        assert!(
            super::parse_version_output(HelperId::Ffprobe, b"ffmpeg version 9.0\n", b"").is_err()
        );
        assert_eq!(super::version_arguments(HelperId::Ffmpeg), &["-version"]);
        assert_eq!(super::version_arguments(HelperId::YtDlp), &["--version"]);
    }

    #[test]
    fn archive_entry_names_are_catalog_constants_and_not_ui_input() {
        let files = [
            InstallFile {
                helper: HelperId::Ffmpeg,
                archive_entry: "bin/ffmpeg.exe",
                staged_name: "ffmpeg.exe",
            },
            InstallFile {
                helper: HelperId::Ffprobe,
                archive_entry: "bin/ffprobe.exe",
                staged_name: "ffprobe.exe",
            },
        ];
        assert!(
            files
                .iter()
                .all(|file| !Path::new(file.archive_entry).is_absolute())
        );
        assert!(files.iter().all(|file| {
            Path::new(file.archive_entry)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        }));
        assert!(!super::is_safe_archive_entry(
            "../../bin/ffmpeg.exe",
            "bin/ffmpeg.exe"
        ));
        assert!(super::is_safe_archive_entry(
            "ffmpeg-9.0.2/bin/ffmpeg.exe",
            "bin/ffmpeg.exe"
        ));
    }
}

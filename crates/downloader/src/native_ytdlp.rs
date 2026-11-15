use std::{
    ffi::OsString,
    fs,
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::Duration,
};

use fetchx_models::YtDlpAudioFormat;
use tokio::sync::{mpsc, watch};
use yt_dlp_rs::{
    DownloadControl, DownloadOptions, Error as NativeYtDlpError, ExtractionOptions, Interrupt,
    YoutubeDl, YoutubeOptions,
};

use crate::{
    NativeMediaError, NativeMediaProcessor, YtDlpError, YtDlpFormat, YtDlpProcessExit,
    YtDlpProcessRunner, YtDlpRunnerEvent, YtDlpRunnerFuture,
};

const NATIVE_VERSION: &str = "yt-dlp-rs 1.0.0-rc.2 (yt-dlp 2026.10.26)";
const MAX_OUTPUT_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_DISCOVERED_FORMATS: usize = 100;
const PROGRESS_PREFIX: &str = "FETCHX_PROGRESS|";

#[derive(Clone, Debug)]
struct NativeYtDlpRequest {
    url: String,
    workspace: PathBuf,
    format: Option<String>,
    rate_limit: Option<u64>,
    audio_format: Option<YtDlpAudioFormat>,
}

pub struct NativeYtDlpProcessRunner;

impl YtDlpProcessRunner for NativeYtDlpProcessRunner {
    fn validate_version<'a>(
        &'a self,
        _executable: &'a Path,
        _timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<String, YtDlpError>> {
        Box::pin(async { Ok(NATIVE_VERSION.to_owned()) })
    }

    fn run<'a>(
        &'a self,
        _executable: &'a Path,
        arguments: &'a [OsString],
        mut stop: watch::Receiver<bool>,
        events: mpsc::Sender<YtDlpRunnerEvent>,
    ) -> YtDlpRunnerFuture<'a, Result<YtDlpProcessExit, YtDlpError>> {
        let request = match parse_request(arguments) {
            Ok(request) => request,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        Box::pin(async move {
            let interrupt = Interrupt::new();
            let worker_interrupt = interrupt.clone();
            let worker =
                tokio::task::spawn_blocking(move || run_request(request, worker_interrupt, events));
            tokio::pin!(worker);

            if *stop.borrow_and_update() {
                interrupt.cancel();
            }
            loop {
                tokio::select! {
                    result = &mut worker => {
                        return match result {
                            Ok(Ok(())) => Ok(YtDlpProcessExit { success: true, code: Some(0) }),
                            Ok(Err(_)) if interrupt.is_requested() => {
                                Ok(YtDlpProcessExit { success: false, code: Some(130) })
                            }
                            Ok(Err(error)) => Err(error),
                            Err(_) => Err(YtDlpError::RuntimeFailure(
                                "native yt-dlp worker terminated unexpectedly".into(),
                            )),
                        };
                    }
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow_and_update() {
                            interrupt.cancel();
                            return match worker.await {
                                Ok(Err(_)) | Ok(Ok(())) => {
                                    Ok(YtDlpProcessExit { success: false, code: Some(130) })
                                }
                                Err(_) => Err(YtDlpError::RuntimeFailure(
                                    "native yt-dlp worker terminated unexpectedly during cancellation".into(),
                                )),
                            };
                        }
                    }
                }
            }
        })
    }

    fn discover_formats<'a>(
        &'a self,
        _executable: &'a Path,
        url: &'a str,
        command_timeout: Duration,
    ) -> YtDlpRunnerFuture<'a, Result<Vec<YtDlpFormat>, YtDlpError>> {
        let url = url.to_owned();
        Box::pin(async move {
            fetchx_media_extractor::is_safe_media_url(&url).map_err(|_| YtDlpError::InvalidUrl)?;
            let interrupt = Interrupt::new();
            let worker_interrupt = interrupt.clone();
            let mut worker =
                tokio::task::spawn_blocking(move || discover_formats(&url, worker_interrupt));
            tokio::select! {
                result = &mut worker => match result {
                    Ok(result) => result,
                    Err(_) => Err(YtDlpError::RuntimeFailure(
                        "native yt-dlp format discovery worker terminated unexpectedly".into(),
                    )),
                },
                _ = tokio::time::sleep(command_timeout) => {
                    interrupt.cancel();
                    let _ = worker.await;
                    Err(YtDlpError::FormatDiscoveryTimeout)
                }
            }
        })
    }
}

fn parse_request(arguments: &[OsString]) -> Result<NativeYtDlpRequest, YtDlpError> {
    let mut format = None;
    let mut rate_limit = None;
    let mut audio_format = None;
    let mut workspace = None;
    let mut url = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].to_string_lossy();
        match argument.as_ref() {
            "--" => {
                url = arguments
                    .get(index + 1)
                    .map(|value| value.to_string_lossy().into_owned());
                break;
            }
            "--format" => {
                format = arguments
                    .get(index + 1)
                    .map(|value| value.to_string_lossy().into_owned());
                index += 1;
            }
            "--paths" => {
                let path = arguments
                    .get(index + 1)
                    .ok_or(YtDlpError::WorkspaceUnavailable)?
                    .to_string_lossy();
                workspace = path
                    .strip_prefix("home:")
                    .map(PathBuf::from)
                    .filter(|path| !path.as_os_str().is_empty());
                index += 1;
            }
            "--limit-rate" => {
                rate_limit = Some(
                    arguments
                        .get(index + 1)
                        .ok_or_else(|| {
                            YtDlpError::RuntimeFailure(
                                "native yt-dlp received a limit-rate option without a value".into(),
                            )
                        })?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| {
                            YtDlpError::RuntimeFailure(
                                "native yt-dlp received an invalid limit-rate value".into(),
                            )
                        })?,
                );
                index += 1;
            }
            "--audio-format" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| {
                        YtDlpError::RuntimeFailure(
                            "native yt-dlp received an audio-format option without a value".into(),
                        )
                    })?
                    .to_string_lossy();
                audio_format = Some(
                    YtDlpAudioFormat::try_from(value.as_ref())
                        .map_err(|_| YtDlpError::NativePostprocessingUnsupported)?,
                );
                index += 1;
            }
            "--sponsorblock-remove"
            | "--write-sub"
            | "--write-auto-sub"
            | "--sub-langs"
            | "--embed-sub"
            | "--embed-metadata" => {
                return Err(YtDlpError::NativePostprocessingUnsupported);
            }
            _ => {}
        }
        index += 1;
    }

    let url = url.ok_or(YtDlpError::InvalidUrl)?;
    fetchx_media_extractor::is_safe_media_url(&url).map_err(|_| YtDlpError::InvalidUrl)?;
    let workspace = workspace.ok_or(YtDlpError::WorkspaceUnavailable)?;
    let metadata =
        fs::symlink_metadata(&workspace).map_err(|_| YtDlpError::WorkspaceUnavailable)?;
    if !metadata.file_type().is_dir() {
        return Err(YtDlpError::WorkspaceUnavailable);
    }
    Ok(NativeYtDlpRequest {
        url,
        workspace,
        format,
        rate_limit,
        audio_format,
    })
}

fn run_request(
    request: NativeYtDlpRequest,
    interrupt: Interrupt,
    events: mpsc::Sender<YtDlpRunnerEvent>,
) -> Result<(), YtDlpError> {
    let client = native_client()?;
    let mut extraction_options = ExtractionOptions::default();
    extraction_options.interrupt = Some(interrupt.clone());
    let extraction = client
        .extract_result_unselected_with_options(&request.url, extraction_options)
        .map_err(|error| map_native_error(error, &interrupt))?;

    let selectors = selectors_for_request(request.format.as_deref(), request.audio_format);
    let mut selected = Vec::new();
    for selector in selectors {
        let mut result = extraction.clone();
        client
            .select_result(&mut result, selector.as_deref())
            .map_err(|error| map_native_error(error, &interrupt))?;
        selected.extend(result.into_entries());
    }

    if selected.is_empty() || selected.len() > 2 {
        return Err(YtDlpError::NativePostprocessingUnsupported);
    }

    let mut streams = Vec::new();
    for mut info in selected {
        if info.requested_formats.is_empty() {
            streams.push(info);
        } else {
            let requested_formats = std::mem::take(&mut info.requested_formats);
            for format in requested_formats {
                let mut stream = info.clone();
                stream.formats = vec![format];
                stream.requested_formats.clear();
                stream
                    .select_format(Some("best"))
                    .map_err(|error| map_native_error(error, &interrupt))?;
                streams.push(stream);
            }
        }
    }
    if !(1..=2).contains(&streams.len()) {
        return Err(YtDlpError::NativePostprocessingUnsupported);
    }
    if request.audio_format == Some(YtDlpAudioFormat::M4a)
        && !streams[0].ext.eq_ignore_ascii_case("m4a")
    {
        return Err(YtDlpError::NativePostprocessingUnsupported);
    }

    let mut downloaded = 0_u64;
    for (index, info) in streams.iter().enumerate() {
        if interrupt.is_requested() {
            return Err(YtDlpError::ProcessFailure { code: Some(130) });
        }
        let extension = safe_extension(&info.ext);
        let path = request
            .workspace
            .join(format!("native-stream-{index}.{extension}"));
        let mut options = DownloadOptions::default();
        options.rate_limit = request.rate_limit;
        options.overwrite = true;
        let mut control = DownloadControl::default();
        control.interrupt = Some(interrupt.clone());
        let event_sender = events.clone();
        let progress = client.download_with_control(info, &path, options, &control, |progress| {
            let cumulative = downloaded.saturating_add(progress.downloaded_bytes);
            let line = format!("{PROGRESS_PREFIX}NA|{cumulative}|NA|NA|NA|NA");
            if event_sender
                .blocking_send(YtDlpRunnerEvent::ProgressLine(line))
                .is_err()
            {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        let size = progress.map_err(|error| map_native_error(error, &interrupt))?;
        downloaded = downloaded.saturating_add(size);
    }

    if let Some(YtDlpAudioFormat::Mp3) = request.audio_format {
        let source = request.workspace.join(format!(
            "native-stream-0.{}",
            safe_extension(&streams[0].ext)
        ));
        let destination = request.workspace.join("native-audio.mp3");
        NativeMediaProcessor::audio_to_mp3(&source, &destination, MAX_OUTPUT_BYTES, || {
            interrupt.is_requested()
        })
        .map_err(map_native_media_error)?;
        fs::remove_file(source).map_err(|error| YtDlpError::RuntimeFailure(error.to_string()))?;
    }
    Ok(())
}

fn selectors_for_request(
    requested: Option<&str>,
    audio_format: Option<YtDlpAudioFormat>,
) -> Vec<Option<String>> {
    if audio_format == Some(YtDlpAudioFormat::Mp3) {
        return vec![Some("bestaudio/best".into())];
    }
    if audio_format == Some(YtDlpAudioFormat::M4a) {
        return vec![Some("bestaudio[ext=m4a]/best[ext=m4a]".into())];
    }
    let Some(requested) = requested else {
        return vec![None];
    };
    if requested == AUTO_FORMAT_SELECTOR {
        return vec![None];
    }
    split_selectors(requested)
        .into_iter()
        .map(|selector| Some(selector.to_owned()))
        .collect()
}

const AUTO_FORMAT_SELECTOR: &str = "bestvideo[ext=mp4][vcodec^=avc1]/bestvideo[ext=webm],bestaudio[ext=m4a][acodec^=mp4a]/bestaudio[ext=webm]";

fn split_selectors(expression: &str) -> Vec<&str> {
    let mut selectors = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (index, byte) in expression.bytes().enumerate() {
        match byte {
            b'[' | b'(' => depth = depth.saturating_add(1),
            b']' | b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let selector = expression[start..index].trim();
                if !selector.is_empty() {
                    selectors.push(selector);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let selector = expression[start..].trim();
    if !selector.is_empty() {
        selectors.push(selector);
    }
    selectors
}

fn discover_formats(url: &str, interrupt: Interrupt) -> Result<Vec<YtDlpFormat>, YtDlpError> {
    let client = native_client()?;
    let mut options = ExtractionOptions::default();
    options.interrupt = Some(interrupt.clone());
    let result = client
        .extract_result_unselected_with_options(url, options)
        .map_err(|error| map_native_error(error, &interrupt))?;
    let mut formats = Vec::new();
    for info in result.into_entries() {
        for format in info.formats {
            if format.vcodec.as_deref() == Some("none") {
                continue;
            }
            let format_id = if format.acodec.as_deref() == Some("none") {
                format!("{}+bestaudio/{}", format.format_id, format.format_id)
            } else {
                format.format_id.clone()
            };
            let extension = Some(format.ext.clone());
            let mut label = format
                .resolution
                .as_deref()
                .or(format.format_note.as_deref())
                .unwrap_or("Video")
                .to_owned();
            if format.acodec.as_deref() == Some("none") {
                label.push_str(" + audio");
            }
            label.push_str(" - ");
            label.push_str(&format.ext);
            formats.push(YtDlpFormat {
                format_id,
                label,
                extension,
                file_size_bytes: format.filesize,
                title: Some(info.title.clone()),
                platform: Some(info.extractor_key.clone()),
            });
            if formats.len() == MAX_DISCOVERED_FORMATS {
                return Ok(formats);
            }
        }
    }
    if formats.is_empty() {
        return Err(YtDlpError::InvalidFormatOutput);
    }
    Ok(formats)
}

fn native_client() -> Result<YoutubeDl, YtDlpError> {
    let mut options = YoutubeOptions::default();
    options.no_playlist = true;
    YoutubeDl::with_youtube_options(options)
        .map_err(|error| YtDlpError::RuntimeFailure(error.to_string()))
}

fn safe_extension(extension: &str) -> &str {
    if (1..=10).contains(&extension.len())
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        extension
    } else {
        "media"
    }
}

fn map_native_error(error: NativeYtDlpError, interrupt: &Interrupt) -> YtDlpError {
    if interrupt.is_requested() {
        return YtDlpError::ProcessFailure { code: Some(130) };
    }
    YtDlpError::RuntimeFailure(error.to_string())
}

fn map_native_media_error(error: NativeMediaError) -> YtDlpError {
    match error {
        NativeMediaError::Cancelled => YtDlpError::ProcessFailure { code: Some(130) },
        other => YtDlpError::RuntimeFailure(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_only_top_level_format_alternatives() {
        assert_eq!(
            split_selectors("bv[height<=1080]+ba/best,18"),
            ["bv[height<=1080]+ba/best", "18"]
        );
    }

    #[test]
    fn maps_adaptive_format_ids_to_separate_native_stream_selections() {
        assert_eq!(
            selectors_for_request(Some("137,bestaudio"), None),
            [Some("137".to_owned()), Some("bestaudio".to_owned())]
        );
    }

    #[test]
    fn m4a_extraction_selects_only_native_m4a_sources() {
        assert_eq!(
            selectors_for_request(None, Some(YtDlpAudioFormat::M4a)),
            [Some("bestaudio[ext=m4a]/best[ext=m4a]".to_owned())]
        );
    }

    #[test]
    fn rejects_missing_or_unsafe_request_paths() {
        assert!(parse_request(&[]).is_err());
        let args = [
            OsString::from("--paths"),
            OsString::from("home:missing"),
            OsString::from("--"),
            OsString::from("https://example.com/video"),
        ];
        assert!(matches!(
            parse_request(&args),
            Err(YtDlpError::WorkspaceUnavailable)
        ));
    }

    #[test]
    fn rejects_legacy_options_not_supported_by_the_native_runner() {
        let args = [
            OsString::from("--embed-metadata"),
            OsString::from("--paths"),
            OsString::from(format!("home:{}", std::env::temp_dir().display())),
            OsString::from("--"),
            OsString::from("https://example.com/video"),
        ];
        assert!(matches!(
            parse_request(&args),
            Err(YtDlpError::NativePostprocessingUnsupported)
        ));
    }
}

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use fetchx_downloader::{
    MediaInfo, MediaStreamKind, NativeMediaError, NativeMediaProcessor, NativeMp4Error,
    NativeMp4Processor, NativeOxideavError, NativeOxideavProcessor,
};

const MAX_OUTPUT_BYTES: u64 = 100 * 1024 * 1024 * 1024;

struct NativeMerge {
    video: PathBuf,
    audio: PathBuf,
    output: PathBuf,
    container: &'static str,
}

struct NativeAudioExtract {
    input: PathBuf,
    output: PathBuf,
}

struct NativeAudioMp3 {
    input: PathBuf,
    output: PathBuf,
}

struct NativeAudioFlac {
    input: PathBuf,
    output: PathBuf,
}

struct NativeRemux {
    input: PathBuf,
    output: PathBuf,
    container: &'static str,
}

fn main() -> ExitCode {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| argument == "-version" || argument == "--version")
    {
        println!("ffmpeg version 1.0.0-fetchx-native");
        return ExitCode::SUCCESS;
    }

    if let Some(merge) = parse_native_merge(&arguments) {
        if merge.container == "mov"
            && has_mp4_extension(&merge.video)
            && has_mp4_extension(&merge.audio)
        {
            match NativeMp4Processor::merge_h264_aac(
                &merge.video,
                &merge.audio,
                &merge.output,
                MAX_OUTPUT_BYTES,
                || false,
            ) {
                Ok(_) => return ExitCode::SUCCESS,
                Err(NativeMp4Error::Cancelled) => return ExitCode::from(130),
                Err(error) => {
                    eprintln!("native MP4 merge unavailable: {error}");
                }
            }
        }
        match NativeOxideavProcessor::merge_video_audio(
            &merge.video,
            &merge.audio,
            &merge.output,
            merge.container,
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeOxideavError::Cancelled) => return ExitCode::from(130),
            Err(NativeOxideavError::UnsupportedInput | NativeOxideavError::Container(_)) => {
                eprintln!("native merge unsupported; using bundled FFmpeg fallback");
            }
            Err(error) => {
                eprintln!("native merge failed: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(extract) = parse_native_audio_extract(&arguments) {
        match NativeMp4Processor::extract_aac(
            &extract.input,
            &extract.output,
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeMp4Error::Cancelled) => return ExitCode::from(130),
            Err(
                NativeMp4Error::InvalidInput
                | NativeMp4Error::UnsupportedCodec
                | NativeMp4Error::RequiredTrackMissing,
            ) => {
                eprintln!("native MP4 AAC extraction unsupported; trying OxideAV");
            }
            Err(error) => {
                eprintln!("native AAC extraction failed: {error}");
                return ExitCode::FAILURE;
            }
        }
        match NativeOxideavProcessor::remux_audio(
            &extract.input,
            &extract.output,
            "mov",
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeOxideavError::Cancelled) => return ExitCode::from(130),
            Err(NativeOxideavError::UnsupportedInput | NativeOxideavError::Container(_)) => {
                eprintln!("native audio extraction unsupported; using bundled FFmpeg fallback");
            }
            Err(error) => {
                eprintln!("native audio extraction failed: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(transcode) = parse_native_audio_mp3(&arguments) {
        match NativeMediaProcessor::audio_to_mp3(
            &transcode.input,
            &transcode.output,
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeMediaError::Cancelled) => return ExitCode::from(130),
            Err(error) => {
                eprintln!("native WAV-to-MP3 conversion unavailable: {error}");
            }
        }
    }
    if let Some(transcode) = parse_native_audio_flac(&arguments) {
        match NativeMediaProcessor::audio_to_flac(
            &transcode.input,
            &transcode.output,
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeMediaError::Cancelled) => return ExitCode::from(130),
            Err(error) => {
                eprintln!("native audio-to-FLAC conversion unavailable: {error}");
            }
        }
    }
    if let Some(remux) = parse_native_remux(&arguments) {
        if remux.container == "mp4"
            && has_mp4_extension(&remux.input)
            && has_mp4_extension(&remux.output)
        {
            match NativeMp4Processor::remux(&remux.input, &remux.output, MAX_OUTPUT_BYTES, || false)
            {
                Ok(_) => return ExitCode::SUCCESS,
                Err(NativeMp4Error::Cancelled) => return ExitCode::from(130),
                Err(NativeMp4Error::InvalidInput | NativeMp4Error::UnsupportedCodec) => {
                    eprintln!("native MP4 remux unsupported; trying OxideAV");
                }
                Err(error) => {
                    eprintln!("native MP4 remux failed: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        match NativeOxideavProcessor::remux(
            &remux.input,
            &remux.output,
            remux.container,
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeOxideavError::Cancelled) => return ExitCode::from(130),
            Err(NativeOxideavError::UnsupportedInput | NativeOxideavError::Container(_)) => {
                eprintln!("native remux unsupported; using bundled FFmpeg fallback");
            }
            Err(error) => {
                eprintln!("native remux failed: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(input) = parse_native_probe(&arguments) {
        if has_mp4_extension(&input) {
            match NativeMp4Processor::probe(&input) {
                Ok(info) => {
                    for (index, track) in info.tracks.iter().enumerate() {
                        let media_type = match track.media_type.as_str() {
                            "video" => "Video",
                            "audio" => "Audio",
                            _ => continue,
                        };
                        let codec = track.codec.as_deref().unwrap_or("unknown");
                        eprintln!("  Stream #0:{index}: {media_type}: {codec}");
                    }
                    return ExitCode::from(1);
                }
                Err(error) => {
                    eprintln!("native MP4 probe unavailable: {error}");
                }
            }
        } else {
            match NativeMediaProcessor::probe_audio(&input, || false) {
                Ok(info) => {
                    for (index, stream) in info.streams.iter().enumerate() {
                        if stream.kind != fetchx_downloader::MediaStreamKind::Audio {
                            continue;
                        }
                        let codec = stream.codec.as_deref().unwrap_or("unknown");
                        eprintln!("  Stream #0:{index}: Audio: {codec}");
                    }
                    return ExitCode::from(1);
                }
                Err(error) => {
                    eprintln!("native audio probe unavailable: {error}");
                }
            }
        }
        if let Some(exit_code) = run_native_oxideav_probe(&input) {
            return exit_code;
        }
    }

    run_fallback(&arguments)
}

fn run_native_oxideav_probe(input: &Path) -> Option<ExitCode> {
    match NativeOxideavProcessor::probe(input, || false) {
        Ok(info) => Some(print_probe_streams(&info)),
        Err(error) => {
            eprintln!("native OxideAV probe unavailable: {error}");
            None
        }
    }
}

fn print_probe_streams(info: &MediaInfo) -> ExitCode {
    for (index, stream) in info.streams.iter().enumerate() {
        let media_type = match stream.kind {
            MediaStreamKind::Video => "Video",
            MediaStreamKind::Audio => "Audio",
            MediaStreamKind::Subtitle => "Subtitle",
            MediaStreamKind::Data | MediaStreamKind::Attachment | MediaStreamKind::Other => {
                continue;
            }
        };
        let codec = stream.codec.as_deref().unwrap_or("unknown");
        eprintln!("  Stream #0:{index}: {media_type}: {codec}");
    }
    ExitCode::from(1)
}

fn parse_native_probe(arguments: &[OsString]) -> Option<PathBuf> {
    let inputs = arguments
        .windows(2)
        .filter(|window| window[0] == "-i")
        .map(|window| path_argument(&window[1]))
        .collect::<Option<Vec<_>>>()?;
    if inputs.len() != 1 || path_argument(arguments.last()?) != inputs.first().cloned() {
        return None;
    }
    inputs.into_iter().next()
}

fn parse_native_audio_extract(arguments: &[OsString]) -> Option<NativeAudioExtract> {
    if !arguments.iter().any(|argument| argument == "-vn") || !has_audio_copy_codec(arguments) {
        return None;
    }
    let inputs = arguments
        .windows(2)
        .filter(|window| window[0] == "-i")
        .map(|window| path_argument(&window[1]))
        .collect::<Option<Vec<_>>>()?;
    if inputs.len() != 1 {
        return None;
    }
    let output = path_argument(arguments.last()?)?;
    if !output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m4a"))
    {
        return None;
    }
    Some(NativeAudioExtract {
        input: inputs.into_iter().next()?,
        output,
    })
}

fn parse_native_audio_mp3(arguments: &[OsString]) -> Option<NativeAudioMp3> {
    let output = path_argument(arguments.last()?)?;
    if !has_extension(&output, "mp3") || arguments.len() < 2 {
        return None;
    }

    let mut input = None;
    let mut has_audio_only = false;
    let mut has_mp3_encoder = false;
    let mut index = 0;
    while index < arguments.len() - 1 {
        match arguments[index].to_str()? {
            "-y" | "-nostdin" | "-hide_banner" => index += 1,
            "-v" | "-loglevel" => {
                arguments.get(index + 1)?.to_str()?;
                index += 2;
            }
            "-i" if input.is_none() => {
                input = Some(path_argument(arguments.get(index + 1)?)?);
                index += 2;
            }
            "-vn" if !has_audio_only => {
                has_audio_only = true;
                index += 1;
            }
            "-acodec" | "-c:a" if !has_mp3_encoder => {
                let codec = arguments.get(index + 1)?.to_str()?;
                if !codec.eq_ignore_ascii_case("libmp3lame") {
                    return None;
                }
                has_mp3_encoder = true;
                index += 2;
            }
            "-b:a" if arguments.get(index + 1)?.to_str()? == "192k" => index += 2,
            "-f" if arguments.get(index + 1)?.to_str()? == "mp3" => index += 2,
            _ => return None,
        }
    }

    let input = input?;
    let extension = input.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "aif"
            | "aiff"
            | "caf"
            | "flac"
            | "m4a"
            | "mka"
            | "mkv"
            | "mp1"
            | "mp2"
            | "mp3"
            | "mp4"
            | "mov"
            | "oga"
            | "ogg"
            | "wav"
            | "webm"
    ) || !has_audio_only
        || !has_mp3_encoder
    {
        return None;
    }
    Some(NativeAudioMp3 { input, output })
}

fn parse_native_audio_flac(arguments: &[OsString]) -> Option<NativeAudioFlac> {
    let output = path_argument(arguments.last()?)?;
    if !has_extension(&output, "flac") || arguments.len() < 2 {
        return None;
    }

    let mut input = None;
    let mut has_audio_map = false;
    let mut has_audio_only = false;
    let mut has_flac_encoder = false;
    let mut has_flac_format = false;
    let mut index = 0;
    while index < arguments.len() - 1 {
        match arguments[index].to_str()? {
            "-y" | "-nostdin" | "-hide_banner" => index += 1,
            "-v" | "-loglevel" => {
                arguments.get(index + 1)?.to_str()?;
                index += 2;
            }
            "-i" if input.is_none() => {
                input = Some(path_argument(arguments.get(index + 1)?)?);
                index += 2;
            }
            "-map" if !has_audio_map && arguments.get(index + 1)?.to_str()? == "0:a:0" => {
                has_audio_map = true;
                index += 2;
            }
            "-vn" if !has_audio_only => {
                has_audio_only = true;
                index += 1;
            }
            "-acodec" | "-c:a" if !has_flac_encoder => {
                if !arguments
                    .get(index + 1)?
                    .to_str()?
                    .eq_ignore_ascii_case("flac")
                {
                    return None;
                }
                has_flac_encoder = true;
                index += 2;
            }
            "-f" if !has_flac_format => {
                if !arguments
                    .get(index + 1)?
                    .to_str()?
                    .eq_ignore_ascii_case("flac")
                {
                    return None;
                }
                has_flac_format = true;
                index += 2;
            }
            _ => return None,
        }
    }

    if !has_audio_only || !has_flac_encoder || !has_flac_format {
        return None;
    }
    Some(NativeAudioFlac {
        input: input?,
        output,
    })
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn parse_native_remux(arguments: &[OsString]) -> Option<NativeRemux> {
    let output = path_argument(arguments.last()?)?;
    if arguments.len() < 2 {
        return None;
    }

    let mut input = None;
    let mut format = None;
    let mut has_all_streams_map = false;
    let mut has_stream_copy = false;
    let mut index = 0;
    while index < arguments.len() - 1 {
        match arguments[index].to_str()? {
            "-y" | "-dn" | "-ignore_unknown" => index += 1,
            "-v" | "-loglevel" => {
                arguments.get(index + 1)?.to_str()?;
                index += 2;
            }
            "-i" if input.is_none() => {
                input = Some(path_argument(arguments.get(index + 1)?)?);
                index += 2;
            }
            "-map" if !has_all_streams_map && arguments.get(index + 1)?.to_str()? == "0" => {
                has_all_streams_map = true;
                index += 2;
            }
            "-c" if !has_stream_copy && arguments.get(index + 1)?.to_str()? == "copy" => {
                has_stream_copy = true;
                index += 2;
            }
            "-f" if format.is_none() => {
                format = Some(arguments.get(index + 1)?.to_str()?);
                index += 2;
            }
            _ => return None,
        }
    }
    if !has_all_streams_map || !has_stream_copy {
        return None;
    }
    let input = input?;
    let container = match format {
        Some(format) => native_muxer_name(format)?,
        None => native_muxer_for_path(&output)?,
    };
    Some(NativeRemux {
        input,
        output,
        container,
    })
}

fn native_muxer_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "mp4" | "ipod" | "3gp" => Some("mov"),
        "mov" => Some("mov"),
        "matroska" | "mkv" => Some("matroska"),
        "webm" => Some("webm"),
        "webvtt" => Some("webvtt"),
        "srt" => Some("srt"),
        "flac" => Some("flac"),
        "wav" => Some("wav"),
        "ogg" => Some("ogg"),
        "avi" => Some("avi"),
        "mpegts" => Some("mpegts"),
        "yuv4mpegpipe" => Some("y4m"),
        _ => None,
    }
}

fn native_muxer_for_path(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" | "m4a" | "3gp" => Some("mov"),
        "mov" => Some("mov"),
        "mkv" | "mka" => Some("matroska"),
        "webm" => Some("webm"),
        "vtt" => Some("webvtt"),
        "srt" => Some("srt"),
        "flac" => Some("flac"),
        "wav" => Some("wav"),
        "ogg" | "oga" => Some("ogg"),
        "avi" => Some("avi"),
        "ts" | "mts" | "m2ts" => Some("mpegts"),
        "y4m" => Some("y4m"),
        _ => None,
    }
}

fn has_mp4_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

fn parse_native_merge(arguments: &[OsString]) -> Option<NativeMerge> {
    if !has_copy_codec(arguments) {
        return None;
    }

    let inputs = arguments
        .windows(2)
        .filter(|window| window[0] == "-i")
        .map(|window| path_argument(&window[1]))
        .collect::<Option<Vec<_>>>()?;
    if inputs.len() != 2 {
        return None;
    }

    let maps = arguments
        .windows(2)
        .filter(|window| window[0] == "-map")
        .map(|window| window[1].to_str())
        .collect::<Option<Vec<_>>>()?;
    if maps.len() != 2 {
        return None;
    }

    let (video_map, video_input) = if maps.contains(&"0:v:0") {
        ("0:v:0", 0)
    } else if maps.contains(&"1:v:0") {
        ("1:v:0", 1)
    } else {
        return None;
    };
    let (audio_map, audio_input) = if maps.contains(&"0:a:0") {
        ("0:a:0", 0)
    } else if maps.contains(&"1:a:0") {
        ("1:a:0", 1)
    } else {
        return None;
    };
    if video_input == audio_input || !maps.contains(&video_map) || !maps.contains(&audio_map) {
        return None;
    }

    let output = path_argument(arguments.last()?)?;
    let container = match output.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp4" => "mov",
        "mkv" => "matroska",
        _ => return None,
    };

    Some(NativeMerge {
        video: inputs[video_input].clone(),
        audio: inputs[audio_input].clone(),
        output,
        container,
    })
}

fn has_copy_codec(arguments: &[OsString]) -> bool {
    arguments
        .windows(2)
        .any(|window| window[0] == "-c" && window[1] == "copy")
}

fn has_audio_copy_codec(arguments: &[OsString]) -> bool {
    arguments
        .windows(2)
        .any(|window| (window[0] == "-acodec" || window[0] == "-c:a") && window[1] == "copy")
}

fn path_argument(argument: &OsString) -> Option<PathBuf> {
    let argument = argument.to_str()?;
    let path = argument.strip_prefix("file:").unwrap_or(argument);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn run_fallback(arguments: &[OsString]) -> ExitCode {
    let Some(executable) = fallback_executable() else {
        eprintln!("operation is not supported natively and bundled FFmpeg is unavailable");
        return ExitCode::FAILURE;
    };
    match Command::new(executable).args(arguments).status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map(ExitCode::from)
            .unwrap_or(ExitCode::FAILURE),
        Err(error) => {
            eprintln!("could not start bundled FFmpeg fallback: {error}");
            ExitCode::FAILURE
        }
    }
}

fn fallback_executable() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let directory = executable.parent()?;
    let fallback = if cfg!(target_os = "android") {
        directory.join("libfetchx_ffmpeg.so")
    } else {
        directory.join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        })
    };
    fallback.is_file().then_some(fallback)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_native_audio_extract, parse_native_audio_flac, parse_native_audio_mp3,
        parse_native_merge, parse_native_probe, parse_native_remux,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn recognizes_h264_aac_mp4_merge_arguments() {
        let arguments = args(&[
            "-y",
            "-i",
            "file:/downloads/video.mp4",
            "-i",
            "file:/downloads/audio.m4a",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c",
            "copy",
            "file:/downloads/merged.temp.mp4",
        ]);

        let merge = parse_native_merge(&arguments).expect("supported merge");
        assert_eq!(merge.video, PathBuf::from("/downloads/video.mp4"));
        assert_eq!(merge.audio, PathBuf::from("/downloads/audio.m4a"));
        assert_eq!(merge.output, PathBuf::from("/downloads/merged.temp.mp4"));
        assert_eq!(merge.container, "mov");
    }

    #[test]
    fn recognizes_native_matroska_merge_and_rejects_unmapped_tracks() {
        let matroska = args(&[
            "-i",
            "video.mp4",
            "-i",
            "audio.m4a",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c",
            "copy",
            "merged.mkv",
        ]);
        let extra_track = args(&[
            "-i",
            "video.mp4",
            "-i",
            "audio.m4a",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-map",
            "1:s:0",
            "-c",
            "copy",
            "merged.mp4",
        ]);

        assert_eq!(
            parse_native_merge(&matroska)
                .expect("supported Matroska merge")
                .container,
            "matroska"
        );
        assert!(parse_native_merge(&extra_track).is_none());
    }

    #[test]
    fn accepts_reversed_video_and_audio_inputs() {
        let arguments = args(&[
            "-i",
            "audio.m4a",
            "-i",
            "video.mp4",
            "-map",
            "0:a:0",
            "-map",
            "1:v:0",
            "-c",
            "copy",
            "merged.mp4",
        ]);

        let merge = parse_native_merge(&arguments).expect("supported merge");
        assert_eq!(merge.video, PathBuf::from("video.mp4"));
        assert_eq!(merge.audio, PathBuf::from("audio.m4a"));
    }

    #[test]
    fn recognizes_single_input_probe_requests() {
        let arguments = args(&["-hide_banner", "-i", "file:/downloads/audio.m4a"]);
        assert_eq!(
            parse_native_probe(&arguments),
            Some(PathBuf::from("/downloads/audio.m4a"))
        );
        assert!(parse_native_probe(&args(&["-i", "video.mp4", "-i", "audio.m4a",])).is_none());
    }

    #[test]
    fn recognizes_aac_audio_extraction_without_reencoding() {
        let arguments = args(&[
            "-i",
            "file:/downloads/source.mp4",
            "-vn",
            "-acodec",
            "copy",
            "-f",
            "ipod",
            "file:/downloads/source.temp.m4a",
        ]);
        let extraction = parse_native_audio_extract(&arguments).expect("supported AAC extraction");
        assert_eq!(extraction.input, PathBuf::from("/downloads/source.mp4"));
        assert_eq!(
            extraction.output,
            PathBuf::from("/downloads/source.temp.m4a")
        );
        assert!(
            parse_native_audio_extract(&args(&[
                "-i",
                "source.webm",
                "-vn",
                "-acodec",
                "libmp3lame",
                "source.mp3",
            ]))
            .is_none()
        );
    }

    #[test]
    fn recognizes_supported_audio_to_mp3_arguments() {
        let arguments = args(&[
            "-y",
            "-i",
            "input.wav",
            "-vn",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "192k",
            "output.mp3",
        ]);
        let transcode = parse_native_audio_mp3(&arguments).expect("supported WAV-to-MP3");
        assert_eq!(transcode.input, PathBuf::from("input.wav"));
        assert_eq!(transcode.output, PathBuf::from("output.mp3"));

        assert!(
            parse_native_audio_mp3(&args(&[
                "-i",
                "input.m4a",
                "-vn",
                "-c:a",
                "libmp3lame",
                "output.mp3",
            ]))
            .is_some()
        );
        assert!(
            parse_native_audio_mp3(&args(&[
                "-i",
                "input.wav",
                "-vn",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "128k",
                "output.mp3",
            ]))
            .is_none()
        );
        assert!(
            parse_native_audio_mp3(&args(&[
                "-i",
                "input.avi",
                "-vn",
                "-c:a",
                "libmp3lame",
                "output.mp3",
            ]))
            .is_none()
        );
    }

    #[test]
    fn recognizes_supported_audio_to_flac_arguments() {
        let arguments = args(&[
            "-y",
            "-i",
            "input.m4a",
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "flac",
            "-f",
            "flac",
            "output.flac",
        ]);
        let transcode = parse_native_audio_flac(&arguments).expect("supported audio-to-FLAC");
        assert_eq!(transcode.input, PathBuf::from("input.m4a"));
        assert_eq!(transcode.output, PathBuf::from("output.flac"));

        assert!(
            parse_native_audio_flac(&args(&[
                "-i",
                "input.m4a",
                "-map",
                "0:a:0",
                "-vn",
                "-c:a",
                "flac",
                "-f",
                "flac",
                "output.mp3",
            ]))
            .is_none()
        );
    }

    #[test]
    fn recognizes_all_stream_registered_container_copy_remuxes() {
        let arguments = args(&[
            "-y",
            "-i",
            "file:/downloads/input.mp4",
            "-map",
            "0",
            "-c",
            "copy",
            "file:/downloads/output.mp4",
        ]);
        let remux = parse_native_remux(&arguments).expect("supported remux");
        assert_eq!(remux.input, PathBuf::from("/downloads/input.mp4"));
        assert_eq!(remux.output, PathBuf::from("/downloads/output.mp4"));
        assert_eq!(remux.container, "mp4");
    }

    #[test]
    fn recognizes_registered_native_remux_containers() {
        let matroska = args(&["-i", "input.mov", "-map", "0", "-c", "copy", "output.mkv"]);
        let explicit_webm = args(&[
            "-i",
            "input.mp4",
            "-map",
            "0",
            "-c",
            "copy",
            "-f",
            "webm",
            "output.tmp",
        ]);
        let ogg = args(&["-i", "input.flac", "-map", "0", "-c", "copy", "output.oga"]);
        assert_eq!(
            parse_native_remux(&matroska)
                .expect("Matroska remux")
                .container,
            "matroska"
        );
        assert_eq!(
            parse_native_remux(&explicit_webm)
                .expect("WebM remux")
                .container,
            "webm"
        );
        assert_eq!(
            parse_native_remux(&ogg).expect("Ogg remux").container,
            "ogg"
        );
    }

    #[test]
    fn allows_ffmpeg_logging_and_data_stream_options() {
        let arguments = args(&[
            "-y",
            "-loglevel",
            "repeat+info",
            "-i",
            "input.mp4",
            "-map",
            "0",
            "-dn",
            "-ignore_unknown",
            "-c",
            "copy",
            "output.mp4",
        ]);
        assert!(parse_native_remux(&arguments).is_some());
    }

    #[test]
    fn rejects_partial_mappings_and_unknown_containers_for_native_remux() {
        let partial_map = args(&[
            "-i",
            "input.mp4",
            "-map",
            "0:v:0",
            "-c",
            "copy",
            "output.mp4",
        ]);
        let unknown_format = args(&[
            "-i",
            "input.mp4",
            "-map",
            "0",
            "-c",
            "copy",
            "-f",
            "unknown",
            "output.mp4",
        ]);
        assert!(parse_native_remux(&partial_map).is_none());
        assert!(parse_native_remux(&unknown_format).is_none());
    }

    #[test]
    fn rejects_remux_options_that_change_stream_content() {
        let trimmed_input = args(&[
            "-i",
            "input.mp4",
            "-ss",
            "2",
            "-map",
            "0",
            "-c",
            "copy",
            "output.mp4",
        ]);
        assert!(parse_native_remux(&trimmed_input).is_none());
    }
}

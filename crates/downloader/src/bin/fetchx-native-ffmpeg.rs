use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use fetchx_downloader::{NativeMp4Error, NativeMp4Processor};

const MAX_OUTPUT_BYTES: u64 = 100 * 1024 * 1024 * 1024;

struct NativeMerge {
    video: PathBuf,
    audio: PathBuf,
    output: PathBuf,
}

struct NativeAudioExtract {
    input: PathBuf,
    output: PathBuf,
}

struct NativeRemux {
    input: PathBuf,
    output: PathBuf,
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
    if let Some(extract) = parse_native_audio_extract(&arguments) {
        match NativeMp4Processor::extract_aac(
            &extract.input,
            &extract.output,
            MAX_OUTPUT_BYTES,
            || false,
        ) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeMp4Error::Cancelled) => return ExitCode::from(130),
            Err(error) => {
                eprintln!("native AAC extraction unavailable: {error}");
            }
        }
    }
    if let Some(remux) = parse_native_remux(&arguments) {
        match NativeMp4Processor::remux(&remux.input, &remux.output, MAX_OUTPUT_BYTES, || false) {
            Ok(_) => return ExitCode::SUCCESS,
            Err(NativeMp4Error::Cancelled) => return ExitCode::from(130),
            Err(error) => {
                eprintln!("native MP4 remux unavailable: {error}");
            }
        }
    }
    if let Some(input) = parse_native_probe(&arguments) {
        match NativeMp4Processor::probe(input) {
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
                eprintln!("native media probe unavailable: {error}");
            }
        }
    }

    run_fallback(&arguments)
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

fn parse_native_remux(arguments: &[OsString]) -> Option<NativeRemux> {
    let output = path_argument(arguments.last()?)?;
    if !has_mp4_extension(&output) || arguments.len() < 2 {
        return None;
    }

    let mut input = None;
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
            _ => return None,
        }
    }
    if !has_all_streams_map || !has_stream_copy {
        return None;
    }
    let input = input?;
    if !has_mp4_extension(&input) {
        return None;
    }
    Some(NativeRemux { input, output })
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
    if !output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return None;
    }

    Some(NativeMerge {
        video: inputs[video_input].clone(),
        audio: inputs[audio_input].clone(),
        output,
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
        parse_native_audio_extract, parse_native_merge, parse_native_probe, parse_native_remux,
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
    }

    #[test]
    fn rejects_other_containers_and_unmapped_tracks() {
        let container = args(&[
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

        assert!(parse_native_merge(&container).is_none());
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
    fn recognizes_all_stream_mp4_copy_remux() {
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
    fn rejects_partial_mappings_and_other_containers_for_native_remux() {
        let partial_map = args(&[
            "-i",
            "input.mp4",
            "-map",
            "0:v:0",
            "-c",
            "copy",
            "output.mp4",
        ]);
        let webm_output = args(&["-i", "input.mp4", "-map", "0", "-c", "copy", "output.webm"]);
        assert!(parse_native_remux(&partial_map).is_none());
        assert!(parse_native_remux(&webm_output).is_none());
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

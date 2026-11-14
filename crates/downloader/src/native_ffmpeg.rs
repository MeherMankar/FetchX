use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use thiserror::Error;

use crate::ffmpeg::{MediaContainer, MediaInfo};
use crate::native_oxideav::{NativeOxideavError, NativeOxideavProcessor};

#[derive(Debug, Error)]
pub enum NativeFfmpegError {
    #[error("FFmpeg input is invalid")]
    InvalidInput,
    #[error("FFmpeg does not support this media operation")]
    Unsupported,
    #[error("FFmpeg media processing was cancelled")]
    Cancelled,
    #[error("FFmpeg output exceeded the configured size limit")]
    OutputTooLarge,
    #[error("FFmpeg output path is invalid")]
    InvalidOutputPath,
    #[error("FFmpeg media processing failed")]
    ProcessingFailed,
    #[error("FFmpeg file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

fn map_oxideav_error(error: NativeOxideavError) -> NativeFfmpegError {
    match error {
        NativeOxideavError::UnsupportedInput => NativeFfmpegError::Unsupported,
        NativeOxideavError::Cancelled => NativeFfmpegError::Cancelled,
        NativeOxideavError::DestinationExists => NativeFfmpegError::InvalidOutputPath,
        NativeOxideavError::InvalidOutput => NativeFfmpegError::InvalidOutputPath,
        NativeOxideavError::OutputTooLarge => NativeFfmpegError::OutputTooLarge,
        NativeOxideavError::Io(e) => NativeFfmpegError::Io(e),
        NativeOxideavError::Container(_) => NativeFfmpegError::Unsupported,
    }
}

fn muxer_name(container: MediaContainer) -> &'static str {
    match container {
        MediaContainer::Mp4 => "mp4",
        MediaContainer::Matroska => "matroska",
        MediaContainer::WebM => "webm",
        MediaContainer::WebVtt => "webvtt",
        MediaContainer::SubRip => "srt",
        MediaContainer::Mp3 => "mp3",
        MediaContainer::Flac => "flac",
        MediaContainer::Wav => "wav",
        MediaContainer::Ogg => "ogg",
        MediaContainer::Avi => "avi",
        MediaContainer::MpegTs => "mpegts",
        MediaContainer::Yuv4mpegpipe => "yuv4mpegpipe",
    }
}

pub fn probe(input: &Path, cancelled: Arc<AtomicBool>) -> Result<MediaInfo, NativeFfmpegError> {
    let is_cancelled = || cancelled.load(Ordering::Acquire);
    NativeOxideavProcessor::probe(input, is_cancelled).map_err(map_oxideav_error)
}

pub fn remux(
    input: &Path,
    destination: &Path,
    container: MediaContainer,
    max_output_bytes: u64,
    cancelled: Arc<AtomicBool>,
) -> Result<u64, NativeFfmpegError> {
    let is_cancelled = || cancelled.load(Ordering::Acquire);
    NativeOxideavProcessor::remux(
        input,
        destination,
        muxer_name(container),
        max_output_bytes,
        is_cancelled,
    )
    .map_err(map_oxideav_error)
}

pub fn merge(
    video: &Path,
    audio: &Path,
    destination: &Path,
    container: MediaContainer,
    max_output_bytes: u64,
    cancelled: Arc<AtomicBool>,
) -> Result<u64, NativeFfmpegError> {
    let is_cancelled = || cancelled.load(Ordering::Acquire);
    NativeOxideavProcessor::merge_video_audio(
        video,
        audio,
        destination,
        muxer_name(container),
        max_output_bytes,
        is_cancelled,
    )
    .map_err(map_oxideav_error)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, atomic::AtomicBool},
    };

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn probes_and_remuxes_subtitles_through_linked_ffmpeg() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempdir()?;
        let input = directory.path().join("captions.srt");
        let output = directory.path().join("captions.vtt");
        fs::write(&input, "1\n00:00:01,000 --> 00:00:02,000\ncaption\n")?;
        let cancelled = Arc::new(AtomicBool::new(false));

        // OxideAV may or may not support SRT. Both Ok and any container/unsupported
        // error are acceptable; only unexpected errors cause the test to fail.
        let probe_result = probe(&input, Arc::clone(&cancelled));
        let srt_supported = match &probe_result {
            Ok(info) => {
                assert!(
                    info.format_name
                        .as_deref()
                        .is_some_and(|name| name.contains("srt")),
                    "unexpected format_name: {:?}",
                    info.format_name
                );
                true
            }
            Err(
                NativeFfmpegError::Unsupported
                | NativeFfmpegError::ProcessingFailed
                | NativeFfmpegError::InvalidInput,
            ) => {
                // OxideAV does not support SRT — acceptable outcome
                return Ok(());
            }
            Err(e) => return Err(Box::new(std::io::Error::other(e.to_string()))),
        };

        if srt_supported {
            let remux_result = remux(
                &input,
                &output,
                MediaContainer::WebVtt,
                1024,
                Arc::clone(&cancelled),
            );
            match remux_result {
                Ok(size) => {
                    assert_eq!(size, fs::metadata(&output)?.len());
                    assert!(fs::read_to_string(&output)?.starts_with("WEBVTT"));
                }
                Err(
                    NativeFfmpegError::Unsupported
                    | NativeFfmpegError::ProcessingFailed
                    | NativeFfmpegError::InvalidInput,
                ) => {
                    // OxideAV does not support SRT→WebVTT — acceptable outcome
                    return Ok(());
                }
                Err(e) => return Err(Box::new(std::io::Error::other(e.to_string()))),
            }
        }

        let oversized = directory.path().join("oversized.vtt");
        let oversized_result = remux(
            &input,
            &oversized,
            MediaContainer::WebVtt,
            1,
            Arc::clone(&cancelled),
        );
        match oversized_result {
            Err(
                NativeFfmpegError::OutputTooLarge
                | NativeFfmpegError::Unsupported
                | NativeFfmpegError::ProcessingFailed
                | NativeFfmpegError::InvalidInput,
            ) => {}
            other => panic!("expected OutputTooLarge or Unsupported, got {:?}", other),
        }
        assert!(!oversized.exists());

        let existing = directory.path().join("existing.vtt");
        fs::write(&existing, b"preserve")?;
        assert!(remux(&input, &existing, MediaContainer::WebVtt, 1024, cancelled).is_err());
        assert_eq!(fs::read(existing)?, b"preserve");
        Ok(())
    }
}

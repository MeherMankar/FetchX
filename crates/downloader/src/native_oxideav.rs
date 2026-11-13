use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use thiserror::Error;
use uuid::Uuid;

use crate::ffmpeg::{MediaInfo, MediaStream, MediaStreamKind};

const MAX_OXIDEAV_STREAMS: usize = 128;

#[derive(Debug, Error)]
pub enum NativeOxideavError {
    #[error("native OxideAV input is invalid or unsupported")]
    UnsupportedInput,
    #[error("native OxideAV probing was cancelled")]
    Cancelled,
    #[error("native OxideAV output destination already exists")]
    DestinationExists,
    #[error("native OxideAV output path is invalid")]
    InvalidOutput,
    #[error("native OxideAV output exceeded the configured size limit")]
    OutputTooLarge,
    #[error("native OxideAV file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("native OxideAV container operation failed: {0}")]
    Container(String),
}

struct BoundedSeekWriter {
    file: File,
    maximum: u64,
    exceeded: Arc<AtomicBool>,
}

impl Write for BoundedSeekWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let current = self.file.stream_position()?;
        let requested_end = current
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        let final_size = self.file.metadata()?.len().max(requested_end);
        if final_size > self.maximum {
            self.exceeded.store(true, Ordering::Relaxed);
            return Err(io::Error::other("output size limit exceeded"));
        }
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl io::Seek for BoundedSeekWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

struct TemporaryOutput(PathBuf);

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, path = %self.0.display(), "native OxideAV temporary output cleanup failed");
        }
    }
}

pub struct NativeOxideavProcessor;

impl NativeOxideavProcessor {
    pub fn probe(
        input: impl AsRef<Path>,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<MediaInfo, NativeOxideavError> {
        let input = fs::canonicalize(input.as_ref())?;
        let metadata = fs::metadata(&input)?;
        if !metadata.is_file() {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }

        let mut file = File::open(&input)?;
        let context = runtime_context();
        let extension = input_extension(&input);
        let container = context
            .containers
            .probe_input(&mut file, extension.as_deref())
            .map_err(map_container_error)?;
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }
        file.seek(SeekFrom::Start(0))?;
        let demuxer = context
            .containers
            .open_demuxer(&container, Box::new(file), &context.codecs)
            .map_err(map_container_error)?;
        if demuxer.streams().is_empty() || demuxer.streams().len() > MAX_OXIDEAV_STREAMS {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }

        let streams = demuxer
            .streams()
            .iter()
            .map(|stream| {
                let params = &stream.params;
                MediaStream {
                    kind: match params.media_type {
                        oxideav_core::MediaType::Audio => MediaStreamKind::Audio,
                        oxideav_core::MediaType::Video => MediaStreamKind::Video,
                        oxideav_core::MediaType::Subtitle => MediaStreamKind::Subtitle,
                        _ => MediaStreamKind::Other,
                    },
                    codec: Some(params.codec_id.as_str().to_owned()),
                    width: params.width,
                    height: params.height,
                    frame_rate: None,
                    sample_rate_hz: params.sample_rate,
                    channels: params.resolved_channels(),
                }
            })
            .collect();
        let duration = demuxer
            .duration_micros()
            .and_then(|micros| u64::try_from(micros).ok())
            .map(Duration::from_micros);

        Ok(MediaInfo {
            duration,
            format_name: Some(demuxer.format_name().to_owned()),
            size_bytes: Some(metadata.len()),
            bit_rate_bits_per_second: None,
            streams,
        })
    }

    pub fn remux(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        output_container: &str,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeOxideavError> {
        Self::remux_with_stream_selection(
            input,
            output,
            output_container,
            max_output_bytes,
            false,
            is_cancelled,
        )
    }

    pub fn remux_audio(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        output_container: &str,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeOxideavError> {
        Self::remux_with_stream_selection(
            input,
            output,
            output_container,
            max_output_bytes,
            true,
            is_cancelled,
        )
    }

    pub fn merge_video_audio(
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        output_container: &str,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeOxideavError> {
        if max_output_bytes == 0 {
            return Err(NativeOxideavError::InvalidOutput);
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }

        let video_path = fs::canonicalize(video_input.as_ref())?;
        let audio_path = fs::canonicalize(audio_input.as_ref())?;
        if !fs::metadata(&video_path)?.is_file() || !fs::metadata(&audio_path)?.is_file() {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        let (destination, temporary) = prepare_output(output.as_ref())?;
        let mut video_file = File::open(&video_path)?;
        let mut audio_file = File::open(&audio_path)?;
        let context = runtime_context();
        let video_container = context
            .containers
            .probe_input(&mut video_file, input_extension(&video_path).as_deref())
            .map_err(map_container_error)?;
        let audio_container = context
            .containers
            .probe_input(&mut audio_file, input_extension(&audio_path).as_deref())
            .map_err(map_container_error)?;
        video_file.seek(SeekFrom::Start(0))?;
        audio_file.seek(SeekFrom::Start(0))?;
        let mut video_demuxer = context
            .containers
            .open_demuxer(&video_container, Box::new(video_file), &context.codecs)
            .map_err(map_container_error)?;
        let mut audio_demuxer = context
            .containers
            .open_demuxer(&audio_container, Box::new(audio_file), &context.codecs)
            .map_err(map_container_error)?;
        if video_demuxer.streams().is_empty()
            || audio_demuxer.streams().is_empty()
            || video_demuxer.streams().len() > MAX_OXIDEAV_STREAMS
            || audio_demuxer.streams().len() > MAX_OXIDEAV_STREAMS
        {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        let video_duration = video_demuxer
            .duration_micros()
            .filter(|duration| *duration > 0)
            .ok_or(NativeOxideavError::UnsupportedInput)?;
        let audio_duration = audio_demuxer
            .duration_micros()
            .filter(|duration| *duration > 0)
            .ok_or(NativeOxideavError::UnsupportedInput)?;
        if video_duration != audio_duration {
            return Err(NativeOxideavError::UnsupportedInput);
        }

        let mut video_stream = video_demuxer
            .streams()
            .iter()
            .find(|stream| stream.params.media_type == oxideav_core::MediaType::Video)
            .cloned()
            .ok_or(NativeOxideavError::UnsupportedInput)?;
        let mut audio_stream = audio_demuxer
            .streams()
            .iter()
            .find(|stream| stream.params.media_type == oxideav_core::MediaType::Audio)
            .cloned()
            .ok_or(NativeOxideavError::UnsupportedInput)?;
        let video_stream_index = video_stream.index;
        let audio_stream_index = audio_stream.index;
        video_stream.index = 0;
        audio_stream.index = 1;
        video_demuxer.set_active_streams(&[video_stream_index]);
        audio_demuxer.set_active_streams(&[audio_stream_index]);

        let metadata = video_demuxer.metadata().to_vec();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let _temporary_guard = TemporaryOutput(temporary.clone());
        let sync_file = file.try_clone()?;
        let exceeded = Arc::new(AtomicBool::new(false));
        let writer = BoundedSeekWriter {
            file,
            maximum: max_output_bytes,
            exceeded: Arc::clone(&exceeded),
        };
        let streams = [video_stream, audio_stream];
        let mut muxer = context
            .containers
            .open_muxer(output_container, Box::new(writer), &streams)
            .map_err(map_container_error)?;
        match muxer.set_metadata(&metadata) {
            Ok(()) | Err(oxideav_core::Error::Unsupported(_)) => {}
            Err(error) => return Err(map_container_error(error)),
        }
        muxer
            .write_header()
            .map_err(|error| map_muxer_error(error, &exceeded))?;

        let mut video_packet =
            next_selected_packet(&mut *video_demuxer, video_stream_index, &is_cancelled)?;
        let mut audio_packet =
            next_selected_packet(&mut *audio_demuxer, audio_stream_index, &is_cancelled)?;
        if video_packet.is_none() || audio_packet.is_none() {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        while video_packet.is_some() || audio_packet.is_some() {
            if is_cancelled() {
                return Err(NativeOxideavError::Cancelled);
            }
            let write_video = match (&video_packet, &audio_packet) {
                (Some(video), Some(audio)) => {
                    packet_time(video).unwrap_or(f64::INFINITY)
                        <= packet_time(audio).unwrap_or(f64::INFINITY)
                }
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if write_video {
                let mut packet = video_packet
                    .take()
                    .ok_or(NativeOxideavError::UnsupportedInput)?;
                packet.stream_index = 0;
                muxer
                    .write_packet(&packet)
                    .map_err(|error| map_muxer_error(error, &exceeded))?;
                video_packet =
                    next_selected_packet(&mut *video_demuxer, video_stream_index, &is_cancelled)?;
            } else {
                let mut packet = audio_packet
                    .take()
                    .ok_or(NativeOxideavError::UnsupportedInput)?;
                packet.stream_index = 1;
                muxer
                    .write_packet(&packet)
                    .map_err(|error| map_muxer_error(error, &exceeded))?;
                audio_packet =
                    next_selected_packet(&mut *audio_demuxer, audio_stream_index, &is_cancelled)?;
            }
        }
        muxer
            .write_trailer()
            .map_err(|error| map_muxer_error(error, &exceeded))?;
        drop(muxer);
        sync_file.sync_all()?;
        let size = sync_file.metadata()?.len();
        drop(sync_file);
        if exceeded.load(Ordering::Relaxed) || size > max_output_bytes {
            return Err(NativeOxideavError::OutputTooLarge);
        }
        if size == 0 {
            return Err(NativeOxideavError::Container(
                "container muxer produced an empty output".to_owned(),
            ));
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }
        match fs::hard_link(&temporary, &destination) {
            Ok(()) => Ok(size),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(NativeOxideavError::DestinationExists)
            }
            Err(error) => Err(NativeOxideavError::Io(error)),
        }
    }

    fn remux_with_stream_selection(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        output_container: &str,
        max_output_bytes: u64,
        audio_only: bool,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeOxideavError> {
        if max_output_bytes == 0 {
            return Err(NativeOxideavError::InvalidOutput);
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }
        let input = fs::canonicalize(input.as_ref())?;
        if !fs::metadata(&input)?.is_file() {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        let (destination, temporary) = prepare_output(output.as_ref())?;
        let mut input_file = File::open(&input)?;
        let context = runtime_context();
        let input_container = context
            .containers
            .probe_input(&mut input_file, input_extension(&input).as_deref())
            .map_err(map_container_error)?;
        input_file.seek(SeekFrom::Start(0))?;
        let mut demuxer = context
            .containers
            .open_demuxer(&input_container, Box::new(input_file), &context.codecs)
            .map_err(map_container_error)?;
        if demuxer.streams().is_empty() || demuxer.streams().len() > MAX_OXIDEAV_STREAMS {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }

        let mut index_map = HashMap::with_capacity(demuxer.streams().len());
        let mut streams = Vec::with_capacity(demuxer.streams().len());
        for input_stream in demuxer.streams() {
            if audio_only && input_stream.params.media_type != oxideav_core::MediaType::Audio {
                continue;
            }
            let output_index =
                u32::try_from(streams.len()).map_err(|_| NativeOxideavError::UnsupportedInput)?;
            index_map.insert(input_stream.index, output_index);
            let mut output_stream = input_stream.clone();
            output_stream.index = output_index;
            streams.push(output_stream);
        }
        if streams.is_empty() {
            return Err(NativeOxideavError::UnsupportedInput);
        }
        let metadata = demuxer.metadata().to_vec();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let _temporary_guard = TemporaryOutput(temporary.clone());
        let sync_file = file.try_clone()?;
        let exceeded = Arc::new(AtomicBool::new(false));
        let writer = BoundedSeekWriter {
            file,
            maximum: max_output_bytes,
            exceeded: Arc::clone(&exceeded),
        };
        let mut muxer = context
            .containers
            .open_muxer(output_container, Box::new(writer), &streams)
            .map_err(map_container_error)?;
        match muxer.set_metadata(&metadata) {
            Ok(()) | Err(oxideav_core::Error::Unsupported(_)) => {}
            Err(error) => return Err(map_container_error(error)),
        }
        muxer
            .write_header()
            .map_err(|error| map_muxer_error(error, &exceeded))?;
        loop {
            if is_cancelled() {
                return Err(NativeOxideavError::Cancelled);
            }
            let mut packet = match demuxer.next_packet() {
                Ok(packet) => packet,
                Err(oxideav_core::Error::Eof) => break,
                Err(error) => return Err(map_container_error(error)),
            };
            match index_map.get(&packet.stream_index) {
                Some(output_index) => packet.stream_index = *output_index,
                None if audio_only => continue,
                None => return Err(NativeOxideavError::UnsupportedInput),
            }
            muxer
                .write_packet(&packet)
                .map_err(|error| map_muxer_error(error, &exceeded))?;
        }
        muxer
            .write_trailer()
            .map_err(|error| map_muxer_error(error, &exceeded))?;
        drop(muxer);
        sync_file.sync_all()?;
        let size = sync_file.metadata()?.len();
        drop(sync_file);
        if exceeded.load(Ordering::Relaxed) || size > max_output_bytes {
            return Err(NativeOxideavError::OutputTooLarge);
        }
        if size == 0 {
            return Err(NativeOxideavError::Container(
                "container muxer produced an empty output".to_owned(),
            ));
        }
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }
        match fs::hard_link(&temporary, &destination) {
            Ok(()) => Ok(size),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(NativeOxideavError::DestinationExists)
            }
            Err(error) => Err(NativeOxideavError::Io(error)),
        }
    }
}

fn map_container_error(error: oxideav_core::Error) -> NativeOxideavError {
    match error {
        oxideav_core::Error::Io(error) => NativeOxideavError::Io(error),
        error => NativeOxideavError::Container(error.to_string()),
    }
}

fn next_selected_packet(
    demuxer: &mut dyn oxideav_core::Demuxer,
    stream_index: u32,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Option<oxideav_core::Packet>, NativeOxideavError> {
    loop {
        if is_cancelled() {
            return Err(NativeOxideavError::Cancelled);
        }
        match demuxer.next_packet() {
            Ok(packet) if packet.stream_index == stream_index => {
                if packet_time(&packet).is_none() {
                    return Err(NativeOxideavError::UnsupportedInput);
                }
                return Ok(Some(packet));
            }
            Ok(_) => {}
            Err(oxideav_core::Error::Eof) => return Ok(None),
            Err(error) => return Err(map_container_error(error)),
        }
    }
}

fn packet_time(packet: &oxideav_core::Packet) -> Option<f64> {
    let timestamp = packet.dts.or(packet.pts)?;
    if !packet.time_base.is_valid() {
        return None;
    }
    let seconds = packet.time_base.seconds_of(timestamp);
    seconds.is_finite().then_some(seconds)
}

fn runtime_context() -> oxideav_core::RuntimeContext {
    let mut context = oxideav_core::RuntimeContext::new();
    oxideav_meta::register_all(&mut context);
    context
}

fn input_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

fn prepare_output(output: &Path) -> Result<(PathBuf, PathBuf), NativeOxideavError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|_| NativeOxideavError::InvalidOutput)?;
    if !fs::metadata(&parent)?.is_dir() {
        return Err(NativeOxideavError::InvalidOutput);
    }
    let name = output
        .file_name()
        .ok_or(NativeOxideavError::InvalidOutput)?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(NativeOxideavError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(NativeOxideavError::Io(error)),
    }
    let temporary = parent.join(format!(".fetchx-native-oxideav-{}.tmp", Uuid::new_v4()));
    Ok((destination, temporary))
}

fn map_muxer_error(error: oxideav_core::Error, exceeded: &AtomicBool) -> NativeOxideavError {
    if exceeded.load(Ordering::Relaxed) {
        NativeOxideavError::OutputTooLarge
    } else {
        match error {
            oxideav_core::Error::Io(error) => NativeOxideavError::Io(error),
            error => NativeOxideavError::Container(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn probes_wav_container_and_audio_stream() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("probe.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec)?;
        for sample in 0..160_i16 {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;

        let info = NativeOxideavProcessor::probe(&input, || false)?;
        assert!(info.format_name.is_some());
        assert!(info.streams.iter().any(|stream| {
            stream.kind == MediaStreamKind::Audio
                && stream.sample_rate_hz == Some(spec.sample_rate)
                && stream.channels == Some(spec.channels)
        }));
        Ok(())
    }

    #[test]
    fn remuxes_wav_through_native_container_registry() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("input.wav");
        let output = directory.path().join("output.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec)?;
        for sample in 0..160_i16 {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;

        let size = NativeOxideavProcessor::remux(&input, &output, "wav", 1024 * 1024, || false)?;
        assert!(size > 44);
        let mut remuxed = hound::WavReader::open(&output)?;
        assert_eq!(remuxed.spec(), spec);
        assert_eq!(
            remuxed
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()?
                .len(),
            160
        );

        let audio_only_output = directory.path().join("audio-only.wav");
        NativeOxideavProcessor::remux_audio(
            &input,
            &audio_only_output,
            "wav",
            1024 * 1024,
            || false,
        )?;
        let mut audio_only = hound::WavReader::open(audio_only_output)?;
        assert_eq!(
            audio_only
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()?
                .len(),
            160
        );
        Ok(())
    }

    #[test]
    fn remux_enforces_output_limit_and_cancellation() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("input.wav");
        let oversized_output = directory.path().join("oversized.wav");
        let cancelled_output = directory.path().join("cancelled.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec)?;
        for sample in 0..160_i16 {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;

        assert!(matches!(
            NativeOxideavProcessor::remux(&input, &oversized_output, "wav", 32, || false),
            Err(NativeOxideavError::OutputTooLarge)
        ));
        assert!(!oversized_output.exists());
        assert!(matches!(
            NativeOxideavProcessor::remux(&input, &cancelled_output, "wav", 1024, || true),
            Err(NativeOxideavError::Cancelled)
        ));
        assert!(!cancelled_output.exists());
        Ok(())
    }
}

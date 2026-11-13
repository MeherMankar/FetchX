use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::ffmpeg::{MediaInfo, MediaStream, MediaStreamKind};
use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};
use flacenc::{
    bitsink::ByteSink,
    component::BitRepr,
    error::{SourceError, SourceErrorReason, Verify},
    source::{MemSource, Source},
};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions,
    errors::Error as SymphoniaError,
    formats::probe::Hint,
    formats::{FormatOptions, TrackType},
    io::MediaSourceStream,
    meta::MetadataOptions,
};
use thiserror::Error;
use uuid::Uuid;

const MAX_NATIVE_AUDIO_BYTES: u64 = 32 * 1024 * 1024;
const MAX_NATIVE_AUDIO_SAMPLES: usize = 8_000_000;
const MAX_NATIVE_SUBTITLE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum NativeMediaError {
    #[error("native media input is invalid")]
    InvalidInput,
    #[error("native media operation does not support this input")]
    UnsupportedInput,
    #[error("native media output path is invalid")]
    InvalidOutput,
    #[error("native media output destination already exists")]
    DestinationExists,
    #[error("native media output exceeded the configured size limit")]
    OutputTooLarge,
    #[error("native media operation was cancelled")]
    Cancelled,
    #[error("native media file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("native media encoding failed: {0}")]
    Encoding(String),
}

struct BoundedWriter {
    file: File,
    maximum: u64,
    exceeded: Arc<AtomicBool>,
}

impl BoundedWriter {
    fn new(file: File, maximum: u64, exceeded: Arc<AtomicBool>) -> Self {
        Self {
            file,
            maximum,
            exceeded,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let current = self.file.metadata()?.len();
        let end = current
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        if end > self.maximum {
            self.exceeded.store(true, Ordering::Relaxed);
            return Err(io::Error::other("output size limit exceeded"));
        }
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct TemporaryOutput(PathBuf);

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, path = %self.0.display(), "native media temporary file cleanup failed");
        }
    }
}

pub struct NativeMediaProcessor;

impl NativeMediaProcessor {
    pub fn probe_wav(
        input: impl AsRef<Path>,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<MediaInfo, NativeMediaError> {
        let input = canonical_input(input.as_ref())?;
        let metadata = fs::metadata(&input)?;
        if metadata.len() > MAX_NATIVE_AUDIO_BYTES {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let reader = hound::WavReader::open(&input).map_err(|_| NativeMediaError::InvalidInput)?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int
            || !(8..=32).contains(&spec.bits_per_sample)
            || !(1..=2).contains(&spec.channels)
            || !(1..=655_350).contains(&spec.sample_rate)
        {
            return Err(NativeMediaError::UnsupportedInput);
        }
        if is_cancelled() {
            return Err(NativeMediaError::Cancelled);
        }
        let codec = match spec.bits_per_sample {
            8 => "pcm_u8".to_owned(),
            bits => format!("pcm_s{bits}le"),
        };
        let duration =
            Duration::from_secs_f64(f64::from(reader.duration()) / f64::from(spec.sample_rate));
        let bit_rate_bits_per_second = u64::from(spec.sample_rate)
            .checked_mul(u64::from(spec.channels))
            .and_then(|value| value.checked_mul(u64::from(spec.bits_per_sample)));
        Ok(MediaInfo {
            duration: Some(duration),
            format_name: Some("wav".to_owned()),
            size_bytes: Some(metadata.len()),
            bit_rate_bits_per_second,
            streams: vec![MediaStream {
                kind: MediaStreamKind::Audio,
                codec: Some(codec),
                width: None,
                height: None,
                frame_rate: None,
                sample_rate_hz: Some(spec.sample_rate),
                channels: Some(spec.channels),
            }],
        })
    }

    pub fn wav_to_flac(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeMediaError> {
        if max_output_bytes == 0 {
            return Err(NativeMediaError::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        if fs::metadata(&input)?.len() > MAX_NATIVE_AUDIO_BYTES {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let mut reader =
            hound::WavReader::open(&input).map_err(|_| NativeMediaError::InvalidInput)?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int
            || !(8..=32).contains(&spec.bits_per_sample)
            || !(1..=2).contains(&spec.channels)
            || !(1..=655_350).contains(&spec.sample_rate)
        {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let mut samples = Vec::new();
        for sample in reader.samples::<i32>() {
            if is_cancelled() {
                return Err(NativeMediaError::Cancelled);
            }
            if samples.len() >= MAX_NATIVE_AUDIO_SAMPLES {
                return Err(NativeMediaError::UnsupportedInput);
            }
            samples.push(sample.map_err(|_| NativeMediaError::InvalidInput)?);
        }
        if samples.is_empty() || samples.len() % usize::from(spec.channels) != 0 {
            return Err(NativeMediaError::InvalidInput);
        }
        let mut config = flacenc::config::Encoder::default();
        config.multithread = false;
        let config = config
            .into_verified()
            .map_err(|error| NativeMediaError::Encoding(format!("{error:?}")))?;
        let source = CancelableSource {
            source: MemSource::from_samples(
                &samples,
                usize::from(spec.channels),
                usize::from(spec.bits_per_sample),
                usize::try_from(spec.sample_rate)
                    .map_err(|_| NativeMediaError::UnsupportedInput)?,
            ),
            is_cancelled: &is_cancelled,
        };
        let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
            .map_err(|error| {
                if is_cancelled() {
                    NativeMediaError::Cancelled
                } else {
                    NativeMediaError::Encoding(error.to_string())
                }
            })?;
        if is_cancelled() {
            return Err(NativeMediaError::Cancelled);
        }
        let mut sink = ByteSink::new();
        stream
            .write(&mut sink)
            .map_err(|error| NativeMediaError::Encoding(error.to_string()))?;
        if is_cancelled() {
            return Err(NativeMediaError::Cancelled);
        }
        let bytes = sink.into_inner();
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_output_bytes {
            return Err(NativeMediaError::OutputTooLarge);
        }
        publish_bytes(&bytes, output.as_ref(), max_output_bytes, &is_cancelled)
    }

    pub fn wav_to_mp3(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeMediaError> {
        if max_output_bytes == 0 {
            return Err(NativeMediaError::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        if fs::metadata(&input)?.len() > MAX_NATIVE_AUDIO_BYTES {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let mut reader =
            hound::WavReader::open(&input).map_err(|_| NativeMediaError::InvalidInput)?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int
            || spec.bits_per_sample != 16
            || !(1..=2).contains(&spec.channels)
            || !matches!(spec.sample_rate, 32_000 | 44_100 | 48_000)
        {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let channels =
            u8::try_from(spec.channels).map_err(|_| NativeMediaError::UnsupportedInput)?;
        let mut samples = Vec::new();
        for sample in reader.samples::<i16>() {
            if is_cancelled() {
                return Err(NativeMediaError::Cancelled);
            }
            if samples.len() >= MAX_NATIVE_AUDIO_SAMPLES {
                return Err(NativeMediaError::UnsupportedInput);
            }
            samples.push(sample.map_err(|_| NativeMediaError::InvalidInput)?);
        }
        encode_mp3_samples(
            &samples,
            spec.sample_rate,
            channels,
            output.as_ref(),
            max_output_bytes,
            &is_cancelled,
        )
    }

    pub fn audio_to_mp3(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeMediaError> {
        if max_output_bytes == 0 {
            return Err(NativeMediaError::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        if fs::metadata(&input)?.len() > MAX_NATIVE_AUDIO_BYTES {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let file = File::open(&input)?;
        let mut hint = Hint::new();
        if let Some(extension) = input.extension().and_then(|value| value.to_str()) {
            hint.with_extension(extension);
        }
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|_| NativeMediaError::UnsupportedInput)?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(NativeMediaError::UnsupportedInput)?;
        let track_id = track.id;
        let codec_params = track
            .codec_params
            .as_ref()
            .ok_or(NativeMediaError::UnsupportedInput)?;
        let audio_params = codec_params
            .audio()
            .ok_or(NativeMediaError::UnsupportedInput)?;
        let mut decoder = symphonia::default::get_codecs()
            .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
            .map_err(|_| NativeMediaError::UnsupportedInput)?;

        let mut samples = Vec::new();
        let mut stream_spec = None;
        loop {
            if is_cancelled() {
                return Err(NativeMediaError::Cancelled);
            }
            let packet = match format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SymphoniaError::ResetRequired) => {
                    return Err(NativeMediaError::UnsupportedInput);
                }
                Err(_) => return Err(NativeMediaError::InvalidInput),
            };
            if packet.track_id != track_id {
                continue;
            }
            let decoded = decoder.decode(&packet).map_err(|error| match error {
                SymphoniaError::IoError(_) | SymphoniaError::DecodeError(_) => {
                    NativeMediaError::InvalidInput
                }
                _ => NativeMediaError::UnsupportedInput,
            })?;
            let decoded_spec = decoded.spec();
            let channels = decoded_spec.channels().count();
            if !(1..=2).contains(&channels)
                || !matches!(decoded_spec.rate(), 32_000 | 44_100 | 48_000)
            {
                return Err(NativeMediaError::UnsupportedInput);
            }
            if stream_spec
                .is_some_and(|(rate, count)| rate != decoded_spec.rate() || count != channels)
            {
                return Err(NativeMediaError::UnsupportedInput);
            }
            stream_spec = Some((decoded_spec.rate(), channels));

            let decoded_sample_count = decoded.samples_interleaved();
            let new_len = samples
                .len()
                .checked_add(decoded_sample_count)
                .ok_or(NativeMediaError::UnsupportedInput)?;
            if new_len > MAX_NATIVE_AUDIO_SAMPLES {
                return Err(NativeMediaError::UnsupportedInput);
            }
            let mut packet_samples = Vec::<i16>::new();
            decoded.copy_to_vec_interleaved(&mut packet_samples);
            samples.extend(packet_samples);
        }
        let (sample_rate, channels) = stream_spec.ok_or(NativeMediaError::InvalidInput)?;
        encode_mp3_samples(
            &samples,
            sample_rate,
            u8::try_from(channels).map_err(|_| NativeMediaError::UnsupportedInput)?,
            output.as_ref(),
            max_output_bytes,
            &is_cancelled,
        )
    }

    pub fn srt_to_webvtt(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeMediaError> {
        if max_output_bytes == 0 {
            return Err(NativeMediaError::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        let file = File::open(&input)?;
        if file.metadata()?.len() > MAX_NATIVE_SUBTITLE_BYTES {
            return Err(NativeMediaError::UnsupportedInput);
        }
        let mut bytes = Vec::new();
        file.take(MAX_NATIVE_SUBTITLE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_NATIVE_SUBTITLE_BYTES {
            return Err(NativeMediaError::UnsupportedInput);
        }
        if is_cancelled() {
            return Err(NativeMediaError::Cancelled);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| NativeMediaError::InvalidInput)?;
        let converted = convert_srt_to_webvtt(text, &is_cancelled)?;
        publish_bytes(
            converted.as_bytes(),
            output.as_ref(),
            max_output_bytes,
            &is_cancelled,
        )
    }
}

fn encode_mp3_samples(
    samples: &[i16],
    sample_rate: u32,
    channels: u8,
    output: &Path,
    max_output_bytes: u64,
    is_cancelled: &impl Fn() -> bool,
) -> Result<u64, NativeMediaError> {
    if !(1..=2).contains(&channels) || !matches!(sample_rate, 32_000 | 44_100 | 48_000) {
        return Err(NativeMediaError::UnsupportedInput);
    }
    let channel_count = usize::from(channels);
    if samples.is_empty() || !samples.len().is_multiple_of(channel_count) {
        return Err(NativeMediaError::InvalidInput);
    }
    let mut encoder = Mp3Encoder::new(EncoderConfig {
        sample_rate,
        channels,
        bitrate: Bitrate::Kbps192,
    })
    .map_err(|error| NativeMediaError::Encoding(error.to_string()))?;
    encoder
        .enable_info_frame()
        .map_err(|error| NativeMediaError::Encoding(error.to_string()))?;

    let samples_per_chunk = 4096 * channel_count;
    let mut output_bytes = Vec::new();
    for chunk in samples.chunks(samples_per_chunk) {
        if is_cancelled() {
            return Err(NativeMediaError::Cancelled);
        }
        encode_mp3_chunk(&mut encoder, chunk, &mut output_bytes, max_output_bytes)?;
    }
    if is_cancelled() {
        return Err(NativeMediaError::Cancelled);
    }
    let mut flush_buffer = vec![0; encoder.max_flush_size()];
    let flushed = encoder
        .flush_into(&mut flush_buffer)
        .map_err(|error| NativeMediaError::Encoding(error.to_string()))?;
    append_bounded(
        &mut output_bytes,
        &flush_buffer[..flushed],
        max_output_bytes,
    )?;
    let info_frame_size = encoder.info_frame_size();
    if info_frame_size > output_bytes.len() {
        return Err(NativeMediaError::Encoding(
            "MP3 encoder produced an incomplete info frame".to_owned(),
        ));
    }
    encoder
        .info_frame_into(&mut output_bytes[..info_frame_size])
        .map_err(|error| NativeMediaError::Encoding(error.to_string()))?;
    publish_bytes(&output_bytes, output, max_output_bytes, is_cancelled)
}

fn encode_mp3_chunk(
    encoder: &mut Mp3Encoder,
    samples: &[i16],
    output: &mut Vec<u8>,
    maximum: u64,
) -> Result<(), NativeMediaError> {
    let mut encoded = vec![0; encoder.max_output_size(samples.len())];
    let written = encoder
        .encode_into(samples, &mut encoded)
        .map_err(|error| NativeMediaError::Encoding(error.to_string()))?;
    append_bounded(output, &encoded[..written], maximum)
}

fn append_bounded(
    output: &mut Vec<u8>,
    encoded: &[u8],
    maximum: u64,
) -> Result<(), NativeMediaError> {
    let size = u64::try_from(output.len())
        .unwrap_or(u64::MAX)
        .checked_add(u64::try_from(encoded.len()).unwrap_or(u64::MAX))
        .ok_or(NativeMediaError::OutputTooLarge)?;
    if size > maximum {
        return Err(NativeMediaError::OutputTooLarge);
    }
    output.extend_from_slice(encoded);
    Ok(())
}

struct CancelableSource<'a> {
    source: MemSource,
    is_cancelled: &'a dyn Fn() -> bool,
}

impl Source for CancelableSource<'_> {
    fn channels(&self) -> usize {
        self.source.channels()
    }

    fn bits_per_sample(&self) -> usize {
        self.source.bits_per_sample()
    }

    fn sample_rate(&self) -> usize {
        self.source.sample_rate()
    }

    fn read_samples<F: flacenc::source::Fill>(
        &mut self,
        block_size: usize,
        destination: &mut F,
    ) -> Result<usize, SourceError> {
        if (self.is_cancelled)() {
            return Err(SourceError::by_reason(SourceErrorReason::IO(None)));
        }
        self.source.read_samples(block_size, destination)
    }

    fn len_hint(&self) -> Option<usize> {
        self.source.len_hint()
    }
}

fn convert_srt_to_webvtt(
    text: &str,
    is_cancelled: &impl Fn() -> bool,
) -> Result<String, NativeMediaError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = text.lines().collect::<Vec<_>>();
    let mut output = String::from("WEBVTT\n\n");
    let mut index = 0;
    let mut cue_count = 0;
    while index < lines.len() {
        if is_cancelled() {
            return Err(NativeMediaError::Cancelled);
        }
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let mut cue_id = None;
        let timing = if lines[index].contains("-->") {
            lines[index]
        } else {
            cue_id = Some(lines[index]);
            index += 1;
            lines
                .get(index)
                .filter(|line| line.contains("-->"))
                .copied()
                .ok_or(NativeMediaError::InvalidInput)?
        };
        if let Some(cue_id) = cue_id {
            output.push_str(cue_id);
            output.push('\n');
        }
        output.push_str(&normalize_timing_line(timing)?);
        output.push('\n');
        index += 1;
        let mut has_text = false;
        while index < lines.len() && !lines[index].trim().is_empty() {
            if is_cancelled() {
                return Err(NativeMediaError::Cancelled);
            }
            output.push_str(lines[index]);
            output.push('\n');
            has_text = true;
            index += 1;
        }
        if !has_text {
            return Err(NativeMediaError::InvalidInput);
        }
        output.push('\n');
        cue_count += 1;
    }
    if cue_count == 0 {
        return Err(NativeMediaError::InvalidInput);
    }
    Ok(output)
}

fn normalize_timing_line(line: &str) -> Result<String, NativeMediaError> {
    let (start, end) = line
        .split_once("-->")
        .ok_or(NativeMediaError::InvalidInput)?;
    if end.contains("-->") {
        return Err(NativeMediaError::InvalidInput);
    }
    let (start, start_milliseconds) = normalize_timestamp(start.trim())?;
    let end = end.trim_start();
    let end_length = end.find(char::is_whitespace).unwrap_or(end.len());
    let (normalized_end, end_milliseconds) = normalize_timestamp(&end[..end_length])?;
    if end_milliseconds <= start_milliseconds {
        return Err(NativeMediaError::InvalidInput);
    }
    Ok(format!(
        "{start} --> {normalized_end}{}",
        &end[end_length..]
    ))
}

fn normalize_timestamp(value: &str) -> Result<(String, u64), NativeMediaError> {
    let (clock, milliseconds) = value
        .split_once([',', '.'])
        .ok_or(NativeMediaError::InvalidInput)?;
    if milliseconds.len() != 3 || !milliseconds.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NativeMediaError::InvalidInput);
    }
    let parts = clock
        .split(':')
        .map(|part| part.parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| NativeMediaError::InvalidInput)?;
    let seconds = match parts.as_slice() {
        [minutes, seconds] if *minutes <= 59 && *seconds <= 59 => minutes
            .checked_mul(60)
            .and_then(|total| total.checked_add(*seconds)),
        [hours, minutes, seconds] if *minutes <= 59 && *seconds <= 59 => hours
            .checked_mul(60)
            .and_then(|total| total.checked_add(*minutes))
            .and_then(|total| total.checked_mul(60))
            .and_then(|total| total.checked_add(*seconds)),
        _ => None,
    };
    let total_milliseconds = seconds
        .and_then(|total| total.checked_mul(1000))
        .and_then(|total| total.checked_add(milliseconds.parse::<u64>().ok()?))
        .ok_or(NativeMediaError::InvalidInput)?;
    Ok((format!("{clock}.{milliseconds}"), total_milliseconds))
}

fn publish_bytes(
    bytes: &[u8],
    output: &Path,
    max_output_bytes: u64,
    is_cancelled: &impl Fn() -> bool,
) -> Result<u64, NativeMediaError> {
    if is_cancelled() {
        return Err(NativeMediaError::Cancelled);
    }
    let (destination, temporary) = prepare_output(output)?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let _temporary_guard = TemporaryOutput(temporary.clone());
    let mut writer = BoundedWriter::new(file, max_output_bytes, Arc::clone(&exceeded));
    writer.write_all(bytes).map_err(|error| {
        if exceeded.load(Ordering::Relaxed) {
            NativeMediaError::OutputTooLarge
        } else {
            NativeMediaError::Io(error)
        }
    })?;
    writer.flush()?;
    writer.file.sync_all()?;
    let size = writer.file.metadata()?.len();
    drop(writer);
    if is_cancelled() {
        return Err(NativeMediaError::Cancelled);
    }
    match fs::hard_link(&temporary, &destination) {
        Ok(()) => Ok(size),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(NativeMediaError::DestinationExists)
        }
        Err(error) => Err(NativeMediaError::Io(error)),
    }
}

fn canonical_input(path: &Path) -> Result<PathBuf, NativeMediaError> {
    let canonical = fs::canonicalize(path).map_err(|_| NativeMediaError::InvalidInput)?;
    if !fs::metadata(&canonical)?.is_file() {
        return Err(NativeMediaError::InvalidInput);
    }
    Ok(canonical)
}

fn prepare_output(output: &Path) -> Result<(PathBuf, PathBuf), NativeMediaError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|_| NativeMediaError::InvalidOutput)?;
    if !fs::metadata(&parent)?.is_dir() {
        return Err(NativeMediaError::InvalidOutput);
    }
    let name = output.file_name().ok_or(NativeMediaError::InvalidOutput)?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(NativeMediaError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(NativeMediaError::Io(error)),
    }
    let temporary = parent.join(format!(".fetchx-native-media-{}.tmp", Uuid::new_v4()));
    Ok((destination, temporary))
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::Path};

    use tempfile::tempdir;

    use super::{NativeMediaError, NativeMediaProcessor};

    fn write_pcm_wav(path: &Path) -> Result<(), Box<dyn Error>> {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        for sample in [-16_000_i16, 16_000, -8_000, 8_000, 0, 0] {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        Ok(())
    }

    fn write_mp3_wav(path: &Path) -> Result<(), Box<dyn Error>> {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        for frame in 0..4_410_i16 {
            writer.write_sample(frame.wrapping_mul(3))?;
            writer.write_sample(frame.wrapping_mul(-2))?;
        }
        writer.finalize()?;
        Ok(())
    }

    #[test]
    fn converts_pcm_wav_to_bounded_flac() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("input.wav");
        let output = directory.path().join("output.flac");
        write_pcm_wav(&input)?;

        let size = NativeMediaProcessor::wav_to_flac(&input, &output, 1024 * 1024, || false)?;
        let bytes = fs::read(&output)?;
        assert_eq!(size, u64::try_from(bytes.len())?);
        assert!(bytes.starts_with(b"fLaC"));
        Ok(())
    }

    #[test]
    fn converts_pcm_wav_to_mp3_with_valid_frame_sync() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("input.wav");
        let output = directory.path().join("output.mp3");
        write_mp3_wav(&input)?;

        let size = NativeMediaProcessor::wav_to_mp3(&input, &output, 1024 * 1024, || false)?;
        let bytes = fs::read(&output)?;
        assert_eq!(size, u64::try_from(bytes.len())?);
        assert!(bytes.len() > 128);
        assert_eq!(bytes[0], 0xff);
        assert_eq!(bytes[1] & 0xe0, 0xe0);
        Ok(())
    }

    #[test]
    fn decodes_wav_flac_and_mp3_sources_to_mp3() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let wav = directory.path().join("input.wav");
        let flac = directory.path().join("input.flac");
        let mp3 = directory.path().join("input.mp3");
        write_mp3_wav(&wav)?;
        NativeMediaProcessor::wav_to_flac(&wav, &flac, 1024 * 1024, || false)?;
        NativeMediaProcessor::wav_to_mp3(&wav, &mp3, 1024 * 1024, || false)?;

        for (input, output_name) in [
            (&wav, "wav.converted.mp3"),
            (&flac, "flac.converted.mp3"),
            (&mp3, "mp3.converted.mp3"),
        ] {
            let output = directory.path().join(output_name);
            NativeMediaProcessor::audio_to_mp3(input, &output, 1024 * 1024, || false)?;
            let bytes = fs::read(output)?;
            assert!(bytes.len() > 128);
            assert_eq!(bytes[0], 0xff);
            assert_eq!(bytes[1] & 0xe0, 0xe0);
        }
        Ok(())
    }

    #[test]
    fn mp3_conversion_respects_output_limit_and_cancellation() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("input.wav");
        write_mp3_wav(&input)?;

        let too_small = directory.path().join("too-small.mp3");
        assert!(matches!(
            NativeMediaProcessor::wav_to_mp3(&input, &too_small, 16, || false),
            Err(NativeMediaError::OutputTooLarge)
        ));
        assert!(!too_small.exists());

        let cancelled = directory.path().join("cancelled.mp3");
        assert!(matches!(
            NativeMediaProcessor::wav_to_mp3(&input, &cancelled, 1024 * 1024, || true),
            Err(NativeMediaError::Cancelled)
        ));
        assert!(!cancelled.exists());
        Ok(())
    }

    #[test]
    fn converts_srt_timestamps_and_preserves_cue_text() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("input.srt");
        let output = directory.path().join("output.vtt");
        fs::write(
            &input,
            "1\r\n00:00:01,250 --> 00:00:02,500\r\nHello, world\r\n",
        )?;

        NativeMediaProcessor::srt_to_webvtt(&input, &output, 1024, || false)?;
        assert_eq!(
            fs::read_to_string(output)?,
            "WEBVTT\n\n1\n00:00:01.250 --> 00:00:02.500\nHello, world\n\n"
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_srt_without_publishing_output() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("invalid.srt");
        let output = directory.path().join("invalid.vtt");
        fs::write(&input, "1\n00:00:02,000 --> 00:00:01,000\nreversed\n")?;

        assert!(matches!(
            NativeMediaProcessor::srt_to_webvtt(&input, &output, 1024, || false),
            Err(NativeMediaError::InvalidInput)
        ));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn cancellation_does_not_publish_audio_output() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("input.wav");
        let output = directory.path().join("output.flac");
        write_pcm_wav(&input)?;

        assert!(matches!(
            NativeMediaProcessor::wav_to_flac(&input, &output, 1024 * 1024, || true),
            Err(NativeMediaError::Cancelled)
        ));
        assert!(!output.exists());
        Ok(())
    }
}

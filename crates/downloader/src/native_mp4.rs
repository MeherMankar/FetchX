//! Native MP4 metadata and stream-copy operations.
//!
//! The user-facing stream-copy behavior is informed by the FFmpeg project
//! (https://ffmpeg.org). This is an original FetchX implementation using the
//! Rust `mp4` crate; it does not copy FFmpeg source code.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use mp4::{
    AacConfig, AvcConfig, ChannelConfig, FourCC, MediaType, Mp4Config, Mp4Reader, Mp4Track,
    Mp4Writer, TrackConfig, TrackType,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum NativeMp4Error {
    #[error("MP4 input is invalid or unsupported")]
    InvalidInput,
    #[error("native MP4 operations currently support H.264 video and AAC audio")]
    UnsupportedCodec,
    #[error("MP4 does not contain the requested video or audio track")]
    RequiredTrackMissing,
    #[error("native MP4 merge does not support mismatched stream durations")]
    UnsupportedTiming,
    #[error("MP4 output path is invalid")]
    InvalidOutput,
    #[error("MP4 output destination already exists")]
    DestinationExists,
    #[error("MP4 output exceeded the configured size limit")]
    OutputTooLarge,
    #[error("native MP4 operation was cancelled")]
    Cancelled,
    #[error("native MP4 operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("MP4 container operation failed: {0}")]
    Container(#[from] mp4::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeMp4Track {
    pub id: u32,
    pub media_type: String,
    pub codec: Option<String>,
    pub duration: Duration,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeMp4Info {
    pub duration: Duration,
    pub size_bytes: u64,
    pub fragmented: bool,
    pub tracks: Vec<NativeMp4Track>,
}

#[derive(Clone, Copy)]
struct SelectedTrack {
    id: u32,
    output_id: u32,
    sample_count: u32,
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
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let current = self.file.stream_position()?;
        let end = current
            .checked_add(buffer.len() as u64)
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        if end > self.maximum {
            self.exceeded.store(true, Ordering::Relaxed);
            return Err(io::Error::other("output size limit exceeded"));
        }
        self.file.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for BoundedWriter {
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
            tracing::warn!(%error, path = %self.0.display(), "native MP4 temporary file cleanup failed");
        }
    }
}

pub struct NativeMp4Processor;

impl NativeMp4Processor {
    pub fn probe(input: impl AsRef<Path>) -> Result<NativeMp4Info, NativeMp4Error> {
        Self::probe_with_cancel(input.as_ref(), || false)
    }

    pub(crate) fn probe_with_cancel(
        input: &Path,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<NativeMp4Info, NativeMp4Error> {
        let input = canonical_input(input)?;
        let size_bytes = fs::metadata(&input)?.len();
        let reader = open_reader(&input)?;
        let mut tracks = Vec::with_capacity(reader.tracks().len());
        for track in reader.tracks().values() {
            if is_cancelled() {
                return Err(NativeMp4Error::Cancelled);
            }
            let media_type = track
                .track_type()
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?;
            let codec = match track.media_type() {
                Ok(MediaType::H264) => Some("h264"),
                Ok(MediaType::AAC) => Some("aac"),
                Ok(MediaType::H265) => Some("hevc"),
                Ok(MediaType::VP9) => Some("vp9"),
                Ok(MediaType::TTXT) => Some("tx3g"),
                Err(_) => None,
            };
            let (sample_rate_hz, channels) = if media_type == TrackType::Audio
                && matches!(track.media_type(), Ok(MediaType::AAC))
            {
                (
                    Some(
                        track
                            .sample_freq_index()
                            .map_err(|_| NativeMp4Error::InvalidInput)?
                            .freq(),
                    ),
                    Some(channel_count(
                        track
                            .channel_config()
                            .map_err(|_| NativeMp4Error::InvalidInput)?,
                    )),
                )
            } else {
                (None, None)
            };
            tracks.push(NativeMp4Track {
                id: track.track_id(),
                media_type: match media_type {
                    TrackType::Video => "video",
                    TrackType::Audio => "audio",
                    TrackType::Subtitle => "subtitle",
                }
                .to_owned(),
                codec: codec.map(str::to_owned),
                duration: track.duration(),
                width: (media_type == TrackType::Video).then(|| track.width()),
                height: (media_type == TrackType::Video).then(|| track.height()),
                sample_rate_hz,
                channels,
            });
        }
        tracks.sort_by_key(|track| track.id);
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        Ok(NativeMp4Info {
            duration: reader.duration(),
            size_bytes,
            fragmented: reader.is_fragmented(),
            tracks,
        })
    }

    pub fn remux(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<NativeMp4Info, NativeMp4Error> {
        if max_output_bytes == 0 {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        let (destination, temporary) = prepare_output(output.as_ref())?;
        let _temporary_guard = TemporaryOutput(temporary.clone());
        let mut reader = open_reader(&input)?;
        let mut configs = Vec::with_capacity(reader.tracks().len());
        let mut selected = Vec::with_capacity(reader.tracks().len());
        let mut track_ids = reader.tracks().keys().copied().collect::<Vec<_>>();
        track_ids.sort_unstable();
        for (index, id) in track_ids.iter().copied().enumerate() {
            let track = reader
                .tracks()
                .get(&id)
                .ok_or(NativeMp4Error::InvalidInput)?;
            configs.push(track_config(track)?);
            selected.push(SelectedTrack {
                id,
                output_id: u32::try_from(index + 1).map_err(|_| NativeMp4Error::InvalidInput)?,
                sample_count: reader.sample_count(id)?,
            });
        }
        if selected.is_empty() {
            return Err(NativeMp4Error::InvalidInput);
        }
        let info = write_output(
            &mut reader,
            &selected,
            &configs,
            &temporary,
            max_output_bytes,
            &is_cancelled,
        )?;
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        publish_output(&temporary, &destination)?;
        Ok(info)
    }

    pub fn merge_h264_aac(
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<NativeMp4Info, NativeMp4Error> {
        if max_output_bytes == 0 {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let video_path = canonical_input(video_input.as_ref())?;
        let audio_path = canonical_input(audio_input.as_ref())?;
        let (destination, temporary) = prepare_output(output.as_ref())?;
        if video_path == destination || audio_path == destination {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let _temporary_guard = TemporaryOutput(temporary.clone());
        let mut video_reader = open_reader(&video_path)?;
        let mut audio_reader = open_reader(&audio_path)?;
        let (video_id, video_track) = first_track(&video_reader, TrackType::Video, MediaType::H264)
            .ok_or(NativeMp4Error::RequiredTrackMissing)?;
        let (audio_id, audio_track) = first_track(&audio_reader, TrackType::Audio, MediaType::AAC)
            .ok_or(NativeMp4Error::RequiredTrackMissing)?;
        if video_track.duration() != audio_track.duration() {
            return Err(NativeMp4Error::UnsupportedTiming);
        }
        let configs = vec![track_config(video_track)?, track_config(audio_track)?];
        let selected = [
            SelectedTrack {
                id: video_id,
                output_id: 1,
                sample_count: video_reader.sample_count(video_id)?,
            },
            SelectedTrack {
                id: audio_id,
                output_id: 2,
                sample_count: audio_reader.sample_count(audio_id)?,
            },
        ];
        let exceeded = Arc::new(AtomicBool::new(false));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let writer = BoundedWriter::new(file, max_output_bytes, Arc::clone(&exceeded));
        let config = mp4_config();
        let mut writer = Mp4Writer::write_start(writer, &config).map_err(|error| {
            if exceeded.load(Ordering::Relaxed) {
                NativeMp4Error::OutputTooLarge
            } else {
                NativeMp4Error::Container(error)
            }
        })?;
        for track in &configs {
            writer.add_track(track).map_err(|error| {
                if exceeded.load(Ordering::Relaxed) {
                    NativeMp4Error::OutputTooLarge
                } else {
                    NativeMp4Error::Container(error)
                }
            })?;
        }
        copy_samples(
            &mut video_reader,
            &mut writer,
            selected[0],
            &is_cancelled,
            &exceeded,
        )?;
        copy_samples(
            &mut audio_reader,
            &mut writer,
            selected[1],
            &is_cancelled,
            &exceeded,
        )?;
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        writer.write_end().map_err(|error| {
            if exceeded.load(Ordering::Relaxed) {
                NativeMp4Error::OutputTooLarge
            } else {
                NativeMp4Error::Container(error)
            }
        })?;
        let writer = writer.into_writer();
        writer.file.sync_all()?;
        drop(writer);
        let info = Self::probe_with_cancel(&temporary, &is_cancelled)?;
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        publish_output(&temporary, &destination)?;
        Ok(info)
    }
}

fn open_reader(path: &Path) -> Result<Mp4Reader<File>, NativeMp4Error> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    Mp4Reader::read_header(file, size).map_err(|_| NativeMp4Error::InvalidInput)
}

fn canonical_input(path: &Path) -> Result<PathBuf, NativeMp4Error> {
    let canonical = fs::canonicalize(path).map_err(|_| NativeMp4Error::InvalidInput)?;
    if !fs::metadata(&canonical)?.is_file() {
        return Err(NativeMp4Error::InvalidInput);
    }
    Ok(canonical)
}

fn prepare_output(output: &Path) -> Result<(PathBuf, PathBuf), NativeMp4Error> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|_| NativeMp4Error::InvalidOutput)?;
    if !fs::metadata(&parent)?.is_dir() {
        return Err(NativeMp4Error::InvalidOutput);
    }
    let name = output.file_name().ok_or(NativeMp4Error::InvalidOutput)?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(NativeMp4Error::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(NativeMp4Error::Io(error)),
    }
    let temporary = parent.join(format!(".fetchx-native-mp4-{}.tmp", Uuid::new_v4()));
    Ok((destination, temporary))
}

fn publish_output(temporary: &Path, destination: &Path) -> Result<(), NativeMp4Error> {
    match fs::hard_link(temporary, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(NativeMp4Error::DestinationExists)
        }
        Err(error) => Err(NativeMp4Error::Io(error)),
    }
}

fn track_config(track: &Mp4Track) -> Result<TrackConfig, NativeMp4Error> {
    let mut config = match track
        .media_type()
        .map_err(|_| NativeMp4Error::UnsupportedCodec)?
    {
        MediaType::H264 => TrackConfig::from(AvcConfig {
            width: track.width(),
            height: track.height(),
            seq_param_set: track
                .sequence_parameter_set()
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?
                .to_vec(),
            pic_param_set: track
                .picture_parameter_set()
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?
                .to_vec(),
        }),
        MediaType::AAC => TrackConfig::from(AacConfig {
            bitrate: track.bitrate(),
            profile: track
                .audio_profile()
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?,
            freq_index: track
                .sample_freq_index()
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?,
            chan_conf: track
                .channel_config()
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?,
        }),
        _ => return Err(NativeMp4Error::UnsupportedCodec),
    };
    config.timescale = track.timescale();
    config.language = track.language().to_owned();
    Ok(config)
}

fn first_track(
    reader: &Mp4Reader<File>,
    expected_type: TrackType,
    expected_media: MediaType,
) -> Option<(u32, &Mp4Track)> {
    let mut ids = reader.tracks().keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    ids.into_iter().find_map(|id| {
        let track = reader.tracks().get(&id)?;
        if matches!(track.track_type(), Ok(track_type) if track_type == expected_type)
            && matches!(track.media_type(), Ok(media_type) if media_type == expected_media)
        {
            Some((id, track))
        } else {
            None
        }
    })
}

fn write_output(
    reader: &mut Mp4Reader<File>,
    selected: &[SelectedTrack],
    configs: &[TrackConfig],
    temporary: &Path,
    max_output_bytes: u64,
    is_cancelled: &impl Fn() -> bool,
) -> Result<NativeMp4Info, NativeMp4Error> {
    let exceeded = Arc::new(AtomicBool::new(false));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    let writer = BoundedWriter::new(file, max_output_bytes, Arc::clone(&exceeded));
    let mut writer = Mp4Writer::write_start(writer, &mp4_config()).map_err(|error| {
        if exceeded.load(Ordering::Relaxed) {
            NativeMp4Error::OutputTooLarge
        } else {
            NativeMp4Error::Container(error)
        }
    })?;
    for config in configs {
        writer.add_track(config).map_err(|error| {
            if exceeded.load(Ordering::Relaxed) {
                NativeMp4Error::OutputTooLarge
            } else {
                NativeMp4Error::Container(error)
            }
        })?;
    }
    for track in selected {
        copy_samples(reader, &mut writer, *track, is_cancelled, &exceeded)?;
    }
    if is_cancelled() {
        return Err(NativeMp4Error::Cancelled);
    }
    writer.write_end().map_err(|error| {
        if exceeded.load(Ordering::Relaxed) {
            NativeMp4Error::OutputTooLarge
        } else {
            NativeMp4Error::Container(error)
        }
    })?;
    let writer = writer.into_writer();
    writer.file.sync_all()?;
    drop(writer);
    let info = NativeMp4Processor::probe_with_cancel(temporary, is_cancelled)?;
    Ok(info)
}

fn copy_samples(
    reader: &mut Mp4Reader<File>,
    writer: &mut Mp4Writer<BoundedWriter>,
    selected: SelectedTrack,
    is_cancelled: &impl Fn() -> bool,
    exceeded: &AtomicBool,
) -> Result<(), NativeMp4Error> {
    for sample_id in 1..=selected.sample_count {
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        let sample = reader
            .read_sample(selected.id, sample_id)?
            .ok_or(NativeMp4Error::InvalidInput)?;
        writer
            .write_sample(selected.output_id, &sample)
            .map_err(|error| {
                if exceeded.load(Ordering::Relaxed) {
                    NativeMp4Error::OutputTooLarge
                } else {
                    NativeMp4Error::Container(error)
                }
            })?;
    }
    Ok(())
}

fn mp4_config() -> Mp4Config {
    Mp4Config {
        major_brand: FourCC::from(*b"isom"),
        minor_version: 512,
        compatible_brands: vec![
            FourCC::from(*b"isom"),
            FourCC::from(*b"iso2"),
            FourCC::from(*b"avc1"),
            FourCC::from(*b"mp41"),
        ],
        timescale: 1000,
    }
}

fn channel_count(config: ChannelConfig) -> u16 {
    match config {
        ChannelConfig::Mono => 1,
        ChannelConfig::Stereo => 2,
        ChannelConfig::Three => 3,
        ChannelConfig::Four => 4,
        ChannelConfig::Five => 5,
        ChannelConfig::FiveOne => 6,
        ChannelConfig::SevenOne => 8,
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs::File, path::Path};

    use mp4::{
        AacConfig, AudioObjectType, AvcConfig, ChannelConfig, FourCC, Mp4Config, Mp4Sample,
        Mp4Writer, SampleFreqIndex, TrackConfig,
    };
    use tempfile::tempdir;

    use super::{NativeMp4Error, NativeMp4Processor};

    fn make_mp4(
        path: &Path,
        track_config: TrackConfig,
        duration: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut writer = Mp4Writer::write_start(
            File::create(path)?,
            &Mp4Config {
                major_brand: FourCC::from(*b"isom"),
                minor_version: 512,
                compatible_brands: vec![FourCC::from(*b"isom"), FourCC::from(*b"mp41")],
                timescale: 1000,
            },
        )?;
        writer.add_track(&track_config)?;
        writer.write_sample(
            1,
            &Mp4Sample {
                start_time: 0,
                duration,
                rendering_offset: 0,
                is_sync: true,
                bytes: vec![0, 0, 0, 1, 0x65].into(),
            },
        )?;
        writer.write_end()?;
        Ok(())
    }

    fn video_config() -> TrackConfig {
        TrackConfig::from(AvcConfig {
            width: 640,
            height: 360,
            seq_param_set: vec![0x67, 0x42, 0, 0x1e],
            pic_param_set: vec![0x68, 0xce, 6, 0xe2],
        })
    }

    fn audio_config() -> TrackConfig {
        TrackConfig::from(AacConfig {
            bitrate: 128_000,
            profile: AudioObjectType::AacLowComplexity,
            freq_index: SampleFreqIndex::Freq44100,
            chan_conf: ChannelConfig::Stereo,
        })
    }

    #[test]
    fn probes_mp4_track_metadata() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video.mp4");
        make_mp4(&input, video_config(), 1000)?;

        let info = NativeMp4Processor::probe(&input)?;
        assert_eq!(info.tracks.len(), 1);
        assert_eq!(info.tracks[0].codec.as_deref(), Some("h264"));
        assert_eq!(info.tracks[0].width, Some(640));
        assert_eq!(info.tracks[0].height, Some(360));
        Ok(())
    }

    #[test]
    fn remuxes_supported_mp4_without_overwriting() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video.mp4");
        let output = directory.path().join("remuxed.mp4");
        make_mp4(&input, video_config(), 1000)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks.len(), 1);
        assert_eq!(info.tracks[0].codec.as_deref(), Some("h264"));
        assert!(matches!(
            NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false),
            Err(NativeMp4Error::DestinationExists)
        ));
        Ok(())
    }

    #[test]
    fn enforces_output_limit_without_publishing_partial_file() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video.mp4");
        let output = directory.path().join("too-large.mp4");
        make_mp4(&input, video_config(), 1000)?;

        assert!(matches!(
            NativeMp4Processor::remux(&input, &output, 8, || false),
            Err(NativeMp4Error::OutputTooLarge)
        ));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn merges_h264_video_and_aac_audio_as_stream_copy() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let video = directory.path().join("video.mp4");
        let audio = directory.path().join("audio.mp4");
        let output = directory.path().join("merged.mp4");
        make_mp4(&video, video_config(), 1000)?;
        make_mp4(&audio, audio_config(), 1000)?;

        let info =
            NativeMp4Processor::merge_h264_aac(&video, &audio, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks.len(), 2);
        assert_eq!(info.tracks[0].codec.as_deref(), Some("h264"));
        assert_eq!(info.tracks[1].codec.as_deref(), Some("aac"));
        Ok(())
    }

    #[test]
    fn rejects_native_merge_when_stream_durations_differ() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let video = directory.path().join("video.mp4");
        let audio = directory.path().join("audio.mp4");
        let output = directory.path().join("merged.mp4");
        make_mp4(&video, video_config(), 1000)?;
        make_mp4(&audio, audio_config(), 500)?;

        assert!(matches!(
            NativeMp4Processor::merge_h264_aac(&video, &audio, &output, 1024 * 1024, || false),
            Err(NativeMp4Error::UnsupportedTiming)
        ));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn rejects_output_when_cancellation_is_requested() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video.mp4");
        let output = directory.path().join("cancelled.mp4");
        make_mp4(&input, video_config(), 1000)?;

        assert!(matches!(
            NativeMp4Processor::remux(&input, &output, 1024 * 1024, || true),
            Err(NativeMp4Error::Cancelled)
        ));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn reports_audio_metadata() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("audio.mp4");
        make_mp4(&input, audio_config(), 1000)?;

        let info = NativeMp4Processor::probe(&input)?;
        assert_eq!(info.tracks[0].sample_rate_hz, Some(44_100));
        assert_eq!(info.tracks[0].channels, Some(2));
        Ok(())
    }
}

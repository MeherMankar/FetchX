//! Native MP4 metadata and stream-copy operations.
//!
//! The user-facing stream-copy behavior is informed by the FFmpeg project
//! (https://ffmpeg.org). This is an original FetchX implementation using the
//! Rust MP4 crates; it does not copy FFmpeg source code.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ebml_webm::{MuxError as WebmMuxError, Muxer as WebmMuxer, types::TrackInfo as WebmTrackInfo};
use mp4::{
    AacConfig, AvcConfig, ChannelConfig, FourCC, MediaType, Mp4Config, Mp4Reader, Mp4Track,
    Mp4Writer, TrackConfig, TrackType,
};
use shiguredo_mp4::{
    TrackKind,
    boxes::SampleEntry,
    demux::{Input, Mp4FileDemuxer, TrackInfo},
    mux::{Mp4FileMuxer, Sample as MuxSample},
};
use thiserror::Error;
use uuid::Uuid;

const MAX_EXTENDED_MP4_HEADER_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXTENDED_MP4_TRACKS: usize = 128;
const MAX_EXTENDED_MP4_SAMPLES: usize = 1_000_000;
const MAX_WEBM_SAMPLE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum NativeMp4Error {
    #[error("MP4 input is invalid or unsupported")]
    InvalidInput,
    #[error("native MP4 operation does not support this codec or track layout")]
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
        let needs_extended_codecs = reader
            .tracks()
            .values()
            .any(|track| track.media_type().is_err());
        if needs_extended_codecs
            && reader
                .tracks()
                .values()
                .any(|track| !matches!(track.track_type(), Ok(TrackType::Video | TrackType::Audio)))
        {
            return Err(NativeMp4Error::UnsupportedCodec);
        }
        let extended_codecs = if needs_extended_codecs {
            let media_track_count = reader
                .tracks()
                .values()
                .filter(|track| {
                    matches!(track.track_type(), Ok(TrackType::Video | TrackType::Audio))
                })
                .count();
            extended_track_codecs(&input, media_track_count, &is_cancelled)?
        } else {
            HashMap::new()
        };
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
            }
            .or_else(|| {
                extended_codecs
                    .get(&track.track_id())
                    .map(|info| info.codec)
            });
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
                (
                    extended_codecs
                        .get(&track.track_id())
                        .and_then(|info| info.sample_rate_hz),
                    extended_codecs
                        .get(&track.track_id())
                        .and_then(|info| info.channels),
                )
            };
            let extended_info = extended_codecs.get(&track.track_id());
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
                width: (media_type == TrackType::Video).then(|| {
                    extended_info
                        .and_then(|info| info.width)
                        .unwrap_or_else(|| track.width())
                }),
                height: (media_type == TrackType::Video).then(|| {
                    extended_info
                        .and_then(|info| info.height)
                        .unwrap_or_else(|| track.height())
                }),
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
        let mut reader = open_reader(&input)?;
        let mut selected = Vec::with_capacity(reader.tracks().len());
        let mut track_ids = reader.tracks().keys().copied().collect::<Vec<_>>();
        track_ids.sort_unstable();
        for (index, id) in track_ids.iter().copied().enumerate() {
            selected.push(SelectedTrack {
                id,
                output_id: u32::try_from(index + 1).map_err(|_| NativeMp4Error::InvalidInput)?,
                sample_count: reader.sample_count(id)?,
            });
        }
        if selected.is_empty() {
            return Err(NativeMp4Error::InvalidInput);
        }
        let configs = match selected
            .iter()
            .map(|selected| {
                reader
                    .tracks()
                    .get(&selected.id)
                    .ok_or(NativeMp4Error::InvalidInput)
                    .and_then(track_config)
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(configs) => configs,
            Err(NativeMp4Error::UnsupportedCodec) => {
                if reader.tracks().values().any(|track| {
                    !matches!(track.track_type(), Ok(TrackType::Video | TrackType::Audio))
                }) {
                    return Err(NativeMp4Error::UnsupportedCodec);
                }
                return remux_extended(&input, output.as_ref(), max_output_bytes, &is_cancelled);
            }
            Err(error) => return Err(error),
        };
        let (destination, temporary) = prepare_output(output.as_ref())?;
        let _temporary_guard = TemporaryOutput(temporary.clone());
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

    pub fn remux_to_webm(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeMp4Error> {
        if max_output_bytes == 0 {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        let expected_track_count = open_reader(&input)?.tracks().len();
        let mut metadata = open_extended_input(&input, expected_track_count, &is_cancelled)?;
        validate_extended_track_layout(&metadata.tracks)?;
        if metadata.tracks.len() != 1 || metadata.tracks[0].kind != TrackKind::Video {
            return Err(NativeMp4Error::UnsupportedCodec);
        }
        let mut track_map = HashMap::new();
        let mut sample_entries = HashMap::new();
        let mut webm_tracks = Vec::with_capacity(metadata.tracks.len());
        let mut video_track_number = None;
        let mut inspected_samples = 0;

        while track_map.len() < metadata.tracks.len() {
            if is_cancelled() {
                return Err(NativeMp4Error::Cancelled);
            }
            inspected_samples += 1;
            if inspected_samples > MAX_EXTENDED_MP4_SAMPLES {
                return Err(NativeMp4Error::UnsupportedCodec);
            }
            let sample = metadata
                .demuxer
                .next_sample()
                .map_err(|_| NativeMp4Error::InvalidInput)?
                .ok_or(NativeMp4Error::InvalidInput)?;
            let track_id = sample.track.track_id;
            if track_map.contains_key(&track_id) {
                continue;
            }
            let entry = sample
                .sample_entry
                .cloned()
                .ok_or(NativeMp4Error::InvalidInput)?;
            let track_number = u64::try_from(webm_tracks.len() + 1)
                .map_err(|_| NativeMp4Error::UnsupportedCodec)?;
            let webm_track = match (sample.track.kind, &entry) {
                (TrackKind::Video, SampleEntry::Vp08(_)) => {
                    let (width, height) = entry
                        .video_resolution()
                        .ok_or(NativeMp4Error::UnsupportedCodec)?;
                    video_track_number = Some(track_number);
                    WebmTrackInfo {
                        track_number,
                        track_type: 1,
                        codec_id: "V_VP8".to_owned(),
                        codec_private: None,
                        width: u32::from(width),
                        height: u32::from(height),
                        sample_rate: 8000.0,
                        channels: 1,
                    }
                }
                (TrackKind::Video, SampleEntry::Vp09(_)) => {
                    let (width, height) = entry
                        .video_resolution()
                        .ok_or(NativeMp4Error::UnsupportedCodec)?;
                    video_track_number = Some(track_number);
                    WebmTrackInfo {
                        track_number,
                        track_type: 1,
                        codec_id: "V_VP9".to_owned(),
                        codec_private: None,
                        width: u32::from(width),
                        height: u32::from(height),
                        sample_rate: 8000.0,
                        channels: 1,
                    }
                }
                _ => return Err(NativeMp4Error::UnsupportedCodec),
            };
            track_map.insert(track_id, track_number);
            sample_entries.insert(track_id, entry);
            webm_tracks.push(webm_track);
        }

        let mut source = open_extended_input(&input, expected_track_count, &is_cancelled)?;
        let (destination, temporary) = prepare_output(output.as_ref())?;
        let exceeded = Arc::new(AtomicBool::new(false));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let _temporary_guard = TemporaryOutput(temporary.clone());
        let mut writer = BoundedWriter::new(file, max_output_bytes, Arc::clone(&exceeded));
        let mut muxer = WebmMuxer::with_options(1_000_000, 1);
        for track in webm_tracks {
            muxer.add_track(track).map_err(map_webm_mux_error)?;
        }
        let mut muxer = muxer.begin();
        let mut muxed_bytes = Vec::new();
        muxer.poll_bytes(&mut muxed_bytes);
        write_bounded(&mut writer, &muxed_bytes, &exceeded)?;
        let mut sample_count = 0;
        loop {
            if is_cancelled() {
                return Err(NativeMp4Error::Cancelled);
            }
            let sample = source
                .demuxer
                .next_sample()
                .map_err(|_| NativeMp4Error::InvalidInput)?;
            let Some(sample) = sample else {
                break;
            };
            sample_count += 1;
            if sample_count > MAX_EXTENDED_MP4_SAMPLES
                || sample.data_size == 0
                || sample.data_size > MAX_WEBM_SAMPLE_BYTES
            {
                return Err(NativeMp4Error::UnsupportedCodec);
            }
            let track_number = *track_map
                .get(&sample.track.track_id)
                .ok_or(NativeMp4Error::UnsupportedCodec)?;
            if let Some(entry) = sample.sample_entry
                && sample_entries.get(&sample.track.track_id) != Some(entry)
            {
                return Err(NativeMp4Error::UnsupportedCodec);
            }
            if sample
                .composition_time_offset
                .is_some_and(|offset| offset != 0)
            {
                return Err(NativeMp4Error::UnsupportedTiming);
            }
            let source_end = sample
                .data_offset
                .checked_add(
                    u64::try_from(sample.data_size).map_err(|_| NativeMp4Error::InvalidInput)?,
                )
                .ok_or(NativeMp4Error::InvalidInput)?;
            if source_end > source.file_size {
                return Err(NativeMp4Error::InvalidInput);
            }
            let timecode = u64::try_from(
                (u128::from(sample.timestamp) * 1000) / u128::from(sample.track.timescale.get()),
            )
            .ok()
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(NativeMp4Error::UnsupportedTiming)?;
            source.file.seek(SeekFrom::Start(sample.data_offset))?;
            let mut payload = Vec::with_capacity(sample.data_size);
            let mut remaining = sample.data_size;
            let mut buffer = [0; 64 * 1024];
            while remaining > 0 {
                if is_cancelled() {
                    return Err(NativeMp4Error::Cancelled);
                }
                let read_size = remaining.min(buffer.len());
                source
                    .file
                    .read_exact(&mut buffer[..read_size])
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::UnexpectedEof {
                            NativeMp4Error::InvalidInput
                        } else {
                            NativeMp4Error::Io(error)
                        }
                    })?;
                payload.extend_from_slice(&buffer[..read_size]);
                remaining -= read_size;
            }
            muxer
                .push_frame(track_number, timecode, sample.keyframe, &payload)
                .map_err(map_webm_mux_error)?;
            muxed_bytes.clear();
            muxer.poll_bytes(&mut muxed_bytes);
            write_bounded(&mut writer, &muxed_bytes, &exceeded)?;
        }
        if sample_count == 0 || video_track_number.is_none() {
            return Err(NativeMp4Error::InvalidInput);
        }
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        muxer.flush();
        muxed_bytes.clear();
        muxer.poll_bytes(&mut muxed_bytes);
        write_bounded(&mut writer, &muxed_bytes, &exceeded)?;
        writer.flush()?;
        writer.file.sync_all()?;
        let size_bytes = writer.file.metadata()?.len();
        drop(writer);
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        publish_output(&temporary, &destination)?;
        Ok(size_bytes)
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
        let mut video_reader = open_reader(&video_path)?;
        let mut audio_reader = open_reader(&audio_path)?;
        let Some((video_id, video_track)) =
            first_track(&video_reader, TrackType::Video, MediaType::H264)
        else {
            return Err(NativeMp4Error::RequiredTrackMissing);
        };
        let Some((audio_id, audio_track)) =
            first_track(&audio_reader, TrackType::Audio, MediaType::AAC)
        else {
            return Err(NativeMp4Error::RequiredTrackMissing);
        };
        let (destination, temporary) = prepare_output(output.as_ref())?;
        if video_path == destination || audio_path == destination {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let _temporary_guard = TemporaryOutput(temporary.clone());
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

    pub fn extract_aac(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<NativeMp4Info, NativeMp4Error> {
        if max_output_bytes == 0 {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let input = canonical_input(input.as_ref())?;
        let mut reader = open_reader(&input)?;
        let Some((track_id, track)) = first_track(&reader, TrackType::Audio, MediaType::AAC) else {
            return Err(NativeMp4Error::RequiredTrackMissing);
        };
        let selected = [SelectedTrack {
            id: track_id,
            output_id: 1,
            sample_count: reader.sample_count(track_id)?,
        }];
        let configs = [track_config(track)?];
        let (destination, temporary) = prepare_output(output.as_ref())?;
        if input == destination {
            return Err(NativeMp4Error::InvalidOutput);
        }
        let _temporary_guard = TemporaryOutput(temporary.clone());
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
}

struct ExtendedInput {
    file: File,
    file_size: u64,
    demuxer: Mp4FileDemuxer,
    tracks: Vec<TrackInfo>,
}

struct ExtendedOutput {
    writer: BoundedWriter,
    exceeded: Arc<AtomicBool>,
    muxer: Mp4FileMuxer,
    sample_count: usize,
    seen_entries: HashSet<TrackKind>,
}

#[derive(Clone, Copy)]
struct ExtendedCodecInfo {
    codec: &'static str,
    width: Option<u16>,
    height: Option<u16>,
    sample_rate_hz: Option<u32>,
    channels: Option<u16>,
}

fn extended_track_codecs(
    path: &Path,
    expected_track_count: usize,
    is_cancelled: &impl Fn() -> bool,
) -> Result<HashMap<u32, ExtendedCodecInfo>, NativeMp4Error> {
    let mut source = open_extended_input(path, expected_track_count, is_cancelled)?;
    let mut seen_tracks = HashSet::new();
    let mut codecs = HashMap::new();
    while seen_tracks.len() < source.tracks.len() {
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        let sample = source
            .demuxer
            .next_sample()
            .map_err(|_| NativeMp4Error::InvalidInput)?
            .ok_or(NativeMp4Error::InvalidInput)?;
        let track_id = sample.track.track_id;
        if seen_tracks.insert(track_id) {
            let entry = sample.sample_entry.ok_or(NativeMp4Error::InvalidInput)?;
            let codec = match &entry {
                SampleEntry::Avc1(_) => Some(("h264", entry.video_resolution())),
                SampleEntry::Hev1(_) | SampleEntry::Hvc1(_) => {
                    Some(("hevc", entry.video_resolution()))
                }
                SampleEntry::Vp09(_) => Some(("vp9", entry.video_resolution())),
                SampleEntry::Vp08(_) => Some(("vp8", entry.video_resolution())),
                SampleEntry::Av01(_) => Some(("av1", entry.video_resolution())),
                SampleEntry::Opus(_) => Some(("opus", None)),
                SampleEntry::Flac(_) => Some(("flac", None)),
                _ => None,
            };
            if let Some((codec, resolution)) = codec {
                codecs.insert(
                    track_id,
                    ExtendedCodecInfo {
                        codec,
                        width: resolution.map(|(width, _)| width),
                        height: resolution.map(|(_, height)| height),
                        sample_rate_hz: entry.audio_sample_rate().map(u32::from),
                        channels: entry.audio_channel_count().map(u16::from),
                    },
                );
            }
        }
    }
    Ok(codecs)
}

fn remux_extended(
    input: &Path,
    output: &Path,
    max_output_bytes: u64,
    is_cancelled: &impl Fn() -> bool,
) -> Result<NativeMp4Info, NativeMp4Error> {
    let expected_track_count = open_reader(input)?.tracks().len();
    let mut source = open_extended_input(input, expected_track_count, is_cancelled)?;
    validate_extended_track_layout(&source.tracks)?;
    let (destination, temporary) = prepare_output(output)?;
    let _temporary_guard = TemporaryOutput(temporary.clone());
    let mut output_state = start_extended_output(&temporary, max_output_bytes)?;
    write_extended_samples(&mut source, &mut output_state, is_cancelled)?;
    if output_state.seen_entries.len() != source.tracks.len() {
        return Err(NativeMp4Error::UnsupportedCodec);
    }
    finish_extended_output(output_state, &temporary, &destination, is_cancelled)
}

fn open_extended_input(
    path: &Path,
    expected_track_count: usize,
    is_cancelled: &impl Fn() -> bool,
) -> Result<ExtendedInput, NativeMp4Error> {
    if expected_track_count == 0 || expected_track_count > MAX_EXTENDED_MP4_TRACKS {
        return Err(NativeMp4Error::UnsupportedCodec);
    }
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut demuxer = Mp4FileDemuxer::new();
    let tracks = loop {
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        if let Ok(tracks) = demuxer.tracks() {
            break tracks.to_vec();
        }
        let required = demuxer
            .required_input()
            .ok_or(NativeMp4Error::InvalidInput)?;
        let size = required.size.ok_or(NativeMp4Error::InvalidInput)?;
        let end = required
            .position
            .checked_add(u64::try_from(size).map_err(|_| NativeMp4Error::InvalidInput)?)
            .ok_or(NativeMp4Error::InvalidInput)?;
        if size == 0 || size > MAX_EXTENDED_MP4_HEADER_BYTES || end > file_size {
            return Err(NativeMp4Error::UnsupportedCodec);
        }
        let mut bytes = vec![0; size];
        file.seek(SeekFrom::Start(required.position))?;
        file.read_exact(&mut bytes).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                NativeMp4Error::InvalidInput
            } else {
                NativeMp4Error::Io(error)
            }
        })?;
        demuxer.handle_input(Input {
            position: required.position,
            data: &bytes,
        });
    };
    if tracks.len() != expected_track_count {
        return Err(NativeMp4Error::UnsupportedCodec);
    }
    Ok(ExtendedInput {
        file,
        file_size,
        demuxer,
        tracks,
    })
}

fn validate_extended_track_layout(tracks: &[TrackInfo]) -> Result<(), NativeMp4Error> {
    if tracks.is_empty()
        || tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .count()
            > 1
        || tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .count()
            > 1
    {
        return Err(NativeMp4Error::UnsupportedCodec);
    }
    Ok(())
}

fn map_webm_mux_error(_error: WebmMuxError) -> NativeMp4Error {
    NativeMp4Error::UnsupportedCodec
}

fn start_extended_output(
    temporary: &Path,
    max_output_bytes: u64,
) -> Result<ExtendedOutput, NativeMp4Error> {
    let exceeded = Arc::new(AtomicBool::new(false));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    let mut writer = BoundedWriter::new(file, max_output_bytes, Arc::clone(&exceeded));
    let muxer = Mp4FileMuxer::new().map_err(|_| NativeMp4Error::UnsupportedCodec)?;
    write_bounded(&mut writer, muxer.initial_boxes_bytes(), &exceeded)?;
    Ok(ExtendedOutput {
        writer,
        exceeded,
        muxer,
        sample_count: 0,
        seen_entries: HashSet::new(),
    })
}

fn write_extended_samples(
    source: &mut ExtendedInput,
    output: &mut ExtendedOutput,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), NativeMp4Error> {
    loop {
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        let sample = source
            .demuxer
            .next_sample()
            .map_err(|_| NativeMp4Error::InvalidInput)?;
        let Some(sample) = sample else {
            break;
        };
        if output.sample_count >= MAX_EXTENDED_MP4_SAMPLES
            || sample.data_size == 0
            || sample.data_size > u32::MAX as usize
        {
            return Err(NativeMp4Error::UnsupportedCodec);
        }
        let sample_entry = sample.sample_entry.cloned();
        if let Some(entry) = &sample_entry {
            if !is_supported_sample_entry(sample.track.kind, entry) {
                return Err(NativeMp4Error::UnsupportedCodec);
            }
            output.seen_entries.insert(sample.track.kind);
        } else if !output.seen_entries.contains(&sample.track.kind) {
            return Err(NativeMp4Error::UnsupportedCodec);
        }
        let source_end = sample
            .data_offset
            .checked_add(u64::try_from(sample.data_size).map_err(|_| NativeMp4Error::InvalidInput)?)
            .ok_or(NativeMp4Error::InvalidInput)?;
        if source_end > source.file_size {
            return Err(NativeMp4Error::InvalidInput);
        }
        let output_offset = output.writer.stream_position()?;
        copy_sample_data(
            &mut source.file,
            &mut output.writer,
            sample.data_offset,
            sample.data_size,
            &output.exceeded,
            is_cancelled,
        )?;
        output
            .muxer
            .append_sample(&MuxSample {
                track_kind: sample.track.kind,
                sample_entry,
                keyframe: sample.keyframe,
                timescale: sample.track.timescale,
                duration: sample.duration,
                composition_time_offset: sample.composition_time_offset,
                data_offset: output_offset,
                data_size: sample.data_size,
            })
            .map_err(|_| NativeMp4Error::UnsupportedCodec)?;
        output.sample_count += 1;
    }
    Ok(())
}

fn is_supported_sample_entry(kind: TrackKind, entry: &SampleEntry) -> bool {
    matches!(
        (kind, entry),
        (
            TrackKind::Video,
            SampleEntry::Avc1(_)
                | SampleEntry::Hev1(_)
                | SampleEntry::Hvc1(_)
                | SampleEntry::Vp08(_)
                | SampleEntry::Vp09(_)
                | SampleEntry::Av01(_)
        ) | (
            TrackKind::Audio,
            SampleEntry::Opus(_) | SampleEntry::Flac(_)
        )
    )
}

fn copy_sample_data(
    source: &mut File,
    writer: &mut BoundedWriter,
    offset: u64,
    size: usize,
    exceeded: &AtomicBool,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), NativeMp4Error> {
    source.seek(SeekFrom::Start(offset))?;
    let mut remaining = size;
    let mut buffer = [0; 64 * 1024];
    while remaining > 0 {
        if is_cancelled() {
            return Err(NativeMp4Error::Cancelled);
        }
        let read_size = remaining.min(buffer.len());
        source
            .read_exact(&mut buffer[..read_size])
            .map_err(|error| {
                if error.kind() == io::ErrorKind::UnexpectedEof {
                    NativeMp4Error::InvalidInput
                } else {
                    NativeMp4Error::Io(error)
                }
            })?;
        write_bounded(writer, &buffer[..read_size], exceeded)?;
        remaining -= read_size;
    }
    Ok(())
}

fn write_bounded(
    writer: &mut BoundedWriter,
    bytes: &[u8],
    exceeded: &AtomicBool,
) -> Result<(), NativeMp4Error> {
    writer.write_all(bytes).map_err(|error| {
        if exceeded.load(Ordering::Relaxed) {
            NativeMp4Error::OutputTooLarge
        } else {
            NativeMp4Error::Io(error)
        }
    })
}

fn finish_extended_output(
    mut output: ExtendedOutput,
    temporary: &Path,
    destination: &Path,
    is_cancelled: &impl Fn() -> bool,
) -> Result<NativeMp4Info, NativeMp4Error> {
    if is_cancelled() {
        return Err(NativeMp4Error::Cancelled);
    }
    let finalized = output
        .muxer
        .finalize()
        .map_err(|_| NativeMp4Error::UnsupportedCodec)?;
    for (offset, bytes) in finalized.offset_and_bytes_pairs() {
        output.writer.seek(SeekFrom::Start(offset))?;
        write_bounded(&mut output.writer, bytes, &output.exceeded)?;
    }
    output.writer.flush()?;
    output.writer.file.sync_all()?;
    drop(output);
    let info = NativeMp4Processor::probe_with_cancel(temporary, is_cancelled)?;
    if is_cancelled() {
        return Err(NativeMp4Error::Cancelled);
    }
    publish_output(temporary, destination)?;
    Ok(info)
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
    use std::{
        error::Error,
        fs::File,
        io::{Seek, SeekFrom, Write},
        num::{NonZeroU16, NonZeroU32},
        path::Path,
    };

    use ebml_webm::Demuxer as WebmDemuxer;
    use mp4::{
        AacConfig, AudioObjectType, AvcConfig, ChannelConfig, FourCC, Mp4Config, Mp4Sample,
        Mp4Writer, SampleFreqIndex, TrackConfig,
    };
    use shiguredo_mp4::{
        FixedPointNumber, TrackKind, Uint,
        boxes::{
            AudioSampleEntryFields, Av01Box, Av1cBox, DflaBox, DopsBox, FlacBox, FlacMetadataBlock,
            Hvc1Box, HvccBox, HvccNalUintArray, OpusBox, SampleEntry, VisualSampleEntryFields,
            Vp08Box, Vp09Box, VpccBox,
        },
        mux::{Mp4FileMuxer, Sample as MuxSample},
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

    fn make_mp4_with_sample_entry(
        path: &Path,
        track_kind: TrackKind,
        sample_entry: SampleEntry,
        timescale: u32,
        duration: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut muxer = Mp4FileMuxer::new()?;
        let mut file = File::create(path)?;
        file.write_all(muxer.initial_boxes_bytes())?;
        let sample_data = [0, 0, 0, 2, 0x26, 0x01];
        let data_offset = file.stream_position()?;
        file.write_all(&sample_data)?;
        muxer.append_sample(&MuxSample {
            track_kind,
            sample_entry: Some(sample_entry),
            keyframe: true,
            timescale: NonZeroU32::new(timescale).ok_or("invalid timescale")?,
            duration,
            composition_time_offset: None,
            data_offset,
            data_size: sample_data.len(),
        })?;
        let finalized = muxer.finalize()?;
        for (offset, bytes) in finalized.offset_and_bytes_pairs() {
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(bytes)?;
        }
        Ok(())
    }

    fn make_mp4_with_video_audio_entries(
        path: &Path,
        video_entry: SampleEntry,
        audio_entry: SampleEntry,
    ) -> Result<(), Box<dyn Error>> {
        let mut muxer = Mp4FileMuxer::new()?;
        let mut file = File::create(path)?;
        file.write_all(muxer.initial_boxes_bytes())?;
        for (track_kind, sample_entry, timescale) in [
            (TrackKind::Video, video_entry, 1000),
            (TrackKind::Audio, audio_entry, 48_000),
        ] {
            let sample_data = [0, 0, 0, 2, 0x01, 0x02];
            let data_offset = file.stream_position()?;
            file.write_all(&sample_data)?;
            muxer.append_sample(&MuxSample {
                track_kind,
                sample_entry: Some(sample_entry),
                keyframe: true,
                timescale: NonZeroU32::new(timescale).ok_or("invalid timescale")?,
                duration: timescale,
                composition_time_offset: None,
                data_offset,
                data_size: sample_data.len(),
            })?;
        }
        let finalized = muxer.finalize()?;
        for (offset, bytes) in finalized.offset_and_bytes_pairs() {
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(bytes)?;
        }
        Ok(())
    }

    fn visual_sample_entry_fields() -> VisualSampleEntryFields {
        VisualSampleEntryFields {
            data_reference_index: NonZeroU16::MIN,
            width: 640,
            height: 360,
            horizresolution: FixedPointNumber::new(0x48, 0),
            vertresolution: FixedPointNumber::new(0x48, 0),
            frame_count: 1,
            compressorname: [0; 32],
            depth: 0x18,
        }
    }

    fn vp9_sample_entry() -> SampleEntry {
        SampleEntry::Vp09(Vp09Box {
            visual: visual_sample_entry_fields(),
            vpcc_box: VpccBox {
                profile: 0,
                level: 10,
                bit_depth: Uint::new(8),
                chroma_subsampling: Uint::new(0),
                video_full_range_flag: Uint::new(0),
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                codec_initialization_data: Vec::new(),
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn vp8_sample_entry() -> SampleEntry {
        SampleEntry::Vp08(Vp08Box {
            visual: visual_sample_entry_fields(),
            vpcc_box: VpccBox {
                profile: 0,
                level: 10,
                bit_depth: Uint::new(8),
                chroma_subsampling: Uint::new(0),
                video_full_range_flag: Uint::new(0),
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                codec_initialization_data: Vec::new(),
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn av1_sample_entry() -> SampleEntry {
        SampleEntry::Av01(Av01Box {
            visual: visual_sample_entry_fields(),
            av1c_box: Av1cBox {
                seq_profile: Uint::new(0),
                seq_level_idx_0: Uint::new(0),
                seq_tier_0: Uint::new(0),
                high_bitdepth: Uint::new(0),
                twelve_bit: Uint::new(0),
                monochrome: Uint::new(0),
                chroma_subsampling_x: Uint::new(1),
                chroma_subsampling_y: Uint::new(1),
                chroma_sample_position: Uint::new(0),
                initial_presentation_delay_minus_one: None,
                config_obus: Vec::new(),
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn opus_sample_entry() -> SampleEntry {
        SampleEntry::Opus(OpusBox {
            audio: AudioSampleEntryFields {
                data_reference_index: NonZeroU16::MIN,
                channelcount: 2,
                samplesize: 16,
                samplerate: FixedPointNumber::new(48_000, 0),
            },
            dops_box: DopsBox {
                output_channel_count: 2,
                pre_skip: 312,
                input_sample_rate: 48_000,
                output_gain: 0,
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn flac_sample_entry() -> SampleEntry {
        SampleEntry::Flac(FlacBox {
            audio: AudioSampleEntryFields {
                data_reference_index: NonZeroU16::MIN,
                channelcount: 2,
                samplesize: 16,
                samplerate: FixedPointNumber::new(48_000, 0),
            },
            dfla_box: DflaBox {
                metadata_blocks: vec![FlacMetadataBlock {
                    last_metadata_block_flag: Uint::new(1),
                    block_type: FlacMetadataBlock::BLOCK_TYPE_STREAMINFO,
                    block_data: vec![0; 34],
                }],
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn hevc_sample_entry() -> SampleEntry {
        let nalu_array = |nal_unit_type, nal_unit| HvccNalUintArray {
            array_completeness: Uint::new(1),
            nal_unit_type: Uint::new(nal_unit_type),
            nalus: vec![nal_unit],
        };
        SampleEntry::Hvc1(Hvc1Box {
            visual: visual_sample_entry_fields(),
            hvcc_box: HvccBox {
                general_profile_space: Uint::new(0),
                general_tier_flag: Uint::new(0),
                general_profile_idc: Uint::new(1),
                general_profile_compatibility_flags: 0,
                general_constraint_indicator_flags: Uint::new(0),
                general_level_idc: 120,
                min_spatial_segmentation_idc: Uint::new(0),
                parallelism_type: Uint::new(0),
                chroma_format_idc: Uint::new(1),
                bit_depth_luma_minus8: Uint::new(0),
                bit_depth_chroma_minus8: Uint::new(0),
                avg_frame_rate: 0,
                constant_frame_rate: Uint::new(0),
                num_temporal_layers: Uint::new(1),
                temporal_id_nested: Uint::new(1),
                length_size_minus_one: Uint::new(3),
                nalu_arrays: vec![
                    nalu_array(32, vec![0x40, 0x01]),
                    nalu_array(33, vec![0x42, 0x01]),
                    nalu_array(34, vec![0x44, 0x01]),
                ],
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn read_first_sample_entry(path: &Path) -> Result<SampleEntry, Box<dyn Error>> {
        let track_count = super::open_reader(path)?.tracks().len();
        let mut input = super::open_extended_input(path, track_count, &|| false)?;
        let sample = input
            .demuxer
            .next_sample()?
            .ok_or("MP4 contains no samples")?;
        sample
            .sample_entry
            .cloned()
            .ok_or_else(|| "MP4 sample has no sample entry".into())
    }

    fn read_all_sample_entries(
        path: &Path,
    ) -> Result<std::collections::HashMap<TrackKind, SampleEntry>, Box<dyn Error>> {
        let track_count = super::open_reader(path)?.tracks().len();
        let mut input = super::open_extended_input(path, track_count, &|| false)?;
        let mut entries = std::collections::HashMap::new();
        while let Some(sample) = input.demuxer.next_sample()? {
            if let Some(entry) = sample.sample_entry {
                entries.insert(sample.track.kind, entry.clone());
            }
        }
        Ok(entries)
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
    fn remuxes_vp9_and_preserves_its_sample_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-vp9.mp4");
        let output = directory.path().join("remuxed-vp9.mp4");
        let sample_entry = vp9_sample_entry();
        make_mp4_with_sample_entry(&input, TrackKind::Video, sample_entry.clone(), 1000, 1000)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks[0].codec.as_deref(), Some("vp9"));
        assert_eq!(read_first_sample_entry(&output)?, sample_entry);
        Ok(())
    }

    #[test]
    fn remuxes_vp8_and_preserves_its_sample_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-vp8.mp4");
        let output = directory.path().join("remuxed-vp8.mp4");
        let sample_entry = vp8_sample_entry();
        make_mp4_with_sample_entry(&input, TrackKind::Video, sample_entry.clone(), 1000, 1000)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks[0].codec.as_deref(), Some("vp8"));
        assert_eq!(read_first_sample_entry(&output)?, sample_entry);
        Ok(())
    }

    #[test]
    fn remuxes_av1_and_preserves_its_sample_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-av1.mp4");
        let output = directory.path().join("remuxed-av1.mp4");
        let sample_entry = av1_sample_entry();
        make_mp4_with_sample_entry(&input, TrackKind::Video, sample_entry.clone(), 1000, 1000)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks[0].codec.as_deref(), Some("av1"));
        assert_eq!(read_first_sample_entry(&output)?, sample_entry);
        Ok(())
    }

    #[test]
    fn remuxes_hevc_and_preserves_its_sample_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-hevc.mp4");
        let output = directory.path().join("remuxed-hevc.mp4");
        let sample_entry = hevc_sample_entry();
        make_mp4_with_sample_entry(&input, TrackKind::Video, sample_entry.clone(), 1000, 1000)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks[0].codec.as_deref(), Some("hevc"));
        assert_eq!(read_first_sample_entry(&output)?, sample_entry);
        Ok(())
    }

    #[test]
    fn remuxes_opus_and_preserves_its_sample_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("audio-opus.mp4");
        let output = directory.path().join("remuxed-opus.mp4");
        let sample_entry = opus_sample_entry();
        make_mp4_with_sample_entry(&input, TrackKind::Audio, sample_entry.clone(), 48_000, 960)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks[0].codec.as_deref(), Some("opus"));
        assert_eq!(info.tracks[0].sample_rate_hz, Some(48_000));
        assert_eq!(info.tracks[0].channels, Some(2));
        assert_eq!(read_first_sample_entry(&output)?, sample_entry);
        Ok(())
    }

    #[test]
    fn remuxes_flac_and_preserves_its_sample_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("audio-flac.mp4");
        let output = directory.path().join("remuxed-flac.mp4");
        let sample_entry = flac_sample_entry();
        make_mp4_with_sample_entry(&input, TrackKind::Audio, sample_entry.clone(), 48_000, 960)?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks[0].codec.as_deref(), Some("flac"));
        assert_eq!(info.tracks[0].sample_rate_hz, Some(48_000));
        assert_eq!(info.tracks[0].channels, Some(2));
        assert_eq!(read_first_sample_entry(&output)?, sample_entry);
        Ok(())
    }

    #[test]
    fn remuxes_video_and_audio_tracks_without_losing_codec_entries() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-opus.mp4");
        let output = directory.path().join("remuxed-video-opus.mp4");
        let video_entry = vp9_sample_entry();
        let audio_entry = opus_sample_entry();
        make_mp4_with_video_audio_entries(&input, video_entry.clone(), audio_entry.clone())?;

        let info = NativeMp4Processor::remux(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks.len(), 2);
        assert!(
            info.tracks
                .iter()
                .any(|track| track.codec.as_deref() == Some("vp9"))
        );
        assert!(
            info.tracks
                .iter()
                .any(|track| track.codec.as_deref() == Some("opus"))
        );
        let entries = read_all_sample_entries(&output)?;
        assert_eq!(entries.get(&TrackKind::Video), Some(&video_entry));
        assert_eq!(entries.get(&TrackKind::Audio), Some(&audio_entry));
        Ok(())
    }

    #[test]
    fn remuxes_vp9_mp4_to_webm() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-vp9.mp4");
        let output = directory.path().join("remuxed.webm");
        make_mp4_with_sample_entry(&input, TrackKind::Video, vp9_sample_entry(), 1000, 1000)?;

        let size = NativeMp4Processor::remux_to_webm(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(size, std::fs::metadata(&output)?.len());
        let bytes = std::fs::read(&output)?;
        let mut demuxer = WebmDemuxer::new();
        demuxer.push_bytes(&bytes);
        let tracks = demuxer.streams();
        assert_eq!(tracks.len(), 1);
        assert!(tracks.iter().any(|track| track.codec_id == "V_VP9"));
        let mut frames = 0;
        while demuxer.poll_frame().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 1);
        Ok(())
    }

    #[test]
    fn bounds_webm_output_without_publishing_a_partial_file() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-vp9.mp4");
        let output = directory.path().join("too-large.webm");
        make_mp4_with_sample_entry(&input, TrackKind::Video, vp9_sample_entry(), 1000, 1000)?;

        assert!(matches!(
            NativeMp4Processor::remux_to_webm(&input, &output, 8, || false),
            Err(NativeMp4Error::OutputTooLarge)
        ));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn webm_path_rejects_audio_tracks_it_cannot_mux_faithfully() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-opus.mp4");
        let output = directory.path().join("remuxed.webm");
        make_mp4_with_video_audio_entries(&input, vp9_sample_entry(), opus_sample_entry())?;

        assert!(matches!(
            NativeMp4Processor::remux_to_webm(&input, &output, 1024 * 1024, || false),
            Err(NativeMp4Error::UnsupportedCodec)
        ));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn bounds_extended_mp4_output_without_publishing_partial_file() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("video-vp9.mp4");
        let output = directory.path().join("too-large-vp9.mp4");
        make_mp4_with_sample_entry(&input, TrackKind::Video, vp9_sample_entry(), 1000, 1000)?;

        assert!(matches!(
            NativeMp4Processor::remux(&input, &output, 8, || false),
            Err(NativeMp4Error::OutputTooLarge)
        ));
        assert!(!output.exists());
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
    fn extracts_aac_audio_as_an_audio_only_mp4() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let input = directory.path().join("audio.mp4");
        let output = directory.path().join("extracted.m4a");
        make_mp4(&input, audio_config(), 1000)?;

        let info = NativeMp4Processor::extract_aac(&input, &output, 1024 * 1024, || false)?;
        assert_eq!(info.tracks.len(), 1);
        assert_eq!(info.tracks[0].media_type, "audio");
        assert_eq!(info.tracks[0].codec.as_deref(), Some("aac"));
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

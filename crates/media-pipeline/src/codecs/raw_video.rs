// Adapted from ffmpreg (Apache-2.0)
use crate::{
    Packet, Parameters, Result,
    frame::{
        AudioFrame, Channels, Frame, Keyframe, Pixel, PixelFormat, SampleFormat, SampleLayout,
        VideoFrame,
    },
    state::{AudioState, CodecState, State, VideoState},
    stream::{CodecId, StreamId, StreamKind},
    time::{Time, TimeBase},
    traits::{DecodeOut, Decoder, EncodeOut, Encoder},
};

// ---------------------------------------------------------------------------
// Helpers shared by decoder and encoder
// ---------------------------------------------------------------------------

pub fn audio_state(params: &Parameters) -> State {
    let codec_state = CodecState {
        extradata: Vec::new(),
        pts: None,
        dts: None,
        last_pts: None,
    };
    State::Audio(AudioState {
        sample_rate: params.sample_rate.unwrap_or(48_000),
        channels: Channels::from_count(params.channels.unwrap_or(2)),
        format: SampleFormat::S16,
        layout: SampleLayout::Interleaved,
        frame_size: 0,
        codec_state,
        samples_consumed: 0,
        samples_produced: 0,
        buffer: Vec::new(),
    })
}

pub fn video_state(params: &Parameters) -> State {
    let codec_state = CodecState {
        extradata: Vec::new(),
        pts: None,
        dts: None,
        last_pts: None,
    };
    let pixel = Pixel {
        depth: 8,
        format: PixelFormat::YUV420,
    };
    State::Video(VideoState {
        width: params.width.unwrap_or(0),
        height: params.height.unwrap_or(0),
        pixel,
        reorder: Vec::new(),
        has_keyframe: false,
        codec_state,
    })
}

pub fn default_codec_id(kind: StreamKind) -> CodecId {
    match kind {
        StreamKind::Audio => CodecId::new("pcm_s16le"),
        StreamKind::Video => CodecId::new("yuv420p"),
        StreamKind::Subtitle => CodecId::new("text"),
        StreamKind::Other => CodecId::new("binary"),
    }
}

pub fn default_time_base(kind: StreamKind) -> TimeBase {
    match kind {
        StreamKind::Audio => TimeBase::new(1, 48_000),
        _ => TimeBase::new(1, 1_000),
    }
}

pub fn packet_time(index: u64, kind: StreamKind) -> Time {
    Time::new(index, default_time_base(kind))
}

// ---------------------------------------------------------------------------
// RawDecoder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RawDecoder {
    kind: StreamKind,
}

impl RawDecoder {
    pub fn new(kind: StreamKind) -> Self {
        Self { kind }
    }
}

impl Decoder for RawDecoder {
    fn create_state(&self, parameters: &Parameters) -> State {
        match self.kind {
            StreamKind::Audio => audio_state(parameters),
            _ => video_state(parameters),
        }
    }

    fn decode(&self, state: &mut State, packet: Packet) -> Result<DecodeOut<State>> {
        let frame = match self.kind {
            StreamKind::Audio => {
                let (sample_rate, channels, format, layout) = match state {
                    State::Audio(audio) => (
                        audio.sample_rate,
                        audio.channels,
                        audio.format,
                        audio.layout,
                    ),
                    State::Video(_) => (
                        48_000,
                        Channels::Stereo,
                        SampleFormat::S16,
                        SampleLayout::Interleaved,
                    ),
                };
                let mut frame =
                    AudioFrame::new(packet.data.clone(), sample_rate, channels, format, layout);
                frame.pts = packet.pts;
                Frame::Audio(frame)
            }
            _ => {
                let (width, height, pixel) = match state {
                    State::Video(video) => (video.width, video.height, video.pixel.clone()),
                    State::Audio(_) => (
                        0,
                        0,
                        Pixel {
                            depth: 8,
                            format: PixelFormat::YUV420,
                        },
                    ),
                };
                Frame::Video(VideoFrame {
                    data: packet.data.clone(),
                    width,
                    height,
                    pixel,
                    keyframe: Keyframe::Key,
                    pts: packet.pts,
                })
            }
        };

        Ok(DecodeOut {
            value: vec![frame],
            state: state.clone(),
        })
    }

    fn flush(&self, state: &mut State) -> Result<DecodeOut<State>> {
        Ok(DecodeOut {
            value: Vec::new(),
            state: state.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// RawEncoder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RawEncoder {
    stream_id: StreamId,
    kind: StreamKind,
}

impl RawEncoder {
    pub fn new(stream_id: StreamId, kind: StreamKind) -> Self {
        Self { stream_id, kind }
    }
}

impl Encoder for RawEncoder {
    fn create_state(&self, parameters: &Parameters) -> State {
        match self.kind {
            StreamKind::Audio => audio_state(parameters),
            _ => video_state(parameters),
        }
    }

    fn encode(&self, state: &mut State, frame: Frame) -> Result<EncodeOut<State>> {
        let packet = match frame {
            Frame::Audio(f) => Packet {
                stream_id: self.stream_id,
                data: f.data,
                pts: f.pts,
                dts: f.pts,
                duration: None,
            },
            Frame::Video(f) => Packet {
                stream_id: self.stream_id,
                data: f.data,
                pts: f.pts,
                dts: f.pts,
                duration: None,
            },
        };

        Ok(EncodeOut {
            value: vec![packet],
            state: state.clone(),
        })
    }

    fn flush(&self, state: &mut State) -> Result<EncodeOut<State>> {
        Ok(EncodeOut {
            value: Vec::new(),
            state: state.clone(),
        })
    }
}

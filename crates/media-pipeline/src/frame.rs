// Adapted from ffmpreg (Apache-2.0)
use crate::time::Time;

// ---------------------------------------------------------------------------
// Audio format types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channels {
    Mono,
    #[default]
    Stereo,
    Quad,
    Surround51,
    Surround71,
    Custom(u8),
}

impl Channels {
    pub const fn count(self) -> u8 {
        match self {
            Channels::Mono => 1,
            Channels::Stereo => 2,
            Channels::Quad => 4,
            Channels::Surround51 => 6,
            Channels::Surround71 => 8,
            Channels::Custom(c) => c,
        }
    }

    pub const fn from_count(value: u8) -> Self {
        Self::from_value(value)
    }

    pub const fn from_value(value: u8) -> Self {
        match value {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            4 => Channels::Quad,
            6 => Channels::Surround51,
            8 => Channels::Surround71,
            _ => Channels::Custom(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    S16,
    S32,
    F32,
    F64,
}

impl SampleFormat {
    pub const fn bytes(self) -> usize {
        match self {
            SampleFormat::S16 => 2,
            SampleFormat::S32 => 4,
            SampleFormat::F32 => 4,
            SampleFormat::F64 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SampleLayout {
    #[default]
    Interleaved,
    Planar,
}

impl SampleLayout {
    pub const fn is_planar(self) -> bool {
        matches!(self, SampleLayout::Planar)
    }
}

#[derive(Debug, Clone)]
pub struct AudioParams {
    pub sample_rate: u32,
    pub channels: Channels,
    pub sample_format: SampleFormat,
    pub sample_layout: SampleLayout,
    pub codec_extradata: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Audio frame
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: Channels,
    pub format: SampleFormat,
    pub layout: SampleLayout,
    pub nb_samples: usize,
    pub pts: Option<Time>,
}

impl AudioFrame {
    pub fn new(
        data: Vec<u8>,
        sample_rate: u32,
        channels: Channels,
        format: SampleFormat,
        layout: SampleLayout,
    ) -> Self {
        let nb_samples = match layout {
            SampleLayout::Interleaved => {
                data.len() / (channels.count() as usize * format.bytes())
            }
            SampleLayout::Planar => data.len() / format.bytes(),
        };
        Self { data, sample_rate, channels, format, layout, nb_samples, pts: None }
    }

    pub fn frame_size(&self) -> usize {
        match self.layout {
            SampleLayout::Interleaved => {
                self.nb_samples * self.channels.count() as usize * self.format.bytes()
            }
            SampleLayout::Planar => self.nb_samples * self.format.bytes(),
        }
    }

    pub fn is_planar(&self) -> bool {
        self.layout.is_planar()
    }
}

// ---------------------------------------------------------------------------
// Video format types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    YUV420,
    YUV422,
    YUV444,
}

#[derive(Debug, Clone)]
pub struct Pixel {
    pub depth: u8,
    pub format: PixelFormat,
}

// ---------------------------------------------------------------------------
// Video frame
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyframe {
    Key,
    NonKey,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pixel: Pixel,
    pub keyframe: Keyframe,
    pub pts: Option<Time>,
}

// ---------------------------------------------------------------------------
// Unified Frame
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Frame {
    Audio(AudioFrame),
    Video(VideoFrame),
}

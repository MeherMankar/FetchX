// Adapted from muxide (MIT OR Apache-2.0)
//! VP9 video codec support for MP4 muxing.

use crate::assert_invariant;

/// VP9 codec configuration extracted from the first keyframe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vp9Config {
    pub width: u32,
    pub height: u32,
    pub profile: u8,
    pub bit_depth: u8,
    pub color_space: u8,
    pub transfer_function: u8,
    pub matrix_coefficients: u8,
    pub level: u8,
    pub full_range_flag: u8,
}

/// Errors that can occur during VP9 parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum Vp9Error {
    FrameTooShort,
    InvalidFrameMarker,
    UnsupportedProfile(u8),
    InvalidBitDepth(u8),
    ParseError(String),
}

impl std::fmt::Display for Vp9Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Vp9Error::FrameTooShort => write!(f, "VP9 frame too short for header"),
            Vp9Error::InvalidFrameMarker => write!(f, "invalid VP9 frame marker"),
            Vp9Error::UnsupportedProfile(p) => write!(f, "unsupported VP9 profile: {}", p),
            Vp9Error::InvalidBitDepth(b) => write!(f, "invalid VP9 bit depth: {}", b),
            Vp9Error::ParseError(msg) => write!(f, "VP9 parse error: {}", msg),
        }
    }
}

impl std::error::Error for Vp9Error {}

struct Vp9BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> Vp9BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    fn read_bit(&mut self) -> Option<u8> {
        if self.byte_pos >= self.data.len() {
            return None;
        }
        let bit = (self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Some(bit)
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        let mut val = 0u32;
        for _ in 0..count {
            val = (val << 1) | (self.read_bit()? as u32);
        }
        Some(val)
    }
}

/// Check if a VP9 frame is a keyframe (intra frame).
pub fn is_vp9_keyframe(frame: &[u8]) -> Result<bool, Vp9Error> {
    if frame.is_empty() {
        return Err(Vp9Error::FrameTooShort);
    }

    let mut r = Vp9BitReader::new(frame);

    let frame_marker = r.read_bits(2).ok_or(Vp9Error::FrameTooShort)?;
    if frame_marker != 2 {
        return Err(Vp9Error::InvalidFrameMarker);
    }

    let profile_low = r.read_bit().ok_or(Vp9Error::FrameTooShort)?;
    let profile_high = r.read_bit().ok_or(Vp9Error::FrameTooShort)?;
    let profile = (profile_high << 1) | profile_low;

    assert_invariant!(
        profile <= 3,
        "VP9 profile must be valid (0-3)",
        "codec::vp9::is_vp9_keyframe"
    );

    if profile == 3 {
        r.read_bit().ok_or(Vp9Error::FrameTooShort)?;
    }

    let show_existing = r.read_bit().ok_or(Vp9Error::FrameTooShort)?;
    if show_existing != 0 {
        return Ok(false);
    }

    let frame_type = r.read_bit().ok_or(Vp9Error::FrameTooShort)?;
    Ok(frame_type == 0)
}

/// Extract VP9 configuration from a keyframe.
pub fn extract_vp9_config(keyframe: &[u8]) -> Option<Vp9Config> {
    if keyframe.len() < 4 {
        return None;
    }

    let mut r = Vp9BitReader::new(keyframe);

    let frame_marker = r.read_bits(2)?;
    if frame_marker != 2 {
        return None;
    }

    let profile_low = r.read_bit()?;
    let profile_high = r.read_bit()?;
    let profile = (profile_high << 1) | profile_low;

    assert_invariant!(
        profile <= 3,
        "INV-402: VP9 profile must be valid (0-3)",
        "codec::vp9::extract_vp9_config"
    );

    if profile == 3 {
        r.read_bit()?;
    }

    let show_existing = r.read_bit()?;
    if show_existing != 0 {
        return None;
    }

    let frame_type = r.read_bit()?;
    if frame_type != 0 {
        return None;
    }

    r.read_bit()?;
    r.read_bit()?;

    let sync0 = r.read_bits(8)?;
    let sync1 = r.read_bits(8)?;
    let sync2 = r.read_bits(8)?;
    if sync0 != 0x49 || sync1 != 0x83 || sync2 != 0x42 {
        return None;
    }

    let bit_depth: u8 = if profile >= 2 {
        let ten_or_twelve = r.read_bit()?;
        if ten_or_twelve != 0 { 12 } else { 10 }
    } else {
        8
    };

    let color_space = r.read_bits(3)? as u8;

    let full_range_flag: u8 = if color_space != 7 {
        let cr = r.read_bit()?;
        if profile == 1 || profile == 3 {
            r.read_bit()?;
            r.read_bit()?;
            r.read_bit()?;
        }
        cr
    } else {
        1
    };

    let width_minus_1 = r.read_bits(16)?;
    let height_minus_1 = r.read_bits(16)?;

    Some(Vp9Config {
        width: width_minus_1 + 1,
        height: height_minus_1 + 1,
        profile,
        bit_depth,
        color_space,
        transfer_function: 0,
        matrix_coefficients: 0,
        level: 0,
        full_range_flag,
    })
}

#[cfg(test)]
pub(crate) fn is_valid_vp9_frame(frame: &[u8]) -> bool {
    if frame.is_empty() {
        return false;
    }
    (frame[0] >> 6) == 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_frame_marker() {
        let invalid_frame = [0x00, 0x00, 0x00];
        assert!(!is_valid_vp9_frame(&invalid_frame));
        assert!(matches!(
            is_vp9_keyframe(&invalid_frame),
            Err(Vp9Error::InvalidFrameMarker)
        ));
    }

    #[test]
    fn test_frame_too_short() {
        let empty_frame: &[u8] = &[];
        assert!(!is_valid_vp9_frame(empty_frame));
        assert!(matches!(
            is_vp9_keyframe(empty_frame),
            Err(Vp9Error::FrameTooShort)
        ));
    }

    #[test]
    fn test_valid_frame_marker() {
        let valid_frame = [0x82, 0x49, 0x83, 0x42, 0x00, 0x00];
        assert!(is_valid_vp9_frame(&valid_frame));
    }

    #[test]
    fn test_is_vp9_keyframe_valid() {
        let keyframe = [0x82];
        assert_eq!(is_vp9_keyframe(&keyframe), Ok(true));
    }

    #[test]
    fn test_is_vp9_keyframe_pframe() {
        let pframe = [0x84];
        assert_eq!(is_vp9_keyframe(&pframe), Ok(false));
    }

    #[test]
    fn test_is_vp9_keyframe_show_existing() {
        let show_existing = [0x88];
        assert_eq!(is_vp9_keyframe(&show_existing), Ok(false));
    }

    #[test]
    fn test_extract_vp9_config_valid() {
        let keyframe = vec![0x82, 0x49, 0x83, 0x42, 0x00, 0x0F, 0xF0, 0x0F, 0xF0];
        let config = extract_vp9_config(&keyframe).unwrap();
        assert_eq!(config.width, 256);
        assert_eq!(config.height, 256);
        assert_eq!(config.profile, 0);
        assert_eq!(config.bit_depth, 8);
        assert_eq!(config.level, 0);
        assert_eq!(config.full_range_flag, 0);
    }

    #[test]
    fn test_extract_vp9_config_invalid_marker() {
        let invalid_frame = [0x00, 0x00, 0x00, 0x00];
        assert!(extract_vp9_config(&invalid_frame).is_none());
    }

    #[test]
    fn test_extract_vp9_config_pframe() {
        let pframe = [0x84, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(extract_vp9_config(&pframe).is_none());
    }
}

// Adapted from muxide (MIT OR Apache-2.0)
//! Opus codec support for MP4 muxing.

/// Default Opus sample rate (48kHz, per Opus specification)
pub const OPUS_SAMPLE_RATE: u32 = 48000;

/// Opus decoder configuration for the dOps box.
#[derive(Debug, Clone)]
pub struct OpusConfig {
    pub version: u8,
    pub output_channel_count: u8,
    pub pre_skip: u16,
    pub input_sample_rate: u32,
    pub output_gain: i16,
    pub channel_mapping_family: u8,
    pub stream_count: Option<u8>,
    pub coupled_count: Option<u8>,
    pub channel_mapping: Option<Vec<u8>>,
}

impl Default for OpusConfig {
    fn default() -> Self {
        Self {
            version: 0,
            output_channel_count: 2,
            pre_skip: 312,
            input_sample_rate: 48000,
            output_gain: 0,
            channel_mapping_family: 0,
            stream_count: None,
            coupled_count: None,
            channel_mapping: None,
        }
    }
}

impl OpusConfig {
    pub fn mono() -> Self {
        Self {
            output_channel_count: 1,
            ..Default::default()
        }
    }

    pub fn stereo() -> Self {
        Self {
            output_channel_count: 2,
            ..Default::default()
        }
    }

    pub fn with_pre_skip(mut self, pre_skip: u16) -> Self {
        self.pre_skip = pre_skip;
        self
    }

    pub fn with_channels(mut self, channels: u8) -> Self {
        self.output_channel_count = channels;
        if channels > 2 {
            self.channel_mapping_family = 1;
        }
        self
    }
}

/// Opus frame duration in samples at 48kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusFrameDuration {
    Ms2_5,
    Ms5,
    Ms10,
    Ms20,
    Ms40,
    Ms60,
}

impl OpusFrameDuration {
    pub fn samples(self) -> u32 {
        match self {
            OpusFrameDuration::Ms2_5 => 120,
            OpusFrameDuration::Ms5 => 240,
            OpusFrameDuration::Ms10 => 480,
            OpusFrameDuration::Ms20 => 960,
            OpusFrameDuration::Ms40 => 1920,
            OpusFrameDuration::Ms60 => 2880,
        }
    }

    pub fn seconds(self) -> f64 {
        self.samples() as f64 / OPUS_SAMPLE_RATE as f64
    }
}

/// Extract frame duration from the Opus TOC byte.
pub fn opus_frame_duration_from_toc(toc: u8) -> Option<OpusFrameDuration> {
    let config = (toc >> 3) & 0x1F;

    if config > 31 {
        return None;
    }

    match config {
        0..=3 => Some(OpusFrameDuration::Ms10),
        4..=7 => Some(OpusFrameDuration::Ms20),
        8..=11 => Some(OpusFrameDuration::Ms40),
        12..=15 => Some(OpusFrameDuration::Ms60),
        16..=19 => Some(OpusFrameDuration::Ms10),
        20..=23 => Some(OpusFrameDuration::Ms20),
        24..=27 => Some(OpusFrameDuration::Ms2_5),
        28..=31 => Some(OpusFrameDuration::Ms5),
        _ => None,
    }
}

/// Extract the frame count from the Opus packet.
pub fn opus_frame_count(packet: &[u8]) -> Option<(u8, bool)> {
    if packet.is_empty() {
        return None;
    }

    let toc = packet[0];
    let code = toc & 0x03;

    if code > 3 {
        return None;
    }

    match code {
        0 => Some((1, false)),
        1 => Some((2, false)),
        2 => Some((2, true)),
        3 => {
            if packet.len() < 2 {
                return None;
            }

            let frame_count_byte = packet[1];
            let is_vbr = (frame_count_byte & 0x80) != 0;
            let count = frame_count_byte & 0x3F;

            if count == 0 {
                return None;
            }

            Some((count, is_vbr))
        }
        _ => None,
    }
}

/// Calculate total sample duration for an Opus packet.
pub fn opus_packet_samples(packet: &[u8]) -> Option<u32> {
    if packet.is_empty() {
        return None;
    }

    let frame_duration = opus_frame_duration_from_toc(packet[0])?;
    let (frame_count, _) = opus_frame_count(packet)?;

    if !(1..=63).contains(&frame_count) {
        return None;
    }

    let samples = frame_duration.samples() * frame_count as u32;

    if samples == 0 {
        return None;
    }

    Some(samples)
}

/// Validate an Opus packet for basic structural correctness.
pub fn is_valid_opus_packet(packet: &[u8]) -> bool {
    if packet.is_empty() {
        return false;
    }
    opus_packet_samples(packet).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opus_config_default() {
        let config = OpusConfig::default();
        assert_eq!(config.version, 0);
        assert_eq!(config.output_channel_count, 2);
        assert_eq!(config.pre_skip, 312);
        assert_eq!(config.input_sample_rate, 48000);
        assert_eq!(config.output_gain, 0);
        assert_eq!(config.channel_mapping_family, 0);
    }

    #[test]
    fn test_opus_config_mono() {
        let config = OpusConfig::mono();
        assert_eq!(config.output_channel_count, 1);
    }

    #[test]
    fn test_opus_config_stereo() {
        let config = OpusConfig::stereo();
        assert_eq!(config.output_channel_count, 2);
    }

    #[test]
    fn test_opus_frame_duration_samples() {
        assert_eq!(OpusFrameDuration::Ms2_5.samples(), 120);
        assert_eq!(OpusFrameDuration::Ms5.samples(), 240);
        assert_eq!(OpusFrameDuration::Ms10.samples(), 480);
        assert_eq!(OpusFrameDuration::Ms20.samples(), 960);
        assert_eq!(OpusFrameDuration::Ms40.samples(), 1920);
        assert_eq!(OpusFrameDuration::Ms60.samples(), 2880);
    }

    #[test]
    fn test_opus_frame_duration_from_toc_silk() {
        assert_eq!(opus_frame_duration_from_toc(0b0000_0000), Some(OpusFrameDuration::Ms10));
        assert_eq!(opus_frame_duration_from_toc(0b0010_0000), Some(OpusFrameDuration::Ms20));
        assert_eq!(opus_frame_duration_from_toc(0b0100_0000), Some(OpusFrameDuration::Ms40));
        assert_eq!(opus_frame_duration_from_toc(0b0110_0000), Some(OpusFrameDuration::Ms60));
    }

    #[test]
    fn test_opus_frame_duration_from_toc_celt() {
        assert_eq!(opus_frame_duration_from_toc(0b1100_0000), Some(OpusFrameDuration::Ms2_5));
        assert_eq!(opus_frame_duration_from_toc(0b1110_0000), Some(OpusFrameDuration::Ms5));
    }

    #[test]
    fn test_opus_frame_count_single() {
        let packet = vec![0b0000_0000, 0x01, 0x02, 0x03];
        assert_eq!(opus_frame_count(&packet), Some((1, false)));
    }

    #[test]
    fn test_opus_frame_count_double_equal() {
        let packet = vec![0b0000_0001, 0x01, 0x02, 0x03];
        assert_eq!(opus_frame_count(&packet), Some((2, false)));
    }

    #[test]
    fn test_opus_frame_count_double_different() {
        let packet = vec![0b0000_0010, 0x01, 0x02, 0x03];
        assert_eq!(opus_frame_count(&packet), Some((2, true)));
    }

    #[test]
    fn test_opus_frame_count_arbitrary() {
        let packet = vec![0b0000_0011, 0b0000_0100];
        assert_eq!(opus_frame_count(&packet), Some((4, false)));

        let packet_vbr = vec![0b0000_0011, 0b1000_0100];
        assert_eq!(opus_frame_count(&packet_vbr), Some((4, true)));
    }

    #[test]
    fn test_opus_packet_samples() {
        let packet = vec![0x20, 0x01, 0x02, 0x03];
        assert_eq!(opus_packet_samples(&packet), Some(960));

        let packet2 = vec![0x21, 0x01, 0x02, 0x03];
        assert_eq!(opus_packet_samples(&packet2), Some(1920));
    }

    #[test]
    fn test_opus_functions_handle_bad_input_gracefully() {
        assert_eq!(opus_frame_count(&[]), None);
        assert_eq!(opus_packet_samples(&[]), None);

        let valid_toc = vec![0xFF];
        assert_eq!(opus_frame_duration_from_toc(valid_toc[0]), Some(OpusFrameDuration::Ms5));
        assert_eq!(opus_packet_samples(&valid_toc), None);

        let invalid_code3 = vec![0x03, 0x00];
        assert_eq!(opus_frame_count(&invalid_code3), None);
        assert_eq!(opus_packet_samples(&invalid_code3), None);
    }

    #[test]
    fn test_is_valid_opus_packet() {
        assert!(is_valid_opus_packet(&[0x20, 0x01, 0x02]));
        assert!(!is_valid_opus_packet(&[]));
    }
}

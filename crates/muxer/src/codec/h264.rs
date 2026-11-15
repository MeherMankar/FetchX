// Adapted from muxide (MIT OR Apache-2.0)
//! H.264/AVC codec configuration extraction.

use super::common::AnnexBNalIter;
use crate::assert_invariant;

/// H.264 NAL unit type constants.
pub mod nal_type {
    /// Non-IDR coded slice (P/B frame)
    pub const NON_IDR_SLICE: u8 = 1;
    /// IDR coded slice (keyframe)
    pub const IDR_SLICE: u8 = 5;
    /// Sequence Parameter Set
    pub const SPS: u8 = 7;
    /// Picture Parameter Set
    pub const PPS: u8 = 8;
}

/// AVC (H.264) codec configuration extracted from NAL units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcConfig {
    /// Sequence Parameter Set NAL unit (without start code)
    pub sps: Vec<u8>,
    /// Picture Parameter Set NAL unit (without start code)
    pub pps: Vec<u8>,
}

impl AvcConfig {
    /// Create a new AVC configuration from SPS and PPS data.
    pub fn new(sps: Vec<u8>, pps: Vec<u8>) -> Self {
        Self { sps, pps }
    }

    /// Extract profile_idc from the SPS.
    pub fn profile_idc(&self) -> u8 {
        self.sps.get(1).copied().unwrap_or(66)
    }

    /// Extract profile_compatibility flags from the SPS.
    pub fn profile_compatibility(&self) -> u8 {
        self.sps.get(2).copied().unwrap_or(0)
    }

    /// Extract level_idc from the SPS.
    pub fn level_idc(&self) -> u8 {
        self.sps.get(3).copied().unwrap_or(31)
    }
}

/// Default SPS for 640x480 @ Baseline Profile, Level 3.0.
pub const DEFAULT_SPS: &[u8] = &[0x67, 0x42, 0x00, 0x1e, 0xda, 0x02, 0x80, 0x2d, 0x8b, 0x11];

/// Default PPS.
pub const DEFAULT_PPS: &[u8] = &[0x68, 0xce, 0x38, 0x80];

/// Extract AVC configuration (SPS/PPS) from an Annex B keyframe.
pub fn extract_avc_config(data: &[u8]) -> Option<AvcConfig> {
    if data.is_empty() {
        return None;
    }

    let mut sps: Option<&[u8]> = None;
    let mut pps: Option<&[u8]> = None;

    for nal in AnnexBNalIter::new(data) {
        if nal.is_empty() {
            continue;
        }

        let nal_type = nal[0] & 0x1f;

        assert_invariant!(
            nal_type <= 31,
            "INV-301: H.264 NAL type must be valid (0-31)",
            "codec::h264::extract_avc_config"
        );

        if nal_type == nal_type::SPS && sps.is_none() {
            sps = Some(nal);
        } else if nal_type == nal_type::PPS && pps.is_none() {
            pps = Some(nal);
        }

        if sps.is_some() && pps.is_some() {
            break;
        }
    }

    if let (Some(sps_data), Some(pps_data)) = (sps, pps) {
        Some(AvcConfig {
            sps: sps_data.to_vec(),
            pps: pps_data.to_vec(),
        })
    } else {
        None
    }
}

/// Create a default AVC configuration for testing/fallback.
pub fn default_avc_config() -> AvcConfig {
    AvcConfig {
        sps: DEFAULT_SPS.to_vec(),
        pps: DEFAULT_PPS.to_vec(),
    }
}

/// Convert Annex B formatted data to AVCC (length-prefixed) format.
pub fn annexb_to_avcc(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();

    for nal in AnnexBNalIter::new(data) {
        if nal.is_empty() {
            continue;
        }
        let len = nal.len() as u32;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nal);
    }

    if out.is_empty() && !data.is_empty() {
        let len = data.len() as u32;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(data);
    }

    out
}

/// Check if the given Annex B data represents a keyframe (IDR slice).
pub fn is_h264_keyframe(data: &[u8]) -> bool {
    for nal in AnnexBNalIter::new(data) {
        if nal.is_empty() {
            continue;
        }
        let nal_type = nal[0] & 0x1f;
        if nal_type == nal_type::IDR_SLICE {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_avc_config_success() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1f, 0x00, 0x00, 0x00, 0x01, 0x68, 0xeb,
            0xe3, 0xcb, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00,
        ];

        let config = extract_avc_config(&data).unwrap();
        assert_eq!(config.sps, &[0x67, 0x64, 0x00, 0x1f]);
        assert_eq!(config.pps, &[0x68, 0xeb, 0xe3, 0xcb]);
    }

    #[test]
    fn test_extract_avc_config_missing_sps() {
        let data = [0x00, 0x00, 0x00, 0x01, 0x68, 0xeb, 0xe3, 0xcb];
        assert!(extract_avc_config(&data).is_none());
    }

    #[test]
    fn test_extract_avc_config_missing_pps() {
        let data = [0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1f];
        assert!(extract_avc_config(&data).is_none());
    }

    #[test]
    fn test_avc_config_accessors() {
        let config = AvcConfig::new(vec![0x67, 0x64, 0x00, 0x28], vec![0x68, 0xeb]);

        assert_eq!(config.profile_idc(), 0x64);
        assert_eq!(config.profile_compatibility(), 0x00);
        assert_eq!(config.level_idc(), 0x28);
    }

    #[test]
    fn test_annexb_to_avcc() {
        let annexb = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x00, 0x00, 0x00, 0x01, 0x68, 0xeb,
        ];

        let avcc = annexb_to_avcc(&annexb);
        assert_eq!(&avcc[0..4], &[0x00, 0x00, 0x00, 0x03]);
        assert_eq!(&avcc[4..7], &[0x67, 0x64, 0x00]);
        assert_eq!(&avcc[7..11], &[0x00, 0x00, 0x00, 0x02]);
        assert_eq!(&avcc[11..13], &[0x68, 0xeb]);
    }

    #[test]
    fn test_annexb_to_avcc_no_start_codes() {
        let data = [0x65, 0x88, 0x84];
        let avcc = annexb_to_avcc(&data);
        assert_eq!(&avcc[0..4], &[0x00, 0x00, 0x00, 0x03]);
        assert_eq!(&avcc[4..7], &[0x65, 0x88, 0x84]);
    }

    #[test]
    fn test_is_keyframe_idr() {
        let idr_frame = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88,
        ];
        assert!(is_h264_keyframe(&idr_frame));
    }

    #[test]
    fn test_is_keyframe_non_idr() {
        let p_frame = [0x00, 0x00, 0x00, 0x01, 0x41, 0x9a];
        assert!(!is_h264_keyframe(&p_frame));
    }

    #[test]
    fn test_is_keyframe_empty() {
        assert!(!is_h264_keyframe(&[]));
    }

    #[test]
    fn test_default_avc_config() {
        let config = default_avc_config();
        assert!(!config.sps.is_empty());
        assert!(!config.pps.is_empty());
        assert_eq!(config.sps[0] & 0x1f, nal_type::SPS);
        assert_eq!(config.pps[0] & 0x1f, nal_type::PPS);
    }
}

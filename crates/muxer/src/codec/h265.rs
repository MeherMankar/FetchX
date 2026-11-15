// Adapted from muxide (MIT OR Apache-2.0)
//! H.265/HEVC codec configuration extraction.

use super::common::AnnexBNalIter;

/// H.265 NAL unit type constants.
pub mod nal_type {
    pub const BLA_W_LP: u8 = 16;
    pub const BLA_W_RADL: u8 = 17;
    pub const BLA_N_LP: u8 = 18;
    pub const IDR_W_RADL: u8 = 19;
    pub const IDR_N_LP: u8 = 20;
    pub const CRA_NUT: u8 = 21;
    pub const VPS: u8 = 32;
    pub const SPS: u8 = 33;
    pub const PPS: u8 = 34;
    pub const AUD: u8 = 35;
    pub const EOS: u8 = 36;
    pub const EOB: u8 = 37;
    pub const FD: u8 = 38;
    pub const PREFIX_SEI: u8 = 39;
    pub const SUFFIX_SEI: u8 = 40;
}

/// HEVC (H.265) codec configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HevcConfig {
    /// Video Parameter Set NAL unit (without start code)
    pub vps: Vec<u8>,
    /// Sequence Parameter Set NAL unit (without start code)
    pub sps: Vec<u8>,
    /// Picture Parameter Set NAL unit (without start code)
    pub pps: Vec<u8>,
}

impl HevcConfig {
    pub fn new(vps: Vec<u8>, sps: Vec<u8>, pps: Vec<u8>) -> Self {
        Self { vps, sps, pps }
    }

    pub fn general_profile_space(&self) -> u8 {
        self.sps.get(3).map(|b| (b >> 6) & 0x03).unwrap_or(0)
    }

    pub fn general_tier_flag(&self) -> bool {
        self.sps
            .get(3)
            .map(|b| (b >> 5) & 0x01 != 0)
            .unwrap_or(false)
    }

    pub fn general_profile_idc(&self) -> u8 {
        self.sps.get(3).map(|b| b & 0x1f).unwrap_or(1)
    }

    pub fn general_level_idc(&self) -> u8 {
        self.sps.get(14).copied().unwrap_or(93)
    }

    pub fn general_profile_tier_level(&self) -> Option<[u8; 12]> {
        let bytes = self.sps.get(3..15)?;
        let mut out = [0u8; 12];
        out.copy_from_slice(bytes);
        Some(out)
    }
}

/// Extract the NAL unit type from an H.265 NAL header.
#[inline]
pub fn hevc_nal_type(nal: &[u8]) -> u8 {
    if nal.is_empty() {
        return 0;
    }
    (nal[0] >> 1) & 0x3f
}

/// Check if the given NAL type represents a keyframe (IRAP).
#[inline]
pub fn is_hevc_keyframe_nal_type(nal_type: u8) -> bool {
    matches!(
        nal_type,
        nal_type::BLA_W_LP
            | nal_type::BLA_W_RADL
            | nal_type::BLA_N_LP
            | nal_type::IDR_W_RADL
            | nal_type::IDR_N_LP
            | nal_type::CRA_NUT
    )
}

use crate::assert_invariant;

/// Extract HEVC configuration (VPS/SPS/PPS) from Annex B keyframe data.
pub fn extract_hevc_config(data: &[u8]) -> Option<HevcConfig> {
    if data.is_empty() {
        return None;
    }

    let mut vps: Option<&[u8]> = None;
    let mut sps: Option<&[u8]> = None;
    let mut pps: Option<&[u8]> = None;

    for nal in AnnexBNalIter::new(data) {
        if nal.is_empty() {
            continue;
        }

        let nal_type = hevc_nal_type(nal);

        assert_invariant!(
            nal_type <= 63,
            "INV-501: HEVC NAL type must be valid (0-63)",
            "codec::h265::extract_hevc_config"
        );

        match nal_type {
            nal_type::VPS if vps.is_none() => vps = Some(nal),
            nal_type::SPS if sps.is_none() => sps = Some(nal),
            nal_type::PPS if pps.is_none() => pps = Some(nal),
            _ => {}
        }

        if vps.is_some() && sps.is_some() && pps.is_some() {
            break;
        }
    }

    if let (Some(vps_data), Some(sps_data), Some(pps_data)) = (vps, sps, pps) {
        assert_invariant!(
            !vps_data.is_empty() && !sps_data.is_empty() && !pps_data.is_empty(),
            "INV-502: HEVC VPS, SPS, and PPS must be non-empty"
        );

        Some(HevcConfig {
            vps: vps_data.to_vec(),
            sps: sps_data.to_vec(),
            pps: pps_data.to_vec(),
        })
    } else {
        None
    }
}

/// Convert Annex B formatted HEVC data to hvcC format (length-prefixed).
pub fn hevc_annexb_to_hvcc(data: &[u8]) -> Vec<u8> {
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

/// Check if the given Annex B data represents an HEVC keyframe (IRAP).
pub fn is_hevc_keyframe(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }

    for nal in AnnexBNalIter::new(data) {
        if nal.is_empty() {
            continue;
        }
        let nal_type = hevc_nal_type(nal);

        assert_invariant!(
            nal_type <= 63,
            "INV-504: HEVC NAL type must be valid (0-63)"
        );

        if is_hevc_keyframe_nal_type(nal_type) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hevc_nal_type_extraction() {
        let vps_nal = [0x40, 0x01, 0x0c];
        assert_eq!(hevc_nal_type(&vps_nal), 32);

        let sps_nal = [0x42, 0x01, 0x01];
        assert_eq!(hevc_nal_type(&sps_nal), 33);
    }

    #[test]
    fn test_is_hevc_keyframe_nal_type() {
        assert!(is_hevc_keyframe_nal_type(nal_type::IDR_W_RADL));
        assert!(is_hevc_keyframe_nal_type(nal_type::IDR_N_LP));
        assert!(is_hevc_keyframe_nal_type(nal_type::CRA_NUT));
        assert!(is_hevc_keyframe_nal_type(nal_type::BLA_W_LP));
        assert!(!is_hevc_keyframe_nal_type(nal_type::VPS));
        assert!(!is_hevc_keyframe_nal_type(nal_type::SPS));
        assert!(!is_hevc_keyframe_nal_type(nal_type::PPS));
    }

    #[test]
    fn test_extract_hevc_config_empty() {
        assert!(extract_hevc_config(&[]).is_none());
    }

    #[test]
    fn test_extract_hevc_config_success() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0x0c, 0x01, 0x00, 0x00, 0x00, 0x01, 0x42, 0x01,
            0x01, 0x21, 0x00, 0x00, 0x00, 0x01, 0x44, 0x01, 0xc0, 0x73,
        ];

        let config = extract_hevc_config(&data).unwrap();
        assert_eq!(hevc_nal_type(&config.vps), nal_type::VPS);
        assert_eq!(hevc_nal_type(&config.sps), nal_type::SPS);
        assert_eq!(hevc_nal_type(&config.pps), nal_type::PPS);
    }

    #[test]
    fn test_extract_hevc_config_missing_vps() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x42, 0x01, 0x01, 0x21, 0x00, 0x00, 0x00, 0x01, 0x44, 0x01,
            0xc0, 0x73,
        ];
        assert!(extract_hevc_config(&data).is_none());
    }

    #[test]
    fn test_hevc_annexb_to_hvcc() {
        let annexb = [
            0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x42, 0x01,
        ];

        let hvcc = hevc_annexb_to_hvcc(&annexb);
        assert_eq!(&hvcc[0..4], &[0x00, 0x00, 0x00, 0x03]);
        assert_eq!(&hvcc[4..7], &[0x40, 0x01, 0x0c]);
        assert_eq!(&hvcc[7..11], &[0x00, 0x00, 0x00, 0x02]);
        assert_eq!(&hvcc[11..13], &[0x42, 0x01]);
    }

    #[test]
    fn test_is_hevc_keyframe() {
        let idr = [
            0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0x00, 0x00, 0x00, 0x01, 0x26, 0x01,
        ];
        assert!(is_hevc_keyframe(&idr));

        let trail = [0x00, 0x00, 0x00, 0x01, 0x02, 0x01];
        assert!(!is_hevc_keyframe(&trail));
    }

    #[test]
    fn test_is_hevc_keyframe_bla_types_are_irap() {
        let bla_w_lp = [0x00, 0x00, 0x00, 0x01, 0x20, 0x01];
        assert!(is_hevc_keyframe(&bla_w_lp));

        let bla_w_radl = [0x00, 0x00, 0x00, 0x01, 0x22, 0x01];
        assert!(is_hevc_keyframe(&bla_w_radl));

        let bla_n_lp = [0x00, 0x00, 0x00, 0x01, 0x24, 0x01];
        assert!(is_hevc_keyframe(&bla_n_lp));

        let cra = [0x00, 0x00, 0x00, 0x01, 0x2a, 0x01];
        assert!(is_hevc_keyframe(&cra));
    }

    #[test]
    fn test_is_hevc_keyframe_no_panic_on_no_start_codes() {
        let raw = [0x42, 0x01, 0x01, 0x21, 0x80, 0x00];
        assert!(!is_hevc_keyframe(&raw));
    }

    #[test]
    fn test_is_hevc_keyframe_no_panic_on_consecutive_start_codes() {
        let consecutive = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x26, 0x01];
        assert!(is_hevc_keyframe(&consecutive));
    }

    #[test]
    fn test_hevc_config_accessors() {
        let config = HevcConfig::new(
            vec![0x40, 0x01],
            vec![
                0x42, 0x01, 0x01, 0x21, 0x80, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03,
                0x00, 0x5d,
            ],
            vec![0x44, 0x01],
        );

        assert_eq!(config.general_profile_space(), 0);
        assert!(config.general_tier_flag());
        assert_eq!(config.general_profile_idc(), 1);
        let level = config.general_level_idc();
        assert!(level <= 186);
    }
}

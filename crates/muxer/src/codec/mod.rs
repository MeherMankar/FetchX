// Adapted from muxide (MIT OR Apache-2.0)
//! Codec configuration extraction for container muxing.
//!
//! This module provides minimal bitstream parsing required to build codec
//! configuration boxes (avcC, hvcC, av1C, dOps). It does NOT perform decoding,
//! transcoding, or frame reconstruction.

pub mod av1;
pub mod common;
pub mod h264;
pub mod h265;
pub mod opus;
pub mod vp9;

pub use av1::{Av1Config, extract_av1_config, is_av1_keyframe, obu_has_extension, obu_type};
pub use common::{AnnexBNalIter, find_start_code};
pub use h264::{AvcConfig, annexb_to_avcc, extract_avc_config, is_h264_keyframe};
pub use h265::{HevcConfig, extract_hevc_config, hevc_annexb_to_hvcc, is_hevc_keyframe};
pub use opus::{OPUS_SAMPLE_RATE, OpusConfig, is_valid_opus_packet, opus_packet_samples};
pub use vp9::{Vp9Config, Vp9Error, extract_vp9_config, is_vp9_keyframe};

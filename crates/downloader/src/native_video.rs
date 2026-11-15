use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use image::{ImageEncoder, codecs::jpeg::JpegEncoder};
use openh264::{
    decoder::Decoder,
    encoder::{BitRate, Encoder, EncoderConfig, FrameRate},
    formats::YUVSource,
    nal_units,
    OpenH264API,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum NativeVideoError {
    #[error("video input is invalid or unsupported")]
    InvalidInput,
    #[error("video codec is not supported for this operation")]
    UnsupportedCodec,
    #[error("video encode was cancelled")]
    Cancelled,
    #[error("video output exceeded the configured size limit")]
    OutputTooLarge,
    #[error("video output path is invalid")]
    InvalidOutput,
    #[error("video processing failed: {0}")]
    Processing(String),
    #[error("video file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// A single YUV420p video frame.
pub struct YuvFrame {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Presentation timestamp in milliseconds.
    pub pts_ms: i64,
}

/// Adapter so openh264 can borrow a YuvFrame.
struct YuvFrameSource<'a>(&'a YuvFrame);

impl YUVSource for YuvFrameSource<'_> {
    fn dimensions(&self) -> (usize, usize) {
        (self.0.width as usize, self.0.height as usize)
    }
    fn y(&self) -> &[u8] {
        &self.0.y
    }
    fn u(&self) -> &[u8] {
        &self.0.u
    }
    fn v(&self) -> &[u8] {
        &self.0.v
    }
    fn strides(&self) -> (usize, usize, usize) {
        (
            self.0.width as usize,
            (self.0.width / 2) as usize,
            (self.0.width / 2) as usize,
        )
    }
}

struct TemporaryOutput(PathBuf);

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub struct NativeVideoProcessor;

impl NativeVideoProcessor {
    /// Encode a sequence of YUV420p frames to H.264 Annex B using OpenH264.
    ///
    /// Returns the number of bytes written to `output`.
    pub fn encode_h264(
        frames: &[YuvFrame],
        fps_num: u32,
        fps_den: u32,
        bitrate_bps: u32,
        output: impl AsRef<Path>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeVideoError> {
        if max_output_bytes == 0 {
            return Err(NativeVideoError::InvalidOutput);
        }
        if frames.is_empty() {
            return Err(NativeVideoError::InvalidInput);
        }

        let width = frames[0].width;
        let height = frames[0].height;

        let fps = if fps_den > 0 {
            fps_num as f32 / fps_den as f32
        } else {
            30.0
        };
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps));

        let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| NativeVideoError::Processing(e.to_string()))?;

        let parent = output
            .as_ref()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let tmp_path = parent.join(format!(".fetchx-h264-{}.tmp", Uuid::new_v4()));
        let _guard = TemporaryOutput(tmp_path.clone());

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        let mut written: u64 = 0;

        for frame in frames {
            if is_cancelled() {
                return Err(NativeVideoError::Cancelled);
            }
            if frame.width != width || frame.height != height {
                return Err(NativeVideoError::InvalidInput);
            }
            let src = YuvFrameSource(frame);
            let bitstream = encoder
                .encode(&src)
                .map_err(|e| NativeVideoError::Processing(e.to_string()))?;

            for layer_index in 0..bitstream.num_layers() {
                let layer = bitstream.layer(layer_index).ok_or_else(|| {
                    NativeVideoError::Processing("invalid H.264 layer".to_owned())
                })?;
                for nal_index in 0..layer.nal_count() {
                    let nal = layer.nal_unit(nal_index).ok_or_else(|| {
                        NativeVideoError::Processing("invalid H.264 NAL unit".to_owned())
                    })?;
                    written = written
                        .checked_add(nal.len() as u64)
                        .ok_or(NativeVideoError::OutputTooLarge)?;
                    if written > max_output_bytes {
                        return Err(NativeVideoError::OutputTooLarge);
                    }
                    file.write_all(nal)?;
                }
            }
        }
        file.flush()?;
        drop(file);

        let size = fs::metadata(&tmp_path)?.len();
        if size > max_output_bytes {
            return Err(NativeVideoError::OutputTooLarge);
        }

        fs::hard_link(&tmp_path, output.as_ref())?;
        Ok(size)
    }

    /// Decode the first H.264 keyframe found after `seek` and write it as JPEG.
    ///
    /// Returns the number of bytes written to `output`.
    pub fn extract_thumbnail(
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        seek: Option<Duration>,
        max_output_bytes: u64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<u64, NativeVideoError> {
        if max_output_bytes == 0 {
            return Err(NativeVideoError::InvalidOutput);
        }

        let input = fs::canonicalize(input.as_ref())?;
        let data = fs::read(&input)?;
        if data.is_empty() {
            return Err(NativeVideoError::InvalidInput);
        }

        // Calculate approximate byte offset for the seek point.
        // We use a rough bytes-per-second estimate to skip ahead.
        let skip_bytes = seek.map(|d| {
            // 1 Mbps rough estimate → 125_000 bytes/s
            let approx = (d.as_secs_f64() * 125_000.0) as usize;
            approx.min(data.len().saturating_sub(1024))
        });

        let start = skip_bytes.unwrap_or(0);
        let slice = &data[start..];

        let mut decoder =
            Decoder::new().map_err(|e| NativeVideoError::Processing(e.to_string()))?;

        // Feed NAL units until we get a decoded frame.
        for nal in nal_units(slice) {
            if is_cancelled() {
                return Err(NativeVideoError::Cancelled);
            }
            match decoder.decode(nal) {
                Ok(Some(yuv)) => {
                    // Convert YUV420p → RGB8
                    let (w, h) = yuv.dimensions();
                    let mut rgb = vec![0u8; w * h * 3];
                    yuv.write_rgb8(&mut rgb);

                    // Encode to JPEG
                    let mut jpeg_bytes: Vec<u8> = Vec::new();
                    let encoder = JpegEncoder::new_with_quality(&mut jpeg_bytes, 85);
                    encoder
                        .write_image(&rgb, w as u32, h as u32, image::ColorType::Rgb8.into())
                        .map_err(|e| NativeVideoError::Processing(e.to_string()))?;

                    let size = jpeg_bytes.len() as u64;
                    if size > max_output_bytes {
                        return Err(NativeVideoError::OutputTooLarge);
                    }

                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(output.as_ref())?;
                    file.write_all(&jpeg_bytes)?;
                    return Ok(size);
                }
                Ok(None) => continue,
                Err(_) => continue, // Skip undecodable NALs
            }
        }

        // If seek left us past all frames, try from the beginning.
        if start > 0 {
            return Self::extract_thumbnail(
                &input,
                output.as_ref(),
                None,
                max_output_bytes,
                is_cancelled,
            );
        }

        Err(NativeVideoError::InvalidInput)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_yuv_frame(width: u32, height: u32, pts_ms: i64) -> YuvFrame {
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 4) as usize;
        YuvFrame {
            y: vec![128u8; y_size],
            u: vec![128u8; uv_size],
            v: vec![128u8; uv_size],
            width,
            height,
            pts_ms,
        }
    }

    #[test]
    fn encode_h264_produces_output() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("out.h264");
        let frames = vec![
            make_yuv_frame(320, 240, 0),
            make_yuv_frame(320, 240, 33),
        ];
        let result = NativeVideoProcessor::encode_h264(
            &frames,
            30,
            1,
            500_000,
            &output,
            10 * 1024 * 1024,
            || false,
        );
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
        assert!(result.unwrap() > 0);
        assert!(output.exists());
    }

    #[test]
    fn encode_h264_respects_size_limit() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("out.h264");
        let frames = vec![make_yuv_frame(320, 240, 0)];
        let result = NativeVideoProcessor::encode_h264(
            &frames,
            30,
            1,
            500_000,
            &output,
            1, // 1 byte limit
            || false,
        );
        assert!(matches!(result, Err(NativeVideoError::OutputTooLarge)));
    }

    #[test]
    fn encode_h264_cancellation() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("out.h264");
        let frames = vec![
            make_yuv_frame(320, 240, 0),
            make_yuv_frame(320, 240, 33),
        ];
        let result = NativeVideoProcessor::encode_h264(
            &frames,
            30,
            1,
            500_000,
            &output,
            10 * 1024 * 1024,
            || true, // immediately cancelled
        );
        assert!(matches!(result, Err(NativeVideoError::Cancelled)));
    }
}

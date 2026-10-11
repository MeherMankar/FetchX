//! Length-prefixed JSON framing for plugin IPC.
//!
//! Uses the same 4-byte little-endian length prefix format as the browser
//! native messaging protocol.  Frames exceeding [`MAX_FRAME_BYTES`] are
//! rejected before allocation.

use std::io::{Read, Write};

use serde::Serialize;
use thiserror::Error;

use crate::protocol::MAX_IPC_PAYLOAD_BYTES;

/// Alias so callers can use this module's name for the limit.
pub const MAX_FRAME_BYTES: u32 = MAX_IPC_PAYLOAD_BYTES;

#[derive(Debug, Error)]
pub enum FramingError {
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("frame length {0} exceeds maximum {MAX_FRAME_BYTES}")]
    FrameTooLarge(u32),
    #[error("serialisation failed: {0}")]
    Serialize(String),
}

/// Reads one length-prefixed frame from `reader`.
///
/// Returns `None` on clean EOF (pipe closed by the other end).
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, FramingError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(FramingError::Io(e)),
    }
    let length = u32::from_le_bytes(len_buf);
    if length == 0 {
        return Ok(Some(Vec::new()));
    }
    if length > MAX_FRAME_BYTES {
        return Err(FramingError::FrameTooLarge(length));
    }
    let mut buf = vec![0u8; length as usize];
    reader.read_exact(&mut buf).map_err(FramingError::Io)?;
    Ok(Some(buf))
}

/// Serialises `value` to JSON and writes it as a length-prefixed frame.
pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), FramingError> {
    let json = serde_json::to_vec(value).map_err(|e| FramingError::Serialize(e.to_string()))?;
    let length = u32::try_from(json.len()).map_err(|_| FramingError::FrameTooLarge(u32::MAX))?;
    if length > MAX_FRAME_BYTES {
        return Err(FramingError::FrameTooLarge(length));
    }
    writer
        .write_all(&length.to_le_bytes())
        .map_err(FramingError::Io)?;
    writer.write_all(&json).map_err(FramingError::Io)?;
    writer.flush().map_err(FramingError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn frame_roundtrip() {
        let value = serde_json::json!({ "version": 1, "type": "ping" });
        let mut buf = Vec::new();
        write_frame(&mut buf, &value).unwrap();

        let len = u32::from_le_bytes(buf[..4].try_into().unwrap());
        assert_eq!(len as usize, buf.len() - 4);

        let frame = read_frame(&mut &buf[..]).unwrap().unwrap();
        let parsed: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn oversized_write_rejected() {
        let giant = vec![42u8; (MAX_FRAME_BYTES + 1) as usize];
        // Build a JSON string that will exceed the limit.
        let s = String::from_utf8(giant).unwrap();
        let value = serde_json::json!({ "data": s });
        let mut buf = Vec::new();
        assert!(matches!(
            write_frame(&mut buf, &value),
            Err(FramingError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn oversized_read_rejected() {
        let len = (MAX_FRAME_BYTES + 1).to_le_bytes();
        let data: Vec<u8> = len.iter().chain(b"x".iter()).cloned().collect();
        assert!(matches!(
            read_frame(&mut &data[..]),
            Err(FramingError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn eof_returns_none() {
        let empty: &[u8] = &[];
        assert!(read_frame(&mut &empty[..]).unwrap().is_none());
    }

    #[test]
    fn truncated_frame_returns_io_error() {
        // Length prefix says 100 bytes but only 4 bytes follow.
        let mut data = 100u32.to_le_bytes().to_vec();
        data.extend_from_slice(b"x");
        assert!(matches!(
            read_frame(&mut &data[..]),
            Err(FramingError::Io(_))
        ));
    }
}

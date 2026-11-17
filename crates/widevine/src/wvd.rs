//! `.wvd` (Widevine Device) file parser.
//!
//! The WVD binary layout (from the WVP2 `util.js` reference implementation):
//!
//! | Offset  | Size   | Field                                     |
//! |---------|--------|-------------------------------------------|
//! | 0–3     | 4      | Magic bytes (any value; first 3 often "WVD") |
//! | 4       | 1      | Device type (1 = generic/Chrome, 2 = Android) |
//! | 5–6     | 2      | Reserved (unused)                         |
//! | 7–8     | 2 (BE) | `private_key_len`                         |
//! | 9…      | `key_len` | PKCS#8 DER private key bytes           |
//! | next 2  | 2 (BE) | `client_id_len`                           |
//! | remaining | `id_len` | ClientIdentification blob             |
//!
//! The 4-byte magic is always present in files produced by pywidevine / WVP2.

use thiserror::Error;

/// Errors that can occur when parsing a `.wvd` file.
#[derive(Clone, Debug, Error)]
pub enum WvdError {
    #[error("WVD file is too short or truncated")]
    TooShort,
    #[error("WVD private key section overflows file")]
    KeyOverflow,
    #[error("WVD client-id section overflows file")]
    IdOverflow,
}

/// Parsed Widevine device loaded from a `.wvd` file.
#[derive(Clone, Debug)]
pub struct WvdDevice {
    /// Device type: 1 = generic/Chrome, 2 = Android.
    pub device_type: u8,
    /// RSA private key in PKCS#8 DER format.
    pub private_key_der: Vec<u8>,
    /// ClientIdentification serialised protobuf blob (opaque to this layer).
    pub client_id_blob: Vec<u8>,
}

/// Parse a `.wvd` binary blob into a [`WvdDevice`].
///
/// The 4-byte magic header is always read and skipped regardless of its value.
/// If the input is shorter than 9 bytes (magic + type + reserved + key-len), returns
/// [`WvdError::TooShort`].
pub fn parse_wvd(bytes: &[u8]) -> Result<WvdDevice, WvdError> {
    // Header: 4 magic + 1 device_type + 2 reserved + 2 private_key_len = 9 bytes minimum
    if bytes.len() < 9 {
        return Err(WvdError::TooShort);
    }

    // byte 4: device type
    let device_type = bytes[4];

    // bytes 7–8: big-endian u16 private key length
    let key_len = u16::from_be_bytes([bytes[7], bytes[8]]) as usize;

    // bytes 9..9+key_len: private key
    let key_start: usize = 9;
    let key_end = key_start
        .checked_add(key_len)
        .ok_or(WvdError::KeyOverflow)?;
    if key_end > bytes.len() {
        return Err(WvdError::KeyOverflow);
    }
    let private_key_der = bytes[key_start..key_end].to_vec();

    // next 2 bytes: big-endian u16 client_id length
    let id_len_start = key_end;
    if id_len_start + 2 > bytes.len() {
        return Err(WvdError::TooShort);
    }
    let id_len = u16::from_be_bytes([bytes[id_len_start], bytes[id_len_start + 1]]) as usize;

    // remaining bytes: client identifier blob
    let id_start = id_len_start + 2;
    let id_end = id_start.checked_add(id_len).ok_or(WvdError::IdOverflow)?;
    if id_end > bytes.len() {
        return Err(WvdError::IdOverflow);
    }
    let client_id_blob = bytes[id_start..id_end].to_vec();

    Ok(WvdDevice {
        device_type,
        private_key_der,
        client_id_blob,
    })
}

impl WvdDevice {
    /// Load and parse a `.wvd` file from the filesystem.
    ///
    /// # Errors
    ///
    /// Returns a [`WvdError`] if the file cannot be read or the binary layout is invalid.
    pub fn from_file(path: &std::path::Path) -> Result<Self, WvdError> {
        let bytes = std::fs::read(path).map_err(|_| WvdError::TooShort)?;
        parse_wvd(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid WVD byte slice:
    /// - 3 magic bytes + 1 version
    /// - 1 byte device_type
    /// - 2 reserved bytes
    /// - 2 BE key length
    /// - key bytes
    /// - 2 BE client_id length
    /// - client_id bytes
    fn make_wvd(device_type: u8, private_key: &[u8], client_id: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"WVD"); // magic
        buf.push(0x01); // version
        buf.push(device_type);
        buf.push(0x00); // reserved
        buf.push(0x00); // reserved
        let key_len = private_key.len() as u16;
        buf.extend_from_slice(&key_len.to_be_bytes());
        buf.extend_from_slice(private_key);
        let id_len = client_id.len() as u16;
        buf.extend_from_slice(&id_len.to_be_bytes());
        buf.extend_from_slice(client_id);
        buf
    }

    #[test]
    fn parses_minimal_wvd() {
        let key = b"private-key-bytes";
        let cid = b"client-id-blob";
        let wvd = make_wvd(2, key, cid);
        let device = parse_wvd(&wvd).expect("should parse");
        assert_eq!(device.device_type, 2);
        assert_eq!(device.private_key_der, key);
        assert_eq!(device.client_id_blob, cid);
    }

    #[test]
    fn parses_device_type_1() {
        let wvd = make_wvd(1, b"k", b"c");
        let device = parse_wvd(&wvd).expect("should parse");
        assert_eq!(device.device_type, 1);
    }

    #[test]
    fn rejects_too_short() {
        assert!(matches!(parse_wvd(&[0u8; 4]), Err(WvdError::TooShort)));
        assert!(matches!(parse_wvd(&[]), Err(WvdError::TooShort)));
    }

    #[test]
    fn rejects_key_overflow() {
        // Header claims key_len = 1000 but no data follows
        let mut buf = vec![b'W', b'V', b'D', 0x01, 0x01, 0x00, 0x00];
        buf.extend_from_slice(&1000u16.to_be_bytes()); // key_len = 1000
        // Only a few bytes of "key"
        buf.extend_from_slice(b"short");
        assert!(matches!(parse_wvd(&buf), Err(WvdError::KeyOverflow)));
    }

    #[test]
    fn rejects_id_overflow() {
        let key = b"k";
        let mut buf = make_wvd(1, key, b"");
        // Corrupt the client_id_len to be huge
        let id_len_offset = 9 + key.len();
        let huge = 9999u16.to_be_bytes();
        buf[id_len_offset] = huge[0];
        buf[id_len_offset + 1] = huge[1];
        assert!(matches!(parse_wvd(&buf), Err(WvdError::IdOverflow)));
    }

    #[test]
    fn empty_key_and_id_are_valid() {
        let wvd = make_wvd(1, b"", b"");
        let device = parse_wvd(&wvd).expect("empty sections are valid");
        assert!(device.private_key_der.is_empty());
        assert!(device.client_id_blob.is_empty());
    }
}

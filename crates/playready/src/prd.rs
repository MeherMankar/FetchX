//! `.prd` device file parsing for pyplayready-compatible device files.
//!
//! Format (pyplayready 0.8.5):
//! - 3 bytes magic: `PRD`
//! - 1 byte version: always `3`
//! - 96 bytes group_key  (ECC-256 private key)
//! - 96 bytes enc_key    (ECC-256 encryption key)
//! - 96 bytes sign_key   (ECC-256 signing key)
//! - 4 bytes big-endian cert_len
//! - cert_len bytes group_certificate (XML)

use thiserror::Error;

/// Errors returned when parsing a `.prd` device file.
#[derive(Debug, Error)]
pub enum PrdError {
    #[error("invalid magic bytes; expected PRD")]
    InvalidMagic,
    #[error("unsupported version {0}; expected 3")]
    UnsupportedVersion(u8),
    #[error("file is too short to contain the required fields")]
    TooShort,
    #[error("I/O error reading .prd file: {0}")]
    Io(#[from] std::io::Error),
}

/// A parsed pyplayready `.prd` device file.
#[derive(Clone, Debug)]
pub struct PrdDevice {
    /// ECC-256 group private key (96 bytes).
    pub group_key: [u8; 96],
    /// ECC-256 encryption key (96 bytes).
    pub enc_key: [u8; 96],
    /// ECC-256 signing key (96 bytes).
    pub sign_key: [u8; 96],
    /// Group certificate XML bytes.
    pub group_certificate: Vec<u8>,
}

impl PrdDevice {
    /// Parse a `.prd` device from raw bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PrdError> {
        // Magic: 3 bytes "PRD"
        if bytes.len() < 3 || &bytes[0..3] != b"PRD" {
            return Err(PrdError::InvalidMagic);
        }

        // Version: 1 byte, must be 3
        if bytes.len() < 4 {
            return Err(PrdError::TooShort);
        }
        let version = bytes[3];
        if version != 3 {
            return Err(PrdError::UnsupportedVersion(version));
        }

        // 3 × 96-byte key fields + 4-byte cert_len = 292 bytes after the 4-byte header
        const HEADER: usize = 4;
        const KEY_LEN: usize = 96;
        const CERT_LEN_OFFSET: usize = HEADER + 3 * KEY_LEN; // 292
        const MIN_LEN: usize = CERT_LEN_OFFSET + 4;           // 296

        if bytes.len() < MIN_LEN {
            return Err(PrdError::TooShort);
        }

        let group_key: [u8; 96] = bytes[HEADER..HEADER + KEY_LEN].try_into().unwrap();
        let enc_key: [u8; 96] = bytes[HEADER + KEY_LEN..HEADER + 2 * KEY_LEN]
            .try_into()
            .unwrap();
        let sign_key: [u8; 96] = bytes[HEADER + 2 * KEY_LEN..HEADER + 3 * KEY_LEN]
            .try_into()
            .unwrap();

        let cert_len = u32::from_be_bytes(
            bytes[CERT_LEN_OFFSET..CERT_LEN_OFFSET + 4]
                .try_into()
                .unwrap(),
        ) as usize;

        let cert_start = CERT_LEN_OFFSET + 4;
        if bytes.len() < cert_start + cert_len {
            return Err(PrdError::TooShort);
        }

        let group_certificate = bytes[cert_start..cert_start + cert_len].to_vec();

        Ok(Self {
            group_key,
            enc_key,
            sign_key,
            group_certificate,
        })
    }

    /// Read and parse a `.prd` device file from disk.
    pub async fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, PrdError> {
        let bytes = tokio::fs::read(path).await?;
        Self::from_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_prd(cert: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"PRD");
        v.push(3);
        v.extend_from_slice(&[0u8; 96]); // group_key
        v.extend_from_slice(&[0u8; 96]); // enc_key
        v.extend_from_slice(&[0u8; 96]); // sign_key
        let cert_len = cert.len() as u32;
        v.extend_from_slice(&cert_len.to_be_bytes());
        v.extend_from_slice(cert);
        v
    }

    #[test]
    fn prd_magic_check() {
        let mut bad = make_valid_prd(b"cert");
        bad[0] = b'X';
        assert!(matches!(PrdDevice::from_bytes(&bad), Err(PrdError::InvalidMagic)));
    }

    #[test]
    fn prd_version_check() {
        let mut bad = make_valid_prd(b"cert");
        bad[3] = 2; // version != 3
        assert!(matches!(
            PrdDevice::from_bytes(&bad),
            Err(PrdError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn prd_field_lengths() {
        // Valid magic and version but truncated (only 10 bytes)
        let mut v = Vec::new();
        v.extend_from_slice(b"PRD");
        v.push(3);
        v.extend_from_slice(&[0u8; 6]);
        assert!(matches!(PrdDevice::from_bytes(&v), Err(PrdError::TooShort)));
    }

    #[test]
    fn prd_parse_success() {
        let cert = b"<GroupCertificate/>";
        let data = make_valid_prd(cert);
        let dev = PrdDevice::from_bytes(&data).expect("should parse");
        assert_eq!(dev.group_certificate, cert);
    }
}

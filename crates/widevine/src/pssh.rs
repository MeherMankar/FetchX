//! PSSH (Protection System Specific Header) box extraction from MPD manifests.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Widevine system ID: ED EF 8B A9 79 D6 4A CE A3 C8 27 DC D5 1D 21 ED
pub const WIDEVINE_SYSTEM_ID: [u8; 16] = [
    0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE, 0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D, 0x21, 0xED,
];

/// A parsed PSSH box.
#[derive(Clone, Debug)]
pub struct PsshBox {
    /// System ID bytes (16 bytes).
    pub system_id: [u8; 16],
    /// Raw PSSH box bytes as found in the manifest.
    pub raw: Vec<u8>,
    /// PSSH data payload (the bytes after the standard PSSH header).
    pub data: Vec<u8>,
}

impl PsshBox {
    /// Parse a raw PSSH box from bytes.
    ///
    /// Returns `None` if the box is too short, has an invalid box type, or cannot be parsed.
    /// Checks the Widevine system ID.
    #[must_use]
    pub fn parse(data: &[u8]) -> Option<Self> {
        // Minimum PSSH box size:
        // 4 (size) + 4 (type "pssh") + 4 (version+flags) + 16 (system_id) + 4 (data_size) = 32
        if data.len() < 32 {
            return None;
        }

        // Box type must be "pssh" (bytes 4–7)
        if &data[4..8] != b"pssh" {
            return None;
        }

        // System ID is at bytes 12–27 (after size+type+version/flags)
        let system_id: [u8; 16] = data[12..28].try_into().ok()?;

        // Only accept Widevine system ID
        if system_id != WIDEVINE_SYSTEM_ID {
            return None;
        }

        // Data length is at bytes 28–31
        let data_len = u32::from_be_bytes(data[28..32].try_into().ok()?) as usize;
        let pssh_data = data.get(32..32 + data_len)?.to_vec();

        Some(PsshBox {
            system_id,
            raw: data.to_vec(),
            data: pssh_data,
        })
    }

    /// Returns `true` if this PSSH box belongs to Widevine.
    #[must_use]
    pub fn is_widevine(&self) -> bool {
        self.system_id == WIDEVINE_SYSTEM_ID
    }
}

/// Extract Widevine PSSH boxes from an MPD manifest XML string.
///
/// Looks for `<cenc:pssh>` and `<mspr:pro>` elements whose base64-decoded
/// contents decode as a valid Widevine PSSH box.
#[must_use]
pub fn extract_pssh_from_mpd(mpd_text: &str) -> Vec<PsshBox> {
    let mut results = Vec::new();

    // Extract values from XML elements matching the given tag names.
    for tag in &["cenc:pssh", "mspr:pro", "pssh"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");

        let mut search = mpd_text;
        while let Some(start_tag) = search.find(open.as_str()) {
            let after_open = &search[start_tag..];
            // Find the end of the opening tag
            let Some(tag_end) = after_open.find('>') else {
                break;
            };
            let content_start = tag_end + 1;
            let content_region = &after_open[content_start..];

            let Some(end_pos) = content_region.find(close.as_str()) else {
                break;
            };

            let b64 = content_region[..end_pos].trim();
            if !b64.is_empty() {
                if let Ok(decoded) = BASE64.decode(b64)
                    && let Some(pssh) = PsshBox::parse(&decoded)
                {
                    results.push(pssh);
                }
            }

            // Advance past what we've processed.
            let consumed = start_tag + content_start + end_pos + close.len();
            search = &search[consumed..];
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_widevine_pssh_box(pssh_data: &[u8]) -> Vec<u8> {
        let data_len = pssh_data.len() as u32;
        let box_size = 32u32 + data_len;
        let mut box_bytes = Vec::new();
        box_bytes.extend_from_slice(&box_size.to_be_bytes());
        box_bytes.extend_from_slice(b"pssh");
        box_bytes.extend_from_slice(&[0u8; 4]); // version + flags
        box_bytes.extend_from_slice(&WIDEVINE_SYSTEM_ID);
        box_bytes.extend_from_slice(&data_len.to_be_bytes());
        box_bytes.extend_from_slice(pssh_data);
        box_bytes
    }

    #[test]
    fn parses_widevine_pssh_box() {
        let pssh_data = b"widevine-init-data";
        let box_bytes = make_widevine_pssh_box(pssh_data);
        let pssh = PsshBox::parse(&box_bytes).expect("should parse");
        assert!(pssh.is_widevine());
        assert_eq!(pssh.data, pssh_data);
    }

    #[test]
    fn rejects_non_widevine_system_id() {
        let mut box_bytes = make_widevine_pssh_box(b"data");
        // Corrupt the system ID
        box_bytes[12] = 0x00;
        assert!(PsshBox::parse(&box_bytes).is_none());
    }

    #[test]
    fn rejects_too_short() {
        assert!(PsshBox::parse(&[0u8; 10]).is_none());
    }

    #[test]
    fn extracts_pssh_from_mpd() {
        let pssh_data = b"widevine-init-data";
        let box_bytes = make_widevine_pssh_box(pssh_data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&box_bytes);
        let mpd = format!(
            r#"<MPD><Period><AdaptationSet><ContentProtection><cenc:pssh>{b64}</cenc:pssh></ContentProtection></AdaptationSet></Period></MPD>"#
        );
        let results = extract_pssh_from_mpd(&mpd);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_widevine());
        assert_eq!(results[0].data, pssh_data);
    }

    #[test]
    fn returns_empty_for_no_pssh() {
        let mpd = r#"<MPD><Period></Period></MPD>"#;
        assert!(extract_pssh_from_mpd(mpd).is_empty());
    }
}

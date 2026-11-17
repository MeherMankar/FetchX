//! PlayReady PSSH box detection and extraction from MPD manifests.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// PlayReady system ID: 9A04F079-9840-4286-AB92-E65BE0885F95
pub const PLAYREADY_SYSTEM_ID: [u8; 16] = [
    0x9A, 0x04, 0xF0, 0x79, 0x98, 0x40, 0x42, 0x86, 0xAB, 0x92, 0xE6, 0x5B, 0xE0, 0x88, 0x5F,
    0x95,
];

/// Returns `true` when `data` looks like a PSSH box whose system ID (bytes 12..28)
/// matches the PlayReady system ID.
#[must_use]
pub fn is_playready_pssh(data: &[u8]) -> bool {
    if data.len() < 28 {
        return false;
    }
    let Ok(id): Result<[u8; 16], _> = data[12..28].try_into() else {
        return false;
    };
    id == PLAYREADY_SYSTEM_ID
}

/// Scan `<cenc:pssh>` and `<mspr:pro>` elements in an MPD XML string, base64-decode
/// each one, and return the first whose bytes 12..28 match the PlayReady system ID.
/// Returns `None` if no matching element is found.
#[must_use]
pub fn extract_pssh_from_mpd(xml: &str) -> Option<Vec<u8>> {
    for tag in &["cenc:pssh", "mspr:pro", "pssh"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");

        let mut search = xml;
        while let Some(start_tag) = search.find(open.as_str()) {
            let after_open = &search[start_tag..];
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
                if let Ok(decoded) = BASE64.decode(b64) {
                    if is_playready_pssh(&decoded) {
                        return Some(decoded);
                    }
                }
            }

            let consumed = start_tag + content_start + end_pos + close.len();
            search = &search[consumed..];
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_playready_pssh_box(pssh_data: &[u8]) -> Vec<u8> {
        let data_len = pssh_data.len() as u32;
        let box_size = 32u32 + data_len;
        let mut v = Vec::new();
        v.extend_from_slice(&box_size.to_be_bytes());
        v.extend_from_slice(b"pssh");
        v.extend_from_slice(&[0u8; 4]); // version + flags
        v.extend_from_slice(&PLAYREADY_SYSTEM_ID);
        v.extend_from_slice(&data_len.to_be_bytes());
        v.extend_from_slice(pssh_data);
        v
    }

    #[test]
    fn pssh_system_id_detection() {
        let pssh = make_playready_pssh_box(b"playready-data");
        assert!(is_playready_pssh(&pssh));
    }

    #[test]
    fn pssh_wrong_system_id_rejected() {
        let mut pssh = make_playready_pssh_box(b"data");
        pssh[12] = 0x00; // corrupt system ID
        assert!(!is_playready_pssh(&pssh));
    }

    #[test]
    fn pssh_too_short_rejected() {
        assert!(!is_playready_pssh(&[0u8; 10]));
    }

    #[test]
    fn mpd_extraction() {
        let pssh_data = b"playready-init-data";
        let box_bytes = make_playready_pssh_box(pssh_data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&box_bytes);
        let mpd = format!(
            r#"<MPD><Period><AdaptationSet><ContentProtection><cenc:pssh>{b64}</cenc:pssh></ContentProtection></AdaptationSet></Period></MPD>"#
        );
        let result = extract_pssh_from_mpd(&mpd);
        assert!(result.is_some());
        let raw = result.unwrap();
        assert!(is_playready_pssh(&raw));
    }

    #[test]
    fn mpd_extraction_none_when_missing() {
        let mpd = r#"<MPD><Period></Period></MPD>"#;
        assert!(extract_pssh_from_mpd(mpd).is_none());
    }
}

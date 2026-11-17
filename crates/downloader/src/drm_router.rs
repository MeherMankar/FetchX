//! Routes PSSH bytes to the correct DRM provider based on the embedded system ID.

pub const WIDEVINE_SYSTEM_ID: [u8; 16] = [
    0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE, 0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D, 0x21, 0xED,
];

pub const PLAYREADY_SYSTEM_ID: [u8; 16] = [
    0x9A, 0x04, 0xF0, 0x79, 0x98, 0x40, 0x42, 0x86, 0xAB, 0x92, 0xE6, 0x5B, 0xE0, 0x88, 0x5F, 0x95,
];

/// Identify the DRM system from a raw PSSH box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmSystem {
    Widevine,
    PlayReady,
    Unknown,
}

/// Inspect bytes 12..28 of a PSSH box to identify the DRM system.
pub fn detect_drm_system(pssh: &[u8]) -> DrmSystem {
    if pssh.len() < 28 {
        return DrmSystem::Unknown;
    }
    let system_id: &[u8; 16] = match pssh[12..28].try_into() {
        Ok(id) => id,
        Err(_) => return DrmSystem::Unknown,
    };
    if *system_id == WIDEVINE_SYSTEM_ID {
        DrmSystem::Widevine
    } else if *system_id == PLAYREADY_SYSTEM_ID {
        DrmSystem::PlayReady
    } else {
        DrmSystem::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pssh(system_id: &[u8; 16]) -> Vec<u8> {
        let mut v = vec![0u8; 32];
        v[4..8].copy_from_slice(b"pssh");
        v[12..28].copy_from_slice(system_id);
        v
    }

    #[test]
    fn detects_widevine() {
        let pssh = make_pssh(&WIDEVINE_SYSTEM_ID);
        assert_eq!(detect_drm_system(&pssh), DrmSystem::Widevine);
    }

    #[test]
    fn detects_playready() {
        let pssh = make_pssh(&PLAYREADY_SYSTEM_ID);
        assert_eq!(detect_drm_system(&pssh), DrmSystem::PlayReady);
    }

    #[test]
    fn detects_unknown() {
        let pssh = make_pssh(&[0xAA; 16]);
        assert_eq!(detect_drm_system(&pssh), DrmSystem::Unknown);
    }

    #[test]
    fn too_short_is_unknown() {
        assert_eq!(detect_drm_system(&[0u8; 10]), DrmSystem::Unknown);
    }
}

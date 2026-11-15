// Adapted from muxide (MIT OR Apache-2.0)
//! Common bitstream utilities for start code scanning.
//!
//! Provides shared infrastructure for parsing Annex B formatted bitstreams
//! (H.264, H.265) which use start code delimiters.

/// Iterator over NAL units in an Annex B bitstream.
///
/// Annex B uses start codes (`0x00 0x00 0x01` or `0x00 0x00 0x00 0x01`) to
/// delimit NAL units. This iterator yields each NAL unit's payload without
/// the start code prefix.
pub struct AnnexBNalIter<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> AnnexBNalIter<'a> {
    /// Create a new iterator over NAL units in the given Annex B data.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }
}

impl<'a> Iterator for AnnexBNalIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let (start_code_pos, start_code_len) = find_start_code(self.data, self.cursor)?;
        let nal_start = start_code_pos + start_code_len;

        let nal_end = match find_start_code(self.data, nal_start) {
            Some((next_pos, _)) => next_pos,
            None => self.data.len(),
        };

        self.cursor = nal_end;
        Some(&self.data[nal_start..nal_end])
    }
}

/// Find the next Annex B start code in the data starting from `from`.
///
/// Returns the position and length of the start code:
/// - 4-byte: `0x00 0x00 0x00 0x01` (length = 4)
/// - 3-byte: `0x00 0x00 0x01` (length = 3)
pub fn find_start_code(data: &[u8], from: usize) -> Option<(usize, usize)> {
    if data.len() < 3 || from >= data.len() {
        return None;
    }

    let mut i = from;
    while i + 3 <= data.len() {
        if i + 4 <= data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            return Some((i, 4));
        }
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            return Some((i, 3));
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_start_code_4byte() {
        let data = [0x00, 0x00, 0x00, 0x01, 0x67];
        assert_eq!(find_start_code(&data, 0), Some((0, 4)));
    }

    #[test]
    fn test_find_start_code_3byte() {
        let data = [0x00, 0x00, 0x01, 0x67];
        assert_eq!(find_start_code(&data, 0), Some((0, 3)));
    }

    #[test]
    fn test_find_start_code_offset() {
        let data = [0xAB, 0xCD, 0x00, 0x00, 0x00, 0x01, 0x67];
        assert_eq!(find_start_code(&data, 0), Some((2, 4)));
        assert_eq!(find_start_code(&data, 3), Some((3, 3)));
        assert_eq!(find_start_code(&data, 6), None);
    }

    #[test]
    fn test_find_start_code_none() {
        let data = [0x00, 0x00, 0x02, 0x67];
        assert_eq!(find_start_code(&data, 0), None);
    }

    #[test]
    fn test_annexb_iter_multiple_nals() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x00, 0x00,
            0x00, 0x01, 0x65, 0x88,
        ];

        let nals: Vec<_> = AnnexBNalIter::new(&data).collect();
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0], &[0x67, 0x42]);
        assert_eq!(nals[1], &[0x68, 0xCE]);
        assert_eq!(nals[2], &[0x65, 0x88]);
    }

    #[test]
    fn test_annexb_iter_empty() {
        let data: [u8; 0] = [];
        let nals: Vec<_> = AnnexBNalIter::new(&data).collect();
        assert!(nals.is_empty());
    }

    #[test]
    fn test_annexb_iter_no_start_code() {
        let data = [0x67, 0x42, 0x00, 0x1f];
        let nals: Vec<_> = AnnexBNalIter::new(&data).collect();
        assert!(nals.is_empty());
    }

    #[test]
    fn test_annexb_iter_mixed_start_codes() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x68, 0xCE,
        ];

        let nals: Vec<_> = AnnexBNalIter::new(&data).collect();
        assert_eq!(nals.len(), 2);
        assert_eq!(nals[0], &[0x67, 0x42]);
        assert_eq!(nals[1], &[0x68, 0xCE]);
    }
}

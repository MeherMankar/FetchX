//! Programmatic format selection for [`crate::MediaInfo`] format lists.
//!
//! All selection logic is pure — no I/O, no network.

use crate::models::MediaFormat;

/// How to select a format from a [`crate::MediaInfo`]'s format list.
#[derive(Clone, Debug, Default)]
pub enum SelectionCriteria {
    /// Best overall quality (highest bitrate, tiebreak by height; prefers
    /// combined AV).
    #[default]
    Best,
    /// Best video-only format (has `video_codec`, no `audio_codec`).
    BestVideoOnly,
    /// Best audio-only format (has `audio_codec`, no `video_codec`).
    BestAudioOnly,
    /// Best combined AV format (has both codecs).
    BestCombined,
    /// Closest format at or below the requested height in pixels.
    /// Falls back to [`SelectionCriteria::Best`] if no format matches.
    MaxHeight(u32),
    /// Select by exact `format_id` (as returned by extraction).
    FormatId(String),
}

/// Pure format selector.
pub struct FormatSelector;

impl FormatSelector {
    /// Select the best matching format from `formats` according to `criteria`.
    ///
    /// Returns `None` only when:
    /// - `formats` is empty, OR
    /// - `criteria` is [`SelectionCriteria::FormatId`] and no format has that id.
    ///
    /// All other criteria have a fallback.
    pub fn select<'a>(
        formats: &'a [MediaFormat],
        criteria: &SelectionCriteria,
    ) -> Option<&'a MediaFormat> {
        if formats.is_empty() {
            return None;
        }
        match criteria {
            SelectionCriteria::Best => Self::best(formats),
            SelectionCriteria::BestVideoOnly => {
                let candidates: Vec<_> = formats
                    .iter()
                    .filter(|f| f.video_codec.is_some() && f.audio_codec.is_none())
                    .collect();
                Self::best_from(candidates).or_else(|| Self::best(formats))
            }
            SelectionCriteria::BestAudioOnly => {
                let candidates: Vec<_> = formats
                    .iter()
                    .filter(|f| f.audio_codec.is_some() && f.video_codec.is_none())
                    .collect();
                Self::best_from(candidates).or_else(|| Self::best(formats))
            }
            SelectionCriteria::BestCombined => {
                let candidates: Vec<_> = formats
                    .iter()
                    .filter(|f| f.video_codec.is_some() && f.audio_codec.is_some())
                    .collect();
                Self::best_from(candidates).or_else(|| Self::best(formats))
            }
            SelectionCriteria::MaxHeight(max_h) => {
                let candidates: Vec<_> = formats
                    .iter()
                    .filter(|f| f.height.is_some_and(|h| h <= *max_h))
                    .collect();
                if candidates.is_empty() {
                    Self::best(formats)
                } else {
                    // Pick the one with the highest height at or below the limit.
                    candidates.into_iter().max_by_key(|f| f.height.unwrap_or(0))
                }
            }
            SelectionCriteria::FormatId(id) => formats.iter().find(|f| &f.id == id),
        }
    }

    /// Pick the format with the highest bitrate (tiebreak: highest height).
    /// If all bitrates are `None`, returns the last format (yt-dlp puts best last).
    fn best(formats: &[MediaFormat]) -> Option<&MediaFormat> {
        Self::best_from(formats.iter().collect())
    }

    fn best_from(mut candidates: Vec<&MediaFormat>) -> Option<&MediaFormat> {
        if candidates.is_empty() {
            return None;
        }
        // If any candidate has a bitrate, rank by bitrate desc then height desc.
        if candidates.iter().any(|f| f.bitrate.is_some()) {
            candidates.sort_by(|a, b| {
                let br = b.bitrate.cmp(&a.bitrate);
                if br != std::cmp::Ordering::Equal {
                    return br;
                }
                b.height.cmp(&a.height)
            });
            candidates.into_iter().next()
        } else {
            // All bitrates are None — return the last (yt-dlp convention: best last).
            candidates.into_iter().last()
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience free function
// ---------------------------------------------------------------------------

/// Convenience wrapper: select a format using the simpler [`FormatPreference`]
/// enum used in the pipeline tests.
///
/// This is a thin adapter; prefer [`FormatSelector::select`] with
/// [`SelectionCriteria`] in production code.
pub fn select_format<'a>(
    formats: &'a [MediaFormat],
    preference: &FormatPreference,
) -> Option<&'a MediaFormat> {
    FormatSelector::select(formats, &preference.to_criteria())
}

/// Simple format-preference enum for use in integration tests and
/// pipeline code.
#[derive(Clone, Debug)]
pub enum FormatPreference {
    BestCombined,
    BestVideo,
    BestAudio,
    FormatId(String),
    MaxHeight(u32),
}

impl FormatPreference {
    fn to_criteria(&self) -> SelectionCriteria {
        match self {
            Self::BestCombined => SelectionCriteria::BestCombined,
            Self::BestVideo => SelectionCriteria::BestVideoOnly,
            Self::BestAudio => SelectionCriteria::BestAudioOnly,
            Self::FormatId(id) => SelectionCriteria::FormatId(id.clone()),
            Self::MaxHeight(h) => SelectionCriteria::MaxHeight(*h),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaProtocol;

    fn make_format(
        id: &str,
        bitrate: Option<u64>,
        height: Option<u32>,
        video_codec: Option<&str>,
        audio_codec: Option<&str>,
    ) -> MediaFormat {
        MediaFormat {
            id: id.to_owned(),
            ext: "mp4".to_owned(),
            protocol: MediaProtocol::Http,
            url: format!("https://cdn.example.com/{id}.mp4"),
            label: id.to_owned(),
            video_codec: video_codec.map(str::to_owned),
            audio_codec: audio_codec.map(str::to_owned),
            width: height.map(|h| h * 16 / 9),
            height,
            fps: None,
            bitrate,
            filesize: None,
            required_headers: vec![],
        }
    }

    #[test]
    fn test_format_selector_best() {
        let formats = vec![
            make_format(
                "low",
                Some(1_000_000),
                Some(480),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "mid",
                Some(3_000_000),
                Some(720),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "high",
                Some(5_000_000),
                Some(1080),
                Some("avc1"),
                Some("mp4a"),
            ),
        ];
        let selected = FormatSelector::select(&formats, &SelectionCriteria::Best).unwrap();
        assert_eq!(selected.id, "high");
    }

    #[test]
    fn test_format_selector_video_only() {
        let formats = vec![
            make_format("av", Some(3_000_000), Some(720), Some("avc1"), Some("mp4a")),
            make_format("vo", Some(4_000_000), Some(1080), Some("avc1"), None),
        ];
        let selected = FormatSelector::select(&formats, &SelectionCriteria::BestVideoOnly).unwrap();
        assert_eq!(selected.id, "vo");
    }

    #[test]
    fn test_format_selector_audio_only() {
        let formats = vec![
            make_format("av", Some(3_000_000), Some(720), Some("avc1"), Some("mp4a")),
            make_format("ao", Some(128_000), None, None, Some("mp4a")),
            make_format("ao2", Some(256_000), None, None, Some("opus")),
        ];
        let selected = FormatSelector::select(&formats, &SelectionCriteria::BestAudioOnly).unwrap();
        assert_eq!(selected.id, "ao2");
    }

    #[test]
    fn test_format_selector_combined() {
        let formats = vec![
            make_format("vo", Some(4_000_000), Some(1080), Some("avc1"), None),
            make_format("ao", Some(128_000), None, None, Some("mp4a")),
            make_format("av", Some(3_000_000), Some(720), Some("avc1"), Some("mp4a")),
        ];
        let selected = FormatSelector::select(&formats, &SelectionCriteria::BestCombined).unwrap();
        assert_eq!(selected.id, "av");
    }

    #[test]
    fn test_format_selector_max_height_exact() {
        let formats = vec![
            make_format(
                "480p",
                Some(1_000_000),
                Some(480),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "720p",
                Some(2_000_000),
                Some(720),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "1080p",
                Some(4_000_000),
                Some(1080),
                Some("avc1"),
                Some("mp4a"),
            ),
        ];
        let selected =
            FormatSelector::select(&formats, &SelectionCriteria::MaxHeight(720)).unwrap();
        assert_eq!(selected.id, "720p");
    }

    #[test]
    fn test_format_selector_max_height_falls_back() {
        let formats = vec![
            make_format(
                "1080p",
                Some(4_000_000),
                Some(1080),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "720p",
                Some(2_000_000),
                Some(720),
                Some("avc1"),
                Some("mp4a"),
            ),
        ];
        // No format at or below 480p — fallback to Best.
        let selected =
            FormatSelector::select(&formats, &SelectionCriteria::MaxHeight(480)).unwrap();
        // Best is 1080p.
        assert_eq!(selected.id, "1080p");
    }

    #[test]
    fn test_format_selector_format_id_found() {
        let formats = vec![
            make_format(
                "fmt1",
                Some(1_000_000),
                Some(480),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "fmt2",
                Some(3_000_000),
                Some(720),
                Some("avc1"),
                Some("mp4a"),
            ),
        ];
        let selected =
            FormatSelector::select(&formats, &SelectionCriteria::FormatId("fmt2".to_owned()))
                .unwrap();
        assert_eq!(selected.id, "fmt2");
    }

    #[test]
    fn test_format_selector_format_id_not_found() {
        let formats = vec![make_format(
            "fmt1",
            Some(1_000_000),
            Some(480),
            Some("avc1"),
            Some("mp4a"),
        )];
        let result =
            FormatSelector::select(&formats, &SelectionCriteria::FormatId("missing".to_owned()));
        assert!(result.is_none());
    }

    #[test]
    fn test_format_selector_empty_formats() {
        let formats: Vec<MediaFormat> = vec![];
        for criteria in [
            SelectionCriteria::Best,
            SelectionCriteria::BestVideoOnly,
            SelectionCriteria::BestAudioOnly,
            SelectionCriteria::BestCombined,
            SelectionCriteria::MaxHeight(720),
            SelectionCriteria::FormatId("x".into()),
        ] {
            assert!(
                FormatSelector::select(&formats, &criteria).is_none(),
                "should return None for empty formats with {criteria:?}"
            );
        }
    }

    #[test]
    fn test_format_selector_malformed_no_bitrate() {
        // All bitrates are None — should return the last format (yt-dlp convention).
        let formats = vec![
            make_format("first", None, Some(480), Some("avc1"), Some("mp4a")),
            make_format("second", None, Some(720), Some("avc1"), Some("mp4a")),
            make_format("last", None, Some(1080), Some("avc1"), Some("mp4a")),
        ];
        let selected = FormatSelector::select(&formats, &SelectionCriteria::Best).unwrap();
        assert_eq!(selected.id, "last");
    }

    // --- FormatPreference convenience wrapper ---

    #[test]
    fn test_select_best_combined_picks_highest_bitrate() {
        let formats = vec![
            make_format("lo", Some(1_000_000), Some(480), Some("avc1"), Some("mp4a")),
            make_format(
                "hi",
                Some(5_000_000),
                Some(1080),
                Some("avc1"),
                Some("mp4a"),
            ),
        ];
        let selected = select_format(&formats, &FormatPreference::BestCombined).unwrap();
        assert_eq!(selected.id, "hi");
    }

    #[test]
    fn test_select_best_video_picks_video_only() {
        let formats = vec![
            make_format("av", Some(3_000_000), Some(720), Some("avc1"), Some("mp4a")),
            make_format("vo", Some(4_000_000), Some(1080), Some("avc1"), None),
        ];
        let selected = select_format(&formats, &FormatPreference::BestVideo).unwrap();
        assert_eq!(selected.id, "vo");
    }

    #[test]
    fn test_select_best_audio_picks_audio_only() {
        let formats = vec![
            make_format("av", Some(3_000_000), Some(720), Some("avc1"), Some("mp4a")),
            make_format("ao", Some(256_000), None, None, Some("opus")),
        ];
        let selected = select_format(&formats, &FormatPreference::BestAudio).unwrap();
        assert_eq!(selected.id, "ao");
    }

    #[test]
    fn test_select_by_format_id() {
        let formats = vec![
            make_format("a", Some(1_000_000), Some(480), Some("avc1"), Some("mp4a")),
            make_format("b", Some(3_000_000), Some(720), Some("avc1"), Some("mp4a")),
        ];
        let selected = select_format(&formats, &FormatPreference::FormatId("a".into())).unwrap();
        assert_eq!(selected.id, "a");
    }

    #[test]
    fn test_select_max_height_picks_correct_resolution() {
        let formats = vec![
            make_format(
                "480p",
                Some(1_000_000),
                Some(480),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "720p",
                Some(2_000_000),
                Some(720),
                Some("avc1"),
                Some("mp4a"),
            ),
            make_format(
                "1080p",
                Some(4_000_000),
                Some(1080),
                Some("avc1"),
                Some("mp4a"),
            ),
        ];
        let selected = select_format(&formats, &FormatPreference::MaxHeight(720)).unwrap();
        assert_eq!(selected.id, "720p");
    }

    #[test]
    fn test_select_returns_none_when_no_match() {
        let formats = vec![make_format(
            "a",
            Some(1_000_000),
            Some(480),
            Some("avc1"),
            Some("mp4a"),
        )];
        let result = select_format(&formats, &FormatPreference::FormatId("missing".into()));
        assert!(result.is_none());
    }

    #[test]
    fn test_select_handles_empty_formats() {
        let formats: Vec<MediaFormat> = vec![];
        assert!(select_format(&formats, &FormatPreference::BestCombined).is_none());
    }
}

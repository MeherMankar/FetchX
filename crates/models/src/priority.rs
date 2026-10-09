use serde::{Deserialize, Serialize};

/// Relative scheduling priority; declaration order defines the explicit ordering.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum DownloadPriority {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "normal")]
    #[default]
    Normal,
    #[serde(rename = "high")]
    High,
}

#[cfg(test)]
mod tests {
    use super::DownloadPriority;

    #[test]
    fn priorities_have_stable_serialization_and_ordering() -> Result<(), serde_json::Error> {
        assert_eq!(serde_json::to_string(&DownloadPriority::Low)?, "\"low\"");
        assert_eq!(
            serde_json::to_string(&DownloadPriority::Normal)?,
            "\"normal\""
        );
        assert_eq!(serde_json::to_string(&DownloadPriority::High)?, "\"high\"");

        assert!(DownloadPriority::Low < DownloadPriority::Normal);
        assert!(DownloadPriority::Normal < DownloadPriority::High);

        Ok(())
    }
}

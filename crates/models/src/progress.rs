use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Progress values independent of any particular download engine.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "DownloadProgressRepr")]
pub struct DownloadProgress {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    speed_bytes_per_second: Option<u64>,
    estimated_remaining_seconds: Option<u64>,
}

impl DownloadProgress {
    pub fn new(
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        speed_bytes_per_second: Option<u64>,
        estimated_remaining_seconds: Option<u64>,
    ) -> Result<Self, DownloadProgressError> {
        if let Some(total_bytes) = total_bytes {
            if downloaded_bytes > total_bytes {
                return Err(DownloadProgressError::DownloadedExceedsTotal {
                    downloaded_bytes,
                    total_bytes,
                });
            }
        }

        Ok(Self {
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_second,
            estimated_remaining_seconds,
        })
    }

    #[must_use]
    pub const fn downloaded_bytes(&self) -> u64 {
        self.downloaded_bytes
    }

    #[must_use]
    pub const fn total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }

    #[must_use]
    pub const fn speed_bytes_per_second(&self) -> Option<u64> {
        self.speed_bytes_per_second
    }

    #[must_use]
    pub const fn estimated_remaining_seconds(&self) -> Option<u64> {
        self.estimated_remaining_seconds
    }

    /// Returns progress in basis points (0–10,000), or `None` if the total is unknown/zero.
    #[must_use]
    pub fn percentage_basis_points(&self) -> Option<u16> {
        let total_bytes = self.total_bytes.filter(|total| *total != 0)?;
        let basis_points = (u128::from(self.downloaded_bytes) * 10_000) / u128::from(total_bytes);
        u16::try_from(basis_points).ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DownloadProgressError {
    #[error("downloaded bytes ({downloaded_bytes}) exceed total bytes ({total_bytes})")]
    DownloadedExceedsTotal {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
}

#[derive(Deserialize)]
struct DownloadProgressRepr {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    speed_bytes_per_second: Option<u64>,
    estimated_remaining_seconds: Option<u64>,
}

impl TryFrom<DownloadProgressRepr> for DownloadProgress {
    type Error = DownloadProgressError;

    fn try_from(value: DownloadProgressRepr) -> Result<Self, Self::Error> {
        Self::new(
            value.downloaded_bytes,
            value.total_bytes,
            value.speed_bytes_per_second,
            value.estimated_remaining_seconds,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{DownloadProgress, DownloadProgressError};

    #[test]
    fn calculates_progress_without_floating_point() -> Result<(), DownloadProgressError> {
        let progress = DownloadProgress::new(1, Some(3), Some(10), Some(1))?;

        assert_eq!(progress.percentage_basis_points(), Some(3_333));
        assert_eq!(progress.downloaded_bytes(), 1);
        assert_eq!(progress.speed_bytes_per_second(), Some(10));
        assert_eq!(progress.estimated_remaining_seconds(), Some(1));
        Ok(())
    }

    #[test]
    fn accepts_unknown_total_and_zero_total() -> Result<(), DownloadProgressError> {
        let unknown = DownloadProgress::new(50, None, None, None)?;
        assert_eq!(unknown.total_bytes(), None);
        assert_eq!(unknown.percentage_basis_points(), None);

        let empty = DownloadProgress::new(0, Some(0), None, None)?;
        assert_eq!(empty.percentage_basis_points(), None);
        Ok(())
    }

    #[test]
    fn rejects_downloaded_bytes_above_known_total() {
        assert_eq!(
            DownloadProgress::new(11, Some(10), None, None),
            Err(DownloadProgressError::DownloadedExceedsTotal {
                downloaded_bytes: 11,
                total_bytes: 10,
            })
        );
    }

    #[test]
    fn serde_deserialization_enforces_progress_invariant() -> Result<(), serde_json::Error> {
        let progress =
            DownloadProgress::new(10, Some(10), None, None).map_err(serde::de::Error::custom)?;
        let serialized = serde_json::to_string(&progress)?;
        let deserialized: DownloadProgress = serde_json::from_str(&serialized)?;

        assert_eq!(progress, deserialized);
        assert!(serde_json::from_str::<DownloadProgress>(
            r#"{"downloaded_bytes":11,"total_bytes":10,"speed_bytes_per_second":null,"estimated_remaining_seconds":null}"#
        )
        .is_err());

        Ok(())
    }
}

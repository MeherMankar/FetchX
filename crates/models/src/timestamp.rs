use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A timestamp represented as milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    #[must_use]
    pub const fn from_unix_millis(milliseconds: u64) -> Self {
        Self(milliseconds)
    }

    /// Returns the current time without requiring an external date/time dependency.
    pub fn now() -> Result<Self, TimestampError> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TimestampError::BeforeUnixEpoch)?;
        let milliseconds =
            u64::try_from(duration.as_millis()).map_err(|_| TimestampError::OutOfRange)?;

        Ok(Self(milliseconds))
    }

    #[must_use]
    pub const fn as_unix_millis(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TimestampError {
    #[error("system time is before the Unix epoch")]
    BeforeUnixEpoch,
    #[error("system time is outside the supported timestamp range")]
    OutOfRange,
}

#[cfg(test)]
mod tests {
    use super::Timestamp;

    #[test]
    fn timestamp_serializes_as_unix_milliseconds() -> Result<(), serde_json::Error> {
        let timestamp = Timestamp::from_unix_millis(1_725_000_000_123);

        assert_eq!(serde_json::to_string(&timestamp)?, "1725000000123");
        let deserialized: Timestamp = serde_json::from_str("1725000000123")?;
        assert_eq!(timestamp, deserialized);

        Ok(())
    }
}

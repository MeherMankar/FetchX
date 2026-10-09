use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Opaque identifier for a download.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DownloadId(Uuid);

impl DownloadId {
    /// Creates a globally unique identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parses the canonical UUID string representation without exposing UUID internals.
    pub fn parse(value: &str) -> Result<Self, DownloadIdParseError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| DownloadIdParseError)
    }
}

impl fmt::Display for DownloadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for DownloadId {
    type Err = DownloadIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Default for DownloadId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("download ID is not a valid UUID")]
pub struct DownloadIdParseError;

#[cfg(test)]
mod tests {
    use super::{DownloadId, DownloadIdParseError};

    #[test]
    fn new_ids_are_distinct_and_serde_round_trips() -> Result<(), serde_json::Error> {
        let id = DownloadId::new();
        let serialized = serde_json::to_string(&id)?;
        let deserialized: DownloadId = serde_json::from_str(&serialized)?;

        assert_eq!(id, deserialized);
        assert_ne!(id, DownloadId::new());
        assert_eq!(id.to_string().parse::<DownloadId>(), Ok(id));
        assert_eq!("invalid".parse::<DownloadId>(), Err(DownloadIdParseError));
        Ok(())
    }
}

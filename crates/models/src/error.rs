use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable, engine-independent category for a download failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DownloadErrorCode {
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "destination")]
    Destination,
    #[serde(rename = "engine")]
    Engine,
    #[serde(rename = "internal")]
    Internal,
}

/// A persistence- and UI-safe summary of a failed download.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "DownloadErrorRepr")]
pub struct DownloadError {
    code: DownloadErrorCode,
    message: String,
}

impl DownloadError {
    pub fn new(
        code: DownloadErrorCode,
        message: impl Into<String>,
    ) -> Result<Self, DownloadErrorMessageError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(DownloadErrorMessageError);
        }

        Ok(Self { code, message })
    }

    #[must_use]
    pub const fn code(&self) -> DownloadErrorCode {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Deserialize)]
struct DownloadErrorRepr {
    code: DownloadErrorCode,
    message: String,
}

impl TryFrom<DownloadErrorRepr> for DownloadError {
    type Error = DownloadErrorMessageError;

    fn try_from(value: DownloadErrorRepr) -> Result<Self, Self::Error> {
        Self::new(value.code, value.message)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("download error message must not be empty")]
pub struct DownloadErrorMessageError;

#[cfg(test)]
mod tests {
    use super::{DownloadError, DownloadErrorCode};

    #[test]
    fn error_has_stable_machine_readable_code() -> Result<(), serde_json::Error> {
        let error = DownloadError::new(DownloadErrorCode::Engine, "engine failed")
            .map_err(serde::de::Error::custom)?;
        let serialized = serde_json::to_string(&error)?;

        assert_eq!(serialized, r#"{"code":"engine","message":"engine failed"}"#);
        let deserialized: DownloadError = serde_json::from_str(&serialized)?;
        assert_eq!(error, deserialized);

        Ok(())
    }

    #[test]
    fn rejects_empty_error_messages() {
        assert!(DownloadError::new(DownloadErrorCode::Source, "  ").is_err());
    }
}

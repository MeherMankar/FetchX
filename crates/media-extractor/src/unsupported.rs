//! [`UnsupportedExtractor`] — always returns [`ExtractionError::UnsupportedUrl`].

use async_trait::async_trait;

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    models::MediaInfo,
};

/// A zero-configuration stub extractor that always fails.
///
/// Used in tests and as a placeholder when no real extractor is configured.
/// `can_handle` always returns `false` so it will never be selected by
/// [`crate::ExtractorRegistry`] in normal operation.
pub struct UnsupportedExtractor;

#[async_trait]
impl MediaExtractor for UnsupportedExtractor {
    fn can_handle(&self, _url: &str) -> bool {
        false
    }

    async fn extract(
        &self,
        _url: &str,
        _opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        Err(ExtractionError::UnsupportedUrl)
    }
}

//! [`ExtractorRegistry`] — ordered list of [`MediaExtractor`]s tried at runtime.

use std::sync::Arc;

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    models::MediaInfo,
    validation::is_safe_media_url,
};

/// Ordered registry of [`MediaExtractor`] implementations.
///
/// Extractors are tried in insertion order. The first extractor whose
/// [`MediaExtractor::can_handle`] returns `true` is used; there is no fallback
/// to later extractors on failure.
///
/// The registry validates the URL with [`is_safe_media_url`] before delegating
/// to any extractor.
pub struct ExtractorRegistry {
    extractors: Vec<Arc<dyn MediaExtractor>>,
}

impl ExtractorRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            extractors: Vec::new(),
        }
    }

    /// Register an extractor. Extractors are tried in registration order.
    pub fn register(&mut self, extractor: Arc<dyn MediaExtractor>) -> &mut Self {
        self.extractors.push(extractor);
        self
    }

    /// Returns a reference to the first extractor that can handle `url`, or
    /// `None` if none can.
    pub fn find(&self, url: &str) -> Option<&Arc<dyn MediaExtractor>> {
        self.extractors.iter().find(|e| e.can_handle(url))
    }

    /// Extract formats for `url`, routing to the first matching extractor.
    ///
    /// Validates the URL with [`is_safe_media_url`] before delegating.
    pub async fn extract(
        &self,
        url: &str,
        opts: ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        is_safe_media_url(url)?;
        let extractor = self.find(url).ok_or(ExtractionError::UnsupportedUrl)?;
        extractor.extract(url, &opts).await
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

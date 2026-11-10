//! [`ExtractorRegistry`] — ordered list of [`MediaExtractor`]s tried at runtime.

use std::sync::Arc;

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    models::MediaInfo,
    validation::is_safe_media_url,
};

/// Ordered registry of [`MediaExtractor`] implementations.
///
/// Extractors are tried in insertion order. An explicit
/// [`ExtractionError::UnsupportedUrl`] result advances to the next matching
/// extractor; other failures are surfaced without fallback.
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
    /// Validates the input URL with [`is_safe_media_url`] before delegating,
    /// then enforces per-format URL/header safety on the returned
    /// [`MediaInfo`] via [`MediaInfo::sanitise_format_urls`] regardless of
    /// which extractor produced it. This keeps the registry/core boundary
    /// protected even when an extractor does not perform inline sanitisation.
    ///
    /// If sanitisation removes every format, [`ExtractionError::NoFormats`] is
    /// returned.
    pub async fn extract(
        &self,
        url: &str,
        opts: ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        is_safe_media_url(url)?;
        for extractor in self.extractors.iter().filter(|e| e.can_handle(url)) {
            match extractor.extract(url, &opts).await {
                Ok(mut info) => {
                    // Enforce per-format SSRF/header safety at the registry boundary so the
                    // invariant holds regardless of which extractor produced `info`.
                    info.sanitise_format_urls();
                    if info.formats.is_empty() {
                        return Err(ExtractionError::NoFormats);
                    }
                    return Ok(info);
                }
                Err(ExtractionError::UnsupportedUrl) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(ExtractionError::UnsupportedUrl)
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

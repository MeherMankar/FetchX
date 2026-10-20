//! Security utilities for the media extraction pipeline.
//!
//! - [`CookieTempFile`]: RAII guard for a Netscape-format cookie temp file.
//! - [`filter_safe_headers`]: validates and filters headers before forwarding.
//! - [`validate_extracted_url`]: SSRF guard applied to format URLs from yt-dlp.

use std::io::Write;

use crate::extractor::ExtractionError;
use crate::validation::is_safe_media_url;

/// Header names that must never be forwarded to the download engine.
const BLOCKED_HEADER_NAMES: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "www-authenticate",
    "x-forwarded-for",
    "x-real-ip",
];

/// Maximum size of a cookie string accepted by [`CookieTempFile::write`].
const MAX_COOKIE_BYTES: usize = 65_536;

// ---------------------------------------------------------------------------
// Cookie temp file
// ---------------------------------------------------------------------------

/// RAII guard that creates a Netscape-format cookie temp file on construction
/// and deletes it when dropped (even on panic).
///
/// # Security
/// - Never logs cookie content.
/// - Limits accepted cookie strings to 65 536 bytes.
/// - Uses [`tempfile::NamedTempFile`] whose `Drop` impl handles deletion.
pub struct CookieTempFile {
    inner: tempfile::NamedTempFile,
}

impl CookieTempFile {
    /// Write `cookie_str` to a new temp file and return the guard.
    ///
    /// Returns an `io::Error` on write failure or if `cookie_str` exceeds
    /// `65_536` bytes.
    pub fn write(cookie_str: &str) -> std::io::Result<Self> {
        if cookie_str.len() > MAX_COOKIE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cookie string exceeds 65 536 byte limit",
            ));
        }
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(cookie_str.as_bytes())?;
        file.flush()?;
        Ok(Self { inner: file })
    }

    /// Path of the temporary cookie file.
    pub fn path(&self) -> &std::path::Path {
        self.inner.path()
    }
}

// ---------------------------------------------------------------------------
// Header filtering
// ---------------------------------------------------------------------------

/// Filter `headers` to only safe, well-formed entries.
///
/// Rules:
/// - Name must match `[A-Za-z0-9\-]+`, max 128 bytes.
/// - Name must not be in the blocked list (case-insensitive).
/// - Value must be printable ASCII only (`0x20`–`0x7E`), max 4 096 bytes.
///
/// Dropped headers are logged at WARN level (name only, never value).
pub fn filter_safe_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            // Length checks.
            if name.is_empty() || name.len() > 128 {
                tracing::warn!(header_name = %name, "dropping header with invalid name length");
                return None;
            }
            if value.len() > 4096 {
                tracing::warn!(header_name = %name, "dropping header with oversized value");
                return None;
            }
            // Name character whitelist: [A-Za-z0-9\-]+
            if !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                tracing::warn!(header_name = %name, "dropping header with invalid name characters");
                return None;
            }
            // Blocklist check (case-insensitive).
            let name_lower = name.to_ascii_lowercase();
            if BLOCKED_HEADER_NAMES.contains(&name_lower.as_str()) {
                tracing::warn!(header_name = %name, "dropping blocked header");
                return None;
            }
            // Value must be printable ASCII.
            if !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
                tracing::warn!(header_name = %name, "dropping header with non-printable ASCII value");
                return None;
            }
            Some((name.clone(), value.clone()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Extracted URL validation
// ---------------------------------------------------------------------------

/// Validate a URL extracted from yt-dlp JSON before passing it to a download
/// engine.
///
/// Applies the same SSRF rules as [`is_safe_media_url`].  Empty URLs are
/// accepted — those route to `YtDlp` and the URL is not used for direct HTTP.
pub fn validate_extracted_url(url: &str) -> Result<(), ExtractionError> {
    if url.is_empty() {
        return Ok(());
    }
    is_safe_media_url(url)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_extracted_url ───────────────────────────────────────────

    #[test]
    fn test_validate_extracted_url_valid_cdn() {
        assert!(validate_extracted_url("https://cdn.example.com/video.mp4").is_ok());
    }

    #[test]
    fn test_validate_extracted_url_empty_is_ok() {
        assert!(validate_extracted_url("").is_ok());
    }

    #[test]
    fn test_validate_extracted_url_private_ip_rejected() {
        assert!(matches!(
            validate_extracted_url("https://192.168.1.1/video.mp4"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn test_validate_extracted_url_loopback_rejected() {
        assert!(matches!(
            validate_extracted_url("http://127.0.0.1/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn test_validate_extracted_url_localhost_rejected() {
        assert!(matches!(
            validate_extracted_url("http://localhost/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    // ── filter_safe_headers ──────────────────────────────────────────────

    #[test]
    fn test_filter_safe_headers_blocks_authorization() {
        let headers = vec![
            ("Authorization".to_owned(), "Bearer token".to_owned()),
            ("Accept".to_owned(), "*/*".to_owned()),
        ];
        let filtered = filter_safe_headers(&headers);
        assert!(
            filtered.iter().all(|(k, _)| k != "Authorization"),
            "Authorization must be dropped"
        );
        assert!(
            filtered.iter().any(|(k, _)| k == "Accept"),
            "Accept must pass through"
        );
    }

    #[test]
    fn test_filter_safe_headers_blocks_cookie() {
        let headers = vec![
            ("Cookie".to_owned(), "session=xyz".to_owned()),
            ("Content-Type".to_owned(), "video/mp4".to_owned()),
        ];
        let filtered = filter_safe_headers(&headers);
        assert!(
            filtered.iter().all(|(k, _)| k != "Cookie"),
            "Cookie must be dropped"
        );
        assert!(
            filtered.iter().any(|(k, _)| k == "Content-Type"),
            "Content-Type must pass through"
        );
    }

    #[test]
    fn test_filter_safe_headers_passes_content_type() {
        let headers = vec![("Content-Type".to_owned(), "video/mp4".to_owned())];
        let filtered = filter_safe_headers(&headers);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "Content-Type");
    }

    #[test]
    fn test_filter_safe_headers_rejects_non_ascii_value() {
        let headers = vec![
            // Contains a non-printable byte (0x01).
            ("Accept".to_owned(), "value\x01".to_owned()),
        ];
        let filtered = filter_safe_headers(&headers);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_safe_headers_rejects_oversized_value() {
        let big = "x".repeat(4097);
        let headers = vec![("Accept".to_owned(), big)];
        let filtered = filter_safe_headers(&headers);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_safe_headers_rejects_invalid_name_chars() {
        let headers = vec![("X Invalid Name".to_owned(), "value".to_owned())];
        let filtered = filter_safe_headers(&headers);
        assert!(filtered.is_empty());
    }

    // ── CookieTempFile ───────────────────────────────────────────────────

    #[test]
    fn test_cookie_temp_file_creates_and_drops() {
        let cookie_str =
            "# Netscape HTTP Cookie File\nexample.com\tFALSE\t/\tFALSE\t0\tsession\tabc123\n";
        let guard = CookieTempFile::write(cookie_str).expect("should create temp file");
        let path = guard.path().to_path_buf();
        assert!(path.exists(), "file should exist while guard is alive");
        drop(guard);
        // After drop, NamedTempFile deletes the file.
        assert!(
            !path.exists(),
            "file should be deleted after guard is dropped"
        );
    }

    #[test]
    fn test_cookie_temp_file_rejects_oversized_cookie() {
        let big = "x".repeat(MAX_COOKIE_BYTES + 1);
        let result = CookieTempFile::write(&big);
        assert!(result.is_err());
    }
}

//! ed2k (eDonkey2000) link engine.
//!
//! ed2k links have the form:
//!   `ed2k://|file|<name>|<size>|<hash>|/`
//!
//! This engine parses the URI, constructs an HTTP download URL via the
//! public eMule HTTP gateway at `http://ed2k.2x4u.de/<hash>/<name>`, and
//! delegates the actual transfer to the `DirectHttpEngine`.

use std::sync::Arc;
use tokio::sync::watch;

use fetchx_models::{DownloadId, DownloadItem, EngineKind};
use fetchx_storage::DownloadRepository;

use crate::{
    DirectHttpEngine, DownloadEngine, DownloadEngineError, ManagedDownloadControl,
    ManagedDownloadEngine,
};

// ── URI parsing ────────────────────────────────────────────────────────────

/// Parsed fields from an ed2k:// URI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ed2kLink {
    /// Original filename from the URI.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// MD4 hash (hex, 32 characters).
    pub hash: String,
}

/// Parse an ed2k:// URI into its component fields.
///
/// Accepts the canonical pipe-delimited format:
/// `ed2k://|file|<name>|<size>|<hash>|/`
pub fn parse_ed2k_uri(uri: &str) -> Option<Ed2kLink> {
    let rest = uri.strip_prefix("ed2k://|file|")?;
    // rest is: `<name>|<size>|<hash>|/` (trailing part may vary)
    let mut parts = rest.splitn(4, '|');
    let name = parts.next()?.to_owned();
    let size: u64 = parts.next()?.parse().ok()?;
    let hash = parts.next()?.to_ascii_lowercase();
    // Validate hash: 32 lowercase hex characters (MD4).
    if hash.len() != 32 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if name.trim().is_empty() {
        return None;
    }
    Some(Ed2kLink { name, size, hash })
}

/// Build an eMule HTTP gateway URL from a parsed ed2k link.
///
/// Uses the public gateway at `http://ed2k.2x4u.de`.  The gateway resolves
/// the ed2k hash to available HTTP sources and redirects the client.
pub fn ed2k_to_http_url(link: &Ed2kLink) -> String {
    // Percent-encode the filename for use in a URL path segment.
    let encoded_name: String = link
        .name
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                vec![b as char]
            } else {
                format!("%{b:02X}").chars().collect()
            }
        })
        .collect();
    format!(
        "http://ed2k.2x4u.de/{}/{}",
        link.hash, encoded_name
    )
}

// ── Engine ────────────────────────────────────────────────────────────────

/// Minimal ed2k engine: parses the URI and delegates to `DirectHttpEngine`.
pub struct Ed2kEngine<R> {
    repository: R,
    http: Arc<DirectHttpEngine<R>>,
}

impl<R> Ed2kEngine<R>
where
    R: DownloadRepository + Clone + 'static,
{
    pub fn new(repository: R, http: Arc<DirectHttpEngine<R>>) -> Self {
        Self { repository, http }
    }
}

impl<R> DownloadEngine for Ed2kEngine<R>
where
    R: DownloadRepository + Clone + 'static,
{
    async fn download(
        &self,
        id: DownloadId,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;

        if item.source().engine() != EngineKind::Ed2k {
            return Err(DownloadEngineError::EngineMismatch);
        }

        let link = parse_ed2k_uri(item.source().url())
            .ok_or(DownloadEngineError::InvalidUrl)?;
        let http_url = ed2k_to_http_url(&link);

        // Rewrite the stored URL to the HTTP gateway URL so the DirectHttp
        // engine reads the correct source from the database.
        self.repository
            .update_source_url(id, &http_url)
            .await?;

        Box::pin(self.http.download(id, cancellation)).await
    }
}

impl<R> ManagedDownloadEngine for Ed2kEngine<R>
where
    R: DownloadRepository + Clone + 'static,
{
    async fn download_managed(
        &self,
        id: DownloadId,
        control: ManagedDownloadControl,
    ) -> Result<DownloadItem, DownloadEngineError> {
        let item = self
            .repository
            .get(id)
            .await?
            .ok_or(DownloadEngineError::NotFound(id))?;

        if item.source().engine() != EngineKind::Ed2k {
            return Err(DownloadEngineError::EngineMismatch);
        }

        let link = parse_ed2k_uri(item.source().url())
            .ok_or(DownloadEngineError::InvalidUrl)?;
        let http_url = ed2k_to_http_url(&link);

        // Rewrite the stored URL to the HTTP gateway URL.
        self.repository
            .update_source_url(id, &http_url)
            .await?;

        Box::pin(self.http.download_managed(id, control)).await
    }

    fn supports_active_pause(&self, _engine: EngineKind) -> bool {
        true
    }

    fn supports_active_pause_for(&self, _item: &DownloadItem) -> bool {
        true
    }

    #[allow(clippy::manual_async_fn)]
    fn cancel_inactive(
        &self,
        _id: DownloadId,
    ) -> impl std::future::Future<Output = Result<(), DownloadEngineError>> + Send {
        async move { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::{Ed2kLink, ed2k_to_http_url, parse_ed2k_uri};

    #[test]
    fn parses_canonical_ed2k_uri() {
        let link = parse_ed2k_uri(
            "ed2k://|file|example.zip|1234567|aabbccddeeff00112233445566778899|/",
        )
        .expect("valid ed2k URI");
        assert_eq!(link.name, "example.zip");
        assert_eq!(link.size, 1_234_567);
        assert_eq!(link.hash, "aabbccddeeff00112233445566778899");
    }

    #[test]
    fn rejects_invalid_hash_length() {
        assert!(parse_ed2k_uri("ed2k://|file|test.zip|100|short|/").is_none());
    }

    #[test]
    fn rejects_non_hex_hash() {
        assert!(
            parse_ed2k_uri("ed2k://|file|test.zip|100|zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz|/")
                .is_none()
        );
    }

    #[test]
    fn builds_emule_http_gateway_url() {
        let link = Ed2kLink {
            name: "example file.zip".to_owned(),
            size: 1234,
            hash: "aabbccddeeff00112233445566778899".to_owned(),
        };
        let url = ed2k_to_http_url(&link);
        assert_eq!(
            url,
            "http://ed2k.2x4u.de/aabbccddeeff00112233445566778899/example%20file.zip"
        );
    }

    #[test]
    fn parses_uri_without_trailing_slash() {
        let link = parse_ed2k_uri(
            "ed2k://|file|test.bin|999|00112233445566778899aabbccddeeff|",
        );
        assert!(link.is_some());
    }
}

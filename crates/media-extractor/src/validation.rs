//! URL safety validation and filename sanitisation.

use crate::extractor::ExtractionError;

/// Validates that a URL is safe to pass to a media extractor.
///
/// Rules (all must pass):
/// 1. Scheme must be `http` or `https`.
/// 2. Must parse as a valid URL.
/// 3. Must have a non-empty host.
/// 4. Must not contain a username or password (credential-injection prevention).
/// 5. Host must not be a loopback address (`127.x.x.x`, `::1`, `"localhost"`).
/// 6. Host must not be a link-local address (`169.254.x.x`, `fe80::…`).
/// 7. Host must not be a private-range address (`10.x`, `172.16–31.x`,
///    `192.168.x`).
///
/// Rules 5–7 are best-effort SSRF prevention for literal IP addresses. DNS
/// rebinding is a separate concern and is out of scope for this function.
///
/// Fragment is accepted and ignored.
///
/// Returns `Ok(())` on success, or
/// `Err(ExtractionError::UnsafeUrl { reason })` on failure.
pub fn is_safe_media_url(url: &str) -> Result<(), ExtractionError> {
    // --- Rule 1: scheme check (fast path, no allocations) ---
    let scheme_end = url.find("://").ok_or(ExtractionError::UnsafeUrl {
        reason: "URL has no scheme",
    })?;
    let scheme = &url[..scheme_end];
    if scheme != "http" && scheme != "https" {
        return Err(ExtractionError::UnsafeUrl {
            reason: "scheme must be http or https",
        });
    }

    // Strip fragment before further checks (the `url` crate preserves it but
    // yt-dlp ignores it — we strip for a clean host/authority check).
    let url_no_frag = url.split('#').next().unwrap_or(url);

    // --- Rule 2: parse as URL ---
    // We use a manual parse here to keep the dep-count minimal (no `url` crate
    // in this crate's Cargo.toml). The scheme + host extraction is straightforward
    // because rules 1–4 are purely syntactic.
    let after_scheme = &url_no_frag[scheme_end + 3..]; // skip "://"

    // --- Rule 4: no embedded credentials (user:pass@host) ---
    // Split at the first '/' to isolate the authority.
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    // The authority may contain '[' for IPv6 literals.
    // Check for '@' which indicates user info.
    if authority.contains('@') {
        return Err(ExtractionError::UnsafeUrl {
            reason: "URL must not contain credentials",
        });
    }

    // --- Rule 3: non-empty host ---
    // Strip optional port.
    let host_raw = if authority.starts_with('[') {
        // IPv6 literal: [::1]:port or [::1]
        let bracket_end = authority.find(']').unwrap_or(authority.len());
        &authority[1..bracket_end]
    } else {
        // Remove port suffix if present.
        authority.split(':').next().unwrap_or(authority)
    };

    if host_raw.is_empty() {
        return Err(ExtractionError::UnsafeUrl {
            reason: "URL has no host",
        });
    }

    // --- Rules 5–7: block loopback / link-local / private IPs ---
    check_host_safety(host_raw)?;

    Ok(())
}

/// Check host-level SSRF guards.
fn check_host_safety(host: &str) -> Result<(), ExtractionError> {
    // Loopback names.
    if host.eq_ignore_ascii_case("localhost") {
        return Err(ExtractionError::UnsafeUrl {
            reason: "loopback host is not allowed",
        });
    }

    // Try to parse as IPv4.
    if let Some(ipv4) = parse_ipv4(host) {
        let [a, b, _c, _d] = ipv4;
        // 127.0.0.0/8 — loopback
        if a == 127 {
            return Err(ExtractionError::UnsafeUrl {
                reason: "loopback address is not allowed",
            });
        }
        // 10.0.0.0/8 — private
        if a == 10 {
            return Err(ExtractionError::UnsafeUrl {
                reason: "private address range is not allowed",
            });
        }
        // 172.16.0.0/12 — private
        if a == 172 && (16..=31).contains(&b) {
            return Err(ExtractionError::UnsafeUrl {
                reason: "private address range is not allowed",
            });
        }
        // 192.168.0.0/16 — private
        if a == 192 && b == 168 {
            return Err(ExtractionError::UnsafeUrl {
                reason: "private address range is not allowed",
            });
        }
        // 169.254.0.0/16 — link-local
        if a == 169 && b == 254 {
            return Err(ExtractionError::UnsafeUrl {
                reason: "link-local address is not allowed",
            });
        }
        return Ok(());
    }

    // Try to parse as IPv6 (only loopback and link-local checks needed for
    // the cases most likely to appear in practice).
    let host_lower = host.to_ascii_lowercase();
    // ::1 is loopback
    if host_lower == "::1" || host_lower == "0:0:0:0:0:0:0:1" {
        return Err(ExtractionError::UnsafeUrl {
            reason: "loopback address is not allowed",
        });
    }
    // fe80::/10 is link-local
    if host_lower.starts_with("fe80") || host_lower.starts_with("fe8") {
        return Err(ExtractionError::UnsafeUrl {
            reason: "link-local address is not allowed",
        });
    }

    Ok(())
}

/// Parse a dotted-decimal IPv4 address string into `[u8; 4]`.
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut parts = s.split('.');
    let a: u8 = parts.next()?.parse().ok()?;
    let b: u8 = parts.next()?.parse().ok()?;
    let c: u8 = parts.next()?.parse().ok()?;
    let d: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_none() {
        Some([a, b, c, d])
    } else {
        None
    }
}

/// Sanitise a media title for use as a filename component.
///
/// Rules applied in order:
/// 1. Strip or replace control characters (0x00–0x1F, 0x7F).
/// 2. Replace characters unsafe on Windows and Unix: `< > : " / \ | ? *`
/// 3. Trim leading/trailing whitespace and dots (Windows device-name protection).
/// 4. Collapse runs of whitespace to a single space.
/// 5. Truncate to 180 Unicode scalar values (leaves room for extension).
/// 6. If the result is empty after the above, return `"media"`.
///
/// The caller is responsible for passing the sanitised string through
/// `FileName::new()` before constructing a [`fetchx_models::DownloadItem`].
pub fn sanitise_title_for_filename(title: &str) -> String {
    // Step 1 & 2: filter/replace dangerous characters.
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || c == '\x7f' {
                ' '
            } else {
                match c {
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                    _ => c,
                }
            }
        })
        .collect();

    // Step 3: trim whitespace and dots.
    let trimmed = cleaned.trim_matches(|c: char| c.is_whitespace() || c == '.');

    // Step 4: collapse runs of whitespace.
    let collapsed = {
        let mut result = String::with_capacity(trimmed.len());
        let mut prev_was_space = false;
        for c in trimmed.chars() {
            if c.is_whitespace() {
                if !prev_was_space {
                    result.push(' ');
                }
                prev_was_space = true;
            } else {
                result.push(c);
                prev_was_space = false;
            }
        }
        result
    };

    // Step 5: truncate to 180 Unicode scalar values.
    let truncated: String = collapsed.chars().take(180).collect();

    // Step 6: fallback.
    if truncated.is_empty() {
        "media".to_owned()
    } else {
        truncated
    }
}

#[cfg(test)]
mod validation_tests {
    use super::{is_safe_media_url, sanitise_title_for_filename};
    use crate::extractor::ExtractionError;

    #[test]
    fn valid_https_url_passes() {
        assert!(is_safe_media_url("https://example.com/video.mp4").is_ok());
    }

    #[test]
    fn valid_http_url_passes() {
        assert!(is_safe_media_url("http://example.com/video.mp4").is_ok());
    }

    #[test]
    fn file_scheme_rejected() {
        assert!(matches!(
            is_safe_media_url("file:///etc/passwd"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn data_scheme_rejected() {
        assert!(matches!(
            is_safe_media_url("data:text/html,<h1>hi</h1>"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn credentials_in_url_rejected() {
        assert!(matches!(
            is_safe_media_url("https://user:pass@example.com/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn loopback_ip_rejected() {
        assert!(matches!(
            is_safe_media_url("http://127.0.0.1/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn localhost_rejected() {
        assert!(matches!(
            is_safe_media_url("http://localhost/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn private_ip_10_rejected() {
        assert!(matches!(
            is_safe_media_url("http://10.0.0.1/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn private_ip_172_rejected() {
        assert!(matches!(
            is_safe_media_url("http://172.16.0.1/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn private_ip_192_168_rejected() {
        assert!(matches!(
            is_safe_media_url("http://192.168.1.1/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn link_local_ip_rejected() {
        assert!(matches!(
            is_safe_media_url("http://169.254.1.1/video"),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn sanitise_strips_control_chars() {
        let result = sanitise_title_for_filename("Hello\x00World\x1f");
        assert!(!result.contains('\x00'));
    }

    #[test]
    fn sanitise_replaces_path_separators() {
        let result = sanitise_title_for_filename("path/to\\file");
        assert!(!result.contains('/'));
        assert!(!result.contains('\\'));
    }

    #[test]
    fn sanitise_truncates_long_title() {
        let long = "a".repeat(300);
        assert!(sanitise_title_for_filename(&long).chars().count() <= 180);
    }

    #[test]
    fn sanitise_empty_falls_back() {
        assert_eq!(sanitise_title_for_filename(""), "media");
        assert_eq!(sanitise_title_for_filename("..."), "media");
    }
}

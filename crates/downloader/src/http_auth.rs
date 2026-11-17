//! HTTP Basic and Digest (RFC 7616) authentication helpers.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Parsed fields from a `WWW-Authenticate` header.
pub struct WwwAuthenticate {
    pub kind: AuthKind,
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub qop: Option<String>,
    pub algorithm: DigestAlgorithm,
}

pub enum AuthKind {
    Basic,
    Digest,
}

#[derive(Clone, Copy)]
pub enum DigestAlgorithm {
    Md5,
    Md5Sess,
}

/// Build a `Basic` authorization header value.
pub fn basic_header(username: &str, password: &str) -> String {
    let credentials = format!("{username}:{password}");
    format!("Basic {}", BASE64.encode(credentials.as_bytes()))
}

/// Parse a `WWW-Authenticate` header value into its fields.
pub fn parse_www_authenticate(header: &str) -> Option<WwwAuthenticate> {
    let header = header.trim();
    if let Some(rest) = header
        .strip_prefix("Basic ")
        .or_else(|| header.strip_prefix("basic "))
    {
        let realm = extract_param(rest, "realm").unwrap_or_default();
        return Some(WwwAuthenticate {
            kind: AuthKind::Basic,
            realm,
            nonce: String::new(),
            opaque: None,
            qop: None,
            algorithm: DigestAlgorithm::Md5,
        });
    }
    if let Some(rest) = header
        .strip_prefix("Digest ")
        .or_else(|| header.strip_prefix("digest "))
    {
        let realm = extract_param(rest, "realm").unwrap_or_default();
        let nonce = extract_param(rest, "nonce").unwrap_or_default();
        let opaque = extract_param(rest, "opaque");
        let qop_raw = extract_param(rest, "qop");
        let qop = qop_raw.as_deref().and_then(|q| {
            q.split(',')
                .map(str::trim)
                .find(|s| *s == "auth")
                .map(str::to_owned)
        });
        let algorithm_str = extract_param(rest, "algorithm")
            .unwrap_or_else(|| "MD5".to_owned())
            .to_ascii_uppercase();
        let algorithm = match algorithm_str.as_str() {
            "MD5-SESS" => DigestAlgorithm::Md5Sess,
            _ => DigestAlgorithm::Md5,
        };
        return Some(WwwAuthenticate {
            kind: AuthKind::Digest,
            realm,
            nonce,
            opaque,
            qop,
            algorithm,
        });
    }
    None
}

/// Build a `Digest` authorization header value (RFC 7616, MD5 path).
pub fn digest_header(
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    challenge: &WwwAuthenticate,
) -> String {
    let cnonce = "fetchx01";
    let nc = "00000001";

    let ha1_input = format!("{}:{}:{}", username, challenge.realm, password);
    let mut ha1 = hex_md5(ha1_input.as_bytes());

    if matches!(challenge.algorithm, DigestAlgorithm::Md5Sess) {
        ha1 = hex_md5(format!("{}:{}:{}", ha1, challenge.nonce, cnonce).as_bytes());
    }

    let ha2 = hex_md5(format!("{}:{}", method, uri).as_bytes());

    let response = if challenge.qop.as_deref() == Some("auth") {
        hex_md5(
            format!(
                "{}:{}:{}:{}:{}:{}",
                ha1, challenge.nonce, nc, cnonce, "auth", ha2
            )
            .as_bytes(),
        )
    } else {
        hex_md5(format!("{}:{}:{}", ha1, challenge.nonce, ha2).as_bytes())
    };

    let mut header = format!(
        r#"Digest username="{}", realm="{}", nonce="{}", uri="{}", response="{}""#,
        username, challenge.realm, challenge.nonce, uri, response
    );

    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(r#", opaque="{}""#, opaque));
    }

    if challenge.qop.is_some() {
        header.push_str(&format!(r#", qop=auth, nc={}, cnonce="{}""#, nc, cnonce));
    }

    header
}

fn hex_md5(input: &[u8]) -> String {
    use md5::Digest;
    let result = md5::Md5::digest(input);
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

fn extract_param(input: &str, key: &str) -> Option<String> {
    let search = format!("{}=", key);
    let start = input.find(search.as_str())? + search.len();
    let rest = &input[start..];
    if let Some(inner) = rest.strip_prefix('"') {
        let end = inner.find('"')?;
        return Some(inner[..end].to_owned());
    }
    let end = rest.find([',', ' ']).unwrap_or(rest.len());
    Some(rest[..end].trim_matches('"').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_challenge() {
        let parsed = parse_www_authenticate(r#"Basic realm="example""#).unwrap();
        assert!(matches!(parsed.kind, AuthKind::Basic));
        assert_eq!(parsed.realm, "example");
    }

    #[test]
    fn parses_digest_challenge() {
        let parsed =
            parse_www_authenticate(r#"Digest realm="test", nonce="abc123", qop="auth""#).unwrap();
        assert!(matches!(parsed.kind, AuthKind::Digest));
        assert_eq!(parsed.nonce, "abc123");
        assert_eq!(parsed.qop.as_deref(), Some("auth"));
    }

    #[test]
    fn basic_header_round_trips() {
        let header = basic_header("user", "pass");
        assert!(header.starts_with("Basic "));
        let decoded = String::from_utf8(
            BASE64
                .decode(header.strip_prefix("Basic ").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, "user:pass");
    }
}

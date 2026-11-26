//! HTTP Basic, Digest (RFC 7616), and NTLM authentication helpers.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Parsed fields from a `WWW-Authenticate` header.
pub struct WwwAuthenticate {
    pub kind: AuthKind,
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub qop: Option<String>,
    pub algorithm: DigestAlgorithm,
    /// The raw NTLM Type 2 challenge token (base64, present only for NTLM kind).
    pub ntlm_token: Option<String>,
}

pub enum AuthKind {
    Basic,
    Digest,
    /// NTLM or Negotiate: the token field carries the base64 Type 2 blob when present.
    Ntlm,
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
            ntlm_token: None,
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
            ntlm_token: None,
        });
    }
    // NTLM or Negotiate scheme (may carry a base64 Type 2 token after the scheme name).
    if let Some(rest) = header
        .strip_prefix("NTLM")
        .or_else(|| header.strip_prefix("Negotiate"))
        .or_else(|| header.strip_prefix("ntlm"))
        .or_else(|| header.strip_prefix("negotiate"))
    {
        let token = rest
            .trim_start_matches(|c: char| c.is_ascii_whitespace())
            .to_owned();
        return Some(WwwAuthenticate {
            kind: AuthKind::Ntlm,
            realm: String::new(),
            nonce: String::new(),
            opaque: None,
            qop: None,
            algorithm: DigestAlgorithm::Md5,
            ntlm_token: if token.is_empty() { None } else { Some(token) },
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

// ---------------------------------------------------------------------------
// NTLM helpers
// ---------------------------------------------------------------------------

/// Builds an NTLM Type 1 (Negotiate) message encoded as `NTLM <base64>`.
///
/// The message requests Unicode, NTLM2, Always Sign, and Extended Session Security flags.
/// No workstation or domain names are embedded (lengths are zero).
pub fn ntlm_negotiate_header() -> String {
    // NTLM Type 1 message layout (little-endian):
    //   Signature  : "NTLMSSP\0"             (8 bytes)
    //   MessageType: 1                        (4 bytes LE)
    //   NegFlags   : 0x00088207               (4 bytes LE)
    //   DomainName : empty Security Buffer   (8 bytes)
    //   Workstation: empty Security Buffer   (8 bytes)
    // Total: 32 bytes
    let mut msg = Vec::with_capacity(32);
    // Signature
    msg.extend_from_slice(b"NTLMSSP\x00");
    // MessageType = 1
    msg.extend_from_slice(&1u32.to_le_bytes());
    // NegotiateFlags:
    //   NTLMSSP_NEGOTIATE_UNICODE             0x00000001
    //   NTLMSSP_NEGOTIATE_OEM                 0x00000002
    //   NTLMSSP_NEGOTIATE_REQUEST_TARGET      0x00000004
    //   NTLMSSP_NEGOTIATE_NTLM                0x00000200
    //   NTLMSSP_NEGOTIATE_ALWAYS_SIGN         0x00008000
    //   NTLMSSP_NEGOTIATE_EXTENDED_SESSIONSEC 0x00080000
    let flags: u32 = 0x0008_8207;
    msg.extend_from_slice(&flags.to_le_bytes());
    // DomainNameFields: Len=0, MaxLen=0, Offset=0
    msg.extend_from_slice(&[0u8; 8]);
    // WorkstationFields: Len=0, MaxLen=0, Offset=0
    msg.extend_from_slice(&[0u8; 8]);
    format!("NTLM {}", BASE64.encode(&msg))
}

/// Parses an NTLM Type 2 (Challenge) message from a base64 blob.
///
/// Returns the 8-byte server challenge and the target name as bytes if present.
pub fn parse_ntlm_challenge(base64_token: &str) -> Option<([u8; 8], Vec<u8>)> {
    let raw = BASE64.decode(base64_token.trim()).ok()?;
    // Minimum size: signature(8) + type(4) + target_name_fields(8) + flags(4) + challenge(8) = 32
    if raw.len() < 32 {
        return None;
    }
    if &raw[0..8] != b"NTLMSSP\x00" {
        return None;
    }
    let msg_type = u32::from_le_bytes(raw[8..12].try_into().ok()?);
    if msg_type != 2 {
        return None;
    }
    let mut challenge = [0u8; 8];
    challenge.copy_from_slice(&raw[24..32]);

    // Extract target name from Security Buffer at offset 12 (len=2, maxlen=2, offset=4)
    let target_len = u16::from_le_bytes(raw[12..14].try_into().ok()?) as usize;
    let target_offset = if raw.len() >= 20 {
        u32::from_le_bytes(raw[16..20].try_into().ok()?) as usize
    } else {
        0
    };
    let target_name = if target_offset + target_len <= raw.len() && target_len > 0 {
        raw[target_offset..target_offset + target_len].to_vec()
    } else {
        Vec::new()
    };

    Some((challenge, target_name))
}

/// Builds an NTLM Type 3 (Authenticate) message encoded as `NTLM <base64>`.
///
/// Implements NTLMv2 response (HMAC-MD5 based). The `server_challenge` is the
/// 8-byte value from the Type 2 message. `client_challenge` should be 8 random bytes;
/// callers may pass a fixed value for testability.
pub fn ntlm_authenticate_header(
    username: &str,
    password: &str,
    domain: &str,
    server_challenge: [u8; 8],
    client_challenge: [u8; 8],
) -> String {
    // NT Hash = MD4(UTF-16LE(password))
    let nt_hash = md4_hash(&utf16le(password));

    // NTLMv2 Hash = HMAC-MD5(NT_hash, UTF-16LE(upper(username) + domain))
    let identity = {
        let mut v = utf16le(&username.to_uppercase());
        v.extend_from_slice(&utf16le(domain));
        v
    };
    let nt_hash_v2 = hmac_md5(&nt_hash, &identity);

    // NTLMv2 blob: timestamp(8) + client_challenge(8) + reserved(4) + target_info_end(4)
    // We use a fixed timestamp (0) since we don't store system time here.
    let mut blob = Vec::with_capacity(32);
    blob.extend_from_slice(&[1u8, 1, 0, 0, 0, 0, 0, 0]); // RespType/HiRespType/reserved
    blob.extend_from_slice(&0u64.to_le_bytes()); // timestamp
    blob.extend_from_slice(&client_challenge); // client challenge
    blob.extend_from_slice(&[0u8; 4]); // reserved
    // MsvAvEOL (empty target info)
    blob.extend_from_slice(&[0u8; 4]);

    // NTChallengeResponse = HMAC-MD5(NTHash_v2, server_challenge || blob) || blob
    let mut sc_blob = Vec::with_capacity(8 + blob.len());
    sc_blob.extend_from_slice(&server_challenge);
    sc_blob.extend_from_slice(&blob);
    let nt_proof = hmac_md5(&nt_hash_v2, &sc_blob);

    let mut nt_response = Vec::with_capacity(16 + blob.len());
    nt_response.extend_from_slice(&nt_proof);
    nt_response.extend_from_slice(&blob);

    // LM response (empty for NTLMv2 — just 24 zeros)
    let lm_response = [0u8; 24];

    // Session key (empty for our purposes)
    let session_key: [u8; 0] = [];

    // Flags (same as negotiate)
    let flags: u32 = 0x0008_8207;

    // Build payload strings (UTF-16LE)
    let domain_bytes = utf16le(domain);
    let username_bytes = utf16le(username);
    let workstation_bytes = utf16le("WORKSTATION");

    // Calculate offsets
    // Fixed header = 72 bytes
    // Layout:
    //   Signature(8) + Type(4) + LmResponseFields(8) + NtResponseFields(8) +
    //   DomainFields(8) + UserNameFields(8) + WorkstationFields(8) + Flags(4) +
    //   SessionKeyFields(8) + = 64 bytes for fields before payload
    // We skip EncryptedRandomSessionKey and add a minimal fixed header of 64 bytes.

    let header_size: u32 = 64;
    let lm_offset: u32 = header_size;
    let nt_offset: u32 = lm_offset + lm_response.len() as u32;
    let domain_offset: u32 = nt_offset + nt_response.len() as u32;
    let username_offset: u32 = domain_offset + domain_bytes.len() as u32;
    let workstation_offset: u32 = username_offset + username_bytes.len() as u32;
    let session_key_offset: u32 = workstation_offset + workstation_bytes.len() as u32;

    let mut msg: Vec<u8> = Vec::new();
    // Signature
    msg.extend_from_slice(b"NTLMSSP\x00");
    // MessageType = 3
    msg.extend_from_slice(&3u32.to_le_bytes());
    // LmChallengeResponseFields
    write_security_buffer(&mut msg, lm_response.len() as u16, lm_offset);
    // NtChallengeResponseFields
    write_security_buffer(&mut msg, nt_response.len() as u16, nt_offset);
    // DomainNameFields
    write_security_buffer(&mut msg, domain_bytes.len() as u16, domain_offset);
    // UserNameFields
    write_security_buffer(&mut msg, username_bytes.len() as u16, username_offset);
    // WorkstationFields
    write_security_buffer(&mut msg, workstation_bytes.len() as u16, workstation_offset);
    // EncryptedRandomSessionKeyFields (empty)
    write_security_buffer(&mut msg, 0, session_key_offset);
    // Payload
    msg.extend_from_slice(&lm_response);
    msg.extend_from_slice(&nt_response);
    msg.extend_from_slice(&domain_bytes);
    msg.extend_from_slice(&username_bytes);
    msg.extend_from_slice(&workstation_bytes);
    msg.extend_from_slice(&session_key);
    // Flags (appended after session key fields in some implementations, but
    // per MS-NLMP the NegotiateFlags field is at offset 60 in the fixed header
    // before the payload). We already wrote 64 bytes of fixed header above.
    // Insert flags at offset 60: replace the last 4 bytes of the header with flags,
    // but since we're building linearly, insert them in the right place.
    // Recalculate: 8+4+8+8+8+8+8+8 = 60 bytes so far → 4 bytes flags → 64 total header.
    // The flags should sit at byte 60. Let's insert them before the payload.
    // We need to insert 4 bytes at position 60. Since we just wrote the 64-byte
    // header above, the flags need to go at the end of the fixed header section.
    // Fix: reconstruct properly.
    drop(msg);

    // Rebuild with correct layout
    let mut out: Vec<u8> = Vec::with_capacity(
        64 + lm_response.len()
            + nt_response.len()
            + domain_bytes.len()
            + username_bytes.len()
            + workstation_bytes.len(),
    );
    // bytes 0..8: Signature
    out.extend_from_slice(b"NTLMSSP\x00");
    // bytes 8..12: MessageType = 3
    out.extend_from_slice(&3u32.to_le_bytes());
    // bytes 12..20: LmChallengeResponseFields
    write_security_buffer(&mut out, lm_response.len() as u16, lm_offset);
    // bytes 20..28: NtChallengeResponseFields
    write_security_buffer(&mut out, nt_response.len() as u16, nt_offset);
    // bytes 28..36: DomainNameFields
    write_security_buffer(&mut out, domain_bytes.len() as u16, domain_offset);
    // bytes 36..44: UserNameFields
    write_security_buffer(&mut out, username_bytes.len() as u16, username_offset);
    // bytes 44..52: WorkstationFields
    write_security_buffer(&mut out, workstation_bytes.len() as u16, workstation_offset);
    // bytes 52..60: EncryptedRandomSessionKeyFields (empty)
    write_security_buffer(&mut out, 0, session_key_offset);
    // bytes 60..64: NegotiateFlags
    out.extend_from_slice(&flags.to_le_bytes());
    // Payload
    out.extend_from_slice(&lm_response);
    out.extend_from_slice(&nt_response);
    out.extend_from_slice(&domain_bytes);
    out.extend_from_slice(&username_bytes);
    out.extend_from_slice(&workstation_bytes);
    out.extend_from_slice(&session_key);

    format!("NTLM {}", BASE64.encode(&out))
}

// ---------------------------------------------------------------------------
// Private crypto helpers
// ---------------------------------------------------------------------------

fn write_security_buffer(buf: &mut Vec<u8>, len: u16, offset: u32) {
    buf.extend_from_slice(&len.to_le_bytes()); // Length
    buf.extend_from_slice(&len.to_le_bytes()); // MaxLength
    buf.extend_from_slice(&offset.to_le_bytes()); // Offset
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

fn md4_hash(input: &[u8]) -> Vec<u8> {
    use md4::Digest as _;
    md4::Md4::digest(input).to_vec()
}

fn hmac_md5(key: &[u8], data: &[u8]) -> Vec<u8> {
    use digest::KeyInit;
    use hmac::{Hmac, Mac};
    use md5::Md5;
    let mut mac = Hmac::<Md5>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
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

    #[test]
    fn parses_ntlm_challenge_header_without_token() {
        let parsed = parse_www_authenticate("NTLM").unwrap();
        assert!(matches!(parsed.kind, AuthKind::Ntlm));
        assert!(parsed.ntlm_token.is_none());
    }

    #[test]
    fn parses_ntlm_challenge_header_with_token() {
        // A minimal valid base64 token for NTLM
        let token = "TlRMTVNTUAACAAAA";
        let header = format!("NTLM {token}");
        let parsed = parse_www_authenticate(&header).unwrap();
        assert!(matches!(parsed.kind, AuthKind::Ntlm));
        assert_eq!(parsed.ntlm_token.as_deref(), Some(token));
    }

    #[test]
    fn parses_negotiate_header() {
        let parsed = parse_www_authenticate("Negotiate").unwrap();
        assert!(matches!(parsed.kind, AuthKind::Ntlm));
        assert!(parsed.ntlm_token.is_none());
    }

    #[test]
    fn ntlm_negotiate_starts_with_ntlm_prefix() {
        let header = ntlm_negotiate_header();
        assert!(header.starts_with("NTLM "), "header = {header}");
        let raw = BASE64
            .decode(header.strip_prefix("NTLM ").unwrap())
            .unwrap();
        assert_eq!(&raw[0..8], b"NTLMSSP\x00");
        assert_eq!(u32::from_le_bytes(raw[8..12].try_into().unwrap()), 1);
    }

    #[test]
    fn ntlm_authenticate_produces_valid_type3() {
        let challenge = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let client = [0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8];
        let header = ntlm_authenticate_header("User", "Password", "DOMAIN", challenge, client);
        assert!(header.starts_with("NTLM "), "header = {header}");
        let raw = BASE64
            .decode(header.strip_prefix("NTLM ").unwrap())
            .unwrap();
        assert_eq!(&raw[0..8], b"NTLMSSP\x00");
        assert_eq!(u32::from_le_bytes(raw[8..12].try_into().unwrap()), 3);
    }
}

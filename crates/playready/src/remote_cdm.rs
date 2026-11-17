//! HTTP client for a pyplayready-compatible remote CDM server.
//!
//! Follows the same session lifecycle as `fetchx-widevine`'s `RemoteCdm`:
//! open → get_license_challenge → POST to license server → parse_license → get_keys → close.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;
use thiserror::Error;

use crate::PlayReadyProvider;
use fetchx_widevine::ContentKey;

/// Errors returned by [`RemotePlayReadyCdm`].
#[derive(Debug, Error)]
pub enum RemotePlayReadyCdmError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned unexpected status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("response JSON was malformed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("server response was missing expected field: {0}")]
    MissingField(&'static str),
    #[error("hex decode failed: {0}")]
    HexDecode(String),
}

/// Remote CDM client for a pyplayready CDM server.
#[derive(Clone, Debug)]
pub struct RemotePlayReadyCdm {
    pub(crate) base_url: String,
    device_name: String,
    secret: String,
    client: reqwest::Client,
}

// ---- Response shapes ----

#[derive(Deserialize)]
struct OpenResponse {
    data: OpenData,
}

#[derive(Deserialize)]
struct OpenData {
    session_id: String,
}

#[derive(Deserialize)]
struct ChallengeResponse {
    data: ChallengeData,
}

#[derive(Deserialize)]
struct ChallengeData {
    challenge_b64: String,
}

#[derive(Deserialize)]
struct KeysResponse {
    data: KeysData,
}

#[derive(Deserialize)]
struct KeysData {
    keys: Vec<KeyEntry>,
}

#[derive(Deserialize)]
struct KeyEntry {
    key: String,
    key_id: String,
}

// ---- Implementation ----

impl RemotePlayReadyCdm {
    /// Create a new `RemotePlayReadyCdm`.
    ///
    /// - `base_url`: Base URL of the pyplayready CDM server (trailing slash stripped).
    /// - `device_name`: Device name registered on the server.
    /// - `secret`: X-Secret-Key header value.
    pub fn new(base_url: &str, device_name: &str, secret: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            device_name: device_name.to_owned(),
            secret: secret.to_owned(),
            client: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}/{}", self.base_url, self.device_name, path)
    }

    async fn open_session(&self) -> Result<String, RemotePlayReadyCdmError> {
        let resp = self
            .client
            .get(self.url("open"))
            .header("X-Secret-Key", &self.secret)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RemotePlayReadyCdmError::BadStatus { status, body });
        }

        let parsed: OpenResponse = resp.json().await?;
        Ok(parsed.data.session_id)
    }

    async fn get_challenge(
        &self,
        session_id: &str,
        pssh: &[u8],
    ) -> Result<Vec<u8>, RemotePlayReadyCdmError> {
        let init_data = BASE64.encode(pssh);
        let body = serde_json::json!({
            "session_id": session_id,
            "init_data": init_data,
        });

        let resp = self
            .client
            .post(self.url("get_license_challenge/STREAMING"))
            .header("X-Secret-Key", &self.secret)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RemotePlayReadyCdmError::BadStatus { status, body });
        }

        let parsed: ChallengeResponse = resp.json().await?;
        BASE64
            .decode(&parsed.data.challenge_b64)
            .map_err(|e| RemotePlayReadyCdmError::HexDecode(e.to_string()))
    }

    async fn send_challenge_to_license_server(
        &self,
        license_url: &str,
        challenge: &[u8],
    ) -> Result<Vec<u8>, RemotePlayReadyCdmError> {
        let resp = self
            .client
            .post(license_url)
            .header("Content-Type", "application/octet-stream")
            .body(challenge.to_vec())
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RemotePlayReadyCdmError::BadStatus { status, body });
        }

        Ok(resp.bytes().await?.to_vec())
    }

    async fn parse_license(
        &self,
        session_id: &str,
        license_bytes: &[u8],
    ) -> Result<(), RemotePlayReadyCdmError> {
        let license_b64 = BASE64.encode(license_bytes);
        let body = serde_json::json!({
            "session_id": session_id,
            "license_message": license_b64,
        });

        let resp = self
            .client
            .post(self.url("parse_license"))
            .header("X-Secret-Key", &self.secret)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RemotePlayReadyCdmError::BadStatus { status, body });
        }

        Ok(())
    }

    async fn fetch_keys(
        &self,
        session_id: &str,
    ) -> Result<Vec<ContentKey>, RemotePlayReadyCdmError> {
        let body = serde_json::json!({ "session_id": session_id });

        let resp = self
            .client
            .post(self.url("get_keys/CONTENT"))
            .header("X-Secret-Key", &self.secret)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(RemotePlayReadyCdmError::BadStatus {
                status,
                body: body_text,
            });
        }

        let parsed: KeysResponse = resp.json().await?;
        parsed
            .data
            .keys
            .into_iter()
            .map(|entry| {
                let key_id = hex_decode(&entry.key_id)?;
                let key = hex_decode(&entry.key)?;
                Ok(ContentKey { key_id, key })
            })
            .collect()
    }

    async fn close_session(&self, session_id: &str) -> Result<(), RemotePlayReadyCdmError> {
        let _ = self
            .client
            .get(self.url(&format!("close/{session_id}")))
            .header("X-Secret-Key", &self.secret)
            .send()
            .await;
        Ok(())
    }
}

fn hex_decode(s: &str) -> Result<Vec<u8>, RemotePlayReadyCdmError> {
    if s.len() % 2 != 0 {
        return Err(RemotePlayReadyCdmError::HexDecode(format!(
            "odd-length hex string: {s}"
        )));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| RemotePlayReadyCdmError::HexDecode(e.to_string()))
        })
        .collect()
}

#[async_trait::async_trait]
impl PlayReadyProvider for RemotePlayReadyCdm {
    async fn get_keys(
        &self,
        pssh: &[u8],
        license_url: &str,
    ) -> Result<Vec<ContentKey>, Box<dyn std::error::Error + Send + Sync>> {
        let session_id = self.open_session().await?;

        let result: Result<Vec<ContentKey>, RemotePlayReadyCdmError> = async {
            let challenge = self.get_challenge(&session_id, pssh).await?;
            let license = self
                .send_challenge_to_license_server(license_url, &challenge)
                .await?;
            self.parse_license(&session_id, &license).await?;
            self.fetch_keys(&session_id).await
        }
        .await;

        // Always close the session, even on failure.
        let _ = self.close_session(&session_id).await;

        Ok(result?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_cdm_url_construction() {
        let cdm = RemotePlayReadyCdm::new("http://localhost:4567/", "device", "secret");
        // Trailing slash must be stripped
        assert_eq!(cdm.base_url, "http://localhost:4567");
    }
}

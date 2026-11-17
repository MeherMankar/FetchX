//! HTTP client for a pywidevine-compatible remote CDM server.
//!
//! Mirrors the TypeScript `RemoteCdm` class from `service-worker.ts`.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;
use thiserror::Error;

use crate::{ContentKey, WidevineProvider};

/// Errors returned by [`RemoteCdm`].
#[derive(Debug, Error)]
pub enum RemoteCdmError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned unexpected status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("response JSON was malformed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("server response was missing expected field: {0}")]
    MissingField(&'static str),
    #[error("hex decode failed for key data: {0}")]
    HexDecode(String),
}

/// A remote CDM client that speaks the pywidevine CDM server HTTP protocol.
#[derive(Clone, Debug)]
pub struct RemoteCdm {
    base_url: String,
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

impl RemoteCdm {
    /// Create a new `RemoteCdm` instance.
    ///
    /// - `base_url`: Base URL of the pywidevine CDM server (e.g. `http://localhost:4567`).
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

    async fn open_session(&self) -> Result<String, RemoteCdmError> {
        let resp = self
            .client
            .get(self.url("open"))
            .header("X-Secret-Key", &self.secret)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RemoteCdmError::BadStatus { status, body });
        }

        let parsed: OpenResponse = resp.json().await?;
        Ok(parsed.data.session_id)
    }

    async fn get_challenge(
        &self,
        session_id: &str,
        pssh: &[u8],
    ) -> Result<Vec<u8>, RemoteCdmError> {
        let init_data = BASE64.encode(pssh);
        let body = serde_json::json!({
            "session_id": session_id,
            "init_data": init_data,
            "privacy_mode": false,
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
            return Err(RemoteCdmError::BadStatus { status, body });
        }

        let parsed: ChallengeResponse = resp.json().await?;
        BASE64
            .decode(&parsed.data.challenge_b64)
            .map_err(|e| RemoteCdmError::HexDecode(e.to_string()))
    }

    async fn send_challenge_to_license_server(
        &self,
        license_url: &str,
        challenge: &[u8],
    ) -> Result<Vec<u8>, RemoteCdmError> {
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
            return Err(RemoteCdmError::BadStatus { status, body });
        }

        Ok(resp.bytes().await?.to_vec())
    }

    async fn parse_license(
        &self,
        session_id: &str,
        license_bytes: &[u8],
    ) -> Result<(), RemoteCdmError> {
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
            return Err(RemoteCdmError::BadStatus { status, body });
        }

        Ok(())
    }

    async fn get_keys(&self, session_id: &str) -> Result<Vec<ContentKey>, RemoteCdmError> {
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
            return Err(RemoteCdmError::BadStatus {
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

    async fn close_session(&self, session_id: &str) -> Result<(), RemoteCdmError> {
        let _ = self
            .client
            .get(self.url(&format!("close/{session_id}")))
            .header("X-Secret-Key", &self.secret)
            .send()
            .await;
        Ok(())
    }
}

fn hex_decode(s: &str) -> Result<Vec<u8>, RemoteCdmError> {
    if !s.len().is_multiple_of(2) {
        return Err(RemoteCdmError::HexDecode(format!(
            "odd-length hex string: {s}"
        )));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| RemoteCdmError::HexDecode(e.to_string()))
        })
        .collect()
}

#[async_trait::async_trait]
impl WidevineProvider for RemoteCdm {
    async fn get_keys(
        &self,
        pssh: &[u8],
        license_url: &str,
    ) -> Result<Vec<ContentKey>, Box<dyn std::error::Error + Send + Sync>> {
        let session_id = self.open_session().await?;

        let result: Result<Vec<ContentKey>, RemoteCdmError> = async {
            let challenge = self.get_challenge(&session_id, pssh).await?;
            let license = self
                .send_challenge_to_license_server(license_url, &challenge)
                .await?;
            self.parse_license(&session_id, &license).await?;
            self.get_keys(&session_id).await
        }
        .await;

        // Always close the session, even on failure.
        let _ = self.close_session(&session_id).await;

        Ok(result?)
    }
}

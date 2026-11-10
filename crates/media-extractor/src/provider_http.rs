//! Bounded HTTP helpers shared by native provider extractors.

use std::time::Duration;

use reqwest::{
    Client, Method, StatusCode, Url,
    header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, LOCATION},
};

use crate::{
    ExtractionError,
    generic::{client_for_url, read_bounded_body, validate_public_url},
};

const MAX_REDIRECTS: usize = 5;
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) struct BoundedResponse {
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub(crate) async fn get(
    url: &Url,
    accept: &str,
    timeout: Duration,
) -> Result<BoundedResponse, ExtractionError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        accept.parse().map_err(|_| ExtractionError::InvalidOutput)?,
    );
    request(url, Method::GET, headers, None, timeout, false).await
}

pub(crate) async fn get_with_headers(
    url: &Url,
    headers: HeaderMap,
    timeout: Duration,
) -> Result<BoundedResponse, ExtractionError> {
    request(url, Method::GET, headers, None, timeout, true).await
}

pub(crate) async fn post(
    url: &Url,
    content_type: &str,
    body: Vec<u8>,
    timeout: Duration,
) -> Result<BoundedResponse, ExtractionError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        "application/json"
            .parse()
            .map_err(|_| ExtractionError::InvalidOutput)?,
    );
    headers.insert(
        CONTENT_TYPE,
        content_type
            .parse()
            .map_err(|_| ExtractionError::InvalidOutput)?,
    );
    request(url, Method::POST, headers, Some(body), timeout, false).await
}

async fn request(
    source: &Url,
    method: Method,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
    timeout: Duration,
    require_same_origin_redirect: bool,
) -> Result<BoundedResponse, ExtractionError> {
    let origin = source.origin();
    let mut url = source.clone();
    for redirect_count in 0..=MAX_REDIRECTS {
        validate_public_url(url.as_str()).await?;
        let client: Client = client_for_url(&url, timeout).await?;
        let mut builder = client
            .request(method.clone(), url.clone())
            .headers(headers.clone());
        if let Some(body) = &body {
            builder = builder.body(body.clone());
        }
        let response = builder
            .send()
            .await
            .map_err(|_| ExtractionError::BackendError {
                message: "provider request failed".to_owned(),
            })?;
        if response.status().is_redirection() {
            if method != Method::GET || redirect_count == MAX_REDIRECTS {
                return Err(ExtractionError::UnsupportedUrl);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(ExtractionError::UnsupportedUrl)?;
            let next = response
                .url()
                .join(location)
                .map_err(|_| ExtractionError::UnsafeUrl {
                    reason: "provider redirect URL is malformed",
                })?;
            if require_same_origin_redirect && next.origin() != origin {
                return Err(ExtractionError::UnsafeUrl {
                    reason: "custom provider headers may not cross origins",
                });
            }
            if url.scheme() == "https" && next.scheme() != "https" {
                return Err(ExtractionError::UnsafeUrl {
                    reason: "HTTPS redirects may not downgrade to HTTP",
                });
            }
            if !matches!(next.scheme(), "http" | "https") {
                return Err(ExtractionError::UnsafeUrl {
                    reason: "provider redirect scheme is not allowed",
                });
            }
            url = next;
            continue;
        }

        if response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(ExtractionError::UnsupportedUrl);
        }
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_bounded_body(response).await?;
        return Ok(BoundedResponse {
            status,
            content_type,
            body,
        });
    }

    Err(ExtractionError::UnsupportedUrl)
}

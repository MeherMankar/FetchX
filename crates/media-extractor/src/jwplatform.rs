//! Bounded native extraction for JW Platform media and selected site wrappers.
//!
//! The implementation adapts the JW Platform API and response parsing from
//! the authorized MeherMankar/yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense):
//! `yt_dlp/extractor/jwplatform.py`, `common.py`, `bundesliga.py`,
//! `businessinsider.py`, and `hollywoodreporter.py`. This is a limited
//! single-video slice; it does not port playlists, manifests, or all wrapper
//! page variants.

use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use reqwest::{
    StatusCode, Url,
    header::{ACCEPT, CONTENT_TYPE, LOCATION},
};
use scraper::{Html, Selector};
use serde_json::Value;

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    generic::{client_for_url, read_bounded_body, validate_public_url},
    models::{MediaFormat, MediaInfo, MediaProtocol},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;
const MAX_FORMATS: usize = 64;

pub struct JwPlatformExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    JwPlatform(String),
    Bundesliga(String),
    BusinessInsider,
    HollywoodReporter,
}

struct BoundedResponse {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

struct ParsedMedia {
    title: String,
    duration: Option<f64>,
    thumbnail: Option<String>,
    formats: Vec<ParsedFormat>,
}

struct ParsedFormat {
    url: Url,
    id: String,
    ext: String,
    protocol: MediaProtocol,
    label: String,
    width: Option<u32>,
    height: Option<u32>,
    bitrate: Option<u64>,
    filesize: Option<u64>,
}

#[async_trait]
impl MediaExtractor for JwPlatformExtractor {
    fn can_handle(&self, source: &str) -> bool {
        parse_route(source).is_some()
    }

    async fn extract(
        &self,
        source: &str,
        options: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        if options.cookies.is_some() || !options.headers.is_empty() {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let timeout = options.timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
        if timeout.is_zero() {
            return Err(ExtractionError::Timeout);
        }

        match tokio::time::timeout(timeout, self.extract_inner(source, timeout)).await {
            Ok(result) => result,
            Err(_) => Err(ExtractionError::Timeout),
        }
    }
}

impl JwPlatformExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let video_id = match route {
            Route::JwPlatform(id) | Route::Bundesliga(id) => id,
            Route::BusinessInsider => {
                let html = fetch_site_page(source, timeout).await?;
                extract_business_insider_id(&html)?
            }
            Route::HollywoodReporter => {
                let html = fetch_site_page(source, timeout).await?;
                extract_hollywood_reporter_id(&html)?
            }
        };

        let api_url = Url::parse(&format!("https://cdn.jwplayer.com/v2/media/{video_id}"))
            .map_err(|_| ExtractionError::InvalidOutput)?;
        let response = fetch_bounded_response(&api_url, timeout, "application/json").await?;
        if response.status != StatusCode::OK
            || !response.content_type.as_deref().is_some_and(|value| {
                let mime = value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                mime == "application/json" || mime.ends_with("+json")
            })
        {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let json: Value =
            serde_json::from_slice(&response.body).map_err(|_| ExtractionError::InvalidOutput)?;
        let parsed = parse_jwplatform_response(&json)?;
        build_media_info(source, parsed).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();

    if matches!(host.as_str(), "content.jwplatform.com" | "cdn.jwplayer.com") {
        return jwplatform_id(&url).map(Route::JwPlatform);
    }

    if host == "bundesliga.com" || host == "www.bundesliga.com" {
        let segments = url.path_segments()?.collect::<Vec<_>>();
        let language = *segments.first()?;
        if language.len() != 2
            || !language.bytes().all(|byte| byte.is_ascii_lowercase())
            || segments.get(1) != Some(&"bundesliga")
            || segments.get(2) != Some(&"videos")
        {
            return None;
        }
        let ids = url
            .query_pairs()
            .filter(|(key, _)| key == "vid")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        return (ids.len() == 1 && valid_video_id(&ids[0]))
            .then(|| Route::Bundesliga(ids[0].clone()));
    }

    if is_business_insider_host(&host) {
        return Some(Route::BusinessInsider);
    }

    if matches!(
        host.as_str(),
        "hollywoodreporter.com" | "www.hollywoodreporter.com"
    ) && url
        .path_segments()
        .is_some_and(|mut segments| segments.next() == Some("video") && segments.next().is_some())
    {
        return Some(Route::HollywoodReporter);
    }

    None
}

fn jwplatform_id(url: &Url) -> Option<String> {
    let segments = url.path_segments()?.collect::<Vec<_>>();
    for (index, segment) in segments.iter().enumerate() {
        let is_id_prefix = matches!(
            *segment,
            "feeds" | "players" | "thumbs" | "previews" | "manifests" | "jw6"
        ) || (*segment == "v2" && segments.get(index + 1) == Some(&"media"));
        if !is_id_prefix {
            continue;
        }

        let id_index = index + if *segment == "v2" { 2 } else { 1 };
        let candidate = segments.get(id_index)?;
        let id = candidate.get(..8)?;
        if valid_video_id(id)
            && candidate[8..].chars().all(|character| {
                character == '-' || character == '.' || character.is_ascii_alphanumeric()
            })
        {
            return Some(id.to_owned());
        }
    }
    None
}

fn valid_video_id(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_business_insider_host(host: &str) -> bool {
    ["businessinsider.com", "businessinsider.nl"]
        .iter()
        .any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
}

async fn fetch_site_page(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "page URL is malformed",
    })?;
    let response = fetch_bounded_response(&url, timeout, "text/html,application/xhtml+xml").await?;
    if response.status != StatusCode::OK
        || !response
            .content_type
            .as_deref()
            .is_some_and(is_html_content_type)
    {
        return Err(ExtractionError::UnsupportedUrl);
    }
    String::from_utf8(response.body).map_err(|_| ExtractionError::InvalidOutput)
}

fn is_html_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|mime| matches!(mime.trim(), "text/html" | "application/xhtml+xml"))
}

async fn fetch_bounded_response(
    source: &Url,
    timeout: Duration,
    accept: &str,
) -> Result<BoundedResponse, ExtractionError> {
    let mut url = source.clone();
    let mut visited = HashSet::new();

    for redirect_count in 0..=MAX_REDIRECTS {
        if !visited.insert(url.as_str().to_owned()) {
            return Err(ExtractionError::UnsupportedUrl);
        }
        validate_public_url(url.as_str()).await?;
        let client = client_for_url(&url, timeout).await?;
        let response = client
            .get(url.clone())
            .header(ACCEPT, accept)
            .send()
            .await
            .map_err(|_| ExtractionError::BackendError {
                message: "JW Platform request failed".to_owned(),
            })?;

        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(ExtractionError::UnsupportedUrl);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(ExtractionError::UnsupportedUrl)?;
            let next_url =
                response
                    .url()
                    .join(location)
                    .map_err(|_| ExtractionError::UnsafeUrl {
                        reason: "redirect URL is malformed",
                    })?;
            if url.scheme() == "https" && next_url.scheme() != "https" {
                return Err(ExtractionError::UnsafeUrl {
                    reason: "HTTPS redirects may not downgrade to HTTP",
                });
            }
            validate_public_url(next_url.as_str()).await?;
            url = next_url;
            continue;
        }

        if response
            .content_length()
            .is_some_and(|length| length > 4 * 1024 * 1024)
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

fn extract_business_insider_id(html: &str) -> Result<String, ExtractionError> {
    let document = Html::parse_document(html);
    let selector =
        Selector::parse("[data-media-id], [id]").map_err(|_| ExtractionError::InvalidOutput)?;

    for element in document.select(&selector) {
        if let Some(id) = element
            .value()
            .attr("data-media-id")
            .filter(|id| valid_video_id(id))
        {
            return Ok(id.to_owned());
        }
    }
    for element in document.select(&selector) {
        if let Some(id) = element
            .value()
            .attr("id")
            .and_then(|value| value.strip_prefix("jwplayer_"))
            .filter(|id| valid_video_id(id))
        {
            return Ok(id.to_owned());
        }
    }

    find_id_property(html)
        .or_else(|| find_id_after_marker(html, "jwplatform.com/players/"))
        .or_else(|| find_id_after_marker(html, "jwplayer_"))
        .ok_or(ExtractionError::UnsupportedUrl)
}

fn extract_hollywood_reporter_id(html: &str) -> Result<String, ExtractionError> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(".vlanding-video-card__link")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let Some(element) = document.select(&selector).next() else {
        return Err(ExtractionError::UnsupportedUrl);
    };
    if element.value().attr("data-video-showcase-type") != Some("jwplayer") {
        return Err(ExtractionError::UnsupportedUrl);
    }
    element
        .value()
        .attr("data-video-showcase-trigger")
        .filter(|id| valid_video_id(id))
        .map(str::to_owned)
        .ok_or(ExtractionError::UnsupportedUrl)
}

fn find_id_after_marker(source: &str, marker: &str) -> Option<String> {
    source.match_indices(marker).find_map(|(index, _)| {
        let value = source[index + marker.len()..].trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, '=' | '"' | '\'' | '\\')
        });
        let id = value.get(..8)?;
        (valid_video_id(id)
            && value[8..]
                .chars()
                .next()
                .is_none_or(|character| !character.is_ascii_alphanumeric()))
        .then(|| id.to_owned())
    })
}

fn find_id_property(source: &str) -> Option<String> {
    source.match_indices("id").find_map(|(index, _)| {
        let before = source[..index].chars().next_back();
        if before.is_some_and(|character| {
            !matches!(character, '"' | '\'' | '{' | ',' | ' ' | '\n' | '\r' | '\t')
        }) {
            return None;
        }

        let mut value = source.get(index + 2..)?;
        value = value.trim_start_matches(|character: char| {
            matches!(character, '"' | '\'' | ' ' | '\n' | '\r' | '\t')
        });
        value = value.strip_prefix(':')?.trim_start();
        value = value.trim_start_matches(['"', '\'']);
        let id = value.get(..8)?;
        let after = value[8..].chars().next();
        (valid_video_id(id)
            && after.is_some_and(|character| {
                matches!(character, '"' | '\'' | ',' | '}' | ' ' | '\n' | '\r' | '\t')
            }))
        .then(|| id.to_owned())
    })
}

fn parse_jwplatform_response(response: &Value) -> Result<ParsedMedia, ExtractionError> {
    let item = match response.get("playlist") {
        Some(Value::Array(items)) if items.len() == 1 => &items[0],
        Some(Value::Array(_)) => return Err(ExtractionError::UnsupportedUrl),
        Some(Value::Object(_)) => response
            .get("playlist")
            .ok_or(ExtractionError::InvalidOutput)?,
        Some(Value::Null) | None => response,
        Some(_) => return Err(ExtractionError::UnsupportedUrl),
    };
    if item.get("drm").is_some_and(is_truthy) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .to_owned();

    let sources = match item.get("sources").and_then(Value::as_array) {
        Some(sources) => sources.as_slice(),
        None if item.get("file").is_some() => std::slice::from_ref(item),
        None => return Err(ExtractionError::UnsupportedUrl),
    };
    let mut formats = Vec::new();
    let mut seen_urls = HashSet::new();
    for source in sources.iter().take(MAX_FORMATS) {
        if source.get("drm").is_some_and(is_truthy) {
            continue;
        }
        let Some(file) = source.get("file").and_then(Value::as_str) else {
            continue;
        };
        let Ok(url) = Url::parse(file) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }

        let source_type = source
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if source_type.contains("drm") {
            continue;
        }
        let ext = format_extension(&url, &source_type);
        if !is_supported_extension(&ext) || !seen_urls.insert(url.as_str().to_owned()) {
            continue;
        }
        let protocol =
            if source_type == "hls" || ext == "m3u8" || url.as_str().contains("format=m3u8-aapl") {
                MediaProtocol::Hls
            } else if source_type == "dash"
                || ext == "mpd"
                || url.as_str().contains("format=mpd-time-csf")
            {
                MediaProtocol::Dash
            } else {
                MediaProtocol::Http
            };
        let label = source
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let height = source
            .get("height")
            .and_then(parse_u64)
            .and_then(|height| u32::try_from(height).ok())
            .or_else(|| label.as_deref().and_then(parse_height));
        let id = label
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("jw-{}", formats.len() + 1));
        let description = height
            .map(|height| format!("{height}p"))
            .or(label)
            .unwrap_or_else(|| "Direct media".to_owned());

        formats.push(ParsedFormat {
            url,
            id,
            ext: ext.clone(),
            protocol,
            label: format!("{description} - {ext}"),
            width: source
                .get("width")
                .and_then(parse_u64)
                .and_then(|width| u32::try_from(width).ok()),
            height,
            bitrate: source.get("bitrate").and_then(parse_u64),
            filesize: source.get("filesize").and_then(parse_u64),
        });
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }

    let duration = response
        .get("duration")
        .and_then(parse_number)
        .or_else(|| item.get("duration").and_then(parse_number))
        .filter(|value| value.is_finite() && *value >= 0.0);
    let thumbnail = item.get("image").and_then(Value::as_str).map(str::to_owned);

    Ok(ParsedMedia {
        title,
        duration,
        thumbnail,
        formats,
    })
}

async fn build_media_info(
    webpage_url: &str,
    parsed: ParsedMedia,
) -> Result<MediaInfo, ExtractionError> {
    let mut formats = Vec::with_capacity(parsed.formats.len());
    let mut unsafe_urls = 0;
    for format in parsed.formats {
        let url = match validate_public_url(format.url.as_str()).await {
            Ok(url) => url,
            Err(ExtractionError::UnsafeUrl { .. }) => {
                unsafe_urls += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        formats.push(MediaFormat {
            id: format.id,
            ext: format.ext,
            protocol: format.protocol,
            url: url.to_string(),
            label: format.label,
            video_codec: None,
            audio_codec: None,
            width: format.width,
            height: format.height,
            fps: None,
            bitrate: format.bitrate,
            filesize: format.filesize,
            required_headers: Vec::new(),
        });
    }
    if formats.is_empty() {
        return if unsafe_urls > 0 {
            Err(ExtractionError::UnsafeUrl {
                reason: "JW Platform media URL resolves to a non-public address",
            })
        } else {
            Err(ExtractionError::UnsupportedUrl)
        };
    }

    let thumbnail_url = match parsed.thumbnail {
        Some(thumbnail) => match validate_public_url(&thumbnail).await {
            Ok(url) => Some(url.to_string()),
            Err(ExtractionError::UnsafeUrl { .. }) => None,
            Err(error) => return Err(error),
        },
        None => None,
    };
    Ok(MediaInfo {
        title: parsed.title,
        duration: parsed.duration,
        thumbnail_url,
        webpage_url: webpage_url.to_owned(),
        extractor_name: "JWPlatform".to_owned(),
        formats,
    })
}

fn format_extension(url: &Url, source_type: &str) -> String {
    let path_extension = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|segment| segment.rsplit_once('.').map(|(_, ext)| ext))
        .map(str::to_ascii_lowercase);
    path_extension.unwrap_or_else(|| match source_type {
        "video/mp4" => "mp4".to_owned(),
        "video/webm" => "webm".to_owned(),
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "hls" => "m3u8".to_owned(),
        "application/dash+xml" | "dash" => "mpd".to_owned(),
        "audio/mpeg" => "mp3".to_owned(),
        "audio/aac" => "aac".to_owned(),
        "audio/ogg" => "ogg".to_owned(),
        _ => String::new(),
    })
}

fn is_supported_extension(extension: &str) -> bool {
    matches!(
        extension,
        "aac" | "m4a" | "m4v" | "m3u8" | "mp3" | "mp4" | "mpd" | "mov" | "ogg" | "ts" | "webm"
    )
}

fn parse_height(label: &str) -> Option<u32> {
    let digits = label
        .trim()
        .split(|character: char| !character.is_ascii_digit())
        .next()?;
    digits.parse().ok()
}

fn parse_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn parse_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::{ExtractOptions, ExtractorRegistry};

    #[test]
    fn recognizes_jwplatform_and_three_site_routes() {
        let extractor = JwPlatformExtractor;
        for url in [
            "https://cdn.jwplayer.com/v2/media/nPripu9l",
            "https://content.jwplatform.com/players/nPripu9l-ALJ3XQCI.js",
            "https://www.bundesliga.com/en/bundesliga/videos?vid=bhhHkKyN",
            "https://uk.businessinsider.com/article-2026",
            "https://www.businessinsider.nl/article",
            "https://www.hollywoodreporter.com/video/example-slug/",
        ] {
            assert!(extractor.can_handle(url), "{url}");
        }
        for url in [
            "https://example.com/video",
            "https://bundesliga.com/en/bundesliga/videos?vid=short",
            "https://www.hollywoodreporter.com/vcategory/videos",
            "https://attackerbusinessinsider.com/article",
        ] {
            assert!(!extractor.can_handle(url), "{url}");
        }
    }

    #[test]
    fn extracts_business_insider_ids_using_supported_upstream_patterns() {
        assert_eq!(
            extract_business_insider_id(r#"<div data-media-id="cjGDb0X9"></div>"#).unwrap(),
            "cjGDb0X9"
        );
        assert_eq!(
            extract_business_insider_id(r#"<div id="jwplayer_5zJwd4FK"></div>"#).unwrap(),
            "5zJwd4FK"
        );
        assert_eq!(
            extract_business_insider_id(r#"{"id":"cjGDb0X9"}"#).unwrap(),
            "cjGDb0X9"
        );
        assert_eq!(
            extract_business_insider_id(
                r#"<iframe src="https://content.jwplatform.com/players/5zJwd4FK-abc.js"></iframe>"#
            )
            .unwrap(),
            "5zJwd4FK"
        );
    }

    #[test]
    fn hollywood_reporter_only_accepts_jwplayer_showcases() {
        assert_eq!(
            extract_hollywood_reporter_id(
                r#"<a class="vlanding-video-card__link" data-video-showcase-type="jwplayer" data-video-showcase-trigger="zH4jZaR5"></a>"#
            )
            .unwrap(),
            "zH4jZaR5"
        );
        assert!(extract_hollywood_reporter_id(
            r#"<a class="vlanding-video-card__link" data-video-showcase-type="youtube" data-video-showcase-trigger="zH4jZaR5"></a>"#
        )
        .is_err());
    }

    #[test]
    fn parses_single_item_media_and_classifies_direct_hls_and_dash_formats() {
        let response = serde_json::json!({
            "duration": 32,
            "playlist": [{
                "mediaid": "nPripu9l",
                "title": "Big Buck Bunny Trailer",
                "image": "https://cdn.jwplayer.com/v2/media/nPripu9l/poster.jpg",
                "sources": [
                    {
                        "file": "https://media.example/video.mp4",
                        "type": "video/mp4",
                        "label": "1080p",
                        "width": 1920,
                        "height": 1080,
                        "bitrate": 2500000,
                        "filesize": 4096
                    },
                    {
                        "file": "https://media.example/master.m3u8",
                        "type": "application/vnd.apple.mpegurl"
                    },
                    {
                        "file": "https://media.example/manifest.mpd",
                        "type": "application/dash+xml"
                    }
                ]
            }]
        });

        let parsed = parse_jwplatform_response(&response).unwrap();
        assert_eq!(parsed.title, "Big Buck Bunny Trailer");
        assert_eq!(parsed.duration, Some(32.0));
        assert_eq!(parsed.formats.len(), 3);
        assert_eq!(parsed.formats[0].protocol, MediaProtocol::Http);
        assert_eq!(parsed.formats[0].height, Some(1080));
        assert_eq!(parsed.formats[0].bitrate, Some(2_500_000));
        assert_eq!(parsed.formats[1].protocol, MediaProtocol::Hls);
        assert_eq!(parsed.formats[2].protocol, MediaProtocol::Dash);
    }

    #[test]
    fn declines_playlists_drm_and_invalid_or_unsupported_sources() {
        let playlist = serde_json::json!({
            "playlist": [
                {"title": "one", "sources": [{"file": "https://media.example/one.mp4"}]},
                {"title": "two", "sources": [{"file": "https://media.example/two.mp4"}]}
            ]
        });
        assert!(matches!(
            parse_jwplatform_response(&playlist),
            Err(ExtractionError::UnsupportedUrl)
        ));

        let drm = serde_json::json!({
            "title": "restricted",
            "drm": {"widevine": {}},
            "sources": [{"file": "https://media.example/video.mp4"}]
        });
        assert!(matches!(
            parse_jwplatform_response(&drm),
            Err(ExtractionError::UnsupportedUrl)
        ));

        let unsupported = serde_json::json!({
            "title": "unsupported",
            "sources": [
                {"file": "file:///private/video.mp4", "type": "video/mp4"},
                {"file": "https://media.example/video.smil", "type": "application/smil"}
            ]
        });
        assert!(matches!(
            parse_jwplatform_response(&unsupported),
            Err(ExtractionError::UnsupportedUrl)
        ));
    }

    struct TestFallback;

    #[async_trait]
    impl MediaExtractor for TestFallback {
        fn can_handle(&self, source: &str) -> bool {
            source.contains("cdn.jwplayer.com/v2/media/")
        }

        async fn extract(
            &self,
            source: &str,
            _options: &ExtractOptions,
        ) -> Result<MediaInfo, ExtractionError> {
            Ok(MediaInfo {
                title: "fallback".to_owned(),
                duration: None,
                thumbnail_url: None,
                webpage_url: source.to_owned(),
                extractor_name: "test-fallback".to_owned(),
                formats: vec![MediaFormat {
                    id: "fallback".to_owned(),
                    ext: "mp4".to_owned(),
                    protocol: MediaProtocol::Http,
                    url: "https://media.example/video.mp4".to_owned(),
                    label: "fallback".to_owned(),
                    video_codec: None,
                    audio_codec: None,
                    width: None,
                    height: None,
                    fps: None,
                    bitrate: None,
                    filesize: None,
                    required_headers: Vec::new(),
                }],
            })
        }
    }

    #[tokio::test]
    async fn caller_access_data_declines_native_and_advances_to_fallback() {
        let url = "https://cdn.jwplayer.com/v2/media/nPripu9l";
        let mut registry = ExtractorRegistry::new();
        registry
            .register(Arc::new(JwPlatformExtractor))
            .register(Arc::new(TestFallback));

        for options in [
            ExtractOptions {
                cookies: Some("caller-provided-cookie".to_owned()),
                ..ExtractOptions::default()
            },
            ExtractOptions {
                headers: vec![("X-Test".to_owned(), "value".to_owned())],
                ..ExtractOptions::default()
            },
        ] {
            let info = registry.extract(url, options).await.unwrap();
            assert_eq!(info.extractor_name, "test-fallback");
        }
    }
}

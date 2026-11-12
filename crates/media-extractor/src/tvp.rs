//! Limited public TVP article, embed, and live-stream extraction.
//!
//! Adapted from the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense),
//! `yt_dlp/extractor/tvp.py`. Geo-blocked, paid, playlist, and unsupported
//! manifest cases are left to the existing fallback.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Url;
use scraper::{Html, Selector};
use serde_json::Value;

use crate::{
    ExtractOptions, ExtractionError, MediaExtractor,
    generic::validate_public_url,
    models::{MediaFormat, MediaInfo, MediaProtocol},
    provider_http,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FORMATS: usize = 32;
const JSONP_CALLBACK: &str = "fetchx";

pub struct TvpExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    Article,
    Embed(String),
    Stream(String),
}

#[derive(Debug)]
struct ParsedFormat {
    id: String,
    url: String,
    ext: String,
    protocol: MediaProtocol,
}

#[derive(Debug)]
struct ParsedInfo {
    title: String,
    duration: Option<f64>,
    thumbnail: Option<String>,
    formats: Vec<ParsedFormat>,
}

#[async_trait]
impl MediaExtractor for TvpExtractor {
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

impl TvpExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let id = match route {
            Route::Article => {
                let page = get_html(source, timeout).await?;
                parse_article_video_id(&page).ok_or(ExtractionError::UnsupportedUrl)?
            }
            Route::Embed(id) => id,
            Route::Stream(channel_id) => {
                let page_url =
                    Url::parse(&format!("https://stream.tvp.pl/?channel_id={channel_id}"))
                        .map_err(|_| ExtractionError::InvalidOutput)?;
                let page = get_html_url(&page_url, timeout).await?;
                let channels = parse_channels(&page)?;
                parse_live_video_id(&channels, &channel_id)?
            }
        };
        let api_url = Url::parse(&format!(
            "https://www.tvp.pl/sess/TVPlayer2/api.php?id={id}&@method=getTvpConfig&@callback={JSONP_CALLBACK}"
        ))
        .map_err(|_| ExtractionError::InvalidOutput)?;
        let response = get_text(&api_url, timeout).await?;
        let value = parse_jsonp(&response)?;
        let parsed = parse_config(&value)?;
        build_info(source, parsed).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    if let Some(id) = source.strip_prefix("tvp:").filter(|id| valid_id(id)) {
        return Some(Route::Embed(id.to_owned()));
    }
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_start_matches("www.");
    if matches!(host, "stream.tvp.pl" | "tvpstream.vod.tvp.pl") {
        let id = url
            .query_pairs()
            .find(|(key, _)| key == "channel_id")
            .map(|(_, value)| value.into_owned())
            .filter(|value| valid_id(value))?;
        return Some(Route::Stream(id));
    }
    if !(host == "tvp.pl"
        || host.ends_with(".tvp.pl")
        || host == "tvp.info"
        || host.ends_with(".tvp.info")
        || host == "tvpworld.com"
        || host.ends_with(".tvpworld.com")
        || host == "swipeto.pl"
        || host.ends_with(".swipeto.pl"))
    {
        return None;
    }
    let path = url.path().to_ascii_lowercase();
    if path.ends_with("/sess/tvplayer.php")
        || path.ends_with("/sess/tvplayer2/embed.php")
        || path.ends_with("/sess/tvplayer2/api.php")
        || path.ends_with("/shared/details.php")
    {
        let id = url
            .query_pairs()
            .find(|(key, _)| matches!(key.as_ref(), "object_id" | "id" | "ID"))
            .map(|(_, value)| value.into_owned())
            .filter(|value| valid_id(value))?;
        return Some(Route::Embed(id));
    }
    let has_id = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .any(|segment| valid_id(segment) && segment.bytes().all(|byte| byte.is_ascii_digit()));
    if !has_id {
        return None;
    }
    Some(Route::Article)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_article_video_id(page: &str) -> Option<String> {
    let document = Html::parse_document(page);
    let selector = Selector::parse("iframe[src]").ok()?;
    for element in document.select(&selector) {
        let source = element.value().attr("src")?;
        if let Ok(url) = Url::parse(source)
            && let Some(id) = url
                .query_pairs()
                .find(|(key, _)| matches!(key.as_ref(), "object_id" | "id" | "ID"))
                .map(|(_, value)| value.into_owned())
                .filter(|value| valid_id(value))
        {
            return Some(id);
        }
    }
    for marker in [
        "data-video-id",
        "object_id",
        "window.__videoData",
        "window.__newsData",
    ] {
        if let Some(value) = page.find(marker).map(|index| &page[index..]) {
            if marker == "window.__videoData" || marker == "window.__newsData" {
                if let Some(json) = parse_balanced_json(value)
                    && let Some(id) = json
                        .get("_id")
                        .or_else(|| json.get("id"))
                        .and_then(value_id)
                {
                    return Some(id);
                }
            } else if let Some(id) = id_after_marker(value, marker) {
                return Some(id);
            }
        }
    }
    None
}

fn id_after_marker(source: &str, marker: &str) -> Option<String> {
    let mut rest = source.get(marker.len()..)?.trim_start();
    if rest.starts_with(':') || rest.starts_with('=') {
        rest = rest[1..].trim_start();
    }
    if let Some(quote) = rest.chars().next().filter(|ch| matches!(ch, '\'' | '"')) {
        let rest = &rest[quote.len_utf8()..];
        let end = rest.find(quote)?;
        let id = &rest[..end];
        return valid_id(id).then(|| id.to_owned());
    }
    let end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    let id = &rest[..end];
    valid_id(id).then(|| id.to_owned())
}

fn parse_channels(page: &str) -> Result<Value, ExtractionError> {
    let value =
        parse_after_marker(page, "window.__channels").ok_or(ExtractionError::UnsupportedUrl)?;
    if value.is_array() {
        Ok(value)
    } else {
        Err(ExtractionError::UnsupportedUrl)
    }
}

fn parse_after_marker(page: &str, marker: &str) -> Option<Value> {
    let index = page.find(marker)?;
    parse_balanced_json(&page[index + marker.len()..])
}

fn parse_balanced_json(source: &str) -> Option<Value> {
    let start = source.find(['{', '['])?;
    let source = &source[start..];
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop()? != ch {
                    return None;
                }
                if stack.is_empty() {
                    return serde_json::from_str(&source[..=index]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_live_video_id(channels: &Value, channel_id: &str) -> Result<String, ExtractionError> {
    let channel = channels
        .as_array()
        .and_then(|values| {
            values
                .iter()
                .find(|value| value.get("id").and_then(value_id).as_deref() == Some(channel_id))
        })
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let item = channel
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|value| value.get("is_live").and_then(Value::as_bool) == Some(true))
        })
        .ok_or(ExtractionError::UnsupportedUrl)?;
    item.get("video_id")
        .and_then(value_id)
        .filter(|id| valid_id(id))
        .ok_or(ExtractionError::UnsupportedUrl)
}

fn value_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
}

async fn get_html(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
    get_html_url(&url, timeout).await
}

async fn get_html_url(url: &Url, timeout: Duration) -> Result<String, ExtractionError> {
    let response = provider_http::get(url, "text/html,application/xhtml+xml", timeout).await?;
    if !response.status.is_success()
        || !response
            .content_type
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains("html"))
    {
        return Err(ExtractionError::UnsupportedUrl);
    }
    String::from_utf8(response.body).map_err(|_| ExtractionError::InvalidOutput)
}

async fn get_text(url: &Url, timeout: Duration) -> Result<String, ExtractionError> {
    let response = provider_http::get(url, "application/json, text/javascript", timeout).await?;
    if !response.status.is_success() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    String::from_utf8(response.body).map_err(|_| ExtractionError::InvalidOutput)
}

fn parse_jsonp(source: &str) -> Result<Value, ExtractionError> {
    if !source.contains(JSONP_CALLBACK) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let value = parse_after_marker(source, JSONP_CALLBACK).ok_or(ExtractionError::InvalidOutput)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(ExtractionError::UnsupportedUrl)
    }
}

fn parse_config(value: &Value) -> Result<ParsedInfo, ExtractionError> {
    let content = value
        .get("content")
        .filter(|value| value.is_object())
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let info = content
        .get("info")
        .filter(|value| value.is_object())
        .ok_or(ExtractionError::UnsupportedUrl)?;
    if ["isGeoBlocked", "geoBlocked", "isPaid", "paymentRequired"]
        .iter()
        .any(|key| flag_is_true(info.get(key)))
    {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let title = ["subtitle", "title", "seoTitle"]
        .iter()
        .find_map(|key| string_field(info, key))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let mut formats = Vec::new();
    for (index, file) in content
        .get("files")
        .and_then(Value::as_array)
        .ok_or(ExtractionError::UnsupportedUrl)?
        .iter()
        .take(MAX_FORMATS)
        .enumerate()
    {
        if file.get("protection").is_some()
            || file.get("keySystemOptions").is_some()
            || file.get("drm").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(source) = file.get("url").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(format) = parse_format(&format!("tvp-{index}"), source, file) {
            formats.push(format);
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let thumbnail = content
        .get("posters")
        .and_then(Value::as_array)
        .and_then(|posters| {
            posters
                .iter()
                .find_map(|poster| string_field(poster, "src"))
        });
    Ok(ParsedInfo {
        title,
        duration: info.get("duration").and_then(number),
        thumbnail,
        formats,
    })
}

fn parse_format(id: &str, source: &str, file: &Value) -> Result<ParsedFormat, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let ext = url
        .path()
        .rsplit('/')
        .next()
        .and_then(|value| {
            value
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .or_else(|| string_field(file, "type").map(|value| value.to_ascii_lowercase()))
        .filter(|ext| {
            matches!(
                ext.as_str(),
                "mp4" | "m4v" | "mp3" | "m4a" | "aac" | "ogg" | "m3u8" | "mpd"
            )
        })
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let protocol = match ext.as_str() {
        "m3u8" => MediaProtocol::Hls,
        "mpd" => MediaProtocol::Dash,
        _ => MediaProtocol::Http,
    };
    Ok(ParsedFormat {
        id: id.to_owned(),
        url: source.to_owned(),
        ext,
        protocol,
    })
}

async fn build_info(source: &str, parsed: ParsedInfo) -> Result<MediaInfo, ExtractionError> {
    let mut formats = Vec::new();
    for format in parsed.formats.into_iter().take(MAX_FORMATS) {
        if let Ok(url) = validate_public_url(&format.url).await {
            formats.push(MediaFormat {
                id: format.id,
                ext: format.ext,
                protocol: format.protocol,
                url: url.to_string(),
                label: "TVP stream".to_owned(),
                video_codec: None,
                audio_codec: None,
                width: None,
                height: None,
                fps: None,
                bitrate: None,
                filesize: None,
                required_headers: Vec::new(),
            });
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let thumbnail_url = match parsed.thumbnail.as_deref() {
        Some(value) => validate_public_url(value)
            .await
            .ok()
            .map(|url| url.to_string()),
        None => None,
    };
    let mut result = MediaInfo {
        title: parsed.title,
        duration: parsed.duration,
        thumbnail_url,
        webpage_url: source.to_owned(),
        extractor_name: "TVP".to_owned(),
        formats,
    };
    result.sanitise_format_urls();
    if result.formats.is_empty() {
        return Err(ExtractionError::NoFormats);
    }
    Ok(result)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn number(value: &Value) -> Option<f64> {
    let value = value.as_f64()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn flag_is_true(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_bool() == Some(true)
            || value.as_str().is_some_and(|value| {
                matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_registered_tvp_article_embed_and_stream_routes() {
        for source in [
            "https://wiadomosci.tvp.pl/50725617/dzieci-na-sprzedaz",
            "https://www.tvp.pl/sess/tvplayer.php?object_id=51247504",
            "https://tvp.info/sess/TVPlayer2/embed.php?ID=50595757",
            "https://stream.tvp.pl/?channel_id=56969941",
            "https://tvpstream.vod.tvp.pl/?channel_id=1455",
        ] {
            assert!(TvpExtractor.can_handle(source), "{source}");
        }
        assert!(!TvpExtractor.can_handle("https://example.com/50725617/video"));
        assert!(!TvpExtractor.can_handle("https://stream.tvp.pl/"));
    }

    #[test]
    fn parses_article_and_live_channel_ids_and_declines_malformed_pages() {
        let article =
            r#"<iframe src="https://www.tvp.pl/sess/tvplayer.php?object_id=51247504"></iframe>"#;
        assert_eq!(parse_article_video_id(article).as_deref(), Some("51247504"));
        let channels = r#"window.__channels = [{"id":56969941,"items":[{"is_live":true,"video_id":51247504}]}];"#;
        let channels = parse_channels(channels).unwrap();
        assert_eq!(
            parse_live_video_id(&channels, "56969941").unwrap(),
            "51247504"
        );
        assert!(parse_article_video_id("<html></html>").is_none());
        assert!(parse_channels("window.__channels = {broken};").is_err());
        assert!(parse_live_video_id(&channels, "other").is_err());
    }

    #[test]
    fn parses_public_config_and_declines_geo_paid_drm_and_malformed_results() {
        let response = serde_json::json!({
            "content": {
                "info": {"title": "Public clip", "duration": 120},
                "files": [
                    {"url": "https://cdn.example/clip.mp4", "type": "mp4"},
                    {"url": "https://cdn.example/locked.mp4", "protection": {"widevine": true}}
                ],
                "posters": [{"src": "https://cdn.example/poster.jpg"}]
            }
        });
        let parsed = parse_config(&response).unwrap();
        assert_eq!(parsed.title, "Public clip");
        assert_eq!(parsed.formats.len(), 1);
        assert!(
            parse_config(&serde_json::json!({
                "content": {
                    "info": {"title": "Restricted", "isGeoBlocked": true},
                    "files": [{"url":"https://cdn.example/locked.mp4"}]
                }
            }))
            .is_err()
        );
        assert!(
            parse_config(&serde_json::json!({
            "content": {
                "info": {"title": "Paid", "isPaid": "true"},
                "files": [{"url":"https://cdn.example/paid.mp4"}]
            }
            }))
            .is_err()
        );
        assert!(parse_config(&serde_json::json!({})).is_err());
        assert!(parse_jsonp("callback({broken})").is_err());
    }

    #[tokio::test]
    async fn declines_caller_credentials_for_registry_fallback() {
        let options = ExtractOptions {
            cookies: Some("caller-cookie".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            TvpExtractor
                .extract(
                    "https://www.tvp.pl/sess/tvplayer.php?object_id=51247504",
                    &options
                )
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

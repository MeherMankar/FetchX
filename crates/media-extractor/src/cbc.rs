//! Bounded public CBC player and listen extraction.
//!
//! This is a limited Rust translation of the authorized yt-dlp fork at
//! revision `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense),
//! `yt_dlp/extractor/cbc.py`. Geo-restricted, playlist, ThePlatform legacy,
//! DRM, and unrecognized CBC responses remain unsupported.

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

pub struct CbcExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    Article,
    Player(String),
    Listen(String),
}

struct ParsedPlayer {
    title: String,
    thumbnail: Option<String>,
    duration: Option<f64>,
    assets: Vec<String>,
}

#[async_trait]
impl MediaExtractor for CbcExtractor {
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

impl CbcExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        match parse_route(source).ok_or(ExtractionError::UnsupportedUrl)? {
            Route::Listen(id) => {
                let url = Url::parse(&format!("https://www.cbc.ca/listen/api/v1/clips/{id}"))
                    .map_err(|_| ExtractionError::InvalidOutput)?;
                let json = get_json(&url, timeout).await?;
                let data = json.get("data").ok_or(ExtractionError::UnsupportedUrl)?;
                build_listen_info(source, &id, data).await
            }
            Route::Player(id) => self.extract_player(source, &id, timeout).await,
            Route::Article => {
                let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
                let html = get_html(&url, timeout).await?;
                let ids = parse_article_media_ids(&html)?;
                if ids.len() != 1 {
                    return Err(ExtractionError::UnsupportedUrl);
                }
                self.extract_player(source, &ids[0], timeout).await
            }
        }
    }

    async fn extract_player(
        &self,
        source: &str,
        id: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let page_url = Url::parse(&format!("https://www.cbc.ca/player/play/{id}"))
            .map_err(|_| ExtractionError::InvalidOutput)?;
        let html = get_html(&page_url, timeout).await?;
        let initial_state = json_after_marker(&html, "window.__INITIAL_STATE__")
            .ok_or(ExtractionError::UnsupportedUrl)?;
        let parsed = parse_player_state(&initial_state)?;
        let mut formats = Vec::new();
        for asset_key in parsed.assets.into_iter().take(MAX_FORMATS) {
            let Ok(asset_url) = Url::parse(&asset_key) else {
                continue;
            };
            let Ok(asset_json) = get_json(&asset_url, timeout).await else {
                continue;
            };
            if let Some(format) = parse_medianet_asset(&asset_json)
                && let Ok(url) = validate_public_url(&format.url).await
            {
                formats.push(MediaFormat {
                    id: format.id,
                    ext: format.ext,
                    protocol: format.protocol,
                    url: url.to_string(),
                    label: format.label,
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
        Ok(MediaInfo {
            title: parsed.title,
            duration: parsed.duration,
            thumbnail_url,
            webpage_url: source.to_owned(),
            extractor_name: format!("CBC ({id})"),
            formats,
        })
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(url.host_str()?, "cbc.ca" | "www.cbc.ca")
    {
        return None;
    }
    let segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.first() == Some(&"player") && segments.get(1) == Some(&"play") {
        let id = match segments.get(2) {
            Some(value) if *value == "video" => segments.get(3).copied(),
            Some(value) => Some(*value),
            None => None,
        };
        if let Some(id) = id.and_then(media_id) {
            return Some(Route::Player(id));
        }
    }
    if segments.first() == Some(&"listen")
        && matches!(segments.get(1), Some(&"cbc-podcasts" | &"live-radio"))
        && segments.len() == 5
        && let Some(id) = segments.last().and_then(|value| numeric_id(value))
    {
        return Some(Route::Listen(id));
    }
    if segments.len() >= 2 && !matches!(segments.first().copied(), Some("player" | "listen")) {
        return Some(Route::Article);
    }
    None
}

fn numeric_id(value: &str) -> Option<String> {
    let digits = value.split_once('-').map_or(value, |(digits, _)| digits);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.to_owned())
}

fn media_id(value: &str) -> Option<String> {
    let value = value.strip_suffix('/').unwrap_or(value);
    let valid = !value.is_empty()
        && value.split_once('.').is_none_or(|(prefix, suffix)| {
            !prefix.is_empty()
                && !suffix.is_empty()
                && prefix.bytes().all(|byte| byte.is_ascii_digit())
                && suffix.bytes().all(|byte| byte.is_ascii_digit())
        });
    valid.then(|| value.to_owned())
}

fn parse_article_media_ids(html: &str) -> Result<Vec<String>, ExtractionError> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("iframe[src], [id^='player-']")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let mut ids = Vec::new();
    for element in document.select(&selector) {
        if let Some(src) = element.value().attr("src")
            && let Ok(url) = Url::parse(src)
            && matches!(url.host_str(), Some("cbc.ca" | "www.cbc.ca"))
            && let Some(id) = url
                .query_pairs()
                .find(|(key, _)| key == "mediaId")
                .map(|(_, value)| value.into_owned())
                .or_else(|| {
                    url.path_segments()
                        .and_then(|mut segments| segments.rfind(|part| !part.is_empty()))
                        .and_then(media_id)
                })
        {
            ids.push(id);
        }
        if let Some(id) = element
            .value()
            .attr("id")
            .and_then(|value| value.strip_prefix("player-"))
            .and_then(numeric_id)
        {
            ids.push(id);
        }
    }
    let mut remaining = html;
    while let Some((index, _)) = remaining
        .match_indices("CBC.APP.Caffeine.initInstance(")
        .next()
    {
        let after_marker = &remaining[index..];
        if let Some(value) = json_after_marker(after_marker, "initInstance(")
            && let Some(id) = value
                .get("mediaId")
                .and_then(Value::as_str)
                .and_then(media_id)
        {
            ids.push(id);
        }
        remaining = &remaining[index + "CBC.APP.Caffeine.initInstance(".len()..];
    }
    if let Some(state) = json_after_marker(html, "window.__INITIAL_STATE__") {
        let mut state_ids = Vec::new();
        if let Some(id) = state
            .pointer("/app/contentId")
            .and_then(Value::as_str)
            .and_then(media_id)
        {
            state_ids.push(id);
        }
        collect_polopoly_media_ids(
            state
                .pointer("/detail/content/body")
                .unwrap_or(&Value::Null),
            &mut state_ids,
        );
        ids.extend(state_ids);
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    Ok(ids)
}

fn collect_polopoly_media_ids(value: &Value, ids: &mut Vec<String>) {
    match value {
        Value::Array(values) => {
            for entry in values {
                collect_polopoly_media_ids(entry, ids);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("polopoly_media")
                && let Some(id) = object
                    .get("content")
                    .and_then(|node| node.get("sourceId"))
                    .and_then(Value::as_str)
                    .and_then(media_id)
            {
                ids.push(id);
            }
            for child in object.values() {
                collect_polopoly_media_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn parse_player_state(state: &Value) -> Result<ParsedPlayer, ExtractionError> {
    let clip = state
        .pointer("/video/currentClip")
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let media = clip.get("media").unwrap_or(&Value::Null);
    let assets = media
        .get("assets")
        .and_then(Value::as_array)
        .ok_or(ExtractionError::UnsupportedUrl)?
        .iter()
        .filter(|asset| asset.get("type").and_then(Value::as_str) == Some("medianet"))
        .filter_map(|asset| asset.get("key").and_then(Value::as_str))
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let title = clip
        .get("title")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .trim()
        .to_owned();
    if assets.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    Ok(ParsedPlayer {
        title,
        thumbnail: clip
            .pointer("/image/url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        duration: media.get("duration").and_then(parse_f64),
        assets,
    })
}

struct ParsedFormat {
    id: String,
    url: String,
    ext: String,
    protocol: MediaProtocol,
    label: String,
}

fn parse_medianet_asset(asset: &Value) -> Option<ParsedFormat> {
    if asset.get("protection").is_some()
        || asset.get("keySystemOptions").is_some()
        || asset.get("drm").is_some()
    {
        return None;
    }
    let url = asset.get("url")?.as_str()?;
    let content_type = asset
        .get("params")?
        .as_array()?
        .iter()
        .find(|param| param.get("name").and_then(Value::as_str) == Some("contentType"))?
        .get("value")?
        .as_str()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    let (ext, protocol) = if content_type.contains("mpegurl") || content_type.contains("m3u8") {
        ("m3u8", MediaProtocol::Hls)
    } else if content_type.contains("dash") || content_type.contains("mpd") {
        ("mpd", MediaProtocol::Dash)
    } else if content_type.contains("mp4") || content_type.contains("m4a") {
        ("mp4", MediaProtocol::Http)
    } else if content_type.contains("mpeg") || content_type.contains("mp3") {
        ("mp3", MediaProtocol::Http)
    } else {
        return None;
    };
    Some(ParsedFormat {
        id: format!(
            "{ext}-{}",
            asset.get("id").and_then(Value::as_str).unwrap_or("source")
        ),
        url: url.to_owned(),
        ext: ext.to_owned(),
        protocol,
        label: ext.to_ascii_uppercase(),
    })
}

async fn build_listen_info(
    source: &str,
    id: &str,
    data: &Value,
) -> Result<MediaInfo, ExtractionError> {
    let url = data
        .get("src")
        .and_then(Value::as_str)
        .or_else(|| data.get("url").and_then(Value::as_str))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let url = validate_public_url(url).await?;
    let title = data
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .trim()
        .to_owned();
    let ext = extension(&url, None);
    Ok(MediaInfo {
        title,
        duration: data.get("duration").and_then(parse_f64),
        thumbnail_url: None,
        webpage_url: source.to_owned(),
        extractor_name: format!("CBC Listen ({id})"),
        formats: vec![MediaFormat {
            id: "audio".to_owned(),
            ext,
            protocol: MediaProtocol::Http,
            url: url.to_string(),
            label: "Audio".to_owned(),
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

fn extension(url: &Url, content_type: Option<&str>) -> String {
    if let Some(ext) = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|segment| {
            segment
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .filter(|ext| matches!(ext.as_str(), "mp3" | "m4a" | "aac" | "mp4" | "m3u8" | "mpd"))
    {
        return ext;
    }
    match content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
    {
        "audio/mpeg" => "mp3",
        "audio/aac" => "aac",
        "audio/mp4" => "m4a",
        "video/mp4" => "mp4",
        _ => "mp3",
    }
    .to_owned()
}

async fn get_html(url: &Url, timeout: Duration) -> Result<String, ExtractionError> {
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

async fn get_json(url: &Url, timeout: Duration) -> Result<Value, ExtractionError> {
    let response = provider_http::get(url, "application/json", timeout).await?;
    if !response.status.is_success()
        || !response
            .content_type
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains("json"))
    {
        return Err(ExtractionError::UnsupportedUrl);
    }
    serde_json::from_slice(&response.body).map_err(|_| ExtractionError::InvalidOutput)
}

fn json_after_marker(source: &str, marker: &str) -> Option<Value> {
    let remainder = &source[source.find(marker)? + marker.len()..];
    let start = remainder.find('{')?;
    let json = &remainder[start..];
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in json.char_indices() {
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
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return serde_json::from_str(&json[..=index]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_f64(value: &Value) -> Option<f64> {
    let number = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    (number.is_finite() && number >= 0.0).then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_cbc_article_player_and_listen_routes() {
        for url in [
            "https://www.cbc.ca/news/canada/example-story",
            "https://www.cbc.ca/player/play/video/9.6424403",
            "https://www.cbc.ca/listen/cbc-podcasts/show/episode/16142603-title",
        ] {
            assert!(CbcExtractor.can_handle(url), "{url}");
        }
        assert!(!CbcExtractor.can_handle("https://cbc.ca/player/news/show"));
        assert!(!CbcExtractor.can_handle("https://notcbc.ca/news/example"));
    }

    #[test]
    fn parses_single_cbc_player_wrappers_and_declines_playlists() {
        let one = r#"<iframe src="https://www.cbc.ca/player/play/123456"></iframe>"#;
        assert_eq!(parse_article_media_ids(one).unwrap(), vec!["123456"]);
        let init = r#"CBC.APP.Caffeine.initInstance({"mediaId":"1.1711287"});"#;
        assert_eq!(parse_article_media_ids(init).unwrap(), vec!["1.1711287"]);
        let multiple = r#"<div id="player-123456"></div><div id="player-789012"></div>"#;
        assert_eq!(parse_article_media_ids(multiple).unwrap().len(), 2);
        assert!(parse_article_media_ids("<html></html>").is_err());
    }

    #[test]
    fn maps_public_medianet_sources_and_rejects_protected_or_unknown_assets() {
        let asset = serde_json::json!({
            "id": "public",
            "url": "https://media.example/audio.mp3",
            "params": [{"name": "contentType", "value": "audio/mpeg"}]
        });
        let parsed = parse_medianet_asset(&asset).unwrap();
        assert_eq!(parsed.ext, "mp3");
        assert_eq!(parsed.protocol, MediaProtocol::Http);
        assert!(
            parse_medianet_asset(&serde_json::json!({
                "url": "https://media.example/encrypted.mp4",
                "protection": {"keySystems": ["widevine"]},
                "params": [{"name": "contentType", "value": "video/mp4"}]
            }))
            .is_none()
        );
        assert!(
            parse_medianet_asset(&serde_json::json!({
                "url": "https://media.example/unknown",
                "params": [{"name": "contentType", "value": "application/octet-stream"}]
            }))
            .is_none()
        );
    }

    #[test]
    fn rejects_missing_or_ambiguous_cbc_player_data() {
        assert!(parse_player_state(&serde_json::json!({})).is_err());
        assert!(parse_player_state(&serde_json::json!({
            "video": {
                "currentClip": {
                    "title": "clip",
                    "media": {"assets": [{"type": "theplatform", "key": "https://media.example/x"}]}
                }
            }
        }))
        .is_err());
        assert!(
            json_after_marker(
                "window.__INITIAL_STATE__ = {broken};",
                "window.__INITIAL_STATE__"
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn refuses_caller_credentials_so_registry_can_fall_back() {
        let options = ExtractOptions {
            cookies: Some("user-cookie".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            CbcExtractor
                .extract(
                    "https://www.cbc.ca/listen/cbc-podcasts/show/episode/123456-title",
                    &options
                )
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

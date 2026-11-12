//! Limited public NDR/N-JOY single-item extraction.
//!
//! Adapted from the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense),
//! `yt_dlp/extractor/ndr.py`. Playlists, subtitle extraction, and manifest
//! expansion remain unsupported.

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

pub struct NdrExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    Article { publisher: Publisher },
    Player { id: String, publisher: Publisher },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Publisher {
    Ndr,
    NJoy,
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
impl MediaExtractor for NdrExtractor {
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

impl NdrExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let (id, publisher) = match route {
            Route::Article { publisher } => {
                let page = get_html(source, timeout).await?;
                let id = parse_article_player_id(&page).ok_or(ExtractionError::UnsupportedUrl)?;
                (id, publisher)
            }
            Route::Player { id, publisher } => (id, publisher),
        };
        let api_url = Url::parse(&format!("https://www.ndr.de/{id}-ppjson.json"))
            .map_err(|_| ExtractionError::InvalidOutput)?;
        let data = get_json(&api_url, timeout).await?;
        let parsed = parse_player_response(&data)?;
        build_info(source, publisher, parsed).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_start_matches("www.");
    let publisher = if host == "ndr.de" || host.ends_with(".ndr.de") {
        Publisher::Ndr
    } else if host == "n-joy.de" || host.ends_with(".n-joy.de") {
        Publisher::NJoy
    } else {
        return None;
    };
    let last = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    if let Some(id) = last.strip_suffix("-ppjson.json").filter(|id| valid_id(id)) {
        return Some(Route::Player {
            id: id.to_owned(),
            publisher,
        });
    }
    if let Some(id) = parse_player_filename(last) {
        return Some(Route::Player { id, publisher });
    }
    let article_id = last
        .strip_suffix(".html")?
        .rsplit_once(',')
        .map(|(_, id)| id)?;
    if !valid_id(article_id) {
        return None;
    }
    Some(Route::Article { publisher })
}

fn parse_player_filename(value: &str) -> Option<String> {
    let stem = value.strip_suffix(".html")?;
    for marker in ["-externalPlayer", "-player"] {
        if let Some((id, _)) = stem.split_once(marker)
            && valid_id(id)
        {
            return Some(id.to_owned());
        }
    }
    None
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_article_player_id(page: &str) -> Option<String> {
    let document = Html::parse_document(page);
    let selector = Selector::parse("meta[name='embedURL'], meta[property='og:video']").ok()?;
    for element in document.select(&selector) {
        let value = element.value().attr("content")?;
        if let Some(id) = value.strip_prefix("ndr:").filter(|id| valid_id(id)) {
            return Some(id.to_owned());
        }
        let url = Url::parse(value).ok()?;
        let last = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
        if let Some(id) = last.strip_suffix("-ppjson.json").filter(|id| valid_id(id)) {
            return Some(id.to_owned());
        }
        if let Some(id) = parse_player_filename(last) {
            return Some(id);
        }
    }
    None
}

async fn get_html(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
    let response = provider_http::get(&url, "text/html,application/xhtml+xml", timeout).await?;
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

fn parse_player_response(value: &Value) -> Result<ParsedInfo, ExtractionError> {
    let playlist = value
        .get("playlist")
        .and_then(Value::as_object)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let config = playlist
        .get("config")
        .and_then(Value::as_object)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let title = config
        .get("title")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .to_owned();
    let mut formats = Vec::new();
    for (format_id, item) in playlist.iter().take(MAX_FORMATS) {
        if format_id == "config" {
            continue;
        }
        if ["drm", "protection", "keySystemOptions", "encrypted"]
            .iter()
            .any(|key| {
                item.get(key)
                    .is_some_and(|value| !value.is_null() && value != false)
            })
        {
            continue;
        }
        let Some(source) = item.get("src").and_then(Value::as_str) else {
            continue;
        };
        if let Ok(format) = parse_format(format_id, source) {
            formats.push(format);
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let thumbnail = config
        .get("poster")
        .and_then(Value::as_object)
        .and_then(|poster| poster.values().find_map(|value| value.get("src")))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(ParsedInfo {
        title,
        duration: config.get("duration").and_then(number),
        thumbnail,
        formats,
    })
}

fn parse_format(id: &str, source: &str) -> Result<ParsedFormat, ExtractionError> {
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

async fn build_info(
    source: &str,
    publisher: Publisher,
    parsed: ParsedInfo,
) -> Result<MediaInfo, ExtractionError> {
    let mut formats = Vec::new();
    for format in parsed.formats.into_iter().take(MAX_FORMATS) {
        if let Ok(url) = validate_public_url(&format.url).await {
            formats.push(MediaFormat {
                id: format.id,
                ext: format.ext,
                protocol: format.protocol,
                url: url.to_string(),
                label: "NDR stream".to_owned(),
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
    let extractor_name = match publisher {
        Publisher::Ndr => "NDR",
        Publisher::NJoy => "N-JOY",
    };
    Ok(MediaInfo {
        title: parsed.title,
        duration: parsed.duration,
        thumbnail_url,
        webpage_url: source.to_owned(),
        extractor_name: extractor_name.to_owned(),
        formats,
    })
}

fn number(value: &Value) -> Option<f64> {
    let value = value.as_f64()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_registered_ndr_and_njoy_routes() {
        for source in [
            "https://www.ndr.de/fernsehen/Party-Poette-und-Parade,hafengeburtstag988.html",
            "https://www.ndr.de/ndraktuell28488-player.html",
            "https://www.ndr.de/ndraktuell28488-ppjson.json",
            "https://www.n-joy.de/entertainment/comedy/Benaissa,comedycontest2480.html",
            "https://www.n-joy.de/comedycontest2480-player_video.html",
        ] {
            assert!(NdrExtractor.can_handle(source), "{source}");
        }
        assert!(!NdrExtractor.can_handle("https://example.com/ndraktuell28488-player.html"));
        assert!(!NdrExtractor.can_handle("https://www.ndr.de/article.html"));
    }

    #[test]
    fn parses_article_player_ids_from_registered_page_markup() {
        let page =
            r#"<meta name="embedURL" content="https://www.ndr.de/soundcheck3366-player.html">"#;
        assert_eq!(
            parse_article_player_id(page).as_deref(),
            Some("soundcheck3366")
        );
        assert_eq!(
            parse_article_player_id(r#"<meta name="embedURL" content="ndr:audio51535">"#)
                .as_deref(),
            Some("audio51535")
        );
        assert!(parse_article_player_id("<html></html>").is_none());
    }

    #[test]
    fn parses_public_player_formats_and_rejects_malformed_or_unsupported_data() {
        let response = serde_json::json!({
            "playlist": {
                "config": {"title": "Public episode", "duration": 120, "poster": {
                    "large": {"src": "https://cdn.example/poster.jpg"}
                }},
                "httpAudio": {"src": "https://cdn.example/audio.mp3", "type": "audio/mpeg"},
                "httpVideo": {"src": "https://cdn.example/video.mp4"}
            }
        });
        let parsed = parse_player_response(&response).unwrap();
        assert_eq!(parsed.title, "Public episode");
        assert_eq!(parsed.formats.len(), 2);
        assert!(parse_player_response(&serde_json::json!({})).is_err());
        assert!(
            parse_player_response(&serde_json::json!({"playlist":{"config":{"title":"empty"}}}))
                .is_err()
        );
        assert!(
            parse_player_response(&serde_json::json!({
                "playlist": {
                    "config": {"title": "Protected"},
                    "drm": {"src": "https://cdn.example/locked.mp4", "protection": {"widevine": true}}
                }
            }))
            .is_err()
        );
        assert!(parse_format("bad", "file:///private.mp4").is_err());
    }

    #[tokio::test]
    async fn declines_caller_credentials_for_registry_fallback() {
        let options = ExtractOptions {
            headers: vec![("X-Test".to_owned(), "value".to_owned())],
            ..ExtractOptions::default()
        };
        assert!(matches!(
            NdrExtractor
                .extract("https://www.ndr.de/ndraktuell28488-player.html", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

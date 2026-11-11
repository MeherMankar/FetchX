//! Limited public Radio France podcast, archive, and live extraction.
//!
//! Ported from the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense),
//! `yt_dlp/extractor/radiofrance.py`. Playlist expansion, HLS manifest
//! expansion, and substations requiring page-embedded configuration remain
//! unsupported.

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
const STATIONS: [&str; 6] = [
    "franceculture",
    "franceinfo",
    "franceinter",
    "francemusique",
    "fip",
    "mouv",
];

pub struct RadioFranceExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    FranceCulture { id: String },
    Legacy { id: String },
    Live { station: String },
}

#[derive(Debug)]
struct ParsedFormat {
    id: String,
    url: String,
    ext: String,
    protocol: MediaProtocol,
    label: String,
}

#[derive(Debug)]
struct ParsedInfo {
    title: String,
    duration: Option<f64>,
    thumbnail: Option<String>,
    formats: Vec<ParsedFormat>,
}

#[async_trait]
impl MediaExtractor for RadioFranceExtractor {
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

impl RadioFranceExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let parsed = match route {
            Route::FranceCulture { id: _ } => {
                let page = get_html(source, timeout).await?;
                parse_podcast_page(&page)?
            }
            Route::Legacy { id: _ } => {
                let page = get_html(source, timeout).await?;
                parse_legacy_page(&page)?
            }
            Route::Live { station } => {
                let url = Url::parse(&format!("https://www.radiofrance.fr/{station}/api/live"))
                    .map_err(|_| ExtractionError::InvalidOutput)?;
                let response = get_json(&url, timeout).await?;
                parse_live_response(&response)?
            }
        };
        build_info(source, parsed).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_start_matches("www.");
    let segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if host == "maison.radiofrance.fr"
        && segments.len() >= 2
        && segments[0] == "radiovisions"
    {
        return Some(Route::Legacy {
            id: segments[1].to_owned(),
        });
    }
    if host == "radiofrance.fr" && !segments.is_empty() && STATIONS.contains(&segments[0]) {
        let station = segments[0];
        if segments.get(1) == Some(&"podcasts") {
            let last = *segments.last()?;
            let (_, id) = last.rsplit_once('-')?;
            if id.len() >= 6 && id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Some(Route::FranceCulture { id: id.to_owned() });
            }
        }
        if segments.len() == 1 || segments.get(1).is_some_and(|segment| segment.starts_with("radio-")) {
            return Some(Route::Live {
                station: station.to_owned(),
            });
        }
    }
    None
}

fn parse_podcast_page(page: &str) -> Result<ParsedInfo, ExtractionError> {
    let value = find_audio_object(page).ok_or(ExtractionError::UnsupportedUrl)?;
    let url = value
        .get("contentUrl")
        .and_then(Value::as_str)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let title = page_title(page)
        .or_else(|| nonempty_string(value.get("name")))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    Ok(ParsedInfo {
        title,
        duration: value.get("duration").and_then(parse_iso_duration),
        thumbnail: value
            .get("thumbnailUrl")
            .and_then(Value::as_str)
            .map(str::to_owned),
        formats: vec![format_from_url("audio", url, None)?],
    })
}

fn find_audio_object(page: &str) -> Option<Value> {
    let document = Html::parse_document(page);
    let selector = Selector::parse("script").ok()?;
    for script in document.select(&selector) {
        let text = script.text().collect::<String>();
        if let Ok(value) = serde_json::from_str::<Value>(&text)
            && let Some(audio) = find_typed_audio_object(&value)
        {
            return Some(audio.clone());
        }
        if let Some(value) = json_object_around_marker(&text, "\"@type\"", "AudioObject")
            && let Some(audio) = find_typed_audio_object(&value)
        {
            return Some(audio.clone());
        }
    }
    None
}

fn find_typed_audio_object(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(object) => {
            if object.get("@type").and_then(Value::as_str) == Some("AudioObject")
                && object.get("contentUrl").and_then(Value::as_str).is_some()
            {
                return Some(value);
            }
            object.values().find_map(find_typed_audio_object)
        }
        Value::Array(values) => values.iter().find_map(find_typed_audio_object),
        _ => None,
    }
}

fn parse_legacy_page(page: &str) -> Result<ParsedInfo, ExtractionError> {
    let document = Html::parse_document(page);
    let title_selector =
        Selector::parse("h1").map_err(|_| ExtractionError::InvalidOutput)?;
    let source_selector = Selector::parse(".jp-jplayer[data-source]")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let title = document
        .select(&title_selector)
        .next()
        .map(|element| element.text().collect::<String>().trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let source = document
        .select(&source_selector)
        .find_map(|element| element.value().attr("data-source"))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let formats = parse_legacy_sources(source)?;
    Ok(ParsedInfo {
        title,
        duration: None,
        thumbnail: None,
        formats,
    })
}

fn parse_legacy_sources(source: &str) -> Result<Vec<ParsedFormat>, ExtractionError> {
    let mut formats = Vec::new();
    for (index, item) in source.split(',').enumerate() {
        let Some((id, raw_url)) = item.split_once(':') else {
            continue;
        };
        let id = id.trim().trim_matches(['\'', '"']);
        let url = raw_url.trim().trim_matches(['\'', '"']);
        if id.is_empty() || url.is_empty() {
            continue;
        }
        if let Ok(format) = format_from_url(id, url, None) {
            formats.push(format);
        }
        if index >= MAX_FORMATS {
            break;
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    Ok(formats)
}

fn parse_live_response(response: &Value) -> Result<ParsedInfo, ExtractionError> {
    let sources = response
        .pointer("/now/media/sources")
        .and_then(Value::as_array)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let mut formats = Vec::new();
    for source in sources.iter().take(MAX_FORMATS) {
        let Some(url) = source.get("url").and_then(Value::as_str) else {
            continue;
        };
        let format = source.get("format").and_then(Value::as_str);
        if let Ok(parsed) = format_from_url(
            format.unwrap_or("live"),
            url,
            format.filter(|format| *format == "hls"),
        ) {
            formats.push(parsed);
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let title = response
        .pointer("/visual/legend")
        .and_then(Value::as_str)
        .or_else(|| {
            let first = response.pointer("/now/firstLine/title").and_then(Value::as_str);
            let second = response.pointer("/now/secondLine/title").and_then(Value::as_str);
            match (first, second) {
                (Some(first), Some(second)) => None.or(Some(Box::leak(
                    format!("{first} - {second}").into_boxed_str(),
                ) as &str)),
                (Some(first), None) => Some(first),
                _ => None,
            }
        })
        .filter(|title| !title.trim().is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .to_owned();
    Ok(ParsedInfo {
        title,
        duration: None,
        thumbnail: None,
        formats,
    })
}

fn format_from_url(
    id: &str,
    source: &str,
    known_format: Option<&str>,
) -> Result<ParsedFormat, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
    let path = url.path().to_ascii_lowercase();
    let (ext, protocol) = if known_format == Some("hls") || path.ends_with(".m3u8") {
        ("m3u8", MediaProtocol::Hls)
    } else {
        let ext = path
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
            .filter(|ext| matches!(*ext, "mp3" | "ogg" | "aac" | "m4a" | "wav" | "opus"))
            .ok_or(ExtractionError::UnsupportedUrl)?;
        (ext, MediaProtocol::Http)
    };
    Ok(ParsedFormat {
        id: id.to_owned(),
        url: source.to_owned(),
        ext: ext.to_owned(),
        protocol,
        label: id.to_owned(),
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
        extractor_name: "Radio France".to_owned(),
        formats,
    })
}

async fn get_html(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "Radio France URL is malformed",
    })?;
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

fn page_title(page: &str) -> Option<String> {
    let document = Html::parse_document(page);
    let selector = Selector::parse("h1, meta[property='og:title']").ok()?;
    for element in document.select(&selector) {
        let title = element
            .value()
            .attr("content")
            .map(str::to_owned)
            .unwrap_or_else(|| element.text().collect::<String>().trim().to_owned());
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}

fn json_object_around_marker(source: &str, marker: &str, expected: &str) -> Option<Value> {
    let marker_index = source.find(marker)?;
    if !source[marker_index..].contains(expected) {
        return None;
    }
    let mut starts = source[..marker_index]
        .match_indices('{')
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    starts.reverse();
    for start in starts {
        if let Some(value) = parse_balanced_json(&source[start..])
            && find_typed_audio_object(&value).is_some()
        {
            return Some(value);
        }
    }
    None
}

fn parse_balanced_json(source: &str) -> Option<Value> {
    let mut depth = 0usize;
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
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return serde_json::from_str(&source[..=index]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_iso_duration(value: &Value) -> Option<f64> {
    if let Some(seconds) = value.as_f64() {
        return (seconds.is_finite() && seconds >= 0.0).then_some(seconds);
    }
    let value = value.as_str()?.strip_prefix("PT")?;
    let mut current = String::new();
    let mut seconds = 0.0;
    for ch in value.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            current.push(ch);
            continue;
        }
        let number = current.parse::<f64>().ok()?;
        current.clear();
        seconds += number
            * match ch {
                'H' => 3600.0,
                'M' => 60.0,
                'S' => 1.0,
                _ => return None,
            };
    }
    (current.is_empty() && seconds.is_finite()).then_some(seconds)
}

fn parse_f64(value: &Value) -> Option<f64> {
    let value = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_podcast_archive_and_three_live_station_routes() {
        for url in [
            "https://www.radiofrance.fr/franceculture/podcasts/science-en-questions/show-8440487",
            "http://maison.radiofrance.fr/radiovisions/one-one",
            "https://www.radiofrance.fr/franceinter/",
            "https://www.radiofrance.fr/franceculture",
            "https://www.radiofrance.fr/mouv",
        ] {
            assert!(RadioFranceExtractor.can_handle(url), "{url}");
        }
        assert!(!RadioFranceExtractor.can_handle("https://www.radiofrance.fr/franceinter/podcasts/series"));
        assert!(!RadioFranceExtractor.can_handle("https://example.com/franceinter"));
    }

    #[test]
    fn parses_podcast_audio_object_and_malformed_pages() {
        let page = r#"<html><h1>Episode</h1><script type="application/ld+json">{"@type":"AudioObject","contentUrl":"https://cdn.example/episode.mp3","duration":"PT1H2M3S","name":"Episode"}</script></html>"#;
        let parsed = parse_podcast_page(page).unwrap();
        assert_eq!(parsed.title, "Episode");
        assert_eq!(parsed.duration, Some(3723.0));
        assert_eq!(parsed.formats[0].ext, "mp3");
        assert!(parse_podcast_page("<html><script>{broken}</script></html>").is_err());
    }

    #[test]
    fn parses_legacy_audio_sources_and_ignores_unsupported_formats() {
        let page = r#"<h1>Archive</h1><div class="jp-jplayer" data-source="mp3:'https://cdn.example/audio.mp3',ogg:'https://cdn.example/audio.ogg',m3u8:'https://cdn.example/audio.m3u8'"></div>"#;
        let parsed = parse_legacy_page(page).unwrap();
        assert_eq!(parsed.formats.len(), 2);
        assert!(parse_legacy_page("<h1>Archive</h1><div data-source='bad'></div>").is_err());
    }

    #[test]
    fn parses_live_api_sources_and_declines_malformed_responses() {
        let response = serde_json::json!({
            "visual": {"legend": "France Inter"},
            "now": {"media": {"sources": [
                {"url": "https://cdn.example/live.aac", "format": "aac"},
                {"url": "https://cdn.example/live.m3u8", "format": "hls"}
            ]}}
        });
        let parsed = parse_live_response(&response).unwrap();
        assert_eq!(parsed.title, "France Inter");
        assert_eq!(parsed.formats.len(), 2);
        assert_eq!(parsed.formats[1].protocol, MediaProtocol::Hls);
        assert!(parse_live_response(&serde_json::json!({})).is_err());
    }

    #[tokio::test]
    async fn refuses_caller_credentials_so_registry_can_fall_back() {
        let options = ExtractOptions {
            cookies: Some("user-cookie".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            RadioFranceExtractor
                .extract("https://www.radiofrance.fr/franceinter/", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

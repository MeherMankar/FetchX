//! Public Vimeo config extraction for selected site wrappers.
//!
//! This limited translation follows the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense):
//! `vimeo.py`, `aeonco.py`, `amara.py`, and `croatianfilm.py`. It supports
//! public progressive files and classifies manifest URLs without expansion.
//! Login, passwords, private/embed-only access, playlists, subtitles, and
//! authenticated API clients remain unsupported.

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
const MAX_FORMATS: usize = 64;

pub struct VimeoExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    Video(String),
    Aeon(String),
    Amara(String),
    CroatianFilm(String),
}

#[async_trait]
impl MediaExtractor for VimeoExtractor {
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

impl VimeoExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let video_id = match parse_route(source).ok_or(ExtractionError::UnsupportedUrl)? {
            Route::Video(video_id) => video_id,
            Route::Aeon(display_id) => resolve_aeon(source, &display_id, timeout).await?,
            Route::Amara(display_id) => resolve_amara(&display_id, timeout).await?,
            Route::CroatianFilm(display_id) => resolve_croatian_film(&display_id, timeout).await?,
        };
        let config_url = Url::parse(&format!("https://player.vimeo.com/video/{video_id}/config"))
            .map_err(|_| ExtractionError::InvalidOutput)?;
        let config = get_json(&config_url, timeout).await?;
        build_media_info(source, &video_id, &config).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let segments = url.path_segments().map(|segments| {
        segments
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
    })?;
    if host == "player.vimeo.com" && segments.len() == 2 && segments[0] == "video" {
        return valid_video_id(segments[1]).then(|| Route::Video(segments[1].to_owned()));
    }
    if host == "vimeo.com" && segments.len() == 1 && valid_video_id(segments[0]) {
        return Some(Route::Video(segments[0].to_owned()));
    }
    if host == "aeon.co" && segments.len() >= 2 && segments[0] == "videos" {
        return Some(Route::Aeon(segments[1].to_owned()));
    }
    if host == "amara.org" {
        let videos_index = segments.iter().position(|segment| *segment == "videos")?;
        let display_id = segments.get(videos_index + 1)?;
        if valid_wrapper_id(display_id) {
            return Some(Route::Amara((*display_id).to_owned()));
        }
    }
    if host == "croatian.film"
        && segments.len() >= 3
        && segments[0].len() == 2
        && segments[0].bytes().all(|byte| byte.is_ascii_lowercase())
        && segments[1] == "films"
        && segments[2].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Some(Route::CroatianFilm(segments[2].to_owned()));
    }
    None
}

fn valid_video_id(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_wrapper_id(value: &str) -> bool {
    (5..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

async fn resolve_aeon(
    source: &str,
    _display_id: &str,
    timeout: Duration,
) -> Result<String, ExtractionError> {
    let page = get_html(source, timeout).await?;
    parse_aeon_embed(&page)?.ok_or(ExtractionError::UnsupportedUrl)
}

fn parse_aeon_embed(page: &str) -> Result<Option<String>, ExtractionError> {
    let document = Html::parse_document(page);
    let selector = Selector::parse(r#"script[type="application/ld+json"]"#)
        .map_err(|_| ExtractionError::InvalidOutput)?;
    for script in document.select(&selector) {
        let Ok(value) = serde_json::from_str::<Value>(&script.text().collect::<String>()) else {
            continue;
        };
        if let Some(embed) = find_string(&value, "embedUrl")
            && let Some(video_id) = vimeo_id_from_url(embed)
        {
            return Ok(Some(video_id));
        }
    }
    Ok(None)
}

async fn resolve_amara(display_id: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let mut url = Url::parse(&format!("https://amara.org/api/videos/{display_id}/"))
        .map_err(|_| ExtractionError::InvalidOutput)?;
    url.query_pairs_mut().append_pair("format", "json");
    let response = get_json(&url, timeout).await?;
    let urls = response
        .get("all_urls")
        .and_then(Value::as_array)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    for value in urls.iter().filter_map(Value::as_str) {
        if let Some(video_id) = vimeo_id_from_url(value) {
            return Ok(video_id);
        }
    }
    Err(ExtractionError::UnsupportedUrl)
}

async fn resolve_croatian_film(
    display_id: &str,
    timeout: Duration,
) -> Result<String, ExtractionError> {
    let url = Url::parse(&format!(
        "https://api.croatian.film/api/videos/{display_id}"
    ))
    .map_err(|_| ExtractionError::InvalidOutput)?;
    let response = get_json(&url, timeout).await?;
    if response.get("errors").is_some() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let vimeo_url = response
        .pointer("/video/vimeoURL")
        .and_then(Value::as_str)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    vimeo_id_from_url(vimeo_url).ok_or(ExtractionError::UnsupportedUrl)
}

fn vimeo_id_from_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let host = url.host_str()?.trim_start_matches("www.");
    if !matches!(host, "vimeo.com" | "player.vimeo.com") {
        return None;
    }
    let segments = url.path_segments()?.filter(|segment| !segment.is_empty());
    segments
        .filter(|segment| segment.bytes().all(|byte| byte.is_ascii_digit()))
        .find(|segment| valid_video_id(segment))
        .map(str::to_owned)
}

async fn get_html(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "Vimeo wrapper URL is malformed",
    })?;
    let response = provider_http::get(&url, "text/html,application/xhtml+xml", timeout).await?;
    if !response.status.is_success() || !response.content_type.as_deref().is_some_and(is_html) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    String::from_utf8(response.body).map_err(|_| ExtractionError::InvalidOutput)
}

async fn get_json(url: &Url, timeout: Duration) -> Result<Value, ExtractionError> {
    let response = provider_http::get(url, "application/json", timeout).await?;
    if !response.status.is_success() || !response.content_type.as_deref().is_some_and(is_json) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    serde_json::from_slice(&response.body).map_err(|_| ExtractionError::InvalidOutput)
}

fn is_html(content_type: &str) -> bool {
    matches!(
        content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "text/html" | "application/xhtml+xml"
    )
}

fn is_json(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    mime == "application/json" || mime.ends_with("+json")
}

fn find_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Object(object) => object
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| object.values().find_map(|value| find_string(value, key))),
        Value::Array(values) => values.iter().find_map(|value| find_string(value, key)),
        _ => None,
    }
}

async fn build_media_info(
    source: &str,
    video_id: &str,
    config: &Value,
) -> Result<MediaInfo, ExtractionError> {
    let video = config.get("video").ok_or(ExtractionError::UnsupportedUrl)?;
    let request = config.get("request").unwrap_or(&Value::Null);
    let files = video
        .get("files")
        .filter(|files| files.as_object().is_some_and(|files| !files.is_empty()))
        .or_else(|| request.get("files"))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let title = video
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .to_owned();
    let mut formats = Vec::new();
    if let Some(progressive) = files.get("progressive").and_then(Value::as_array) {
        for file in progressive.iter().take(MAX_FORMATS) {
            let Some(url) = file.get("url").and_then(Value::as_str) else {
                continue;
            };
            let Ok(url) = Url::parse(url) else {
                continue;
            };
            let Ok(url) = validate_public_url(url.as_str()).await else {
                continue;
            };
            let quality = file
                .get("quality")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let ext = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .and_then(|name| {
                    name.rsplit_once('.')
                        .map(|(_, ext)| ext.to_ascii_lowercase())
                })
                .filter(|ext| matches!(ext.as_str(), "mp4" | "mov" | "webm"))
                .unwrap_or_else(|| "mp4".to_owned());
            let id = format!("http-{quality}");
            formats.push(MediaFormat {
                id: id.clone(),
                ext: ext.clone(),
                protocol: MediaProtocol::Http,
                url: url.to_string(),
                label: format!("{quality} - {ext}"),
                video_codec: None,
                audio_codec: None,
                width: file.get("width").and_then(parse_u32),
                height: file.get("height").and_then(parse_u32),
                fps: file.get("fps").and_then(parse_f64),
                bitrate: file.get("bitrate").and_then(parse_u64),
                filesize: None,
                required_headers: Vec::new(),
            });
        }
    }
    for (field, protocol, ext) in [
        ("hls", MediaProtocol::Hls, "m3u8"),
        ("dash", MediaProtocol::Dash, "mpd"),
    ] {
        let Some(cdns) = files
            .get(field)
            .and_then(|value| value.get("cdns"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (cdn, data) in cdns.iter().take(MAX_FORMATS.saturating_sub(formats.len())) {
            let Some(manifest) = data.get("url").and_then(Value::as_str) else {
                continue;
            };
            let Ok(manifest) = Url::parse(manifest) else {
                continue;
            };
            let Ok(manifest) = validate_public_url(manifest.as_str()).await else {
                continue;
            };
            formats.push(MediaFormat {
                id: format!("{field}-{cdn}"),
                ext: ext.to_owned(),
                protocol: protocol.clone(),
                url: manifest.to_string(),
                label: format!("{field} - {cdn}"),
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
    let thumbnail = video
        .get("thumbs")
        .and_then(|thumbs| thumbs.get("base"))
        .and_then(Value::as_str)
        .or_else(|| video.get("thumb").and_then(Value::as_str));
    let thumbnail_url = match thumbnail {
        Some(value) => validate_public_url(value)
            .await
            .ok()
            .map(|url| url.to_string()),
        None => None,
    };
    Ok(MediaInfo {
        title,
        duration: video
            .get("duration")
            .and_then(parse_f64)
            .filter(|value| value.is_finite() && *value >= 0.0),
        thumbnail_url,
        webpage_url: source.to_owned(),
        extractor_name: format!("Vimeo ({video_id})"),
        formats,
    })
}

fn parse_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn parse_u32(value: &Value) -> Option<u32> {
    parse_u64(value).and_then(|value| u32::try_from(value).ok())
}

fn parse_f64(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_vimeo_and_the_selected_site_routes() {
        for url in [
            "https://aeon.co/videos/example-feature",
            "https://amara.org/en/videos/kYkK1VUTWW5I/info/example",
            "https://www.croatian.film/hr/films/72472",
            "https://player.vimeo.com/video/123456789",
        ] {
            assert!(VimeoExtractor.can_handle(url), "{url}");
        }
        assert!(!VimeoExtractor.can_handle("https://notaeon.co/videos/example"));
        assert!(!VimeoExtractor.can_handle("https://www.croatian.film/en/films/not-a-number"));
    }

    #[test]
    fn resolves_only_vimeo_video_links_and_ids() {
        assert_eq!(
            vimeo_id_from_url("https://player.vimeo.com/video/123456789"),
            Some("123456789".to_owned())
        );
        assert_eq!(
            vimeo_id_from_url("https://vimeo.com/123456789/secret"),
            Some("123456789".to_owned())
        );
        assert_eq!(
            vimeo_id_from_url("https://youtube.com/watch?v=123456789"),
            None
        );
    }

    #[test]
    fn resolves_aeon_json_ld_embed_without_following_external_urls() {
        let page = r#"<script type="application/ld+json">{"@type":"VideoObject","embedUrl":"https://player.vimeo.com/video/123456789"}</script>"#;
        assert_eq!(
            parse_aeon_embed(page).unwrap(),
            Some("123456789".to_owned())
        );
        assert_eq!(
            parse_aeon_embed(
                r#"<script type="application/ld+json">{"embedUrl":"https://example.com/video/123456789"}</script>"#
            )
            .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn maps_public_progressive_and_manifest_formats() {
        let config = serde_json::json!({
            "video": {
                "title": "Public Vimeo",
                "duration": 12,
                "thumbs": {"base": "https://i.vimeocdn.com/video/poster.jpg"},
                "files": {
                    "progressive": [{
                        "url": "https://vod-progressive.akamaized.net/video.mp4",
                        "quality": "720p",
                        "width": 1280,
                        "height": 720,
                        "fps": 30
                    }],
                    "hls": {"cdns": {"ak": {"url": "https://vod-adaptive.akamaized.net/master.m3u8"}}},
                    "dash": {"cdns": {"ak": {"url": "https://vod-adaptive.akamaized.net/master.mpd"}}}
                }
            }
        });
        let info = build_media_info("https://vimeo.com/123456789", "123456789", &config)
            .await
            .unwrap();
        assert_eq!(info.title, "Public Vimeo");
        assert_eq!(info.formats.len(), 3);
        assert!(info.formats.iter().any(|format| format.ext == "mp4"));
        assert!(
            info.formats
                .iter()
                .any(|format| format.protocol == MediaProtocol::Hls)
        );
        assert!(
            info.formats
                .iter()
                .any(|format| format.protocol == MediaProtocol::Dash)
        );
        assert!(
            build_media_info(
                "https://vimeo.com/123456789",
                "123456789",
                &serde_json::json!({
                    "video": {"title": "Restricted", "files": {"progressive": []}}
                })
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn declines_user_cookies_for_the_existing_fallback() {
        let options = ExtractOptions {
            cookies: Some("unlogged-cookie-value".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            VimeoExtractor
                .extract("https://aeon.co/videos/example-feature", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

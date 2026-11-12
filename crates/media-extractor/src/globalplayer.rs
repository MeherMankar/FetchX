//! Limited public GlobalPlayer live, audio-episode, and video extraction.
//!
//! Adapted from the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense),
//! `yt_dlp/extractor/globalplayer.py`. Playlist expansion is unsupported.

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

pub struct GlobalPlayerExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    Live { station: String },
    AudioEpisode { id: String, podcast: bool },
    Video { id: String },
}

#[derive(Debug)]
struct ParsedInfo {
    id: String,
    title: String,
    thumbnail: Option<String>,
    duration: Option<f64>,
    url: String,
    default_ext: Option<&'static str>,
}

#[async_trait]
impl MediaExtractor for GlobalPlayerExtractor {
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

impl GlobalPlayerExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let page = get_page_props(source, timeout).await?;
        let parsed = match route {
            Route::Live { station: _ } => {
                let station_meta = page
                    .get("station")
                    .filter(|value| value.is_object())
                    .ok_or(ExtractionError::UnsupportedUrl)?;
                let station_id =
                    string_field(station_meta, "id").ok_or(ExtractionError::UnsupportedUrl)?;
                let playable = get_playable(&station_id, timeout).await?;
                let stream = playback_url(&playable)?;
                ParsedInfo {
                    id: station_id,
                    title: required_string(station_meta, "name")?,
                    thumbnail: string_field(station_meta, "brandLogo"),
                    duration: None,
                    url: stream,
                    default_ext: Some("aac"),
                }
            }
            Route::AudioEpisode { id, podcast } => {
                let meta = if podcast {
                    page.pointer("/podcastEpisode/metadata")
                } else {
                    page.pointer("/catchupEpisode/metadata")
                }
                .ok_or(ExtractionError::UnsupportedUrl)?;
                let playable = get_playable(&id, timeout).await?;
                ParsedInfo {
                    id,
                    title: required_string(meta, "title")?,
                    thumbnail: meta
                        .pointer("/image/url")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    duration: meta.get("duration").and_then(number),
                    url: playback_url(&playable)?,
                    default_ext: None,
                }
            }
            Route::Video { id } => {
                let meta = page
                    .get("videoData")
                    .filter(|value| value.is_object())
                    .ok_or(ExtractionError::UnsupportedUrl)?;
                if is_restricted(meta) {
                    return Err(ExtractionError::UnsupportedUrl);
                }
                ParsedInfo {
                    id,
                    title: required_string(meta, "title")?,
                    thumbnail: meta
                        .pointer("/image/url")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    duration: meta.get("duration").and_then(number),
                    url: required_string(meta, "url")?,
                    default_ext: None,
                }
            }
        };
        build_info(source, parsed).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(url.host_str()?, "www.globalplayer.com" | "globalplayer.com")
    {
        return None;
    }
    let segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        ["live", station, _region] if valid_id(station) => Some(Route::Live {
            station: (*station).to_owned(),
        }),
        ["podcasts", "episodes", id] if valid_id(id) => Some(Route::AudioEpisode {
            id: (*id).to_owned(),
            podcast: true,
        }),
        ["catchup", _station, _region, "episodes", id] if valid_id(id) => {
            Some(Route::AudioEpisode {
                id: (*id).to_owned(),
                podcast: false,
            })
        }
        ["videos", id] if valid_id(id) => Some(Route::Video {
            id: (*id).to_owned(),
        }),
        _ => None,
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

async fn get_page_props(source: &str, timeout: Duration) -> Result<Value, ExtractionError> {
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
    let page = String::from_utf8(response.body).map_err(|_| ExtractionError::InvalidOutput)?;
    parse_page_props(&page)
}

fn parse_page_props(page: &str) -> Result<Value, ExtractionError> {
    let document = Html::parse_document(page);
    let selector =
        Selector::parse("script#__NEXT_DATA__").map_err(|_| ExtractionError::InvalidOutput)?;
    let script = document
        .select(&selector)
        .next()
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let value: Value = serde_json::from_str(&script.text().collect::<String>())
        .map_err(|_| ExtractionError::InvalidOutput)?;
    value
        .pointer("/props/pageProps")
        .filter(|props| props.is_object())
        .cloned()
        .ok_or(ExtractionError::UnsupportedUrl)
}

async fn get_playable(id: &str, timeout: Duration) -> Result<Value, ExtractionError> {
    if !valid_id(id) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let url = Url::parse(&format!(
        "https://bff-web-guacamole.musicradio.com/playables/{id}"
    ))
    .map_err(|_| ExtractionError::InvalidOutput)?;
    let response = provider_http::get(&url, "application/json", timeout).await?;
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

fn playback_url(data: &Value) -> Result<String, ExtractionError> {
    data.get("playback")
        .and_then(Value::as_array)
        .and_then(|playbacks| {
            playbacks.iter().find_map(|playback| {
                let can_use = playback.get("canUse").is_some_and(|value| {
                    value.as_str() == Some("true") || value.as_bool() == Some(true)
                });
                (can_use && !is_restricted(playback))
                    .then(|| playback.get("url").and_then(Value::as_str))
                    .flatten()
            })
        })
        .map(str::to_owned)
        .ok_or(ExtractionError::UnsupportedUrl)
}

fn is_restricted(value: &Value) -> bool {
    ["drm", "protection", "keySystemOptions", "encrypted"]
        .iter()
        .any(|key| {
            value
                .get(key)
                .is_some_and(|flag| !flag.is_null() && flag != false)
        })
}

async fn build_info(source: &str, parsed: ParsedInfo) -> Result<MediaInfo, ExtractionError> {
    let url = validate_public_url(&parsed.url).await?;
    let path = url.path().to_ascii_lowercase();
    let ext = path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .filter(|ext| {
            matches!(
                *ext,
                "aac" | "m4a" | "mp3" | "ogg" | "opus" | "wav" | "mp4" | "webm" | "m3u8" | "mpd"
            )
        })
        .or(parsed.default_ext)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let protocol = match ext {
        "m3u8" => MediaProtocol::Hls,
        "mpd" => MediaProtocol::Dash,
        _ => MediaProtocol::Http,
    };
    let thumbnail_url = match parsed.thumbnail.as_deref() {
        Some(value) => validate_public_url(value)
            .await
            .ok()
            .map(|value| value.to_string()),
        None => None,
    };
    let mut info = MediaInfo {
        title: parsed.title,
        duration: parsed.duration,
        thumbnail_url,
        webpage_url: source.to_owned(),
        extractor_name: "GlobalPlayer".to_owned(),
        formats: vec![MediaFormat {
            id: parsed.id,
            ext: ext.to_owned(),
            protocol,
            url: url.to_string(),
            label: "GlobalPlayer stream".to_owned(),
            video_codec: None,
            audio_codec: None,
            width: None,
            height: None,
            fps: None,
            bitrate: None,
            filesize: None,
            required_headers: Vec::new(),
        }],
    };
    info.sanitise_format_urls();
    if info.formats.is_empty() {
        return Err(ExtractionError::NoFormats);
    }
    Ok(info)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_string(value: &Value, key: &str) -> Result<String, ExtractionError> {
    string_field(value, key).ok_or(ExtractionError::UnsupportedUrl)
}

fn number(value: &Value) -> Option<f64> {
    let value = value.as_f64()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_registered_globalplayer_single_item_routes() {
        for source in [
            "https://www.globalplayer.com/live/smoothchill/uk/",
            "https://www.globalplayer.com/podcasts/episodes/7DrorSc/",
            "https://www.globalplayer.com/catchup/lbc/uk/episodes/2zGmrV6DnvogKkNCXkwkQ8HQTA/",
            "https://www.globalplayer.com/videos/2JsSZ7Gm2uP/",
        ] {
            assert!(GlobalPlayerExtractor.can_handle(source), "{source}");
        }
        assert!(!GlobalPlayerExtractor.can_handle("https://www.globalplayer.com/playlists/8bLk/"));
        assert!(!GlobalPlayerExtractor.can_handle("https://example.com/videos/123"));
    }

    #[test]
    fn parses_page_props_and_rejects_malformed_or_ambiguous_pages() {
        let page = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"videoData":{"title":"Clip","url":"https://cdn.example/clip.mp4"}}}}</script>"#;
        let props = parse_page_props(page).unwrap();
        let video = props.get("videoData").unwrap();
        assert_eq!(required_string(video, "title").unwrap(), "Clip");
        assert!(parse_page_props("<html></html>").is_err());
        assert!(
            parse_page_props(r#"<script id="__NEXT_DATA__">{"props":{"pageProps":[]}}</script>"#)
                .is_err()
        );
    }

    #[test]
    fn selects_only_publicly_playable_sources_and_rejects_malformed_responses() {
        let response = serde_json::json!({
            "playback": [
                {"canUse": "false", "url": "https://cdn.example/restricted.mp3"},
                {"canUse": "true", "url": "https://cdn.example/drm.mp3", "protection": {"widevine": true}},
                {"canUse": "true", "url": "https://cdn.example/episode.mp3"}
            ]
        });
        assert_eq!(
            playback_url(&response).unwrap(),
            "https://cdn.example/episode.mp3"
        );
        assert!(playback_url(&serde_json::json!({"playback": []})).is_err());
        assert!(playback_url(&serde_json::json!({"playback": [{"canUse":"true"}]})).is_err());
    }

    #[tokio::test]
    async fn declines_caller_credentials_for_registry_fallback() {
        let options = ExtractOptions {
            cookies: Some("caller-cookie".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            GlobalPlayerExtractor
                .extract("https://www.globalplayer.com/videos/2JsSZ7Gm2uP/", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

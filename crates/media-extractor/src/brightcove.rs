//! Public, bounded Brightcove Playback API extraction.
//!
//! This is a focused translation of the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense):
//! `yt_dlp/extractor/brightcove.py` and the NineNews, Craftsy, and AMC
//! Networks wrappers. Authenticated, legacy, playlist, DRM, and manifest
//! expansion paths remain unsupported.

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

pub struct BrightcoveExtractor;

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlayerRef {
    account_id: String,
    player_id: String,
    embed_id: String,
    video_id: String,
}

#[async_trait]
impl MediaExtractor for BrightcoveExtractor {
    fn can_handle(&self, source: &str) -> bool {
        parse_player_url(source).is_some() || parse_site_route(source)
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

impl BrightcoveExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let player = if let Some(player) = parse_player_url(source) {
            player
        } else {
            discover_site_player(source, timeout).await?
        };
        let config_url = Url::parse(&format!(
            "https://players.brightcove.net/{}/{}/config.json",
            player.account_id,
            player_key_path(&player)
        ))
        .map_err(|_| ExtractionError::InvalidOutput)?;
        let config = get_json(&config_url, "application/json", timeout).await?;
        let mut policy_key = config
            .pointer("/video_cloud/policy_key")
            .and_then(Value::as_str)
            .map(str::to_owned);

        if policy_key.is_none() {
            let script_url = config_url
                .join("index.min.js")
                .map_err(|_| ExtractionError::InvalidOutput)?;
            let script_response =
                provider_http::get(&script_url, "application/javascript,*/*", timeout).await?;
            if !script_response.status.is_success() {
                return Err(ExtractionError::UnsupportedUrl);
            }
            let script = std::str::from_utf8(&script_response.body)
                .map_err(|_| ExtractionError::InvalidOutput)?;
            policy_key = extract_policy_key(script);
        }
        let policy_key = policy_key.ok_or(ExtractionError::UnsupportedUrl)?;
        if policy_key.is_empty() || policy_key.len() > 4096 {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let api_url = Url::parse(&format!(
            "https://edge.api.brightcove.com/playback/v1/accounts/{}/videos/{}",
            player.account_id, player.video_id
        ))
        .map_err(|_| ExtractionError::InvalidOutput)?;
        let mut response_headers = reqwest::header::HeaderMap::new();
        let accept = format!("application/json;pk={policy_key}")
            .parse()
            .map_err(|_| ExtractionError::InvalidOutput)?;
        response_headers.insert(reqwest::header::ACCEPT, accept);
        let response = provider_http::get_with_headers(&api_url, response_headers, timeout).await?;
        if !response.status.is_success() || !is_json(response.content_type.as_deref()) {
            return Err(ExtractionError::UnsupportedUrl);
        }
        let json: Value =
            serde_json::from_slice(&response.body).map_err(|_| ExtractionError::InvalidOutput)?;
        let parsed = parse_playback_response(&json)?;
        let mut formats = Vec::new();
        for parsed_format in parsed.formats.into_iter().take(MAX_FORMATS) {
            if let Ok(url) = validate_public_url(parsed_format.url.as_str()).await {
                formats.push(MediaFormat {
                    id: parsed_format.id,
                    ext: parsed_format.ext,
                    protocol: parsed_format.protocol,
                    url: url.to_string(),
                    label: parsed_format.label,
                    video_codec: parsed_format.video_codec,
                    audio_codec: None,
                    width: parsed_format.width,
                    height: parsed_format.height,
                    fps: None,
                    bitrate: parsed_format.bitrate,
                    filesize: parsed_format.filesize,
                    required_headers: Vec::new(),
                });
            }
        }
        if formats.is_empty() {
            return Err(ExtractionError::UnsupportedUrl);
        }
        let thumbnail_url = match parsed.thumbnail {
            Some(url) => validate_public_url(&url)
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
            extractor_name: "Brightcove".to_owned(),
            formats,
        })
    }
}

#[derive(Debug)]
struct ParsedPlayback {
    title: String,
    duration: Option<f64>,
    thumbnail: Option<String>,
    formats: Vec<ParsedFormat>,
}

#[derive(Debug)]
struct ParsedFormat {
    id: String,
    url: Url,
    ext: String,
    protocol: MediaProtocol,
    label: String,
    width: Option<u32>,
    height: Option<u32>,
    bitrate: Option<u64>,
    filesize: Option<u64>,
    video_codec: Option<String>,
}

fn parse_player_url(source: &str) -> Option<PlayerRef> {
    let url = Url::parse(source).ok()?;
    if url.host_str()? != "players.brightcove.net" {
        return None;
    }
    let segments = url.path_segments()?.collect::<Vec<_>>();
    if segments.len() != 3 || segments[2] != "index.html" {
        return None;
    }
    let account_id = segments[0];
    if account_id.is_empty() || !account_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (player_id, embed_id) = segments[1].rsplit_once('_')?;
    if player_id.is_empty()
        || embed_id.is_empty()
        || !player_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || !embed_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    let ids = url
        .query_pairs()
        .filter(|(key, _)| matches!(key.as_ref(), "videoId" | "videoID" | "idVideo"))
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    if ids.len() != 1 || ids[0].is_empty() || !ids[0].bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(PlayerRef {
        account_id: account_id.to_owned(),
        player_id: player_id.to_owned(),
        embed_id: embed_id.to_owned(),
        video_id: ids[0].clone(),
    })
}

fn player_key_path(player: &PlayerRef) -> String {
    format!("{}_{}", player.player_id, player.embed_id)
}

fn parse_site_route(source: &str) -> bool {
    let Ok(url) = Url::parse(source) else {
        return false;
    };
    if url.scheme() != "https" && url.scheme() != "http" {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    if (host == "craftsy.com" || host == "www.craftsy.com")
        && url.path_segments().is_some_and(|mut segments| {
            segments.next() == Some("class") && segments.next().is_some()
        })
    {
        return true;
    }
    if (host == "9news.com.au" || host == "www.9news.com.au")
        && url
            .path_segments()
            .is_some_and(|segments| segments.count() >= 3)
    {
        return true;
    }
    matches!(
        host,
        "amc.com" | "www.amc.com" | "bbcamerica.com" | "www.bbcamerica.com"
    ) && url.path().starts_with("/shows/")
}

async fn discover_site_player(
    source: &str,
    timeout: Duration,
) -> Result<PlayerRef, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "Brightcove page URL is malformed",
    })?;
    let response = provider_http::get(&url, "text/html,application/xhtml+xml", timeout).await?;
    if !response.status.is_success() || !response.content_type.as_deref().is_some_and(is_html) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let html = std::str::from_utf8(&response.body).map_err(|_| ExtractionError::InvalidOutput)?;
    match url.host_str().unwrap_or_default() {
        "craftsy.com" | "www.craftsy.com" => parse_craftsy_player(html),
        "9news.com.au" | "www.9news.com.au" => parse_ninenews_player(html),
        _ => parse_amc_player(html),
    }
}

fn parse_craftsy_player(html: &str) -> Result<PlayerRef, ExtractionError> {
    let document = Html::parse_document(html);
    let selector =
        Selector::parse("video-js[data-video-id]").map_err(|_| ExtractionError::InvalidOutput)?;
    let player = document
        .select(&selector)
        .next()
        .ok_or(ExtractionError::UnsupportedUrl)?;
    player_ref(
        player.value().attr("data-account"),
        Some("default"),
        Some("default"),
        player.value().attr("data-video-id"),
    )
}

fn parse_ninenews_player(html: &str) -> Result<PlayerRef, ExtractionError> {
    let state = json_after_marker(html, "__INITIAL_STATE__")?;
    let video_id = state
        .pointer("/videoIndex/currentVideo/brightcoveId")
        .and_then(Value::as_str)
        .or_else(|| {
            state
                .pointer("/article/media")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("type").and_then(Value::as_str) == Some("video"))
                })
                .and_then(|item| item.pointer("/urn").and_then(Value::as_str))
        });
    let account_id = state
        .pointer("/videoIndex/config/account")
        .and_then(Value::as_str)
        .or_else(|| {
            state
                .pointer("/videoIndex/config/video/account")
                .and_then(Value::as_str)
        });
    player_ref(account_id, Some("default"), Some("default"), video_id)
}

fn parse_amc_player(html: &str) -> Result<PlayerRef, ExtractionError> {
    let state = json_after_marker(html, "window.initialData")?;
    let initial = state.get("initialData").unwrap_or(&state);
    let video_id = initial
        .pointer("/properties/videoId")
        .and_then(Value::as_str);
    let account_id = initial
        .pointer("/config/brightcove/accountId")
        .and_then(Value::as_str);
    let player_id = initial
        .pointer("/config/brightcove/playerId")
        .and_then(Value::as_str)
        .unwrap_or("default");
    player_ref(account_id, Some(player_id), Some("default"), video_id)
}

fn player_ref(
    account_id: Option<&str>,
    player_id: Option<&str>,
    embed_id: Option<&str>,
    video_id: Option<&str>,
) -> Result<PlayerRef, ExtractionError> {
    let valid_numeric =
        |value: &str| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    let account_id = account_id.filter(|value| valid_numeric(value));
    let video_id = video_id.filter(|value| valid_numeric(value));
    let player_id = player_id.filter(|value| !value.is_empty());
    let embed_id = embed_id.filter(|value| !value.is_empty());
    match (account_id, player_id, embed_id, video_id) {
        (Some(account_id), Some(player_id), Some(embed_id), Some(video_id)) => Ok(PlayerRef {
            account_id: account_id.to_owned(),
            player_id: player_id.to_owned(),
            embed_id: embed_id.to_owned(),
            video_id: video_id.to_owned(),
        }),
        _ => Err(ExtractionError::UnsupportedUrl),
    }
}

fn json_after_marker(source: &str, marker: &str) -> Result<Value, ExtractionError> {
    let marker_index = source.find(marker).ok_or(ExtractionError::UnsupportedUrl)?;
    let after_marker = &source[marker_index + marker.len()..];
    if marker == "window.initialData"
        && let Some(template_start) = after_marker.find("String.raw`")
    {
        let template = &after_marker[template_start + "String.raw`".len()..];
        let end = template.find("`)").ok_or(ExtractionError::UnsupportedUrl)?;
        return serde_json::from_str(&template[..end]).map_err(|_| ExtractionError::InvalidOutput);
    }
    let json_start = after_marker
        .find('{')
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let json = &after_marker[json_start..];
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in json.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return serde_json::from_str(&json[..=index])
                        .map_err(|_| ExtractionError::InvalidOutput);
                }
            }
            _ => {}
        }
    }
    Err(ExtractionError::UnsupportedUrl)
}

fn extract_policy_key(script: &str) -> Option<String> {
    for marker in ["policyKey:", "policyKey :"] {
        for (index, _) in script.match_indices(marker) {
            let mut value = script[index + marker.len()..].trim_start();
            value = value.strip_prefix(['\'', '"'])?;
            let end = value.find(['\'', '"'])?;
            let key = &value[..end];
            if !key.is_empty() {
                return Some(key.to_owned());
            }
        }
    }
    None
}

async fn get_json(url: &Url, accept: &str, timeout: Duration) -> Result<Value, ExtractionError> {
    let response = provider_http::get(url, accept, timeout).await?;
    if !response.status.is_success() || !is_json(response.content_type.as_deref()) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    serde_json::from_slice(&response.body).map_err(|_| ExtractionError::InvalidOutput)
}

fn is_json(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let mime = value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        mime == "application/json" || mime.ends_with("+json")
    })
}

fn is_html(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|mime| matches!(mime.trim(), "text/html" | "application/xhtml+xml"))
}

fn parse_playback_response(response: &Value) -> Result<ParsedPlayback, ExtractionError> {
    let title = response
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .to_owned();
    let sources = response
        .get("sources")
        .and_then(Value::as_array)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let mut formats = Vec::new();
    for source in sources {
        let container = source
            .get("container")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let media_type = source
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if container.eq_ignore_ascii_case("WVM")
            || container.eq_ignore_ascii_case("ISM")
            || source
                .get("key_systems")
                .is_some_and(|value| value.as_object().is_some_and(|map| !map.is_empty()))
        {
            continue;
        }
        let Some(source_url) = source.get("src").and_then(Value::as_str) else {
            continue;
        };
        let Ok(url) = Url::parse(source_url) else {
            continue;
        };
        let ext = extension(&url, &media_type, container);
        if !matches!(
            ext.as_str(),
            "mp4" | "webm" | "mov" | "mp3" | "m4a" | "m3u8" | "mpd"
        ) {
            continue;
        }
        let protocol = match ext.as_str() {
            "m3u8" => MediaProtocol::Hls,
            "mpd" => MediaProtocol::Dash,
            _ => MediaProtocol::Http,
        };
        let width = source.get("width").and_then(parse_u32);
        let height = source.get("height").and_then(parse_u32);
        let bitrate = source.get("avg_bitrate").and_then(parse_u64);
        let format_id = format!(
            "{}{}{}",
            if protocol == MediaProtocol::Http {
                "http"
            } else if protocol == MediaProtocol::Hls {
                "hls"
            } else {
                "dash"
            },
            height.map(|value| format!("-{value}p")).unwrap_or_default(),
            bitrate.map(|value| format!("-{value}")).unwrap_or_default()
        );
        formats.push(ParsedFormat {
            id: format_id.clone(),
            url,
            ext,
            protocol,
            label: format_id,
            width,
            height,
            bitrate,
            filesize: source.get("size").and_then(parse_u64),
            video_codec: source
                .get("codec")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let duration = response
        .get("duration")
        .and_then(parse_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|milliseconds| milliseconds / 1000.0);
    let thumbnail = response
        .get("poster")
        .or_else(|| response.get("thumbnail"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(ParsedPlayback {
        title,
        duration,
        thumbnail,
        formats,
    })
}

fn extension(url: &Url, media_type: &str, container: &str) -> String {
    if media_type.contains("mpegurl") || media_type == "application/x-mpegurl" {
        return "m3u8".to_owned();
    }
    if media_type.contains("dash+xml") {
        return "mpd".to_owned();
    }
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|segment| {
            segment
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .filter(|value| value.len() <= 8 && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .or_else(|| {
            match media_type {
                "video/mp4" => Some("mp4"),
                "video/webm" => Some("webm"),
                "video/quicktime" => Some("mov"),
                "audio/mp4" => Some("m4a"),
                "audio/mpeg" => Some("mp3"),
                _ => match container.to_ascii_lowercase().as_str() {
                    "mp4" => Some("mp4"),
                    "webm" => Some("webm"),
                    "mov" => Some("mov"),
                    _ => None,
                },
            }
            .map(str::to_owned)
        })
        .unwrap_or_default()
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
    fn recognizes_brightcove_and_three_registered_site_routes() {
        for url in [
            "https://players.brightcove.net/123456789/default_default/index.html?videoId=987654321",
            "https://www.craftsy.com/class/test-course/",
            "https://www.9news.com.au/news/example/story-id",
            "https://www.amc.com/shows/example/videos/example-video",
        ] {
            assert!(BrightcoveExtractor.can_handle(url), "{url}");
        }
        for url in [
            "https://players.brightcove.net/123456789/default_default/index.html?playlistId=123",
            "https://notcraftsy.com/class/test",
            "https://amc.com/not-a-show/video",
        ] {
            assert!(!BrightcoveExtractor.can_handle(url), "{url}");
        }
    }

    #[test]
    fn extracts_the_brightcove_references_from_selected_wrapper_fixtures() {
        let craftsy = r#"<video-js data-account="123456789" data-video-id="987654321"></video-js>"#;
        assert_eq!(
            parse_craftsy_player(craftsy).unwrap(),
            PlayerRef {
                account_id: "123456789".to_owned(),
                player_id: "default".to_owned(),
                embed_id: "default".to_owned(),
                video_id: "987654321".to_owned(),
            }
        );

        let nine = r#"var __INITIAL_STATE__ = {"videoIndex":{"currentVideo":{"brightcoveId":"987654321"},"config":{"account":"123456789"}}};"#;
        assert_eq!(parse_ninenews_player(nine).unwrap().account_id, "123456789");

        let amc = r#"window.initialData = JSON.parse(String.raw`{"initialData":{"properties":{"videoId":"987654321"},"config":{"brightcove":{"accountId":"123456789","playerId":"player-abc"}}}}`);"#;
        let player = parse_amc_player(amc).unwrap();
        assert_eq!(player.video_id, "987654321");
        assert_eq!(player.player_id, "player-abc");
    }

    #[test]
    fn parses_playback_formats_and_rejects_drm_or_manifest_expansion() {
        let response = serde_json::json!({
            "name": "Public media",
            "duration": 90000,
            "poster": "https://media.example/poster.jpg",
            "sources": [
                {"src":"https://media.example/720.mp4","type":"video/mp4","container":"MP4","width":1280,"height":720,"avg_bitrate":1800000,"size":1000,"codec":"h264"},
                {"src":"https://media.example/master.m3u8","type":"application/x-mpegURL"},
                {"src":"https://media.example/drm.mp4","container":"WVM"},
                {"src":"https://media.example/encrypted.mp4","key_systems":{"widevine":{}}}
            ]
        });
        let parsed = parse_playback_response(&response).unwrap();
        assert_eq!(parsed.title, "Public media");
        assert_eq!(parsed.duration, Some(90.0));
        assert_eq!(parsed.formats.len(), 2);
        assert_eq!(parsed.formats[0].bitrate, Some(1_800_000));
        assert_eq!(parsed.formats[0].protocol, MediaProtocol::Http);
        assert_eq!(parsed.formats[1].protocol, MediaProtocol::Hls);
    }

    #[test]
    fn json_and_policy_key_parsers_reject_missing_or_malformed_data() {
        assert!(
            json_after_marker("var __INITIAL_STATE__ = {broken};", "__INITIAL_STATE__").is_err()
        );
        assert_eq!(
            extract_policy_key(r#"catalog({policyKey:"public-key"});"#).as_deref(),
            Some("public-key")
        );
        assert_eq!(extract_policy_key("no policy key"), None);
    }

    #[tokio::test]
    async fn declines_user_cookies_for_the_existing_fallback() {
        let options = ExtractOptions {
            cookies: Some("unlogged-cookie-value".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            BrightcoveExtractor
                .extract("https://www.craftsy.com/class/test-course/", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

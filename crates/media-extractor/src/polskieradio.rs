//! Limited public Polskie Radio audio extraction.
//!
//! Ported from the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense),
//! `yt_dlp/extractor/polskieradio.py`. This supports single public audio
//! attachments and podcast tracks; playlists and station streams are left to
//! the existing fallback.

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

pub struct PolskieRadioExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    ModernArticle(String),
    LegacyArticle(String),
    PodcastTrack(String),
}

#[derive(Debug)]
struct AudioEntry {
    id: String,
    url: String,
    title: Option<String>,
    duration: Option<f64>,
    filesize: Option<u64>,
    thumbnail: Option<String>,
}

#[async_trait]
impl MediaExtractor for PolskieRadioExtractor {
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

impl PolskieRadioExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let (entry, title, thumbnail) = match route {
            Route::ModernArticle(_id) => {
                let page = get_html(source, timeout).await?;
                let state = parse_nextjs_state(&page)?;
                let article = state
                    .pointer("/props/pageProps/data/articleData")
                    .or_else(|| state.pointer("/props/pageProps/post/data"))
                    .ok_or(ExtractionError::UnsupportedUrl)?;
                let article_title = nonempty_string(article.get("title"));
                let audio = parse_modern_attachments(article)?;
                let (entry, thumb) = if audio.is_empty() {
                    let content = article
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or(ExtractionError::UnsupportedUrl)?;
                    let (entries, thumb) = parse_legacy_players(content)?;
                    if entries.len() != 1 {
                        return Err(ExtractionError::UnsupportedUrl);
                    }
                    (entries.into_iter().next().unwrap(), thumb)
                } else {
                    if audio.len() != 1 {
                        return Err(ExtractionError::UnsupportedUrl);
                    }
                    (audio.into_iter().next().unwrap(), None)
                };
                (entry, article_title, thumb)
            }
            Route::LegacyArticle(id) => {
                let page = get_html(source, timeout).await?;
                let (entries, thumbnail) = parse_legacy_players(&page)?;
                if entries.len() != 1 {
                    return Err(ExtractionError::UnsupportedUrl);
                }
                (entries.into_iter().next().unwrap(), Some(id), thumbnail)
            }
            Route::PodcastTrack(id) => {
                let api_url = Url::parse("https://apipodcasts.polskieradio.pl/api/audio")
                    .map_err(|_| ExtractionError::InvalidOutput)?;
                let body = serde_json::to_vec(&serde_json::json!({"guids": [id]}))
                    .map_err(|_| ExtractionError::InvalidOutput)?;
                let response = provider_http::post(
                    &api_url,
                    "application/json",
                    body,
                    timeout,
                )
                .await?;
                if !response.status.is_success()
                    || !response
                        .content_type
                        .as_deref()
                        .is_some_and(|value| value.to_ascii_lowercase().contains("json"))
                {
                    return Err(ExtractionError::UnsupportedUrl);
                }
                let json: Value = serde_json::from_slice(&response.body)
                    .map_err(|_| ExtractionError::InvalidOutput)?;
                let entries = parse_podcast_response(&json)?;
                if entries.len() != 1 {
                    return Err(ExtractionError::UnsupportedUrl);
                }
                (entries.into_iter().next().unwrap(), None, None)
            }
        };
        build_info(source, entry, title, thumbnail).await
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
    if host == "podcasty.polskieradio.pl"
        && segments.len() == 2
        && segments[0] == "track"
        && valid_guid(segments[1])
    {
        return Some(Route::PodcastTrack(segments[1].to_ascii_lowercase()));
    }
    if matches!(host, "polskieradio.pl" | "polskieradio24.pl")
        && segments.len() >= 4
        && segments[..2].iter().all(|segment| {
            !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit())
        })
        && segments[2].eq_ignore_ascii_case("artykul")
        && segments[3]
            .split(',')
            .next()
            .is_some_and(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Some(Route::LegacyArticle(
            segments[3].split(',').next()?.to_owned(),
        ));
    }
    let recognized_domain = host == "polskieradio.pl"
        || host.ends_with(".polskieradio.pl")
        || host == "radiokierowcow.pl"
        || host.ends_with(".radiokierowcow.pl");
    if recognized_domain
        && segments.len() >= 2
        && segments[0].eq_ignore_ascii_case("artykul")
        && segments[1]
            .split(',')
            .next()
            .is_some_and(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Some(Route::ModernArticle(
            segments[1].split(',').next()?.to_owned(),
        ));
    }
    None
}

fn valid_guid(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(parts)
            .all(|(length, part)| {
                part.len() == *length && part.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
}

fn parse_nextjs_state(page: &str) -> Result<Value, ExtractionError> {
    let document = Html::parse_document(page);
    let selector = Selector::parse("script#__NEXT_DATA__")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let text = document
        .select(&selector)
        .next()
        .map(|script| script.text().collect::<String>())
        .ok_or(ExtractionError::UnsupportedUrl)?;
    serde_json::from_str(&text).map_err(|_| ExtractionError::InvalidOutput)
}

fn parse_modern_attachments(article: &Value) -> Result<Vec<AudioEntry>, ExtractionError> {
    let attachments = article
        .get("attachments")
        .and_then(Value::as_array)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    Ok(attachments
        .iter()
        .filter(|attachment| attachment.get("fileType").and_then(Value::as_str) == Some("Audio"))
        .filter_map(|attachment| {
            let url = attachment.get("file")?.as_str()?;
            let id = nonempty_string(attachment.get("id"))
                .or_else(|| uuid_from_url(url))
                .or_else(|| Url::parse(url).ok()?.path_segments()?.next_back().map(str::to_owned))
                .unwrap_or_else(|| format!("audio-{}", url.len()));
            Some(AudioEntry {
                id,
                url: url.to_owned(),
                title: nonempty_string(attachment.get("description")),
                duration: attachment.get("length").and_then(parse_f64),
                filesize: attachment.get("fileSize").and_then(Value::as_u64),
                thumbnail: None,
            })
        })
        .collect())
}

fn parse_legacy_players(content: &str) -> Result<(Vec<AudioEntry>, Option<String>), ExtractionError> {
    let document = Html::parse_document(content);
    let selector = Selector::parse("[data-media]").map_err(|_| ExtractionError::InvalidOutput)?;
    let mut entries = Vec::new();
    for element in document.select(&selector) {
        let Some(raw) = element.value().attr("data-media") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(url) = value.get("file").and_then(Value::as_str) else {
            continue;
        };
        let Some(title) = nonempty_string(value.get("desc")) else {
            continue;
        };
        entries.push(AudioEntry {
            id: nonempty_string(value.get("id")).unwrap_or_else(|| format!("legacy-{}", entries.len() + 1)),
            url: url.to_owned(),
            title: Some(title),
            duration: value.get("length").and_then(parse_f64),
            filesize: None,
            thumbnail: None,
        });
    }
    Ok((entries, None))
}

fn parse_podcast_response(response: &Value) -> Result<Vec<AudioEntry>, ExtractionError> {
    let items = response.as_array().ok_or(ExtractionError::UnsupportedUrl)?;
    Ok(items
        .iter()
        .filter_map(|item| {
            Some(AudioEntry {
                id: nonempty_string(item.get("guid"))?,
                url: nonempty_string(item.get("url"))?,
                title: nonempty_string(item.get("title")),
                duration: item.get("length").and_then(parse_f64),
                filesize: item.get("fileSize").and_then(Value::as_u64),
                thumbnail: nonempty_string(item.get("image")),
            })
        })
        .collect())
}

async fn build_info(
    source: &str,
    entry: AudioEntry,
    page_title: Option<String>,
    page_thumbnail: Option<String>,
) -> Result<MediaInfo, ExtractionError> {
    let url = validate_public_url(&entry.url).await?;
    let thumbnail_url = match entry.thumbnail.as_deref().or(page_thumbnail.as_deref()) {
        Some(value) => validate_public_url(value)
            .await
            .ok()
            .map(|url| url.to_string()),
        None => None,
    };
    let title = entry
        .title
        .or(page_title)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let ext = audio_extension(&url);
    Ok(MediaInfo {
        title,
        duration: entry.duration,
        thumbnail_url,
        webpage_url: source.to_owned(),
        extractor_name: format!("Polskie Radio ({})", entry.id),
        formats: vec![MediaFormat {
            id: entry.id,
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
            filesize: entry.filesize,
            required_headers: Vec::new(),
        }],
    })
}

async fn get_html(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "Polskie Radio URL is malformed",
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

fn uuid_from_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    url.path_segments()?
        .filter_map(|segment| valid_guid(segment).then(|| segment.to_ascii_lowercase()))
        .next()
}

fn audio_extension(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|segment| segment.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()))
        .filter(|ext| matches!(ext.as_str(), "mp3" | "m4a" | "aac" | "ogg" | "opus" | "wav"))
        .unwrap_or_else(|| "mp3".to_owned())
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_f64(value: &Value) -> Option<f64> {
    let value = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_each_registered_polskie_radio_route() {
        for url in [
            "https://jedynka.polskieradio.pl/artykul/1587943",
            "https://www.polskieradio.pl/8/2382/Artykul/2534482,Zagarysci",
            "https://podcasty.polskieradio.pl/track/6eafe403-cb8f-4756-b896-4455c3713c32",
        ] {
            assert!(PolskieRadioExtractor.can_handle(url), "{url}");
        }
        assert!(!PolskieRadioExtractor.can_handle("https://podcasty.polskieradio.pl/podcast/8/"));
        assert!(!PolskieRadioExtractor.can_handle("https://example.com/artykul/123"));
    }

    #[test]
    fn extracts_one_modern_article_audio_attachment() {
        let article = serde_json::json!({
            "title": "Article",
            "attachments": [{
                "fileType": "Audio",
                "file": "https://static.prsa.pl/audio/episode.mp3",
                "description": "Episode",
                "length": 321,
                "fileSize": 1000
            }]
        });
        let audio = parse_modern_attachments(&article).unwrap();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].title.as_deref(), Some("Episode"));
        assert_eq!(audio[0].duration, Some(321.0));
        assert!(parse_modern_attachments(&serde_json::json!({"attachments": []}))
            .unwrap()
            .is_empty());
        assert!(parse_modern_attachments(&serde_json::json!({})).is_err());
    }

    #[test]
    fn parses_legacy_player_and_declines_multi_item_pages() {
        let content = r#"<div data-media='{"id":"42","file":"https://static.prsa.pl/audio/show.mp3","desc":"Show","length":120}'></div>"#;
        let (entries, _) = parse_legacy_players(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "42");
        let many = format!("{content}{content}");
        assert_eq!(parse_legacy_players(&many).unwrap().0.len(), 2);
        assert!(parse_legacy_players("<div data-media='not json'></div>")
            .unwrap()
            .0
            .is_empty());
    }

    #[test]
    fn parses_podcast_api_audio_and_rejects_malformed_or_empty_responses() {
        let response = serde_json::json!([{
            "guid": "6eafe403-cb8f-4756-b896-4455c3713c32",
            "url": "https://static.prsa.pl/audio/track.mp3",
            "title": "Track",
            "length": 120,
            "fileSize": 999
        }]);
        let entries = parse_podcast_response(&response).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filesize, Some(999));
        assert!(parse_podcast_response(&serde_json::json!({})).is_err());
        assert!(parse_podcast_response(&serde_json::json!([])).unwrap().is_empty());
    }

    #[tokio::test]
    async fn refuses_caller_credentials_so_registry_can_fall_back() {
        let options = ExtractOptions {
            headers: vec![("X-Test".to_owned(), "value".to_owned())],
            ..ExtractOptions::default()
        };
        assert!(matches!(
            PolskieRadioExtractor
                .extract("https://jedynka.polskieradio.pl/artykul/1587943", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

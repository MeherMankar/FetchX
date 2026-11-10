//! Bounded native extraction for common HTML media declarations.
//!
//! The supported HTML declarations are adapted from yt-dlp's GenericIE
//! (`yt_dlp/extractor/generic.py`) at commit
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679`. This is a focused behavior
//! port, not a port of its site-specific/player/manifest extraction code.

use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use async_trait::async_trait;
use reqwest::{
    Client, Response, StatusCode, Url,
    header::{ACCEPT, CONTENT_TYPE, LOCATION},
};
use scraper::{Html, Selector};

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    models::{MediaFormat, MediaInfo, MediaProtocol},
    validation::is_safe_media_url,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_FORMATS: usize = 64;

/// Native fallback for pages that declare a directly accessible media URL.
///
/// The extractor performs no Python or subprocess invocation. Pages requiring
/// cookies or caller-provided headers are explicitly unsupported here so the
/// registered legacy extractor can handle them.
pub struct GenericExtractor;

#[async_trait]
impl MediaExtractor for GenericExtractor {
    fn can_handle(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    }

    async fn extract(
        &self,
        url: &str,
        opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        if opts.cookies.is_some() || !opts.headers.is_empty() {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let timeout = effective_timeout(opts.timeout);
        if timeout.is_zero() {
            return Err(ExtractionError::Timeout);
        }

        match tokio::time::timeout(timeout, self.extract_inner(url, timeout)).await {
            Ok(result) => result,
            Err(_) => Err(ExtractionError::Timeout),
        }
    }
}

impl GenericExtractor {
    async fn extract_inner(
        &self,
        url: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let page = fetch_html_page(url, timeout).await?;
        let parsed = parse_html_page(&page.url, &page.body)?;
        if parsed.candidates.is_empty() {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let mut formats = Vec::new();
        let mut unsafe_urls = 0usize;
        for candidate in parsed.candidates.into_iter().take(MAX_FORMATS) {
            let media_url = match validate_public_url(&candidate.url).await {
                Ok(url) => url,
                Err(ExtractionError::UnsafeUrl { .. }) => {
                    unsafe_urls += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let ext = extension_for(&media_url, candidate.mime_type.as_deref());
            let protocol = protocol_for(&media_url, candidate.mime_type.as_deref());
            formats.push(MediaFormat {
                id: format!("generic-{}", formats.len() + 1),
                ext,
                protocol,
                url: media_url.to_string(),
                label: format!(
                    "Direct media - {}",
                    extension_for(&media_url, candidate.mime_type.as_deref())
                ),
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

        if formats.is_empty() {
            if unsafe_urls > 0 {
                return Err(ExtractionError::UnsafeUrl {
                    reason: "media URL resolves to a non-public address",
                });
            }
            return Err(ExtractionError::UnsupportedUrl);
        }

        let thumbnail_url = match parsed.thumbnail_url {
            Some(url) => match validate_public_url(&url).await {
                Ok(url) => Some(url.to_string()),
                Err(ExtractionError::UnsafeUrl { .. }) => None,
                Err(error) => return Err(error),
            },
            None => None,
        };
        let mut info = MediaInfo {
            title: parsed.title.unwrap_or_else(|| {
                page.url
                    .host_str()
                    .unwrap_or_default()
                    .trim_start_matches("www.")
                    .to_owned()
            }),
            duration: parsed.duration,
            thumbnail_url,
            webpage_url: page.url.to_string(),
            extractor_name: "generic".to_owned(),
            formats,
        };
        info.sanitise_format_urls();
        if info.formats.is_empty() {
            return Err(ExtractionError::NoFormats);
        }
        Ok(info)
    }
}

struct FetchedPage {
    url: Url,
    body: String,
}

async fn fetch_html_page(source: &str, timeout: Duration) -> Result<FetchedPage, ExtractionError> {
    let mut url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "URL is malformed",
    })?;
    validate_url_structure(&url)?;
    let mut visited = HashSet::new();

    for redirect_count in 0..=MAX_REDIRECTS {
        if !visited.insert(url.as_str().to_owned()) {
            return Err(ExtractionError::UnsupportedUrl);
        }
        let client = client_for_url(&url, timeout).await?;
        let response = client
            .get(url.clone())
            .header(ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .await
            .map_err(|_| ExtractionError::BackendError {
                message: "page request failed".to_owned(),
            })?;

        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(ExtractionError::UnsupportedUrl);
            }
            let Some(location) = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
            else {
                return Err(ExtractionError::UnsupportedUrl);
            };
            let next_url =
                response
                    .url()
                    .join(location)
                    .map_err(|_| ExtractionError::UnsafeUrl {
                        reason: "redirect URL is malformed",
                    })?;
            validate_redirect(&url, &next_url)?;
            url = next_url;
            continue;
        }

        if response.status() != StatusCode::OK
            || !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(is_html_content_type)
            || response
                .content_length()
                .is_some_and(|length| length > MAX_PAGE_BYTES as u64)
        {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let final_url = response.url().clone();
        let body = read_bounded_body(response).await?;
        return Ok(FetchedPage {
            url: final_url,
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }

    Err(ExtractionError::UnsupportedUrl)
}

async fn read_bounded_body(mut response: Response) -> Result<Vec<u8>, ExtractionError> {
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| ExtractionError::BackendError {
                message: "page response could not be read".to_owned(),
            })?;
        let Some(chunk) = chunk else {
            return Ok(body);
        };
        append_bounded_chunk(&mut body, &chunk)?;
    }
}

async fn client_for_url(url: &Url, timeout: Duration) -> Result<Client, ExtractionError> {
    let host = url.host_str().ok_or(ExtractionError::UnsafeUrl {
        reason: "URL has no host",
    })?;
    let port = url
        .port_or_known_default()
        .ok_or(ExtractionError::UnsafeUrl {
            reason: "URL has no port",
        })?;
    let addresses = resolve_public_addresses(host, port).await?;
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout.min(MAX_CONNECT_TIMEOUT))
        .timeout(timeout)
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| ExtractionError::BackendError {
            message: "page request client could not be created".to_owned(),
        })
}

async fn validate_public_url(source: &str) -> Result<Url, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "media URL is malformed",
    })?;
    validate_url_structure(&url)?;
    let host = url.host_str().ok_or(ExtractionError::UnsafeUrl {
        reason: "media URL has no host",
    })?;
    let port = url
        .port_or_known_default()
        .ok_or(ExtractionError::UnsafeUrl {
            reason: "media URL has no port",
        })?;
    resolve_public_addresses(host, port).await?;
    Ok(url)
}

async fn resolve_public_addresses(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, ExtractionError> {
    if host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host.to_ascii_lowercase().ends_with(".local")
    {
        return Err(ExtractionError::UnsafeUrl {
            reason: "local host is not allowed",
        });
    }

    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| ExtractionError::BackendError {
                message: "host name could not be resolved".to_owned(),
            })?
            .collect::<Vec<_>>()
    };
    if addresses.is_empty() {
        return Err(ExtractionError::BackendError {
            message: "host name could not be resolved".to_owned(),
        });
    }
    validate_public_addresses(&addresses)?;
    Ok(addresses)
}

fn validate_public_addresses(addresses: &[SocketAddr]) -> Result<(), ExtractionError> {
    if addresses.is_empty() {
        return Err(ExtractionError::BackendError {
            message: "host name could not be resolved".to_owned(),
        });
    }
    if addresses
        .iter()
        .any(|address| is_non_public_ip(address.ip()))
    {
        return Err(ExtractionError::UnsafeUrl {
            reason: "host resolves to a non-public address",
        });
    }
    Ok(())
}

fn validate_url_structure(url: &Url) -> Result<(), ExtractionError> {
    is_safe_media_url(url.as_str())?;
    if url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost")
    }) {
        return Err(ExtractionError::UnsafeUrl {
            reason: "local host is not allowed",
        });
    }
    Ok(())
}

fn validate_redirect(previous: &Url, next: &Url) -> Result<(), ExtractionError> {
    validate_url_structure(next)?;
    if previous.scheme() == "https" && next.scheme() == "http" {
        return Err(ExtractionError::UnsafeUrl {
            reason: "HTTPS redirects may not downgrade to HTTP",
        });
    }
    Ok(())
}

fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_non_public_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_non_public_ipv6(ipv6),
    }
}

fn is_non_public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || octets[0] == 0
        || octets[0] == 100 && (64..=127).contains(&octets[1])
        || octets[0] == 192 && (octets[1] == 0 || octets[1] == 88 && octets[2] == 99)
        || octets[0] == 192 && octets[1] == 0 && octets[2] == 2
        || octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)
        || octets[0] == 198 && octets[1] == 51 && octets[2] == 100
        || octets[0] == 203 && octets[1] == 0 && octets[2] == 113
        || octets[0] >= 240
}

fn is_non_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(ipv4) = ip.to_ipv4() {
        return is_non_public_ipv4(ipv4);
    }
    let segments = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unicast_link_local()
        || ip.is_unique_local()
        || segments[0] & 0xffc0 == 0xfec0
        || segments[0] & 0xe000 != 0x2000
        || segments[0] == 0x2001 && segments[1] <= 0x01ff
        || segments[0] == 0x2001 && segments[1] == 0x0db8
        || segments[0] == 0x2002
}

struct ParsedPage {
    title: Option<String>,
    duration: Option<f64>,
    thumbnail_url: Option<String>,
    candidates: Vec<MediaCandidate>,
}

struct MediaCandidate {
    url: String,
    mime_type: Option<String>,
}

fn parse_html_page(base_url: &Url, body: &str) -> Result<ParsedPage, ExtractionError> {
    let document = Html::parse_document(body);
    let meta_selector = Selector::parse("meta[property], meta[name]")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let mut metadata = Vec::new();
    for element in document.select(&meta_selector) {
        let value = element
            .value()
            .attr("content")
            .or_else(|| element.value().attr("value"))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(value) = value {
            let key = element
                .value()
                .attr("property")
                .or_else(|| element.value().attr("name"))
                .unwrap_or_default();
            metadata.push((key.to_ascii_lowercase(), value.to_owned()));
        }
    }

    let title = first_meta(&metadata, &["og:title", "twitter:title"]).or_else(|| {
        Selector::parse("title")
            .ok()
            .and_then(|selector| document.select(&selector).next())
            .map(|element| element.text().collect::<String>().trim().to_owned())
            .filter(|title| !title.is_empty())
    });
    let duration = first_meta(
        &metadata,
        &[
            "video:duration",
            "og:video:duration",
            "twitter:player:duration",
        ],
    )
    .as_deref()
    .and_then(parse_duration_seconds)
    .or_else(|| {
        Selector::parse("video").ok().and_then(|selector| {
            document.select(&selector).find_map(|video| {
                video
                    .value()
                    .attr("duration")
                    .and_then(parse_duration_seconds)
            })
        })
    });
    let thumbnail_url = first_meta(
        &metadata,
        &[
            "og:image",
            "og:image:url",
            "twitter:image",
            "twitter:image:src",
        ],
    )
    .and_then(|value| resolve_candidate_url(base_url, &value))
    .or_else(|| {
        Selector::parse("video[poster]")
            .ok()
            .and_then(|selector| {
                document
                    .select(&selector)
                    .find_map(|video| video.value().attr("poster"))
            })
            .and_then(|poster| resolve_candidate_url(base_url, poster))
    });
    let og_video_type = first_meta(&metadata, &["og:video:type"]);

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for key in ["twitter:player:stream"] {
        for value in metadata
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value)
        {
            push_candidate(&mut candidates, &mut seen, base_url, value, None);
        }
    }
    let og_video_mime = og_video_type.as_deref();
    if og_video_mime.is_none_or(is_video_mime) {
        for key in ["og:video", "og:video:url", "og:video:secure_url"] {
            for value in metadata
                .iter()
                .filter(|(name, _)| name == key)
                .map(|(_, value)| value)
            {
                push_candidate(&mut candidates, &mut seen, base_url, value, og_video_mime);
            }
        }
    }

    let video_selector = Selector::parse("video").map_err(|_| ExtractionError::InvalidOutput)?;
    let source_selector =
        Selector::parse("video source[src]").map_err(|_| ExtractionError::InvalidOutput)?;
    for video in document.select(&video_selector) {
        if let Some(src) = video.value().attr("src") {
            push_candidate(&mut candidates, &mut seen, base_url, src, None);
        }
        for source in video.select(&source_selector) {
            push_candidate(
                &mut candidates,
                &mut seen,
                base_url,
                source.value().attr("src").unwrap_or_default(),
                source.value().attr("type"),
            );
        }
    }

    candidates.retain(is_media_candidate);
    candidates.truncate(MAX_FORMATS);
    Ok(ParsedPage {
        title,
        duration,
        thumbnail_url,
        candidates,
    })
}

fn first_meta(metadata: &[(String, String)], names: &[&str]) -> Option<String> {
    metadata
        .iter()
        .find(|(name, _)| names.contains(&name.as_str()))
        .map(|(_, value)| value.clone())
}

fn push_candidate(
    candidates: &mut Vec<MediaCandidate>,
    seen: &mut HashSet<String>,
    base_url: &Url,
    source: &str,
    mime_type: Option<&str>,
) {
    if !mime_type.is_none_or(is_video_mime) {
        return;
    }
    let Some(url) = resolve_candidate_url(base_url, source) else {
        return;
    };
    if !seen.insert(url.clone()) {
        return;
    }
    candidates.push(MediaCandidate {
        url,
        mime_type: mime_type.map(str::to_owned),
    });
}

fn resolve_candidate_url(base_url: &Url, source: &str) -> Option<String> {
    let url = base_url.join(source.trim()).ok()?;
    is_safe_media_url(url.as_str()).ok()?;
    Some(url.to_string())
}

fn is_media_candidate(candidate: &MediaCandidate) -> bool {
    let url = match Url::parse(&candidate.url) {
        Ok(url) => url,
        Err(_) => return false,
    };
    let Some(ext) = path_extension(&url) else {
        return true;
    };
    matches!(
        ext.as_str(),
        "mp4"
            | "m4v"
            | "mov"
            | "mkv"
            | "webm"
            | "ogv"
            | "ogg"
            | "avi"
            | "flv"
            | "3gp"
            | "ts"
            | "mpeg"
            | "mpg"
            | "wmv"
            | "m3u8"
            | "mpd"
    ) || candidate.mime_type.as_deref().is_some_and(is_video_mime)
}

fn extension_for(url: &Url, mime_type: Option<&str>) -> String {
    path_extension(url)
        .or_else(|| mime_type.and_then(extension_for_mime))
        .unwrap_or_else(|| "mp4".to_owned())
}

fn path_extension(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(Iterator::last)
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .filter(|ext| {
            !ext.is_empty()
                && ext.len() <= 8
                && ext.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase)
}

fn protocol_for(url: &Url, mime_type: Option<&str>) -> MediaProtocol {
    let extension = url
        .path_segments()
        .and_then(Iterator::last)
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime_type = mime_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default();
    if extension == "m3u8"
        || matches!(
            mime_type.to_ascii_lowercase().as_str(),
            "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "audio/mpegurl"
        )
    {
        MediaProtocol::Hls
    } else if extension == "mpd" || mime_type.eq_ignore_ascii_case("application/dash+xml") {
        MediaProtocol::Dash
    } else {
        MediaProtocol::Http
    }
}

fn extension_for_mime(mime_type: &str) -> Option<String> {
    match mime_type
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "video/mp4" => Some("mp4".to_owned()),
        "video/webm" => Some("webm".to_owned()),
        "video/quicktime" => Some("mov".to_owned()),
        "video/x-matroska" => Some("mkv".to_owned()),
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "audio/mpegurl" => {
            Some("m3u8".to_owned())
        }
        "application/dash+xml" => Some("mpd".to_owned()),
        _ => None,
    }
}

fn is_video_mime(mime_type: &str) -> bool {
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    mime_type.starts_with("video/")
        || mime_type.starts_with("audio/")
        || matches!(
            mime_type.as_str(),
            "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "application/dash+xml"
        )
}

fn is_html_content_type(content_type: &str) -> bool {
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

fn parse_duration_seconds(value: &str) -> Option<f64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<f64>() {
        return (seconds.is_finite() && seconds >= 0.0).then_some(seconds);
    }
    let mut rest = value.strip_prefix("PT")?;
    let mut seconds = 0.0;
    while !rest.is_empty() {
        let split = rest.find(|character: char| !character.is_ascii_digit() && character != '.')?;
        let amount = rest[..split].parse::<f64>().ok()?;
        let unit = rest.as_bytes()[split] as char;
        let multiplier = match unit {
            'H' => 3600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => return None,
        };
        seconds += amount * multiplier;
        rest = &rest[split + 1..];
    }
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

fn effective_timeout(requested: Option<Duration>) -> Duration {
    requested.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT)
}

fn append_bounded_chunk(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), ExtractionError> {
    if body.len().saturating_add(chunk.len()) > MAX_PAGE_BYTES {
        return Err(ExtractionError::UnsupportedUrl);
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_url() -> Url {
        Url::parse("https://page.example/path/watch").expect("valid test URL")
    }

    #[test]
    fn parses_common_html_video_declarations_and_resolves_relative_urls() {
        let html = r#"
            <html><head>
              <meta property="og:title" content="Open Graph title">
              <meta property="video:duration" content="PT1M2.5S">
              <meta property="og:image" content="../thumb.jpg">
              <meta name="twitter:player:stream" content="/twitter.mp4">
              <meta property="og:video:type" content="video/mp4">
              <meta property="og:video:url" content="//cdn.example/og.m3u8">
            </head><body>
              <video src="fallback.mp4" poster="/poster.jpg">
                <source src="media.webm" type="video/webm">
                <source src="dash.mpd" type="application/dash+xml">
              </video>
            </body></html>
        "#;
        let parsed = parse_html_page(&base_url(), html).expect("parse document");

        assert_eq!(parsed.title.as_deref(), Some("Open Graph title"));
        assert_eq!(parsed.duration, Some(62.5));
        assert_eq!(
            parsed.thumbnail_url.as_deref(),
            Some("https://page.example/thumb.jpg")
        );
        let urls = parsed
            .candidates
            .iter()
            .map(|candidate| candidate.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            [
                "https://page.example/twitter.mp4",
                "https://cdn.example/og.m3u8",
                "https://page.example/path/fallback.mp4",
                "https://page.example/path/media.webm",
                "https://page.example/path/dash.mpd",
            ]
        );
    }

    #[test]
    fn prefers_og_title_then_twitter_then_document_title() {
        let html = r#"
            <meta property="og:title" content="Open Graph title">
            <meta name="twitter:title" content="Twitter title">
            <title>Document title</title>
            <video src="https://media.example/video.mp4"></video>
        "#;
        let parsed = parse_html_page(&base_url(), html).expect("parse document");
        assert_eq!(parsed.title.as_deref(), Some("Open Graph title"));
    }

    #[test]
    fn uses_html5_poster_when_no_thumbnail_metadata_exists() {
        let parsed = parse_html_page(
            &base_url(),
            r#"<video src="clip.mp4" poster="../poster.jpg"></video>"#,
        )
        .expect("parse document");
        assert_eq!(
            parsed.thumbnail_url.as_deref(),
            Some("https://page.example/poster.jpg")
        );
    }

    #[test]
    fn classifies_http_hls_and_dash_formats() {
        let base = base_url();
        assert_eq!(
            protocol_for(&base.join("clip.mp4").expect("join"), None),
            MediaProtocol::Http
        );
        assert_eq!(
            protocol_for(&base.join("list.m3u8").expect("join"), None),
            MediaProtocol::Hls
        );
        assert_eq!(
            protocol_for(
                &base.join("manifest").expect("join"),
                Some("application/dash+xml")
            ),
            MediaProtocol::Dash
        );
    }

    #[test]
    fn rejects_non_media_and_unsafe_declarations() {
        let html = r#"
            <meta name="twitter:player:stream" content="javascript:alert(1)">
            <meta property="og:video:url" content="http://127.0.0.1/private.mp4">
            <video src="https://page.example/player.js"></video>
            <video src="/good.mp4"></video>
        "#;
        let parsed = parse_html_page(&base_url(), html).expect("parse document");
        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.candidates[0].url, "https://page.example/good.mp4");
    }

    #[test]
    fn rejects_non_video_open_graph_type() {
        let html = r#"
            <meta property="og:video:type" content="application/x-shockwave-flash">
            <meta property="og:video:url" content="/player.swf">
            <video src="/direct.mp4"></video>
        "#;
        let parsed = parse_html_page(&base_url(), html).expect("parse document");
        assert_eq!(parsed.candidates.len(), 1);
        assert!(parsed.candidates[0].url.ends_with("/direct.mp4"));
    }

    #[test]
    fn bounds_supported_page_content_types_and_response_chunks() {
        assert!(is_html_content_type("text/html; charset=utf-8"));
        assert!(is_html_content_type("application/xhtml+xml"));
        assert!(!is_html_content_type("application/octet-stream"));

        let mut body = vec![0; MAX_PAGE_BYTES - 1];
        assert!(append_bounded_chunk(&mut body, &[1]).is_ok());
        assert!(append_bounded_chunk(&mut body, &[2]).is_err());
    }

    #[test]
    fn bounds_timeout_and_rejects_non_public_dns_addresses() {
        assert_eq!(
            effective_timeout(Some(Duration::from_secs(120))),
            MAX_TIMEOUT
        );
        assert_eq!(effective_timeout(None), DEFAULT_TIMEOUT);
        for address in [
            "127.0.0.1".parse().expect("IPv4"),
            "10.1.2.3".parse().expect("IPv4"),
            "169.254.1.1".parse().expect("IPv4"),
            "100.64.0.1".parse().expect("IPv4"),
            "::1".parse().expect("IPv6"),
            "fc00::1".parse().expect("IPv6"),
            "::ffff:192.168.1.2".parse().expect("IPv6"),
        ] {
            assert!(is_non_public_ip(address), "{address} must be blocked");
        }
        for address in [
            "8.8.8.8".parse().expect("IPv4"),
            "2606:4700:4700::1111".parse().expect("IPv6"),
        ] {
            assert!(!is_non_public_ip(address), "{address} must be allowed");
        }
        let mixed_dns_answer = [
            "8.8.8.8:443".parse().expect("public socket address"),
            "127.0.0.1:443".parse().expect("private socket address"),
        ];
        assert!(matches!(
            validate_public_addresses(&mixed_dns_answer),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[test]
    fn rejects_redirects_that_downgrade_or_target_private_addresses() {
        let previous = Url::parse("https://page.example/watch").expect("URL");
        let downgrade = Url::parse("http://cdn.example/video.mp4").expect("URL");
        assert!(matches!(
            validate_redirect(&previous, &downgrade),
            Err(ExtractionError::UnsafeUrl { .. })
        ));

        let private = Url::parse("https://127.0.0.1/video.mp4").expect("URL");
        assert!(matches!(
            validate_redirect(&previous, &private),
            Err(ExtractionError::UnsafeUrl { .. })
        ));
    }

    #[tokio::test]
    async fn declines_pages_with_caller_credentials_or_headers() {
        let extractor = GenericExtractor;
        let with_cookies = extractor
            .extract(
                "https://page.example/watch",
                &ExtractOptions {
                    cookies: Some("session=secret".to_owned()),
                    ..ExtractOptions::default()
                },
            )
            .await;
        let with_headers = extractor
            .extract(
                "https://page.example/watch",
                &ExtractOptions {
                    headers: vec![("X-Api-Key".to_owned(), "secret".to_owned())],
                    ..ExtractOptions::default()
                },
            )
            .await;
        assert!(matches!(with_cookies, Err(ExtractionError::UnsupportedUrl)));
        assert!(matches!(with_headers, Err(ExtractionError::UnsupportedUrl)));
    }

    #[tokio::test]
    async fn returns_timeout_before_starting_a_request_when_budget_is_zero() {
        let error = GenericExtractor
            .extract(
                "https://page.example/watch",
                &ExtractOptions {
                    timeout: Some(Duration::ZERO),
                    ..ExtractOptions::default()
                },
            )
            .await
            .expect_err("zero timeout should fail before networking");
        assert!(matches!(error, ExtractionError::Timeout));
    }
}

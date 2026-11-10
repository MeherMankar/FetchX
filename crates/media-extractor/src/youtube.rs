//! Focused native YouTube video extraction.
//!
//! The URL forms and player-response fields follow the authorized
//! MeherMankar/yt-dlp source at
//! https://github.com/MeherMankar/yt-dlp, revision
//! d58c4588a54271fe74172e2fc2f09a4a44f70679 (Unlicense):
//! `yt_dlp/extractor/youtube/_video.py` and `_base.py`.
//! This adapts one unauthenticated `tv_samsung` player request and direct-format
//! metadata only; it does not port alternate clients or challenge solving.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{
    StatusCode, Url,
    header::{ACCEPT, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT},
};
use serde_json::{Value, json};

use crate::{
    extractor::{ExtractOptions, ExtractionError, MediaExtractor},
    generic::{client_for_url, read_bounded_body, validate_public_url},
    models::{MediaFormat, MediaInfo, MediaProtocol},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FORMATS: usize = 64;
const TV_CLIENT_VERSION: &str = "5.20260707";
const TV_CLIENT_USER_AGENT: &str = "Mozilla/5.0 (SMART-TV; Linux; Tizen 2.4.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/2.4.0 TV Safari/538.1";

pub struct YoutubeExtractor;

#[async_trait]
impl MediaExtractor for YoutubeExtractor {
    fn can_handle(&self, source: &str) -> bool {
        Url::parse(source)
            .is_ok_and(|url| is_youtube_host(&url) && extract_video_id(&url).is_some())
    }

    async fn extract(
        &self,
        source: &str,
        opts: &ExtractOptions,
    ) -> Result<MediaInfo, ExtractionError> {
        if opts.cookies.is_some() || !opts.headers.is_empty() {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
        let video_id = extract_video_id(&url).ok_or(ExtractionError::UnsupportedUrl)?;
        let timeout = opts.timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
        if timeout.is_zero() {
            return Err(ExtractionError::Timeout);
        }

        let webpage_url = format!("https://www.youtube.com/watch?v={video_id}");
        match tokio::time::timeout(
            timeout,
            self.extract_inner(&webpage_url, &video_id, timeout),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(ExtractionError::Timeout),
        }
    }
}

impl YoutubeExtractor {
    async fn extract_inner(
        &self,
        webpage_url: &str,
        video_id: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let player_response = fetch_player_response(video_id, timeout).await?;
        let parsed = parse_player_response(video_id, &player_response)?;

        let mut formats = Vec::new();
        let mut rejected_unsafe_urls = 0;
        for candidate in parsed.formats.into_iter().take(MAX_FORMATS) {
            match validate_public_url(&candidate.url).await {
                Ok(url) => formats.push(candidate.into_format(url)),
                Err(ExtractionError::UnsafeUrl { .. }) => {
                    rejected_unsafe_urls += 1;
                }
                Err(error) => return Err(error),
            }
        }

        if formats.is_empty() {
            if rejected_unsafe_urls > 0 {
                return Err(ExtractionError::UnsafeUrl {
                    reason: "YouTube media URL resolves to a non-public address",
                });
            }
            return Err(ExtractionError::UnsupportedUrl);
        }

        let thumbnail_url = match parsed.thumbnail_url {
            Some(url) => match validate_public_url(&url).await {
                Ok(url) if url.scheme() == "https" => Some(url.to_string()),
                Ok(_) | Err(ExtractionError::UnsafeUrl { .. }) => None,
                Err(error) => return Err(error),
            },
            None => None,
        };

        let mut info = MediaInfo {
            title: parsed.title,
            duration: parsed.duration,
            thumbnail_url,
            webpage_url: webpage_url.to_owned(),
            extractor_name: "youtube".to_owned(),
            formats,
        };
        info.sanitise_format_urls();
        if info.formats.is_empty() {
            return Err(ExtractionError::NoFormats);
        }
        Ok(info)
    }
}

async fn fetch_player_response(
    video_id: &str,
    timeout: Duration,
) -> Result<Value, ExtractionError> {
    let mut endpoint = Url::parse("https://www.youtube.com/youtubei/v1/player")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    endpoint
        .query_pairs_mut()
        .append_pair("prettyPrint", "false");
    let client = client_for_url(&endpoint, timeout).await?;
    let request_body = player_request_body(video_id);
    let body = serde_json::to_vec(&request_body).map_err(|_| ExtractionError::InvalidOutput)?;
    let response = client
        .post(endpoint)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .header(ORIGIN, "https://www.youtube.com")
        .header(REFERER, "https://www.youtube.com/")
        .header(USER_AGENT, TV_CLIENT_USER_AGENT)
        .header("X-YouTube-Client-Name", "7")
        .header("X-YouTube-Client-Version", TV_CLIENT_VERSION)
        .body(body)
        .send()
        .await
        .map_err(|_| ExtractionError::BackendError {
            message: "YouTube player request failed".to_owned(),
        })?;
    if response.status() != StatusCode::OK {
        return Err(ExtractionError::UnsupportedUrl);
    }

    let body = read_bounded_body(response).await?;
    serde_json::from_slice(&body).map_err(|_| ExtractionError::UnsupportedUrl)
}

fn player_request_body(video_id: &str) -> Value {
    json!({
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "context": {
            "client": {
                "clientName": "TVHTML5",
                "clientVersion": TV_CLIENT_VERSION,
                "deviceMake": "Samsung",
                "deviceModel": "UKS9800",
                "userAgent": TV_CLIENT_USER_AGENT,
                "osName": "Tizen",
                "osVersion": "2.4.0",
                "hl": "en",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0
            }
        },
        "playbackContext": {
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS"
            }
        }
    })
}

pub(crate) fn is_youtube_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    matches!(
        host.as_str(),
        "youtu.be"
            | "www.youtu.be"
            | "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtube-nocookie.com"
            | "www.youtube-nocookie.com"
    ) || host.ends_with(".youtube.com")
        || host.ends_with(".youtube-nocookie.com")
}

fn extract_video_id(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https")
        || !is_youtube_host(url)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }

    let host = url.host_str()?.to_ascii_lowercase();
    let path_segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let candidate = if host == "youtu.be" || host == "www.youtu.be" {
        path_segments.first().map(|segment| (*segment).to_owned())
    } else if path_segments
        .first()
        .is_some_and(|segment| *segment == "watch")
    {
        url.query_pairs()
            .find_map(|(key, value)| (key == "v").then_some(value))
            .map(|value| value.into_owned())
    } else if path_segments.len() >= 2
        && matches!(path_segments[0], "embed" | "shorts" | "live" | "v" | "e")
    {
        path_segments.get(1).map(|segment| (*segment).to_owned())
    } else {
        None
    }?;

    is_valid_video_id(&candidate).then_some(candidate)
}

fn is_valid_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

struct ParsedPlayerResponse {
    title: String,
    duration: Option<f64>,
    thumbnail_url: Option<String>,
    formats: Vec<DirectFormat>,
}

struct DirectFormat {
    id: String,
    url: String,
    ext: String,
    quality: Option<String>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f64>,
    bitrate: Option<u64>,
    filesize: Option<u64>,
}

impl DirectFormat {
    fn into_format(self, url: Url) -> MediaFormat {
        let label = format!(
            "{} - {}",
            self.quality.unwrap_or_else(|| format!("itag {}", self.id)),
            self.ext
        );
        MediaFormat {
            id: self.id,
            ext: self.ext,
            protocol: MediaProtocol::Http,
            url: url.to_string(),
            label,
            video_codec: self.video_codec,
            audio_codec: self.audio_codec,
            width: self.width,
            height: self.height,
            fps: self.fps,
            bitrate: self.bitrate,
            filesize: self.filesize,
            required_headers: Vec::new(),
        }
    }
}

fn parse_player_response(
    requested_video_id: &str,
    response: &Value,
) -> Result<ParsedPlayerResponse, ExtractionError> {
    if response
        .get("playabilityStatus")
        .and_then(|status| status.get("status"))
        .and_then(Value::as_str)
        .is_some_and(|status| status != "OK")
    {
        return Err(ExtractionError::UnsupportedUrl);
    }

    let details = response
        .get("videoDetails")
        .and_then(Value::as_object)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    if details
        .get("videoId")
        .and_then(Value::as_str)
        .is_none_or(|video_id| video_id != requested_video_id)
    {
        return Err(ExtractionError::UnsupportedUrl);
    }

    let microformat = response
        .get("microformat")
        .and_then(|value| value.get("playerMicroformatRenderer"));
    let title = details
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| {
            microformat
                .and_then(|value| value.get("title"))
                .and_then(parse_text_object)
        })
        .filter(|title| !title.trim().is_empty())
        .ok_or(ExtractionError::UnsupportedUrl)?
        .to_owned();

    let duration = details
        .get("lengthSeconds")
        .and_then(parse_number)
        .or_else(|| {
            microformat
                .and_then(|value| value.get("lengthSeconds"))
                .and_then(parse_number)
        });
    let thumbnail_url = details
        .get("thumbnail")
        .and_then(|value| value.get("thumbnails"))
        .and_then(Value::as_array)
        .and_then(|thumbnails| {
            thumbnails
                .iter()
                .filter_map(|thumbnail| {
                    let url = thumbnail.get("url")?.as_str()?;
                    let width = thumbnail.get("width").and_then(Value::as_u64).unwrap_or(0);
                    let height = thumbnail.get("height").and_then(Value::as_u64).unwrap_or(0);
                    Some((url, width.saturating_mul(height)))
                })
                .max_by_key(|(_, resolution)| *resolution)
                .map(|(url, _)| url.to_owned())
        });

    let mut formats = Vec::new();
    if let Some(streaming_data) = response.get("streamingData").and_then(Value::as_object)
        && !streaming_data
            .get("licenseInfos")
            .and_then(Value::as_array)
            .is_some_and(|licenses| !licenses.is_empty())
    {
        for key in ["formats", "adaptiveFormats"] {
            if let Some(streams) = streaming_data.get(key).and_then(Value::as_array) {
                for stream in streams {
                    if let Some(format) = parse_direct_format(stream) {
                        formats.push(format);
                        if formats.len() == MAX_FORMATS {
                            break;
                        }
                    }
                }
            }
            if formats.len() == MAX_FORMATS {
                break;
            }
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }

    Ok(ParsedPlayerResponse {
        title,
        duration,
        thumbnail_url,
        formats,
    })
}

fn parse_direct_format(stream: &Value) -> Option<DirectFormat> {
    if stream.get("signatureCipher").is_some()
        || stream.get("cipher").is_some()
        || stream
            .get("drmFamilies")
            .and_then(Value::as_array)
            .is_some_and(|families| !families.is_empty())
    {
        return None;
    }

    let url = stream.get("url")?.as_str()?;
    let parsed_url = Url::parse(url).ok()?;
    if parsed_url.scheme() != "https"
        || !is_googlevideo_host(parsed_url.host_str()?)
        || !parsed_url.username().is_empty()
        || parsed_url.password().is_some()
        || parsed_url.port().is_some_and(|port| port != 443)
        || parsed_url.query_pairs().any(|(key, _)| key == "n")
    {
        return None;
    }

    let id = stream.get("itag").and_then(parse_u64)?.to_string();
    let mime_type = stream.get("mimeType")?.as_str()?;
    let (media_type, subtype, codecs) = parse_mime_type(mime_type)?;
    if media_type != "video" && media_type != "audio" {
        return None;
    }
    let ext = extension_for_mime(media_type, subtype)?;
    let (video_codec, audio_codec) = parse_codecs(media_type, codecs);
    let quality = stream
        .get("qualityLabel")
        .and_then(Value::as_str)
        .or_else(|| stream.get("quality").and_then(Value::as_str))
        .or_else(|| {
            stream
                .get("audioQuality")
                .and_then(Value::as_str)
                .map(|quality| quality.strip_prefix("AUDIO_QUALITY_").unwrap_or(quality))
        })
        .map(str::to_owned);

    Some(DirectFormat {
        id,
        url: url.to_owned(),
        ext: ext.to_owned(),
        quality,
        video_codec,
        audio_codec,
        width: stream
            .get("width")
            .and_then(parse_u64)
            .and_then(|value| u32::try_from(value).ok()),
        height: stream
            .get("height")
            .and_then(parse_u64)
            .and_then(|value| u32::try_from(value).ok()),
        fps: stream.get("fps").and_then(parse_number),
        bitrate: stream.get("bitrate").and_then(parse_u64),
        filesize: stream.get("contentLength").and_then(parse_u64),
    })
}

fn parse_mime_type(mime_type: &str) -> Option<(&str, &str, Option<&str>)> {
    let (media_type, parameters) = mime_type.split_once(';').unwrap_or((mime_type, ""));
    let (kind, subtype) = media_type.trim().split_once('/')?;
    let codecs = parameters
        .split_once("codecs=")
        .map(|(_, value)| value.trim().trim_matches('"'));
    Some((kind, subtype, codecs))
}

fn extension_for_mime(media_type: &str, subtype: &str) -> Option<&'static str> {
    match (media_type, subtype) {
        ("video", "mp4") => Some("mp4"),
        ("video", "webm") => Some("webm"),
        ("video", "3gpp") => Some("3gp"),
        ("audio", "mp4") => Some("m4a"),
        ("audio", "webm") => Some("webm"),
        ("audio", "ogg") => Some("ogg"),
        ("audio", "mpeg") => Some("mp3"),
        _ => None,
    }
}

fn parse_codecs(media_type: &str, codecs: Option<&str>) -> (Option<String>, Option<String>) {
    let codecs = codecs
        .into_iter()
        .flat_map(|codecs| codecs.split(','))
        .map(str::trim)
        .filter(|codec| !codec.is_empty())
        .collect::<Vec<_>>();
    if media_type == "audio" {
        return (None, codecs.first().map(|codec| (*codec).to_owned()));
    }

    let video_codec = codecs
        .iter()
        .find(|codec| is_video_codec(codec))
        .map(|codec| (*codec).to_owned());
    let audio_codec = codecs
        .iter()
        .find(|codec| is_audio_codec(codec))
        .map(|codec| (*codec).to_owned());
    (video_codec, audio_codec)
}

fn is_video_codec(codec: &str) -> bool {
    ["avc", "vp", "hev", "hvc", "av01", "theora", "dv"]
        .iter()
        .any(|prefix| codec.starts_with(prefix))
}

fn is_audio_codec(codec: &str) -> bool {
    ["mp4a", "opus", "vorbis", "ac-3", "ec-3", "flac", "alac"]
        .iter()
        .any(|prefix| codec.starts_with(prefix))
}

fn is_googlevideo_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "googlevideo.com" || host.ends_with(".googlevideo.com")
}

fn parse_text_object(value: &Value) -> Option<&str> {
    value.get("simpleText").and_then(Value::as_str).or_else(|| {
        value
            .get("runs")
            .and_then(Value::as_array)
            .and_then(|runs| runs.first())
            .and_then(|run| run.get("text"))
            .and_then(Value::as_str)
    })
}

fn parse_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
        .filter(|number| number.is_finite() && *number >= 0.0)
}

fn parse_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaExtractor;
    use serde_json::json;

    const VIDEO_ID: &str = "dQw4w9WgXcQ";

    #[test]
    fn recognizes_supported_youtube_video_url_forms() {
        for source in [
            format!("https://www.youtube.com/watch?v={VIDEO_ID}"),
            format!("https://youtu.be/{VIDEO_ID}?t=10"),
            format!("https://m.youtube.com/shorts/{VIDEO_ID}"),
            format!("https://www.youtube.com/embed/{VIDEO_ID}"),
            format!("https://www.youtube.com/live/{VIDEO_ID}"),
            format!("https://youtube-nocookie.com/embed/{VIDEO_ID}"),
        ] {
            let url = Url::parse(&source).expect("valid URL");
            assert_eq!(
                extract_video_id(&url).as_deref(),
                Some(VIDEO_ID),
                "{source}"
            );
            assert!(YoutubeExtractor.can_handle(&source), "{source}");
        }
    }

    #[test]
    fn rejects_non_video_or_lookalike_youtube_urls() {
        for source in [
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/playlist?list=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/shorts/not_a_video_id",
            "javascript://youtube.com/watch?v=dQw4w9WgXcQ",
        ] {
            assert!(!YoutubeExtractor.can_handle(source), "{source}");
        }
        assert!(is_youtube_host(
            &Url::parse("https://www.youtube.com/playlist").expect("valid URL")
        ));
    }

    #[test]
    fn builds_public_tv_player_request_without_tokens_or_credentials() {
        let request = player_request_body(VIDEO_ID);
        assert_eq!(request["videoId"].as_str(), Some(VIDEO_ID));
        assert_eq!(
            request["context"]["client"]["clientName"].as_str(),
            Some("TVHTML5")
        );
        assert_eq!(
            request["context"]["client"]["clientVersion"].as_str(),
            Some(TV_CLIENT_VERSION)
        );
        assert_eq!(
            request["context"]["client"]["userAgent"].as_str(),
            Some(TV_CLIENT_USER_AGENT)
        );
        assert!(request.get("serviceIntegrityDimensions").is_none());
    }

    #[test]
    fn maps_only_direct_usable_formats_and_player_metadata() {
        let response = json!({
            "videoDetails": {
                "videoId": VIDEO_ID,
                "title": "Sample title",
                "lengthSeconds": "123",
                "thumbnail": {
                    "thumbnails": [
                        {"url":"https://i.ytimg.com/vi/sample/default.jpg","width":120,"height":90},
                        {"url":"https://i.ytimg.com/vi/sample/hq.jpg","width":480,"height":360}
                    ]
                }
            },
            "streamingData": {
                "formats": [{
                    "itag": 18,
                    "url": "https://rr1.googlevideo.com/videoplayback?sig=plain",
                    "mimeType": "video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"",
                    "qualityLabel": "360p",
                    "width": 640,
                    "height": 360,
                    "fps": 30,
                    "bitrate": 500000,
                    "contentLength": "12345"
                }],
                "adaptiveFormats": [
                    {
                        "itag": 251,
                        "url": "https://rr2.googlevideo.com/videoplayback?sig=plain",
                        "mimeType": "audio/webm; codecs=\"opus\"",
                        "audioQuality": "AUDIO_QUALITY_MEDIUM",
                        "bitrate": 128000
                    },
                    {
                        "itag": 137,
                        "signatureCipher": "s=encrypted",
                        "mimeType": "video/mp4; codecs=\"avc1.640028\""
                    },
                    {
                        "itag": 248,
                        "url": "https://rr3.googlevideo.com/videoplayback?n=challenge",
                        "mimeType": "video/webm; codecs=\"vp9\""
                    },
                    {
                        "itag": 999,
                        "url": "https://rr4.googlevideo.com/videoplayback",
                        "mimeType": "video/mp4; codecs=\"avc1.640028\"",
                        "drmFamilies": ["widevine"]
                    }
                ]
            }
        });

        let parsed = parse_player_response(VIDEO_ID, &response).expect("supported player data");
        assert_eq!(parsed.title, "Sample title");
        assert_eq!(parsed.duration, Some(123.0));
        assert_eq!(
            parsed.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/sample/hq.jpg")
        );
        assert_eq!(parsed.formats.len(), 2);

        let video = &parsed.formats[0];
        assert_eq!(video.id, "18");
        assert_eq!(video.ext, "mp4");
        assert_eq!(video.quality.as_deref(), Some("360p"));
        assert_eq!(video.video_codec.as_deref(), Some("avc1.42001E"));
        assert_eq!(video.audio_codec.as_deref(), Some("mp4a.40.2"));
        assert_eq!(video.width, Some(640));
        assert_eq!(video.height, Some(360));
        assert_eq!(video.fps, Some(30.0));
        assert_eq!(video.bitrate, Some(500000));
        assert_eq!(video.filesize, Some(12345));

        let audio = &parsed.formats[1];
        assert_eq!(audio.id, "251");
        assert_eq!(audio.ext, "webm");
        assert_eq!(audio.audio_codec.as_deref(), Some("opus"));
        assert_eq!(audio.quality.as_deref(), Some("MEDIUM"));
    }

    #[test]
    fn rejects_response_for_another_video_and_unsupported_streams() {
        let mismatched = json!({
            "videoDetails":{"videoId":"aaaaaaaaaaa","title":"Wrong video"},
            "streamingData":{"formats":[]}
        });
        assert!(matches!(
            parse_player_response(VIDEO_ID, &mismatched),
            Err(ExtractionError::UnsupportedUrl)
        ));
        let restricted = json!({
            "playabilityStatus":{"status":"LOGIN_REQUIRED"},
            "videoDetails":{"videoId":VIDEO_ID,"title":"Restricted"},
            "streamingData":{"formats":[]}
        });
        assert!(matches!(
            parse_player_response(VIDEO_ID, &restricted),
            Err(ExtractionError::UnsupportedUrl)
        ));
        let licensed = json!({
            "videoDetails":{"videoId":VIDEO_ID,"title":"Licensed"},
            "streamingData":{
                "licenseInfos":[{"drmFamily":"widevine"}],
                "formats":[{
                    "itag":18,
                    "url":"https://rr.googlevideo.com/videoplayback",
                    "mimeType":"video/mp4; codecs=\"avc1, mp4a\""
                }]
            }
        });
        assert!(matches!(
            parse_player_response(VIDEO_ID, &licensed),
            Err(ExtractionError::UnsupportedUrl)
        ));

        for stream in [
            json!({
                "itag":1,
                "url":"https://rr.googlevideo.com/videoplayback",
                "mimeType":"video/mp4; codecs=\"avc1\"",
                "signatureCipher":"s=encrypted"
            }),
            json!({
                "itag":1,
                "url":"https://rr.googlevideo.com/videoplayback?n=challenge",
                "mimeType":"video/mp4; codecs=\"avc1\""
            }),
            json!({
                "itag":1,
                "url":"http://rr.googlevideo.com/videoplayback",
                "mimeType":"video/mp4; codecs=\"avc1\""
            }),
            json!({
                "itag":1,
                "url":"https://googlevideo.com.evil.example/videoplayback",
                "mimeType":"video/mp4; codecs=\"avc1\""
            }),
            json!({
                "itag":1,
                "url":"https://rr.googlevideo.com:8443/videoplayback",
                "mimeType":"video/mp4; codecs=\"avc1\""
            }),
            json!({
                "itag":1,
                "url":"https://rr.googlevideo.com/videoplayback",
                "mimeType":"application/octet-stream"
            }),
        ] {
            assert!(parse_direct_format(&stream).is_none());
        }
    }

    #[tokio::test]
    async fn declines_caller_credentials_and_zero_timeout_without_networking() {
        let url = format!("https://www.youtube.com/watch?v={VIDEO_ID}");
        for options in [
            ExtractOptions {
                cookies: Some("session=secret".to_owned()),
                ..ExtractOptions::default()
            },
            ExtractOptions {
                headers: vec![("X-Api-Key".to_owned(), "secret".to_owned())],
                ..ExtractOptions::default()
            },
        ] {
            assert!(matches!(
                YoutubeExtractor.extract(&url, &options).await,
                Err(ExtractionError::UnsupportedUrl)
            ));
        }
        assert!(matches!(
            YoutubeExtractor
                .extract(
                    &url,
                    &ExtractOptions {
                        timeout: Some(Duration::ZERO),
                        ..ExtractOptions::default()
                    }
                )
                .await,
            Err(ExtractionError::Timeout)
        ));
    }

    #[test]
    fn generic_extractor_defers_youtube_urls_to_specialized_fallback_chain() {
        use crate::GenericExtractor;

        assert!(
            !GenericExtractor.can_handle(&format!("https://www.youtube.com/watch?v={VIDEO_ID}"))
        );
        assert!(!GenericExtractor.can_handle(&format!("https://youtu.be/{VIDEO_ID}")));
    }
}

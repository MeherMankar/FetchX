//! Focused native YouTube video extraction.
//!
//! The URL forms and player-response fields follow the authorized
//! MeherMankar/yt-dlp source at
//! https://github.com/MeherMankar/yt-dlp, revision
//! d58c4588a54271fe74172e2fc2f09a4a44f70679 (Unlicense):
//! `yt_dlp/extractor/youtube/_video.py` and `_base.py`.
//! Signature and `n` challenges use the pinned yt-dlp EJS solver in the
//! in-process QuickJS wrapper from `youtube_js`.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use reqwest::{
    StatusCode, Url,
    header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderValue, ORIGIN, REFERER, USER_AGENT},
};
use serde_json::{Value, json};

use crate::{
    extractor::{
        ExtractOptions, ExtractionError, MediaExtractor, YoutubePlayerClient, YoutubePoToken,
        YoutubePoTokenContext,
    },
    generic::{client_for_url, read_bounded_body, validate_public_url},
    models::{MediaFormat, MediaInfo, MediaProtocol},
    youtube_js::{ChallengeRequest, ChallengeResults, solve_challenges},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FORMATS: usize = 64;
const MAX_PLAYER_SCRIPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_COOKIE_HEADER_BYTES: usize = 8 * 1024;
const PLAYER_SCRIPT_PATH: &str = "/s/player/{}/player_ias.vflset/en_US/base.js";
const TV_CLIENT_VERSION: &str = "5.20260707";
const TV_CLIENT_USER_AGENT: &str = "Mozilla/5.0 (SMART-TV; Linux; Tizen 2.4.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/2.4.0 TV Safari/538.1";
const WEB_CLIENT_VERSION: &str = "2.20260708.00.00";

#[derive(Clone, Copy)]
struct PlayerClientProfile {
    client: YoutubePlayerClient,
    client_name: &'static str,
    client_version: &'static str,
    client_id: &'static str,
    user_agent: Option<&'static str>,
    device_make: Option<&'static str>,
    device_model: Option<&'static str>,
    os_name: Option<&'static str>,
    os_version: Option<&'static str>,
    supports_cookies: bool,
    requires_gvs_token: bool,
}

const PLAYER_CLIENTS: [PlayerClientProfile; 3] = [
    PlayerClientProfile {
        client: YoutubePlayerClient::TvSamsung,
        client_name: "TVHTML5",
        client_version: TV_CLIENT_VERSION,
        client_id: "7",
        user_agent: Some(TV_CLIENT_USER_AGENT),
        device_make: Some("Samsung"),
        device_model: Some("UKS9800"),
        os_name: Some("Tizen"),
        os_version: Some("2.4.0"),
        supports_cookies: true,
        requires_gvs_token: false,
    },
    PlayerClientProfile {
        client: YoutubePlayerClient::VisionOs,
        client_name: "VISIONOS",
        client_version: "1.02",
        client_id: "101",
        user_agent: Some(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15",
        ),
        device_make: Some("Apple"),
        device_model: Some("RealityDevice17,1"),
        os_name: Some("visionOS"),
        os_version: Some("26.5.23O471"),
        supports_cookies: false,
        requires_gvs_token: false,
    },
    PlayerClientProfile {
        client: YoutubePlayerClient::Web,
        client_name: "WEB",
        client_version: WEB_CLIENT_VERSION,
        client_id: "1",
        user_agent: None,
        device_make: None,
        device_model: None,
        os_name: None,
        os_version: None,
        supports_cookies: true,
        requires_gvs_token: true,
    },
];

#[derive(Clone)]
struct PlayerRequest {
    profile: PlayerClientProfile,
    player_token: Option<String>,
    gvs_token: Option<String>,
}

type PoTokenMap = HashMap<(YoutubePlayerClient, YoutubePoTokenContext), String>;

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
        if !opts.headers.is_empty() {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let url = Url::parse(source).map_err(|_| ExtractionError::UnsupportedUrl)?;
        let video_id = extract_video_id(&url).ok_or(ExtractionError::UnsupportedUrl)?;
        let tokens = normalize_po_tokens(&opts.youtube_po_tokens)?;
        if let Some(cookies) = &opts.cookies {
            validate_cookie_jar(cookies)?;
        }
        let timeout = opts.timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
        if timeout.is_zero() {
            return Err(ExtractionError::Timeout);
        }

        let webpage_url = format!("https://www.youtube.com/watch?v={video_id}");
        match tokio::time::timeout(
            timeout,
            self.extract_inner(
                &webpage_url,
                &video_id,
                timeout,
                opts.cookies.as_deref(),
                &tokens,
            ),
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
        cookies: Option<&str>,
        tokens: &PoTokenMap,
    ) -> Result<MediaInfo, ExtractionError> {
        let requests = select_player_requests(cookies.is_some(), tokens);
        let mut selected = None;
        let mut access_requirement = None;
        for request in requests {
            let player_response =
                match fetch_player_response(video_id, timeout, &request, cookies).await {
                    Ok(response) => response,
                    Err(ExtractionError::UnsupportedUrl) => continue,
                    Err(error) => return Err(error),
                };
            match classify_player_response(&player_response) {
                PlayerResponseClass::Playable => {
                    if let Ok(parsed) = parse_player_response_with_token(
                        video_id,
                        &player_response,
                        request.gvs_token.clone(),
                    ) {
                        selected = Some((player_response, parsed));
                        break;
                    }
                }
                PlayerResponseClass::AuthenticationRequired(reason) => {
                    access_requirement = Some(reason);
                }
                PlayerResponseClass::Unsupported => {}
            }
        }
        let (player_response, mut parsed) = match selected {
            Some(selected) => selected,
            None => {
                return match access_requirement {
                    Some(reason) => Err(ExtractionError::AuthenticationRequired { reason }),
                    None => Err(ExtractionError::UnsupportedUrl),
                };
            }
        };
        if parsed.has_unhandled_challenges {
            return Err(ExtractionError::UnsupportedUrl);
        }

        let mut request = ChallengeRequest::default();
        for format in &parsed.formats {
            if let Some(challenge) = &format.signature_challenge {
                request.signatures.push(challenge.clone());
            }
            if let Some(challenge) = &format.n_challenge {
                request.n.push(challenge.clone());
            }
        }
        request.signatures.sort();
        request.signatures.dedup();
        request.n.sort();
        request.n.dedup();

        let challenge_results = if request.signatures.is_empty() && request.n.is_empty() {
            ChallengeResults::default()
        } else {
            let script_url = match player_script_url(&player_response)? {
                Some(url) => url,
                None => fetch_player_script_url(timeout).await?,
            };
            let player_script = fetch_player_script(&script_url, timeout).await?;
            solve_challenges(player_script, request).await?
        };

        let mut formats = Vec::new();
        let mut rejected_unsafe_urls = 0;
        for candidate in parsed.formats.drain(..).take(MAX_FORMATS) {
            let mut media_url =
                Url::parse(&candidate.url).map_err(|_| ExtractionError::UnsupportedUrl)?;
            apply_challenge_results(&mut media_url, &candidate, &challenge_results)?;
            if let Some(token) = &candidate.gvs_po_token {
                set_query_parameter(&mut media_url, "pot", token);
            }
            match validate_public_url(media_url.as_str()).await {
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
    request: &PlayerRequest,
    cookies: Option<&str>,
) -> Result<Value, ExtractionError> {
    let mut endpoint = Url::parse("https://www.youtube.com/youtubei/v1/player")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    endpoint
        .query_pairs_mut()
        .append_pair("prettyPrint", "false");
    let client = client_for_url(&endpoint, timeout).await?;
    let request_body = player_request_body(video_id, request);
    let body = serde_json::to_vec(&request_body).map_err(|_| ExtractionError::InvalidOutput)?;
    let mut request_builder = client
        .post(endpoint)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .header(ORIGIN, "https://www.youtube.com")
        .header(REFERER, "https://www.youtube.com/")
        .header("X-YouTube-Client-Name", request.profile.client_id)
        .header("X-YouTube-Client-Version", request.profile.client_version)
        .body(body);
    if let Some(user_agent) = request.profile.user_agent {
        request_builder = request_builder.header(USER_AGENT, user_agent);
    }
    if request.profile.supports_cookies
        && let Some(cookies) = cookies
        && let Some(cookie_header) = youtube_cookie_header(cookies)?
    {
        request_builder = request_builder.header(COOKIE, cookie_header);
    }
    let response = request_builder
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

fn player_request_body(video_id: &str, request: &PlayerRequest) -> Value {
    let profile = request.profile;
    let mut client = json!({
        "clientName": profile.client_name,
        "clientVersion": profile.client_version,
        "hl": "en"
    });
    for (field, value) in [
        ("deviceMake", profile.device_make),
        ("deviceModel", profile.device_model),
        ("userAgent", profile.user_agent),
        ("osName", profile.os_name),
        ("osVersion", profile.os_version),
    ] {
        if let Some(value) = value {
            client[field] = Value::String(value.to_owned());
        }
    }

    let mut body = json!({
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "context": { "client": client },
        "playbackContext": {
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS"
            }
        }
    });
    if let Some(token) = &request.player_token {
        body["serviceIntegrityDimensions"] = json!({"poToken": token});
    }
    body
}

fn select_player_requests(has_cookies: bool, tokens: &PoTokenMap) -> Vec<PlayerRequest> {
    PLAYER_CLIENTS
        .iter()
        .copied()
        .filter_map(|profile| {
            if has_cookies && !profile.supports_cookies {
                return None;
            }
            let gvs_token = tokens
                .get(&(profile.client, YoutubePoTokenContext::Gvs))
                .cloned();
            if profile.requires_gvs_token && gvs_token.is_none() {
                return None;
            }
            Some(PlayerRequest {
                profile,
                player_token: tokens
                    .get(&(profile.client, YoutubePoTokenContext::Player))
                    .cloned(),
                gvs_token,
            })
        })
        .collect()
}

fn normalize_po_tokens(tokens: &[YoutubePoToken]) -> Result<PoTokenMap, ExtractionError> {
    let mut normalized = PoTokenMap::new();
    for token in tokens {
        if token.token.len() > 4096 || token.token.is_empty() {
            return Err(ExtractionError::BackendError {
                message: "YouTube PO token is malformed".to_owned(),
            });
        }
        let value =
            URL_SAFE
                .decode(token.token.as_bytes())
                .map_err(|_| ExtractionError::BackendError {
                    message: "YouTube PO token is malformed".to_owned(),
                })?;
        if value.is_empty() {
            return Err(ExtractionError::BackendError {
                message: "YouTube PO token is malformed".to_owned(),
            });
        }
        let value = URL_SAFE.encode(value);
        if normalized
            .insert((token.client, token.context), value)
            .is_some()
        {
            return Err(ExtractionError::BackendError {
                message: "duplicate YouTube PO token scope".to_owned(),
            });
        }
    }
    Ok(normalized)
}

fn validate_cookie_jar(cookies: &str) -> Result<(), ExtractionError> {
    if cookies.len() > 65_536 {
        return Err(ExtractionError::BackendError {
            message: "YouTube cookie data exceeds the size limit".to_owned(),
        });
    }
    for line in cookies.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 7
            || fields[0].is_empty()
            || !matches!(fields[1], "TRUE" | "FALSE" | "true" | "false")
            || !fields[2].starts_with('/')
            || !matches!(fields[3], "TRUE" | "FALSE" | "true" | "false")
            || fields[4].parse::<i64>().is_err()
            || !is_cookie_name(fields[5])
            || !is_cookie_value(fields[6])
        {
            return Err(ExtractionError::BackendError {
                message: "YouTube cookie data is malformed".to_owned(),
            });
        }
    }
    Ok(())
}

fn youtube_cookie_header(cookies: &str) -> Result<Option<HeaderValue>, ExtractionError> {
    validate_cookie_jar(cookies)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let host = "www.youtube.com";
    let request_path = "/youtubei/v1/player";
    let mut pairs = Vec::new();
    for line in cookies.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        let domain = fields[0].trim_start_matches('.').to_ascii_lowercase();
        let includes_subdomains = fields[1].eq_ignore_ascii_case("true");
        let expires = fields[4].parse::<i64>().unwrap_or_default();
        let domain_matches = host == domain
            || domain == "youtube.com" && includes_subdomains && host.ends_with(".youtube.com");
        let path_matches = request_path == fields[2]
            || request_path
                .strip_prefix(fields[2])
                .is_some_and(|suffix| fields[2].ends_with('/') || suffix.starts_with('/'));
        if !domain_matches || !path_matches {
            continue;
        }
        if expires > 0 && expires <= now {
            continue;
        }
        pairs.push(format!("{}={}", fields[5], fields[6]));
    }
    if pairs.is_empty() {
        return Ok(None);
    }
    let value = pairs.join("; ");
    if value.len() > MAX_COOKIE_HEADER_BYTES {
        return Err(ExtractionError::BackendError {
            message: "YouTube cookie header exceeds the size limit".to_owned(),
        });
    }
    HeaderValue::from_str(&value)
        .map(Some)
        .map_err(|_| ExtractionError::BackendError {
            message: "YouTube cookie data is malformed".to_owned(),
        })
}

fn is_cookie_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn is_cookie_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| (0x21..=0x7e).contains(&byte) && !matches!(byte, b'"' | b',' | b';' | b'\\'))
}
fn player_script_url(response: &Value) -> Result<Option<Url>, ExtractionError> {
    let Some(raw) = response
        .pointer("/assets/js")
        .or_else(|| response.pointer("/playerConfig/assets/js"))
    else {
        return Ok(None);
    };
    let raw = raw.as_str().ok_or(ExtractionError::UnsupportedUrl)?;
    let base =
        Url::parse("https://www.youtube.com/").map_err(|_| ExtractionError::InvalidOutput)?;
    let url = base
        .join(raw)
        .map_err(|_| ExtractionError::UnsupportedUrl)?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.youtube.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || !url.path().starts_with("/s/player/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ExtractionError::UnsupportedUrl);
    }
    Ok(Some(url))
}

async fn fetch_player_script_url(timeout: Duration) -> Result<Url, ExtractionError> {
    let endpoint = Url::parse("https://www.youtube.com/iframe_api")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let client = client_for_url(&endpoint, timeout).await?;
    let response = client
        .get(endpoint)
        .header(ACCEPT, "text/javascript,*/*")
        .header(USER_AGENT, TV_CLIENT_USER_AGENT)
        .send()
        .await
        .map_err(|_| ExtractionError::BackendError {
            message: "YouTube player version request failed".to_owned(),
        })?;
    if response.status() != StatusCode::OK {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let body = read_bounded_body(response).await?;
    let body = String::from_utf8(body).map_err(|_| ExtractionError::UnsupportedUrl)?;
    let normalized = body.replace("\\/", "/");
    let player_id = extract_player_id(&normalized).ok_or(ExtractionError::UnsupportedUrl)?;
    let path = PLAYER_SCRIPT_PATH.replacen("{}", &player_id, 1);
    let url = Url::parse(&format!("https://www.youtube.com{path}"))
        .map_err(|_| ExtractionError::UnsupportedUrl)?;
    Ok(url)
}

fn extract_player_id(iframe_api: &str) -> Option<String> {
    iframe_api.match_indices("/player/").find_map(|(index, _)| {
        let version = iframe_api.get(index + "/player/".len()..)?;
        let id = version.get(..8)?;
        if id.bytes().all(|byte| byte.is_ascii_hexdigit())
            && version.as_bytes().get(8) == Some(&b'/')
        {
            Some(id.to_owned())
        } else {
            None
        }
    })
}

async fn fetch_player_script(url: &Url, timeout: Duration) -> Result<String, ExtractionError> {
    let client = client_for_url(url, timeout).await?;
    let response = client
        .get(url.clone())
        .header(ACCEPT, "text/javascript,*/*")
        .header(USER_AGENT, TV_CLIENT_USER_AGENT)
        .send()
        .await
        .map_err(|_| ExtractionError::BackendError {
            message: "YouTube player script request failed".to_owned(),
        })?;
    if response.status() != StatusCode::OK {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let body = read_bounded_body(response).await?;
    if body.len() > MAX_PLAYER_SCRIPT_BYTES {
        return Err(ExtractionError::UnsupportedUrl);
    }
    String::from_utf8(body).map_err(|_| ExtractionError::UnsupportedUrl)
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
    has_unhandled_challenges: bool,
}

struct DirectFormat {
    id: String,
    url: String,
    gvs_po_token: Option<String>,
    signature_param: Option<String>,
    signature_challenge: Option<String>,
    signature_value: Option<String>,
    n_challenge: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlayerResponseClass {
    Playable,
    AuthenticationRequired(&'static str),
    Unsupported,
}

fn classify_player_response(response: &Value) -> PlayerResponseClass {
    let status = response
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str);
    let reason = response
        .pointer("/playabilityStatus/reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match status {
        Some("OK") | None => PlayerResponseClass::Playable,
        Some("LOGIN_REQUIRED") => {
            PlayerResponseClass::AuthenticationRequired(access_reason(&reason))
        }
        Some("AGE_CHECK_REQUIRED") => PlayerResponseClass::AuthenticationRequired("age-restricted"),
        Some("UNPLAYABLE") if reason.contains("age") => {
            PlayerResponseClass::AuthenticationRequired("age-restricted")
        }
        Some("UNPLAYABLE")
            if reason.contains("sign in")
                || reason.contains("verify")
                || reason.contains("not a bot")
                || reason.contains("robot") =>
        {
            PlayerResponseClass::AuthenticationRequired(access_reason(&reason))
        }
        Some(_) => PlayerResponseClass::Unsupported,
    }
}

fn access_reason(reason: &str) -> &'static str {
    if reason.contains("age") {
        "age-restricted"
    } else if reason.contains("verify") || reason.contains("not a bot") || reason.contains("robot")
    {
        "verification-required"
    } else {
        "sign-in-required"
    }
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

#[cfg(test)]
fn parse_player_response(
    requested_video_id: &str,
    response: &Value,
) -> Result<ParsedPlayerResponse, ExtractionError> {
    parse_player_response_with_token(requested_video_id, response, None)
}

fn parse_player_response_with_token(
    requested_video_id: &str,
    response: &Value,
    gvs_po_token: Option<String>,
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
    let mut has_unhandled_challenges = false;
    if let Some(streaming_data) = response.get("streamingData").and_then(Value::as_object)
        && !streaming_data
            .get("licenseInfos")
            .and_then(Value::as_array)
            .is_some_and(|licenses| !licenses.is_empty())
    {
        for key in ["formats", "adaptiveFormats"] {
            if let Some(streams) = streaming_data.get(key).and_then(Value::as_array) {
                for stream in streams {
                    if stream
                        .get("drmFamilies")
                        .and_then(Value::as_array)
                        .is_some_and(|families| !families.is_empty())
                    {
                        continue;
                    }
                    if let Some(format) =
                        parse_direct_format_with_token(stream, gvs_po_token.as_deref())
                    {
                        formats.push(format);
                        if formats.len() == MAX_FORMATS {
                            break;
                        }
                    } else if stream_has_challenges(stream) {
                        has_unhandled_challenges = true;
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
        has_unhandled_challenges,
    })
}

#[cfg(test)]
fn parse_direct_format(stream: &Value) -> Option<DirectFormat> {
    parse_direct_format_with_token(stream, None)
}

fn parse_direct_format_with_token(
    stream: &Value,
    gvs_po_token: Option<&str>,
) -> Option<DirectFormat> {
    if stream
        .get("drmFamilies")
        .and_then(Value::as_array)
        .is_some_and(|families| !families.is_empty())
    {
        return None;
    }

    let (url, signature_param, signature_challenge, signature_value) =
        if let Some(url) = stream.get("url").and_then(Value::as_str) {
            (url.to_owned(), None, None, None)
        } else {
            let cipher = match (stream.get("signatureCipher"), stream.get("cipher")) {
                (Some(signature_cipher), None) | (None, Some(signature_cipher)) => {
                    signature_cipher.as_str()?
                }
                (Some(signature_cipher), Some(cipher))
                    if signature_cipher.as_str()? == cipher.as_str()? =>
                {
                    signature_cipher.as_str()?
                }
                (None, None) | (Some(_), Some(_)) => return None,
            };
            let parsed = parse_signature_cipher(cipher)?;
            (
                parsed.url,
                Some(parsed.parameter),
                parsed.challenge,
                parsed.signature,
            )
        };
    let parsed_url = Url::parse(&url).ok()?;
    if parsed_url.scheme() != "https"
        || !is_googlevideo_host(parsed_url.host_str()?)
        || !parsed_url.username().is_empty()
        || parsed_url.password().is_some()
        || parsed_url.port().is_some_and(|port| port != 443)
    {
        return None;
    }
    let n_values = parsed_url
        .query_pairs()
        .filter_map(|(key, value)| (key == "n").then_some(value.into_owned()))
        .collect::<Vec<_>>();
    if n_values.len() > 1 || n_values.first().is_some_and(String::is_empty) {
        return None;
    }
    if signature_param.as_deref() == Some("n") && !n_values.is_empty() {
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
        url,
        gvs_po_token: gvs_po_token.map(str::to_owned),
        signature_param,
        signature_challenge,
        signature_value,
        n_challenge: n_values.into_iter().next(),
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

struct ParsedSignatureCipher {
    url: String,
    parameter: String,
    challenge: Option<String>,
    signature: Option<String>,
}

fn parse_signature_cipher(cipher: &str) -> Option<ParsedSignatureCipher> {
    let mut url = None;
    let mut challenge = None;
    let mut signature = None;
    let mut parameter = None;
    for (key, value) in url::form_urlencoded::parse(cipher.as_bytes()) {
        match key.as_ref() {
            "url" if url.is_none() => url = Some(value.into_owned()),
            "s" if challenge.is_none() => challenge = Some(value.into_owned()),
            "sig" | "signature" if signature.is_none() => {
                signature = Some(value.into_owned());
            }
            "sp" if parameter.is_none() => parameter = Some(value.into_owned()),
            "url" | "s" | "sig" | "signature" | "sp" => return None,
            _ => {}
        }
    }
    if challenge.as_deref().is_some_and(str::is_empty)
        || signature.as_deref().is_some_and(str::is_empty)
        || challenge.is_some() && signature.is_some()
        || challenge.is_none() && signature.is_none()
    {
        return None;
    }
    let parameter = parameter.unwrap_or_else(|| "signature".to_owned());
    if !is_safe_query_key(&parameter) {
        return None;
    }
    Some(ParsedSignatureCipher {
        url: url?,
        parameter,
        challenge,
        signature,
    })
}

fn is_safe_query_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn stream_has_challenges(stream: &Value) -> bool {
    stream.get("signatureCipher").is_some()
        || stream.get("cipher").is_some()
        || stream
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
            .is_some_and(|url| url.query_pairs().any(|(key, _)| key == "n"))
}

fn apply_challenge_results(
    url: &mut Url,
    format: &DirectFormat,
    results: &ChallengeResults,
) -> Result<(), ExtractionError> {
    if let Some(parameter) = &format.signature_param {
        let signature = if let Some(challenge) = &format.signature_challenge {
            results
                .signatures
                .get(challenge)
                .ok_or(ExtractionError::UnsupportedUrl)?
        } else {
            format
                .signature_value
                .as_ref()
                .ok_or(ExtractionError::UnsupportedUrl)?
        };
        set_query_parameter(url, parameter, signature);
    }
    if let Some(challenge) = &format.n_challenge {
        let transformed = results
            .n
            .get(challenge)
            .ok_or(ExtractionError::UnsupportedUrl)?;
        set_query_parameter(url, "n", transformed);
    }
    Ok(())
}

fn set_query_parameter(url: &mut Url, key: &str, value: &str) {
    let mut query = url
        .query_pairs()
        .filter(|(existing, _)| existing != key)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    query.push((key.to_owned(), value.to_owned()));
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(query);
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
        let request = player_request_body(
            VIDEO_ID,
            &PlayerRequest {
                profile: PLAYER_CLIENTS[0],
                player_token: None,
                gvs_token: None,
            },
        );
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
    fn selects_only_pinned_clients_with_the_required_user_token() {
        let without_tokens = select_player_requests(false, &PoTokenMap::new());
        assert_eq!(
            without_tokens
                .iter()
                .map(|request| request.profile.client)
                .collect::<Vec<_>>(),
            [
                YoutubePlayerClient::TvSamsung,
                YoutubePlayerClient::VisionOs
            ]
        );

        let web_token = URL_SAFE.encode(b"user-provided-web-gvs-token");
        let tokens = [(
            (YoutubePlayerClient::Web, YoutubePoTokenContext::Gvs),
            web_token.clone(),
        )]
        .into_iter()
        .collect();
        let with_web_token = select_player_requests(false, &tokens);
        assert_eq!(
            with_web_token
                .iter()
                .map(|request| request.profile.client)
                .collect::<Vec<_>>(),
            [
                YoutubePlayerClient::TvSamsung,
                YoutubePlayerClient::VisionOs,
                YoutubePlayerClient::Web
            ]
        );
        assert_eq!(
            with_web_token[2].gvs_token.as_deref(),
            Some(web_token.as_str())
        );
        assert!(
            select_player_requests(true, &PoTokenMap::new())
                .iter()
                .all(|request| request.profile.client != YoutubePlayerClient::VisionOs)
        );
    }

    #[test]
    fn puts_only_caller_supplied_player_token_in_player_request() {
        let player_token = URL_SAFE.encode(b"provided-player-token");
        let request = PlayerRequest {
            profile: PLAYER_CLIENTS[0],
            player_token: Some(player_token.clone()),
            gvs_token: None,
        };
        let body = player_request_body(VIDEO_ID, &request);
        assert_eq!(
            body["serviceIntegrityDimensions"]["poToken"].as_str(),
            Some(player_token.as_str())
        );
        assert_eq!(
            body["context"]["client"]["clientName"].as_str(),
            Some("TVHTML5")
        );
    }

    #[test]
    fn classifies_player_response_access_reasons_without_preserving_server_text() {
        for (status, reason, expected) in [
            (
                "LOGIN_REQUIRED",
                "Sign in to confirm you are not a bot",
                PlayerResponseClass::AuthenticationRequired("verification-required"),
            ),
            (
                "LOGIN_REQUIRED",
                "Sign in to continue",
                PlayerResponseClass::AuthenticationRequired("sign-in-required"),
            ),
            (
                "UNPLAYABLE",
                "Sign in to confirm your age",
                PlayerResponseClass::AuthenticationRequired("age-restricted"),
            ),
            ("ERROR", "Unknown error", PlayerResponseClass::Unsupported),
            ("OK", "", PlayerResponseClass::Playable),
        ] {
            let response = json!({
                "playabilityStatus": {
                    "status": status,
                    "reason": reason
                }
            });
            assert_eq!(classify_player_response(&response), expected);
        }
    }

    #[test]
    fn forwards_only_valid_youtube_scoped_cookies_and_redacts_secrets() {
        let cookies = concat!(
            "# Netscape HTTP Cookie File\n",
            "#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tcaller-secret\n",
            ".googlevideo.com\tTRUE\t/\tTRUE\t0\tOTHER\tshould-not-forward\n",
        );
        let header = youtube_cookie_header(cookies)
            .expect("valid Netscape cookie jar")
            .expect("youtube cookie");
        assert_eq!(header.to_str().ok(), Some("SID=caller-secret"));
        assert!(
            !format!(
                "{:?}",
                YoutubePoToken {
                    client: YoutubePlayerClient::Web,
                    context: YoutubePoTokenContext::Gvs,
                    token: URL_SAFE.encode(b"token-secret")
                }
            )
            .contains("token-secret")
        );
        let options = ExtractOptions {
            cookies: Some(cookies.to_owned()),
            youtube_po_tokens: vec![YoutubePoToken {
                client: YoutubePlayerClient::Web,
                context: YoutubePoTokenContext::Gvs,
                token: URL_SAFE.encode(b"token-secret"),
            }],
            ..ExtractOptions::default()
        };
        let debug = format!("{options:?}");
        assert!(!debug.contains("caller-secret"));
        assert!(!debug.contains("should-not-forward"));
        assert!(!debug.contains("token-secret"));
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
                        "signatureCipher": "url=https%3A%2F%2Frr5.googlevideo.com%2Fvideoplayback%3Fx%3D1&s=encrypted&sp=sig",
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
        assert_eq!(parsed.formats.len(), 4);
        assert!(!parsed.has_unhandled_challenges);

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

        let ciphered = &parsed.formats[2];
        assert_eq!(ciphered.id, "137");
        assert_eq!(ciphered.signature_challenge.as_deref(), Some("encrypted"));
        assert_eq!(ciphered.signature_param.as_deref(), Some("sig"));
        assert_eq!(ciphered.n_challenge, None);
        assert_eq!(parsed.formats[3].id, "248");
        assert_eq!(parsed.formats[3].n_challenge.as_deref(), Some("challenge"));
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
                "signatureCipher":"s=encrypted"
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

    #[test]
    fn applies_transformed_signature_and_n_without_dropping_other_query_pairs() {
        let stream = json!({
            "itag":137,
            "signatureCipher":"url=https%3A%2F%2Frr5.googlevideo.com%2Fvideoplayback%3Fn%3Dabc123%26keep%3Dyes&s=encrypted&sp=signature",
            "mimeType":"video/mp4; codecs=\"avc1.640028\""
        });
        let format = parse_direct_format(&stream).expect("well-formed cipher");
        let mut url = Url::parse(&format.url).expect("valid media URL");
        let results = ChallengeResults {
            n: [("abc123".to_owned(), "321cba".to_owned())]
                .into_iter()
                .collect(),
            signatures: [("encrypted".to_owned(), "detpyrcne".to_owned())]
                .into_iter()
                .collect(),
        };
        apply_challenge_results(&mut url, &format, &results)
            .expect("all challenges have validated results");
        let query = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            query.get("signature").map(|value| value.as_ref()),
            Some("detpyrcne")
        );
        assert_eq!(query.get("n").map(|value| value.as_ref()), Some("321cba"));
        assert_eq!(query.get("keep").map(|value| value.as_ref()), Some("yes"));
    }

    #[test]
    fn attaches_only_the_client_scoped_caller_gvs_token_to_stream_urls() {
        let token = URL_SAFE.encode(b"caller-gvs-token");
        let response = json!({
            "videoDetails": {"videoId": VIDEO_ID, "title": "Sample"},
            "streamingData": {
                "formats": [{
                    "itag": 18,
                    "url": "https://rr.googlevideo.com/videoplayback?keep=yes",
                    "mimeType": "video/mp4; codecs=\"avc1\""
                }]
            }
        });
        let parsed = parse_player_response_with_token(VIDEO_ID, &response, Some(token.clone()))
            .expect("format response");
        let format = parsed.formats.into_iter().next().expect("direct format");
        let mut url = Url::parse(&format.url).expect("valid stream URL");
        if let Some(token) = &format.gvs_po_token {
            set_query_parameter(&mut url, "pot", token);
        }
        let query = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            query.get("pot").map(|value| value.as_ref()),
            Some(token.as_str())
        );
        assert_eq!(query.get("keep").map(|value| value.as_ref()), Some("yes"));
        let no_token = parse_player_response(VIDEO_ID, &response)
            .expect("format response without supplied token");
        assert!(no_token.formats[0].gvs_po_token.is_none());
    }

    #[test]
    fn malformed_or_unsupported_challenges_defer_the_video_to_fallback() {
        let response = json!({
            "videoDetails":{"videoId":VIDEO_ID,"title":"Sample"},
            "streamingData":{
                "formats":[{
                    "itag":18,
                    "url":"https://rr.googlevideo.com/videoplayback",
                    "mimeType":"video/mp4; codecs=\"avc1\""
                }],
                "adaptiveFormats":[{
                    "itag":137,
                    "signatureCipher":"s=encrypted",
                    "mimeType":"video/mp4; codecs=\"avc1\""
                }]
            }
        });
        let parsed = parse_player_response(VIDEO_ID, &response).expect("direct format exists");
        assert!(parsed.has_unhandled_challenges);

        let mut url =
            Url::parse("https://rr.googlevideo.com/videoplayback?n=abc").expect("valid URL");
        let format = parse_direct_format(&json!({
            "itag":18,
            "url":"https://rr.googlevideo.com/videoplayback?n=abc",
            "mimeType":"video/mp4; codecs=\"avc1\""
        }))
        .expect("n challenge format");
        assert!(matches!(
            apply_challenge_results(&mut url, &format, &ChallengeResults::default()),
            Err(ExtractionError::UnsupportedUrl)
        ));
    }

    #[test]
    fn accepts_only_player_script_urls_on_youtube_and_extracts_iframe_version() {
        let response =
            json!({"assets":{"js":"/s/player/12345678/player_ias.vflset/en_US/base.js"}});
        assert_eq!(
            player_script_url(&response)
                .expect("safe player script URL")
                .as_ref()
                .map(Url::as_str),
            Some("https://www.youtube.com/s/player/12345678/player_ias.vflset/en_US/base.js")
        );
        for raw in [
            "https://evil.example/s/player/12345678/player.js",
            "//127.0.0.1/s/player/12345678/player.js",
            "/youtubei/v1/player",
        ] {
            let response = json!({"assets":{"js":raw}});
            assert!(
                matches!(
                    player_script_url(&response),
                    Err(ExtractionError::UnsupportedUrl)
                ),
                "{raw}"
            );
        }
        assert!(
            player_script_url(&json!({}))
                .expect("missing script URL uses iframe fallback")
                .is_none()
        );
        assert_eq!(
            extract_player_id("var api='/player/1234abcd/iframe_api';").as_deref(),
            Some("1234abcd")
        );
        assert!(extract_player_id("var api='/player/not-a-version/';").is_none());
    }

    #[tokio::test]
    async fn declines_caller_credentials_and_zero_timeout_without_networking() {
        let url = format!("https://www.youtube.com/watch?v={VIDEO_ID}");
        let invalid_cookie_format = ExtractOptions {
            cookies: Some("session=secret".to_owned()),
            ..ExtractOptions::default()
        };
        assert!(matches!(
            YoutubeExtractor.extract(&url, &invalid_cookie_format).await,
            Err(ExtractionError::BackendError { .. })
        ));
        let extra_headers = ExtractOptions {
            headers: vec![("X-Api-Key".to_owned(), "secret".to_owned())],
            ..ExtractOptions::default()
        };
        assert!(matches!(
            YoutubeExtractor.extract(&url, &extra_headers).await,
            Err(ExtractionError::UnsupportedUrl)
        ));
        let invalid_token = ExtractOptions {
            youtube_po_tokens: vec![YoutubePoToken {
                client: YoutubePlayerClient::Web,
                context: YoutubePoTokenContext::Gvs,
                token: "not-base64?".to_owned(),
            }],
            ..ExtractOptions::default()
        };
        assert!(matches!(
            YoutubeExtractor.extract(&url, &invalid_token).await,
            Err(ExtractionError::BackendError { .. })
        ));
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

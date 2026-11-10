//! Public Kaltura media extraction for selected site wrappers.
//!
//! This bounded translation follows the authorized yt-dlp fork at revision
//! `d58c4588a54271fe74172e2fc2f09a4a44f70679` (Unlicense):
//! `kaltura.py`, `azmedien.py`, `inc.py`, and `unitednations.py`. It requests
//! public entry metadata and ready flavor assets only; it does not fetch
//! session-bound media, captions, playlists, or manifests.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Url;
use scraper::{Html, Selector};
use serde_json::{Value, json};

use crate::{
    ExtractOptions, ExtractionError, MediaExtractor,
    generic::validate_public_url,
    models::{MediaFormat, MediaInfo, MediaProtocol},
    provider_http,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FORMATS: usize = 64;

pub struct KalturaExtractor;

#[derive(Debug, Eq, PartialEq)]
enum Route {
    AzMedien {
        entry_id: Option<String>,
        display_id: String,
    },
    Inc {
        display_id: String,
    },
    UnitedNations {
        display_id: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct Entry {
    partner_id: String,
    entry_id: String,
}

#[async_trait]
impl MediaExtractor for KalturaExtractor {
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

impl KalturaExtractor {
    async fn extract_inner(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<MediaInfo, ExtractionError> {
        let route = parse_route(source).ok_or(ExtractionError::UnsupportedUrl)?;
        let (entry, display_title) = match route {
            Route::AzMedien {
                entry_id: Some(entry_id),
                display_id,
            } => (
                Entry {
                    partner_id: "1719221".to_owned(),
                    entry_id,
                },
                display_id,
            ),
            Route::AzMedien {
                entry_id: None,
                display_id,
            } => {
                let page = get_html(source, timeout).await?;
                let entry_id = find_embedded_entry_id(&page)?;
                (
                    Entry {
                        partner_id: "1719221".to_owned(),
                        entry_id,
                    },
                    display_id,
                )
            }
            Route::Inc { display_id } => {
                let page = get_html(source, timeout).await?;
                (parse_inc_entry(&page)?, display_id)
            }
            Route::UnitedNations { display_id } => {
                let page = get_html(source, timeout).await?;
                (parse_un_entry(&page)?, display_id)
            }
        };
        let (metadata, assets) = get_public_entry(&entry, timeout).await?;
        build_media_info(source, &display_title, &entry.entry_id, metadata, assets).await
    }
}

fn parse_route(source: &str) -> Option<Route> {
    let url = Url::parse(source).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut host = url
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if let Some(without_tv) = host.strip_prefix("tv.") {
        host = without_tv.to_owned();
    }
    let segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if matches!(
        host.as_str(),
        "telezueri.ch" | "telebaern.tv" | "telem1.ch" | "tvo-online.ch"
    ) && segments.len() >= 2
        && valid_display_id(segments[segments.len() - 1])
    {
        let entry_id = url
            .fragment()
            .and_then(|fragment| fragment.strip_prefix("video="))
            .filter(|value| valid_entry_id(value))
            .map(str::to_owned);
        return Some(Route::AzMedien {
            entry_id,
            display_id: segments[segments.len() - 1].to_owned(),
        });
    }
    if host == "inc.com" && url.path().ends_with(".html") && segments.len() >= 2 {
        let display_id = segments.last()?.strip_suffix(".html")?.to_owned();
        return (!display_id.is_empty()).then_some(Route::Inc { display_id });
    }
    if host == "webtv.un.org"
        && segments.len() >= 4
        && matches!(segments[0], "ar" | "zh" | "en" | "fr" | "ru" | "es")
        && segments[1] == "asset"
        && segments[2].bytes().all(|byte| byte.is_ascii_alphanumeric())
        && valid_entry_id(segments[3])
    {
        return Some(Route::UnitedNations {
            display_id: segments[3].to_owned(),
        });
    }
    None
}

fn valid_entry_id(value: &str) -> bool {
    (2..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_display_id(value: &str) -> bool {
    let Some((slug, id)) = value.rsplit_once('-') else {
        return false;
    };
    !slug.is_empty() && !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
}

async fn get_html(source: &str, timeout: Duration) -> Result<String, ExtractionError> {
    let url = Url::parse(source).map_err(|_| ExtractionError::UnsafeUrl {
        reason: "Kaltura wrapper URL is malformed",
    })?;
    let response = provider_http::get(&url, "text/html,application/xhtml+xml", timeout).await?;
    if !response.status.is_success() || !response.content_type.as_deref().is_some_and(is_html) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    String::from_utf8(response.body).map_err(|_| ExtractionError::InvalidOutput)
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

fn parse_inc_entry(html: &str) -> Result<Entry, ExtractionError> {
    let partner_id = capture_ascii_digits_after(html, "_bizo_data_partner_id")
        .unwrap_or_else(|| "1034971".to_owned());
    let entry_id = capture_after_prefix(html, "kaltura_player_")
        .filter(|value| valid_entry_id(value))
        .or_else(|| find_string_after_key(html, "vid_kaltura_id"))
        .filter(|value| valid_entry_id(value))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    Ok(Entry {
        partner_id,
        entry_id,
    })
}

fn parse_un_entry(html: &str) -> Result<Entry, ExtractionError> {
    let partner_id =
        capture_ascii_digits_after(html, "partnerId:").ok_or(ExtractionError::UnsupportedUrl)?;
    let entry_id = capture_after_marker(html, "kentryID")
        .filter(|value| valid_entry_id(value))
        .ok_or(ExtractionError::UnsupportedUrl)?;
    Ok(Entry {
        partner_id,
        entry_id,
    })
}

fn find_embedded_entry_id(html: &str) -> Result<String, ExtractionError> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("[data-kaltura-id], [data-entry-id]")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    for element in document.select(&selector) {
        if let Some(value) = element
            .value()
            .attr("data-kaltura-id")
            .or_else(|| element.value().attr("data-entry-id"))
            .filter(|value| valid_entry_id(value))
        {
            return Ok(value.to_owned());
        }
    }
    find_string_after_key(html, "kalturaId")
        .filter(|value| valid_entry_id(value))
        .ok_or(ExtractionError::UnsupportedUrl)
}

fn capture_ascii_digits_after(source: &str, marker: &str) -> Option<String> {
    let remainder = &source[source.find(marker)? + marker.len()..];
    let remainder = remainder.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '=' | ':' | '\'' | '"')
    });
    let digits = remainder.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| remainder[..digits].to_owned())
}

fn capture_after_prefix(source: &str, marker: &str) -> Option<String> {
    let remainder = &source[source.find(marker)? + marker.len()..];
    let remainder = remainder.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\'' | '"')
    });
    let length = remainder
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    (length > 0).then(|| remainder[..length].to_owned())
}

fn capture_after_marker(source: &str, marker: &str) -> Option<String> {
    let remainder = &source[source.find(marker)? + marker.len()..];
    let remainder = remainder.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '=' | ':')
    });
    let quote = remainder.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return capture_after_prefix(source, marker);
    }
    let value = &remainder[quote.len_utf8()..];
    let end = value.find(quote)?;
    Some(value[..end].to_owned())
}

fn find_string_after_key(source: &str, key: &str) -> Option<String> {
    let key_index = source.find(key)?;
    let remainder = &source[key_index + key.len()..];
    let colon = remainder.find(':')?;
    let value = remainder[colon + 1..].trim_start();
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let value = &value[quote.len_utf8()..];
    Some(value[..value.find(quote)?].to_owned())
}

async fn get_public_entry(
    entry: &Entry,
    timeout: Duration,
) -> Result<(Value, Vec<Value>), ExtractionError> {
    let url = Url::parse("https://www.kaltura.com/api_v3/service/multirequest")
        .map_err(|_| ExtractionError::InvalidOutput)?;
    let request = build_multirequest_request(entry)?;
    let response = provider_http::post(
        &url,
        "application/json",
        serde_json::to_vec(&request).map_err(|_| ExtractionError::InvalidOutput)?,
        timeout,
    )
    .await?;
    if !response.status.is_success() || !response.content_type.as_deref().is_some_and(is_json) {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let json: Value =
        serde_json::from_slice(&response.body).map_err(|_| ExtractionError::InvalidOutput)?;
    let values = json.as_array().ok_or(ExtractionError::UnsupportedUrl)?;
    if values
        .iter()
        .any(|value| value.get("objectType").and_then(Value::as_str) == Some("KalturaAPIException"))
    {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let metadata = values
        .get(1)
        .and_then(|value| value.pointer("/objects/0"))
        .cloned()
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let assets = values
        .get(2)
        .and_then(Value::as_array)
        .cloned()
        .ok_or(ExtractionError::UnsupportedUrl)?;
    Ok((metadata, assets))
}

fn build_multirequest_request(entry: &Entry) -> Result<Value, ExtractionError> {
    let partner_id = entry
        .partner_id
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(ExtractionError::UnsupportedUrl)?;
    Ok(json!({
        "apiVersion": "3.3.0",
        "clientTag": "html5:v3.1.0",
        "format": 1,
        "ks": "",
        "partnerId": partner_id,
        "1": {
            "expiry": 86400,
            "service": "session",
            "action": "startWidgetSession",
            "widgetId": format!("_{partner_id}")
        },
        "2": {
            "action": "list",
            "filter": {"redirectFromEntryId": entry.entry_id},
            "service": "baseentry",
            "ks": "{1:result:ks}",
            "responseProfile": {
                "type": 1,
                "fields": "createdAt,dataUrl,duration,name,plays,thumbnailUrl,userId"
            }
        },
        "3": {
            "action": "getbyentryid",
            "entryId": entry.entry_id,
            "service": "flavorAsset",
            "ks": "{1:result:ks}"
        }
    }))
}

async fn build_media_info(
    source: &str,
    display_title: &str,
    entry_id: &str,
    metadata: Value,
    assets: Vec<Value>,
) -> Result<MediaInfo, ExtractionError> {
    let title = metadata
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(display_title)
        .to_owned();
    let data_url = metadata
        .get("dataUrl")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && !value.contains("/playManifest/")
                && !value.contains('?')
                && !value.contains('#')
        })
        .ok_or(ExtractionError::UnsupportedUrl)?;
    let mut formats = Vec::new();
    for asset in assets.into_iter().take(MAX_FORMATS * 2) {
        let file_ext = asset
            .get("fileExt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if asset.get("status").and_then(parse_u64) != Some(2)
            || file_ext.eq_ignore_ascii_case("wvm")
            || file_ext.eq_ignore_ascii_case("chun")
        {
            continue;
        }
        let Some(asset_id) = asset.get("id").and_then(Value::as_str) else {
            continue;
        };
        if asset_id.is_empty() || !asset_id.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let normalized_ext = file_ext.to_ascii_lowercase();
        let ext = if matches!(
            normalized_ext.as_str(),
            "mp4" | "mov" | "webm" | "m4a" | "mp3"
        ) {
            normalized_ext
        } else if asset.get("containerFormat").and_then(Value::as_str) == Some("qt") {
            "mov".to_owned()
        } else {
            "mp4".to_owned()
        };
        let media_url = Url::parse(&format!(
            "{}/flavorId/{asset_id}",
            data_url.trim_end_matches('/')
        ))
        .map_err(|_| ExtractionError::InvalidOutput)?;
        let Ok(media_url) = validate_public_url(media_url.as_str()).await else {
            continue;
        };
        let bitrate = asset.get("bitrate").and_then(parse_u64);
        let height = asset.get("height").and_then(parse_u32);
        let width = asset.get("width").and_then(parse_u32);
        let id = format!(
            "{ext}{}",
            bitrate.map(|value| format!("-{value}")).unwrap_or_default()
        );
        formats.push(MediaFormat {
            id: id.clone(),
            ext,
            protocol: MediaProtocol::Http,
            url: media_url.to_string(),
            label: height.map(|value| format!("{value}p - {id}")).unwrap_or(id),
            video_codec: asset
                .get("videoCodecId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            audio_codec: None,
            width,
            height,
            fps: asset.get("frameRate").and_then(parse_f64),
            bitrate: bitrate.map(|value| value.saturating_mul(1000)),
            filesize: asset
                .get("size")
                .and_then(parse_u64)
                .map(|size| size.saturating_mul(1024)),
            required_headers: Vec::new(),
        });
        if formats.len() == MAX_FORMATS {
            break;
        }
    }
    if formats.is_empty() {
        return Err(ExtractionError::UnsupportedUrl);
    }
    let thumbnail_url = metadata
        .get("thumbnailUrl")
        .and_then(Value::as_str)
        .and_then(|url| reqwest::Url::parse(url).ok());
    let thumbnail_url = match thumbnail_url {
        Some(url) => validate_public_url(url.as_str())
            .await
            .ok()
            .map(|url| url.to_string()),
        None => None,
    };
    Ok(MediaInfo {
        title,
        duration: metadata
            .get("duration")
            .and_then(parse_f64)
            .filter(|value| value.is_finite() && *value >= 0.0),
        thumbnail_url,
        webpage_url: source.to_owned(),
        extractor_name: format!("Kaltura ({entry_id})"),
        formats,
    })
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
    fn recognizes_the_selected_kaltura_site_routes() {
        for url in [
            "https://tv.telezueri.ch/sonntalk/story-133214569#video=0_7xjo9lf1",
            "https://www.inc.com/editor/name-of-story.html",
            "https://webtv.un.org/en/asset/k1o/k1o7stmi6p",
        ] {
            assert!(KalturaExtractor.can_handle(url), "{url}");
        }
        assert!(!KalturaExtractor.can_handle("https://notinc.com/story.html"));
        assert!(!KalturaExtractor.can_handle("https://webtv.un.org/en/asset/k1o/a?entry_id=bad"));
    }

    #[test]
    fn parses_selected_wrapper_embeds_without_accepting_arbitrary_ids() {
        let az_medien = r#"<div data-kaltura-id="1_anruz3wy"></div>"#;
        assert_eq!(find_embedded_entry_id(az_medien).unwrap(), "1_anruz3wy");
        assert!(find_embedded_entry_id(r#"<div data-kaltura-id="../private"></div>"#).is_err());

        let inc = r#"var _bizo_data_partner_id = "123456"; id="kaltura_player_1_abcd1234";"#;
        assert_eq!(
            parse_inc_entry(inc).unwrap(),
            Entry {
                partner_id: "123456".to_owned(),
                entry_id: "1_abcd1234".to_owned()
            }
        );
        let un = r#"partnerId: 2503451; const kentryID = "1_o7stmi6p";"#;
        assert_eq!(parse_un_entry(un).unwrap().partner_id, "2503451");
        assert_eq!(parse_un_entry(un).unwrap().entry_id, "1_o7stmi6p");
        assert!(parse_inc_entry(r#"kaltura_player_../private"#).is_err());
    }

    #[test]
    fn builds_the_public_widget_multirequest_without_user_credentials() {
        let request = build_multirequest_request(&Entry {
            partner_id: "1719221".to_owned(),
            entry_id: "1_abcd1234".to_owned(),
        })
        .unwrap();
        assert_eq!(request["partnerId"], 1719221);
        assert_eq!(request["1"]["action"], "startWidgetSession");
        assert_eq!(request["2"]["service"], "baseentry");
        assert_eq!(request["2"]["ks"], "{1:result:ks}");
        assert_eq!(request["3"]["entryId"], "1_abcd1234");
        assert!(request.get("cookie").is_none());
    }

    #[tokio::test]
    async fn maps_public_ready_flavors_and_rejects_protected_or_manifest_data() {
        let metadata = json!({
            "name": "Public talk",
            "duration": 120,
            "thumbnailUrl": "https://example.com/thumbnail.jpg",
            "dataUrl": "https://example.com/entry/serveFlavor"
        });
        let assets = vec![
            json!({"status":2,"id":"1234","fileExt":"mp4","bitrate":800,"height":720,"size":1000}),
            json!({"status":2,"id":"1235","fileExt":"WVM"}),
            json!({"status":1,"id":"1236","fileExt":"mp4"}),
        ];
        let info = build_media_info(
            "https://inc.com/a/b.html",
            "display title",
            "1_abcd1234",
            metadata,
            assets,
        )
        .await
        .unwrap();
        assert_eq!(info.title, "Public talk");
        assert_eq!(info.duration, Some(120.0));
        assert_eq!(info.formats.len(), 1);
        assert_eq!(
            info.formats[0].url,
            "https://example.com/entry/serveFlavor/flavorId/1234"
        );
        assert!(
            build_media_info(
                "https://inc.com/a/b.html",
                "title",
                "1_abcd1234",
                json!({"name":"Restricted", "dataUrl":"https://example.com/playManifest/x"}),
                vec![]
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
            KalturaExtractor
                .extract("https://www.inc.com/editor/name-of-story.html", &options)
                .await,
            Err(ExtractionError::UnsupportedUrl)
        ));
    }
}

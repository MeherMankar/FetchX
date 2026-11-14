use serde::{Deserialize, Serialize};
use url::Url;

/// Identifies the kind of engine associated with a download source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum EngineKind {
    #[serde(rename = "direct_http")]
    DirectHttp,
    #[serde(rename = "aria2")]
    Aria2,
    #[serde(rename = "yt_dlp")]
    YtDlp,
    /// Native parallel HLS downloader (fetches all segments concurrently).
    #[serde(rename = "hls")]
    Hls,
    /// Native Rust BitTorrent engine (librqbit). Handles magnet URIs and .torrent files.
    #[serde(rename = "native_torrent")]
    NativeTorrent,
    /// FTP download engine (plain FTP, ftp:// scheme).
    #[serde(rename = "ftp")]
    Ftp,
    /// FTPS download engine (FTP over TLS, ftps:// scheme).
    #[serde(rename = "ftps")]
    Ftps,
    /// SFTP download engine (SSH File Transfer Protocol, sftp:// scheme).
    #[serde(rename = "sftp")]
    Sftp,
    /// Metalink / Metalink4 orchestration engine (.metalink / .meta4 manifests).
    #[serde(rename = "metalink")]
    Metalink,
}

impl EngineKind {
    /// Infer the native engine from a source URL.
    ///
    /// This is syntactic routing only; callers still validate the URL and
    /// verify that the inferred engine is available before queue admission.
    #[must_use]
    pub fn from_source_url(source: &str) -> Self {
        let Ok(url) = Url::parse(source) else {
            return Self::DirectHttp;
        };
        match url.scheme() {
            "magnet" | "torrent+file" => Self::NativeTorrent,
            "ftp" => Self::Ftp,
            "ftps" => Self::Ftps,
            "sftp" => Self::Sftp,
            "http" | "https" => {
                let filename = url
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .unwrap_or_default();
                let extension = filename
                    .rsplit_once('.')
                    .map(|(_, extension)| extension.to_ascii_lowercase());
                match extension.as_deref() {
                    Some("m3u8") => Self::Hls,
                    Some("meta4" | "metalink") => Self::Metalink,
                    Some("mpd" | "html" | "htm" | "php" | "asp" | "aspx") => Self::YtDlp,
                    Some(_) => Self::DirectHttp,
                    None if is_known_media_host(&url) || is_media_page_path(url.path()) => {
                        Self::YtDlp
                    }
                    None => Self::DirectHttp,
                }
            }
            _ => Self::DirectHttp,
        }
    }
}

fn is_known_media_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    [
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "twitch.tv",
        "dailymotion.com",
        "tiktok.com",
        "facebook.com",
        "instagram.com",
        "soundcloud.com",
    ]
    .iter()
    .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn is_media_page_path(path: &str) -> bool {
    let path_segments = path.to_ascii_lowercase();
    let path_segments = path_segments.split('/');
    ["watch", "video", "videos", "stream", "player", "embed"]
        .iter()
        .any(|pattern| path_segments.clone().any(|segment| segment == *pattern))
}

#[cfg(test)]
mod tests {
    use super::EngineKind;

    #[test]
    fn engine_names_are_stable_and_round_trip() -> Result<(), serde_json::Error> {
        for (engine, expected) in [
            (EngineKind::DirectHttp, "\"direct_http\""),
            (EngineKind::Aria2, "\"aria2\""),
            (EngineKind::YtDlp, "\"yt_dlp\""),
            (EngineKind::Hls, "\"hls\""),
            (EngineKind::NativeTorrent, "\"native_torrent\""),
            (EngineKind::Ftp, "\"ftp\""),
            (EngineKind::Ftps, "\"ftps\""),
            (EngineKind::Sftp, "\"sftp\""),
            (EngineKind::Metalink, "\"metalink\""),
        ] {
            let serialized = serde_json::to_string(&engine)?;
            assert_eq!(serialized, expected);

            let deserialized: EngineKind = serde_json::from_str(&serialized)?;
            assert_eq!(engine, deserialized);
        }

        Ok(())
    }

    #[test]
    fn source_url_inference_selects_engines_by_source_type() {
        for (source, expected) in [
            ("https://example.test/video.mp4", EngineKind::DirectHttp),
            ("https://example.test/watch", EngineKind::YtDlp),
            ("https://www.youtube.com/watch?v=abc", EngineKind::YtDlp),
            ("https://example.test/playlist.m3u8", EngineKind::Hls),
            ("https://example.test/file.meta4", EngineKind::Metalink),
            ("ftp://example.test/file.zip", EngineKind::Ftp),
            ("ftps://example.test/file.zip", EngineKind::Ftps),
            ("sftp://example.test/file.zip", EngineKind::Sftp),
            (
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                EngineKind::NativeTorrent,
            ),
        ] {
            assert_eq!(EngineKind::from_source_url(source), expected, "{source}");
        }
    }
}

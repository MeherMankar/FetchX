use serde::{Deserialize, Serialize};

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
}

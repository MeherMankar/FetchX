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
        ] {
            let serialized = serde_json::to_string(&engine)?;
            assert_eq!(serialized, expected);

            let deserialized: EngineKind = serde_json::from_str(&serialized)?;
            assert_eq!(engine, deserialized);
        }

        Ok(())
    }
}

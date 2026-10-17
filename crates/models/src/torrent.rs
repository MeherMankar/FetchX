use std::collections::{BTreeSet, HashSet};

use bendy::decoding::{Decoder, Object};
use sha1::{Digest, Sha1};
use thiserror::Error;

const MAX_TORRENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TORRENT_FILES: usize = 100_000;
const MAX_TORRENT_NAME_BYTES: usize = 1024;
const MAX_PATH_SEGMENT_BYTES: usize = 255;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentFile {
    /// Path relative to the torrent's root directory.
    pub path: Vec<String>,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentMetadata {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub files: Vec<TorrentFile>,
    pub trackers: Vec<String>,
    pub piece_length: u64,
    pub piece_count: usize,
}

impl TorrentMetadata {
    #[must_use]
    pub fn selection_all(&self) -> TorrentFileSelection {
        TorrentFileSelection {
            selected: vec![true; self.files.len()],
        }
    }

    #[must_use]
    pub fn selection_none(&self) -> TorrentFileSelection {
        TorrentFileSelection {
            selected: vec![false; self.files.len()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TorrentFileSelection {
    selected: Vec<bool>,
}

impl TorrentFileSelection {
    pub fn from_indices(
        file_count: usize,
        indices: impl IntoIterator<Item = usize>,
    ) -> Result<Self, TorrentMetadataError> {
        let mut selected = vec![false; file_count];
        for index in indices {
            let Some(value) = selected.get_mut(index) else {
                return Err(TorrentMetadataError::InvalidSelection);
            };
            *value = true;
        }
        Ok(Self { selected })
    }

    pub fn select_all(&mut self) {
        self.selected.fill(true);
    }

    pub fn deselect_all(&mut self) {
        self.selected.fill(false);
    }

    pub fn set_selected(
        &mut self,
        index: usize,
        selected: bool,
    ) -> Result<(), TorrentMetadataError> {
        let Some(value) = self.selected.get_mut(index) else {
            return Err(TorrentMetadataError::InvalidSelection);
        };
        *value = selected;
        Ok(())
    }

    pub fn selected_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| selected.then_some(index))
    }

    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|selected| **selected).count()
    }

    pub fn selected_size(&self, metadata: &TorrentMetadata) -> Result<u64, TorrentMetadataError> {
        if self.selected.len() != metadata.files.len() {
            return Err(TorrentMetadataError::InvalidSelection);
        }
        self.selected_indices().try_fold(0_u64, |total, index| {
            total
                .checked_add(metadata.files[index].size)
                .ok_or(TorrentMetadataError::InvalidMetainfo)
        })
    }

    pub fn select_directory(
        &mut self,
        metadata: &TorrentMetadata,
        directory: &[String],
        selected: bool,
    ) -> Result<(), TorrentMetadataError> {
        if self.selected.len() != metadata.files.len() {
            return Err(TorrentMetadataError::InvalidSelection);
        }
        for (index, file) in metadata.files.iter().enumerate() {
            if file.path.len() > directory.len()
                && file.path.iter().take(directory.len()).eq(directory.iter())
            {
                self.selected[index] = selected;
            }
        }
        Ok(())
    }
}

pub fn parse_torrent_metainfo(bytes: &[u8]) -> Result<TorrentMetadata, TorrentMetadataError> {
    if bytes.is_empty() || bytes.len() > MAX_TORRENT_BYTES {
        return Err(TorrentMetadataError::InvalidMetainfo);
    }

    let mut decoder = Decoder::new(bytes).with_max_depth(64);
    let root = decoder
        .next_object()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
        .ok_or(TorrentMetadataError::InvalidMetainfo)?;
    let mut root = root
        .try_into_dictionary()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;

    let mut info = None;
    let mut trackers = Vec::new();
    while let Some((key, value)) = root
        .next_pair()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
    {
        match key {
            b"info" => {
                if info.is_some() {
                    return Err(TorrentMetadataError::InvalidMetainfo);
                }
                info = Some(parse_info(value)?);
            }
            b"announce" => {
                let tracker = read_text(value)?;
                validate_tracker(&tracker)?;
                trackers.push(tracker);
            }
            b"announce-list" => {
                trackers.extend(parse_tracker_tiers(value)?);
            }
            _ => {}
        }
    }
    root.into_raw()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
    if decoder
        .next_object()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
        .is_some()
    {
        return Err(TorrentMetadataError::InvalidMetainfo);
    }

    let info = info.ok_or(TorrentMetadataError::InvalidMetainfo)?;
    let trackers = trackers
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(TorrentMetadata {
        info_hash: info.info_hash,
        name: info.name,
        total_size: info.total_size,
        files: info.files,
        trackers,
        piece_length: info.piece_length,
        piece_count: info.piece_count,
    })
}

struct ParsedInfo {
    info_hash: String,
    name: String,
    total_size: u64,
    files: Vec<TorrentFile>,
    piece_length: u64,
    piece_count: usize,
}

fn parse_info(object: Object<'_, '_>) -> Result<ParsedInfo, TorrentMetadataError> {
    let mut dict = object
        .try_into_dictionary()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
    let mut name = None;
    let mut utf8_name = None;
    let mut piece_length = None;
    let mut pieces = None;
    let mut single_file_length = None;
    let mut multi_files = None;

    while let Some((key, value)) = dict
        .next_pair()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
    {
        match key {
            b"name" | b"name.utf-8" => {
                let candidate = read_text(value)?;
                if candidate.is_empty() || candidate.len() > MAX_TORRENT_NAME_BYTES {
                    return Err(TorrentMetadataError::InvalidMetainfo);
                }
                if key == b"name" {
                    name = Some(candidate);
                } else {
                    utf8_name = Some(candidate);
                }
            }
            b"piece length" => piece_length = Some(read_u64(value)?),
            b"pieces" => pieces = Some(read_bytes(value)?),
            b"length" => single_file_length = Some(read_u64(value)?),
            b"files" => multi_files = Some(parse_files(value)?),
            _ => {}
        }
    }
    let raw_info = dict
        .into_raw()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;

    let name = utf8_name
        .or(name)
        .ok_or(TorrentMetadataError::InvalidMetainfo)?;
    validate_path_segment(&name)?;
    let piece_length = piece_length
        .filter(|length| *length > 0)
        .ok_or(TorrentMetadataError::InvalidMetainfo)?;
    let pieces = pieces.ok_or(TorrentMetadataError::InvalidMetainfo)?;
    if pieces.len() % 20 != 0 {
        return Err(TorrentMetadataError::InvalidMetainfo);
    }
    let piece_count = pieces.len() / 20;

    let files = match (single_file_length, multi_files) {
        (Some(length), None) => vec![TorrentFile {
            path: vec![name.clone()],
            size: length,
        }],
        (None, Some(files)) if !files.is_empty() => files,
        _ => return Err(TorrentMetadataError::InvalidMetainfo),
    };
    if files.len() > MAX_TORRENT_FILES {
        return Err(TorrentMetadataError::InvalidMetainfo);
    }
    validate_file_paths(&files)?;
    let total_size = files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.size)
            .ok_or(TorrentMetadataError::InvalidMetainfo)
    })?;
    let expected_piece_count = total_size
        .checked_add(piece_length - 1)
        .ok_or(TorrentMetadataError::InvalidMetainfo)?
        / piece_length;
    if expected_piece_count != piece_count as u64 {
        return Err(TorrentMetadataError::InvalidMetainfo);
    }

    let info_hash = Sha1::digest(raw_info)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(ParsedInfo {
        info_hash,
        name,
        total_size,
        files,
        piece_length,
        piece_count,
    })
}

fn parse_files(object: Object<'_, '_>) -> Result<Vec<TorrentFile>, TorrentMetadataError> {
    let mut list = object
        .try_into_list()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
    let mut files = Vec::new();
    while let Some(value) = list
        .next_object()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
    {
        if files.len() >= MAX_TORRENT_FILES {
            return Err(TorrentMetadataError::InvalidMetainfo);
        }
        let mut dict = value
            .try_into_dictionary()
            .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
        let mut size = None;
        let mut path = None;
        let mut utf8_path = None;
        while let Some((key, value)) = dict
            .next_pair()
            .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
        {
            match key {
                b"length" => size = Some(read_u64(value)?),
                b"path" | b"path.utf-8" => {
                    let segments = parse_path(value)?;
                    if key == b"path" {
                        path = Some(segments);
                    } else {
                        utf8_path = Some(segments);
                    }
                }
                _ => {}
            }
        }
        dict.into_raw()
            .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
        files.push(TorrentFile {
            path: utf8_path
                .or(path)
                .ok_or(TorrentMetadataError::InvalidMetainfo)?,
            size: size.ok_or(TorrentMetadataError::InvalidMetainfo)?,
        });
    }
    Ok(files)
}

fn parse_path(object: Object<'_, '_>) -> Result<Vec<String>, TorrentMetadataError> {
    let mut list = object
        .try_into_list()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
    let mut path = Vec::new();
    while let Some(value) = list
        .next_object()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
    {
        let segment = read_text(value)?;
        validate_path_segment(&segment)?;
        path.push(segment);
    }
    if path.is_empty() {
        return Err(TorrentMetadataError::UnsafePath);
    }
    Ok(path)
}

fn parse_tracker_tiers(object: Object<'_, '_>) -> Result<Vec<String>, TorrentMetadataError> {
    let mut tiers = object
        .try_into_list()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
    let mut trackers = Vec::new();
    while let Some(tier) = tiers
        .next_object()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
    {
        let mut urls = tier
            .try_into_list()
            .map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
        while let Some(url) = urls
            .next_object()
            .map_err(|_| TorrentMetadataError::InvalidMetainfo)?
        {
            let tracker = read_text(url)?;
            validate_tracker(&tracker)?;
            trackers.push(tracker);
            if trackers.len() > 256 {
                return Err(TorrentMetadataError::InvalidMetainfo);
            }
        }
    }
    Ok(trackers)
}

fn read_text(object: Object<'_, '_>) -> Result<String, TorrentMetadataError> {
    String::from_utf8(read_bytes(object)?).map_err(|_| TorrentMetadataError::InvalidMetainfo)
}

fn read_bytes(object: Object<'_, '_>) -> Result<Vec<u8>, TorrentMetadataError> {
    object
        .try_into_bytes()
        .map(<[u8]>::to_vec)
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)
}

fn read_u64(object: Object<'_, '_>) -> Result<u64, TorrentMetadataError> {
    let Object::Integer(value) = object else {
        return Err(TorrentMetadataError::InvalidMetainfo);
    };
    value
        .parse::<u64>()
        .map_err(|_| TorrentMetadataError::InvalidMetainfo)
}

fn validate_tracker(tracker: &str) -> Result<(), TorrentMetadataError> {
    let parsed = url::Url::parse(tracker).map_err(|_| TorrentMetadataError::InvalidMetainfo)?;
    if !matches!(parsed.scheme(), "http" | "https" | "udp")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(TorrentMetadataError::InvalidMetainfo);
    }
    Ok(())
}

fn validate_path_segment(segment: &str) -> Result<(), TorrentMetadataError> {
    if segment.is_empty()
        || segment.len() > MAX_PATH_SEGMENT_BYTES
        || matches!(segment, "." | "..")
        || segment.starts_with(['/', '\\'])
        || segment.contains(['/', '\\', ':'])
        || segment.trim_end_matches([' ', '.']) != segment
        || segment.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
        })
        || {
            let basename = segment
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            matches!(
                basename.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
        }
    {
        return Err(TorrentMetadataError::UnsafePath);
    }
    Ok(())
}

fn validate_file_paths(files: &[TorrentFile]) -> Result<(), TorrentMetadataError> {
    let mut paths = HashSet::new();
    for file in files {
        if file.path.is_empty() {
            return Err(TorrentMetadataError::UnsafePath);
        }
        for segment in &file.path {
            validate_path_segment(segment)?;
        }
        let folded = file
            .path
            .iter()
            .map(|segment| segment.to_lowercase())
            .collect::<Vec<_>>();
        if !paths.insert(folded) {
            return Err(TorrentMetadataError::ConflictingPaths);
        }
    }
    for file in files {
        for parent_len in 1..file.path.len() {
            let parent = file.path[..parent_len]
                .iter()
                .map(|segment| segment.to_lowercase())
                .collect::<Vec<_>>();
            if paths.contains(&parent) {
                return Err(TorrentMetadataError::ConflictingPaths);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TorrentMetadataError {
    #[error("torrent metainfo is malformed or exceeds supported limits")]
    InvalidMetainfo,
    #[error("torrent contains an unsafe file path")]
    UnsafePath,
    #[error("torrent contains duplicate or conflicting file paths")]
    ConflictingPaths,
    #[error("torrent selection does not match the file list")]
    InvalidSelection,
}

/// Real-time torrent telemetry sourced exclusively from aria2 JSON-RPC.
///
/// Fields not provided by aria2 are represented as `None`; callers must treat
/// those values as unavailable rather than assuming zero.
#[derive(Clone, Debug, Default)]
pub struct TorrentTelemetry {
    pub download_speed_bps: Option<u64>,
    pub upload_speed_bps: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub peer_count: Option<u64>,
    /// Number of connected remote peers that aria2 reports as complete seeders.
    pub completed_peer_count: Option<u64>,
    pub seed_count: Option<u64>,
    /// Upload / download ratio. `None` when either counter is unavailable or download is zero.
    pub share_ratio: Option<f64>,
    /// Seeding status reported by aria2, or `None` when unavailable.
    pub seeding: Option<bool>,
    /// Tracker URIs reported by aria2, or `None` when unavailable.
    pub tracker_uris: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{TorrentMetadataError, parse_torrent_metainfo};
    use crate::{DownloadSource, EngineKind};

    fn multi_file_metainfo(paths: &[(&[&str], u64)]) -> Vec<u8> {
        let mut encoded = b"d8:announce22:https://tracker.test/a4:infod5:filesl".to_vec();
        for (path, size) in paths {
            encoded.extend_from_slice(format!("d6:lengthi{size}e4:pathl").as_bytes());
            for segment in *path {
                encoded.extend_from_slice(format!("{}:{segment}", segment.len()).as_bytes());
            }
            encoded.extend_from_slice(b"ee");
        }
        encoded.extend_from_slice(b"e4:name4:root12:piece lengthi16384e6:pieces20:");
        encoded.extend_from_slice(&[7; 20]);
        encoded.extend_from_slice(b"ee");
        encoded
    }

    #[test]
    fn parses_multi_file_metainfo_and_selection_sizes() -> Result<(), TorrentMetadataError> {
        let metadata = parse_torrent_metainfo(&multi_file_metainfo(&[
            (&["docs", "one.txt"], 3),
            (&["docs", "two.txt"], 5),
        ]))?;
        assert_eq!(metadata.total_size, 8);
        assert_eq!(metadata.files.len(), 2);
        assert_eq!(metadata.trackers, ["https://tracker.test/a"]);
        assert_eq!(metadata.info_hash.len(), 40);

        let all_selection = metadata.selection_all();
        assert_eq!(all_selection.selected_count(), 2);
        assert_eq!(all_selection.selected_size(&metadata)?, 8);

        let mut selection = metadata.selection_none();
        selection.set_selected(1, true)?;
        assert_eq!(selection.selected_count(), 1);
        assert_eq!(selection.selected_size(&metadata)?, 5);
        selection.select_directory(&metadata, &["docs".to_owned()], true)?;
        assert_eq!(selection.selected_count(), 2);
        assert_eq!(selection.selected_size(&metadata)?, 8);
        selection.deselect_all();
        assert_eq!(selection.selected_size(&metadata)?, 0);
        Ok(())
    }

    #[test]
    fn rejects_traversal_and_duplicate_or_conflicting_paths() {
        assert_eq!(
            parse_torrent_metainfo(&multi_file_metainfo(&[(&["..", "escape.txt"], 1)])),
            Err(TorrentMetadataError::UnsafePath)
        );
        assert_eq!(
            parse_torrent_metainfo(&multi_file_metainfo(&[
                (&["docs", "same.txt"], 1),
                (&["docs", "same.txt"], 1),
            ])),
            Err(TorrentMetadataError::ConflictingPaths)
        );
        assert_eq!(
            parse_torrent_metainfo(&multi_file_metainfo(&[
                (&["docs", "Readme"], 1),
                (&["docs", "README"], 1),
            ])),
            Err(TorrentMetadataError::ConflictingPaths)
        );
        for unsafe_segment in [
            "",
            ".",
            "..",
            "/absolute",
            r"\rooted",
            r"\\server",
            "C:",
            "name.",
            "CON.txt",
            "nul",
        ] {
            assert_eq!(
                parse_torrent_metainfo(&multi_file_metainfo(&[(&[unsafe_segment, "file"], 1)])),
                Err(TorrentMetadataError::UnsafePath),
                "{unsafe_segment}"
            );
        }
        assert_eq!(
            parse_torrent_metainfo(&multi_file_metainfo(&[
                (&["root"], 1),
                (&["root", "child"], 1),
            ])),
            Err(TorrentMetadataError::ConflictingPaths)
        );
    }

    #[test]
    fn rejects_malformed_bencode_and_inconsistent_piece_metadata() {
        assert_eq!(
            parse_torrent_metainfo(b"d4:infoe"),
            Err(TorrentMetadataError::InvalidMetainfo)
        );
        let mut bytes = multi_file_metainfo(&[(&["docs", "one.txt"], 3)]);
        let index = bytes
            .windows(b"16384".len())
            .position(|window| window == b"16384")
            .expect("fixture contains piece length");
        bytes.splice(index..index + 5, b"0".iter().copied());
        assert_eq!(
            parse_torrent_metainfo(&bytes),
            Err(TorrentMetadataError::InvalidMetainfo)
        );
    }

    #[test]
    fn local_torrent_sources_round_trip_and_reject_wrong_engine_or_extension() {
        let torrent_path = Path::new(r"C:\FetchX\legal-fixture.torrent");
        let source = DownloadSource::new_torrent_file(torrent_path, EngineKind::Aria2)
            .expect("absolute Windows .torrent path is representable");
        assert!(source.is_torrent_file());
        assert_eq!(source.torrent_file_path(), Some(torrent_path.to_path_buf()));
        assert!(DownloadSource::new_torrent_file(torrent_path, EngineKind::DirectHttp).is_err());
        assert!(
            DownloadSource::new_torrent_file(
                Path::new(r"C:\FetchX\not-a-torrent.txt"),
                EngineKind::Aria2
            )
            .is_err()
        );
    }

    #[test]
    fn parses_single_file_and_hashes_the_original_info_dictionary() {
        let bytes = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:\
                      01234567890123456789ee";
        let metadata = parse_torrent_metainfo(bytes).expect("valid single-file metainfo");
        assert_eq!(metadata.name, "x");
        assert_eq!(metadata.total_size, 1);
        assert_eq!(metadata.files[0].path, ["x"]);
        assert_eq!(
            metadata.info_hash,
            "8940844489832856a7f018801641e9c97451ba30"
        );
    }
}

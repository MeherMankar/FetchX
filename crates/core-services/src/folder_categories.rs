use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use fetchx_models::FolderCategoriesConfig;

/// Given a completed file path and config, returns the destination path after
/// categorisation. Returns None if categorisation is disabled, no rule matches,
/// or the file is already in the target folder.
pub fn categorise(path: &Path, config: &FolderCategoriesConfig) -> Option<PathBuf> {
    if !config.enabled {
        return None;
    }

    // Extract the lowercase extension from the filename.
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;

    if ext.is_empty() {
        return None;
    }

    // Build extension → folder map from built-in rules then custom rules.
    let rules = build_rules(config);

    let folder = rules.get(&ext)?;
    let parent = path.parent()?;

    let dest_dir = if Path::new(folder.as_str()).is_absolute() {
        PathBuf::from(folder)
    } else {
        parent.join(folder)
    };

    // Already in the target folder — nothing to do.
    let already_in_folder = if Path::new(folder.as_str()).is_absolute() {
        parent == dest_dir
    } else {
        parent.file_name().and_then(|name| name.to_str()) == Some(folder.as_str())
    };
    if already_in_folder {
        return None;
    }

    Some(dest_dir.join(path.file_name()?))
}

fn build_rules(config: &FolderCategoriesConfig) -> HashMap<String, String> {
    let mut rules: HashMap<String, String> = HashMap::new();

    for rule in [
        &config.compressed,
        &config.documents,
        &config.music,
        &config.programs,
        &config.video,
        &config.images,
    ] {
        for ext in rule
            .extensions
            .split(',')
            .map(|e| e.trim().to_ascii_lowercase())
            .filter(|e| !e.is_empty())
        {
            rules.insert(ext, rule.folder.clone());
        }
    }

    // Parse custom rules: "Folder=ext,ext;Folder2=ext". Custom wins.
    for entry in config.custom.split(';') {
        let eq = match entry.find('=') {
            Some(i) if i >= 1 => i,
            _ => continue,
        };
        let folder = entry[..eq].trim();
        if folder.is_empty() {
            continue;
        }
        let exts = &entry[eq + 1..];
        for ext in exts
            .split(',')
            .map(|e| e.trim().to_ascii_lowercase())
            .filter(|e| !e.is_empty())
        {
            rules.insert(ext, folder.to_owned());
        }
    }

    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use fetchx_models::FolderCategoriesConfig;

    #[test]
    fn disabled_returns_none() {
        let config = FolderCategoriesConfig {
            enabled: false,
            ..FolderCategoriesConfig::default()
        };
        assert!(categorise(Path::new("/downloads/file.mp4"), &config).is_none());
    }

    #[test]
    fn categorises_video() {
        let config = FolderCategoriesConfig {
            enabled: true,
            ..FolderCategoriesConfig::default()
        };
        let result = categorise(Path::new("/downloads/movie.mp4"), &config);
        assert_eq!(result, Some(PathBuf::from("/downloads/Video/movie.mp4")));
    }

    #[test]
    fn no_match_returns_none() {
        let config = FolderCategoriesConfig {
            enabled: true,
            ..FolderCategoriesConfig::default()
        };
        assert!(categorise(Path::new("/downloads/file.xyz"), &config).is_none());
    }

    #[test]
    fn already_categorised_returns_none() {
        let config = FolderCategoriesConfig {
            enabled: true,
            ..FolderCategoriesConfig::default()
        };
        // Parent directory is already "Video"
        assert!(categorise(Path::new("/downloads/Video/movie.mp4"), &config).is_none());
    }

    #[test]
    fn custom_rule_wins() {
        let config = FolderCategoriesConfig {
            enabled: true,
            custom: "Movies=mp4".to_owned(),
            ..FolderCategoriesConfig::default()
        };
        let result = categorise(Path::new("/downloads/movie.mp4"), &config);
        assert_eq!(result, Some(PathBuf::from("/downloads/Movies/movie.mp4")));
    }

    #[test]
    fn no_extension_returns_none() {
        let config = FolderCategoriesConfig {
            enabled: true,
            ..FolderCategoriesConfig::default()
        };
        assert!(categorise(Path::new("/downloads/Makefile"), &config).is_none());
    }
}

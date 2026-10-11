//! Plugin manifest schema.
//!
//! Each plugin must ship a `plugin.json` file in its directory.
//! The manifest is validated before any process is started.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// API version the manifest format conforms to.
/// Increment when breaking manifest schema changes are introduced.
pub const PLUGIN_API_VERSION: u32 = 1;

/// Maximum allowed size of a manifest file in bytes (16 KiB).
pub const MAX_MANIFEST_BYTES: usize = 16_384;

// ============================================================
// Capabilities
// ============================================================

/// Explicit capability grants.
///
/// A plugin may only exercise capabilities listed in its manifest and
/// accepted by the host.  Unknown capabilities cause the manifest to be
/// rejected.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// The plugin provides a download engine that the queue can dispatch to.
    Engine,
    /// The plugin post-processes completed downloads (e.g. transcoding, renaming).
    PostProcessor,
    /// The plugin resolves URLs into direct download links.
    UrlResolver,
    /// The plugin provides metadata for a given URL or file.
    Metadata,
}

/// Platform the plugin executable supports.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
    #[serde(rename = "any")]
    Any,
}

// ============================================================
// Raw manifest (as stored on disk)
// ============================================================

/// Validated plugin manifest.
///
/// Constructed only through [`PluginManifestRaw::validate`].
/// All fields have been checked for invariants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    /// Unique, URL-safe plugin identifier.  Must match `^[a-z0-9_-]{2,64}$`.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Semver-compatible version string.
    pub version: String,
    /// Plugin API version this manifest targets.
    pub api_version: u32,
    /// Short description (≤ 256 characters).
    pub description: String,
    /// Plugin author name/contact.
    pub author: String,
    /// Absolute path to the plugin executable.
    pub executable: PathBuf,
    /// Capabilities this plugin requests.
    pub capabilities: Vec<Capability>,
    /// Supported platforms.  Empty means "all".
    pub platforms: Vec<Platform>,
}

/// Raw deserialised manifest before validation.
///
/// Received from the JSON file on disk; never trusted directly.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestRaw {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub description: String,
    pub author: String,
    /// Path to the plugin executable.  Must be absolute.
    pub executable: PathBuf,
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub platforms: Vec<Platform>,
}

impl PluginManifestRaw {
    /// Validates all fields and returns a [`PluginManifest`] on success.
    pub fn validate(self) -> Result<PluginManifest, ManifestValidationError> {
        // ── ID ───────────────────────────────────────────────────────────
        if self.id.is_empty() || self.id.len() > 64 {
            return Err(ManifestValidationError::InvalidId(self.id));
        }
        if !self
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(ManifestValidationError::InvalidId(self.id));
        }
        if self.id.len() < 2 {
            return Err(ManifestValidationError::InvalidId(self.id));
        }

        // ── Required string fields ───────────────────────────────────────
        if self.name.trim().is_empty() {
            return Err(ManifestValidationError::EmptyField("name"));
        }
        if self.version.trim().is_empty() {
            return Err(ManifestValidationError::EmptyField("version"));
        }
        if self.author.trim().is_empty() {
            return Err(ManifestValidationError::EmptyField("author"));
        }
        if self.description.len() > 256 {
            return Err(ManifestValidationError::DescriptionTooLong);
        }

        // ── API version ──────────────────────────────────────────────────
        if self.api_version != PLUGIN_API_VERSION {
            return Err(ManifestValidationError::UnsupportedApiVersion(
                self.api_version,
            ));
        }

        // ── Executable path ──────────────────────────────────────────────
        // Must be absolute — no relative paths, no path traversal tricks.
        if !self.executable.is_absolute() {
            return Err(ManifestValidationError::ExecutableNotAbsolute(
                self.executable.to_string_lossy().into_owned(),
            ));
        }
        // Detect traversal components.
        use std::path::Component;
        for component in self.executable.components() {
            if matches!(component, Component::ParentDir) {
                return Err(ManifestValidationError::PathTraversal(
                    self.executable.to_string_lossy().into_owned(),
                ));
            }
        }

        // ── Capabilities ─────────────────────────────────────────────────
        if self.capabilities.is_empty() {
            return Err(ManifestValidationError::NoCapabilities);
        }

        Ok(PluginManifest {
            id: self.id,
            name: self.name,
            version: self.version,
            api_version: self.api_version,
            description: self.description,
            author: self.author,
            executable: self.executable,
            capabilities: self.capabilities,
            platforms: self.platforms,
        })
    }
}

// ============================================================
// Validation errors
// ============================================================

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ManifestValidationError {
    #[error("plugin ID `{0}` is invalid (must match ^[a-z0-9_-]{{2,64}}$)")]
    InvalidId(String),
    #[error("required field `{0}` must not be empty")]
    EmptyField(&'static str),
    #[error("description exceeds the 256 character limit")]
    DescriptionTooLong,
    #[error("api_version {0} is not supported by this host (expected {PLUGIN_API_VERSION})")]
    UnsupportedApiVersion(u32),
    #[error("executable path `{0}` must be absolute")]
    ExecutableNotAbsolute(String),
    #[error("executable path `{0}` contains path traversal components")]
    PathTraversal(String),
    #[error("manifest must declare at least one capability")]
    NoCapabilities,
    #[error("manifest JSON is invalid: {0}")]
    Json(String),
    #[error("manifest file is too large (max {MAX_MANIFEST_BYTES} bytes)")]
    FileTooLarge,
}

/// Parse and validate a manifest from a JSON byte slice.
pub fn parse_manifest(bytes: &[u8]) -> Result<PluginManifest, ManifestValidationError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestValidationError::FileTooLarge);
    }
    let raw: PluginManifestRaw =
        serde_json::from_slice(bytes).map_err(|e| ManifestValidationError::Json(e.to_string()))?;
    raw.validate()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_raw(id: &str) -> PluginManifestRaw {
        PluginManifestRaw {
            id: id.to_owned(),
            name: "Test Plugin".to_owned(),
            version: "0.1.0".to_owned(),
            api_version: PLUGIN_API_VERSION,
            description: "A test plugin".to_owned(),
            author: "Test Author".to_owned(),
            executable: if cfg!(windows) {
                PathBuf::from(r"C:\plugins\test\test-plugin.exe")
            } else {
                PathBuf::from("/opt/plugins/test/test-plugin")
            },
            capabilities: vec![Capability::Engine],
            platforms: vec![],
        }
    }

    #[test]
    fn valid_manifest_passes() {
        assert!(minimal_raw("my-plugin").validate().is_ok());
    }

    #[test]
    fn empty_id_is_rejected() {
        assert!(matches!(
            minimal_raw("").validate(),
            Err(ManifestValidationError::InvalidId(_))
        ));
    }

    #[test]
    fn id_too_short_is_rejected() {
        assert!(matches!(
            minimal_raw("a").validate(),
            Err(ManifestValidationError::InvalidId(_))
        ));
    }

    #[test]
    fn id_with_uppercase_is_rejected() {
        assert!(matches!(
            minimal_raw("MyPlugin").validate(),
            Err(ManifestValidationError::InvalidId(_))
        ));
    }

    #[test]
    fn id_with_spaces_is_rejected() {
        assert!(matches!(
            minimal_raw("my plugin").validate(),
            Err(ManifestValidationError::InvalidId(_))
        ));
    }

    #[test]
    fn unsupported_api_version_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.api_version = 99;
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::UnsupportedApiVersion(99))
        ));
    }

    #[test]
    fn no_capabilities_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.capabilities = vec![];
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::NoCapabilities)
        ));
    }

    #[test]
    fn empty_name_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.name = "  ".to_owned();
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::EmptyField("name"))
        ));
    }

    #[test]
    fn description_too_long_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.description = "x".repeat(257);
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::DescriptionTooLong)
        ));
    }

    #[test]
    fn relative_executable_path_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.executable = PathBuf::from("relative/path/plugin");
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::ExecutableNotAbsolute(_))
        ));
    }

    #[test]
    fn path_traversal_is_rejected() {
        let mut raw = minimal_raw("my-plugin");
        raw.executable = if cfg!(windows) {
            PathBuf::from(r"C:\plugins\..\system32\cmd.exe")
        } else {
            PathBuf::from("/opt/plugins/../../../bin/sh")
        };
        assert!(matches!(
            raw.validate(),
            Err(ManifestValidationError::PathTraversal(_))
        ));
    }

    #[test]
    fn oversized_manifest_is_rejected() {
        let json = vec![b'x'; MAX_MANIFEST_BYTES + 1];
        assert!(matches!(
            parse_manifest(&json),
            Err(ManifestValidationError::FileTooLarge)
        ));
    }

    #[test]
    fn duplicate_id_handled_by_registry() {
        // Duplicate detection is the registry's job, not the manifest's.
        // Two manifests with the same ID are individually valid.
        let a = minimal_raw("my-plugin").validate().unwrap();
        let b = minimal_raw("my-plugin").validate().unwrap();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn unknown_fields_rejected_by_deserializer() {
        let json = br#"{
            "id": "my-plugin",
            "name": "Test",
            "version": "0.1.0",
            "api_version": 1,
            "description": "desc",
            "author": "auth",
            "executable": "/opt/plugins/test",
            "capabilities": ["engine"],
            "platforms": [],
            "unexpected_field": true
        }"#;
        assert!(parse_manifest(json).is_err());
    }
}

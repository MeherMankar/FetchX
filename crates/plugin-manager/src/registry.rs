//! In-memory plugin registry.
//!
//! The registry stores validated manifests keyed by plugin ID.
//! It enforces uniqueness and provides list/get operations.
//! Process lifecycle management is in [`crate::PluginManager`].

use std::collections::HashMap;

use fetchx_plugin_api::PluginManifest;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginRegistryError {
    #[error("plugin `{0}` is already registered")]
    DuplicateId(String),
    #[error("plugin `{0}` was not found")]
    NotFound(String),
}

/// A simple in-memory registry of validated plugin manifests.
///
/// No secrets or download state are stored here.
#[derive(Default)]
pub struct PluginRegistry {
    manifests: HashMap<String, PluginManifest>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a manifest.  Returns an error on duplicate ID.
    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), PluginRegistryError> {
        let id = manifest.id.clone();
        if self.manifests.contains_key(&id) {
            return Err(PluginRegistryError::DuplicateId(id));
        }
        self.manifests.insert(id, manifest);
        Ok(())
    }

    /// Remove a manifest by ID.  Returns an error if not found.
    pub fn unregister(&mut self, id: &str) -> Result<PluginManifest, PluginRegistryError> {
        self.manifests
            .remove(id)
            .ok_or_else(|| PluginRegistryError::NotFound(id.to_owned()))
    }

    /// Look up a manifest by ID.
    pub fn get(&self, id: &str) -> Option<&PluginManifest> {
        self.manifests.get(id)
    }

    /// List all registered manifests, sorted by ID for determinism.
    pub fn list(&self) -> Vec<&PluginManifest> {
        let mut items: Vec<&PluginManifest> = self.manifests.values().collect();
        items.sort_by_key(|m| &m.id);
        items
    }

    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fetchx_plugin_api::{Capability, PLUGIN_API_VERSION, PluginManifest};

    use super::*;

    fn manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: "Test".to_owned(),
            version: "0.1.0".to_owned(),
            api_version: PLUGIN_API_VERSION,
            description: "desc".to_owned(),
            author: "author".to_owned(),
            executable: PathBuf::from("/opt/plugin"),
            capabilities: vec![Capability::Engine],
            platforms: vec![],
        }
    }

    #[test]
    fn register_and_retrieve() {
        let mut reg = PluginRegistry::new();
        reg.register(manifest("my-plugin")).unwrap();
        assert!(reg.get("my-plugin").is_some());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn duplicate_registration_fails() {
        let mut reg = PluginRegistry::new();
        reg.register(manifest("my-plugin")).unwrap();
        assert!(matches!(
            reg.register(manifest("my-plugin")),
            Err(PluginRegistryError::DuplicateId(_))
        ));
    }

    #[test]
    fn unregister_removes_entry() {
        let mut reg = PluginRegistry::new();
        reg.register(manifest("my-plugin")).unwrap();
        reg.unregister("my-plugin").unwrap();
        assert!(reg.get("my-plugin").is_none());
        assert!(reg.is_empty());
    }

    #[test]
    fn unregister_missing_fails() {
        let mut reg = PluginRegistry::new();
        assert!(matches!(
            reg.unregister("ghost"),
            Err(PluginRegistryError::NotFound(_))
        ));
    }

    #[test]
    fn list_is_sorted_by_id() {
        let mut reg = PluginRegistry::new();
        reg.register(manifest("zz-plugin")).unwrap();
        reg.register(manifest("aa-plugin")).unwrap();
        let ids: Vec<&str> = reg.list().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["aa-plugin", "zz-plugin"]);
    }
}

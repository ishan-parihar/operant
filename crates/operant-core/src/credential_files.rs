use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Session-scoped file/cache mount manifest for remote environments.
///
/// Remote backends (Docker, Modal, SSH) create sandboxes with no host files.
/// This struct tracks which credential files and cache directories should be
/// mounted or synced into those sandboxes so the agent can access them.
///
/// **Mounts** — session-scoped registry of credential files (keyed by tool/skill name).
/// **Cache dirs** — gateway-cached uploads, browser screenshots, TTS audio, etc.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialFiles {
    mounts: HashMap<String, Vec<String>>,
    cache_dirs: HashMap<String, Vec<String>>,
}

impl CredentialFiles {
    /// Create an empty credential files registry.
    pub fn new() -> Self {
        Self {
            mounts: HashMap::new(),
            cache_dirs: HashMap::new(),
        }
    }

    /// Add a credential file path under the given key.
    ///
    /// Multiple paths can be registered for the same key.
    pub fn add_mount(&mut self, key: &str, path: &str) {
        self.mounts
            .entry(key.to_string())
            .or_insert_with(Vec::new)
            .push(path.to_string());
    }

    /// Get all credential file paths registered under the given key.
    pub fn get_mounts(&self, key: &str) -> Option<&Vec<String>> {
        self.mounts.get(key)
    }

    /// Add a cache directory path under the given key.
    ///
    /// Multiple paths can be registered for the same key.
    pub fn add_cache(&mut self, key: &str, path: &str) {
        self.cache_dirs
            .entry(key.to_string())
            .or_insert_with(Vec::new)
            .push(path.to_string());
    }

    /// Get all cache directory paths registered under the given key.
    pub fn get_cache(&self, key: &str) -> Option<&Vec<String>> {
        self.cache_dirs.get(key)
    }

    /// Remove all registered mounts and cache directories.
    pub fn clear(&mut self) {
        self.mounts.clear();
        self.cache_dirs.clear();
    }

    /// Returns `true` if no mounts or cache directories are registered.
    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty() && self.cache_dirs.is_empty()
    }

    /// Returns a reference to the full mounts map.
    pub fn all_mounts(&self) -> &HashMap<String, Vec<String>> {
        &self.mounts
    }

    /// Returns a reference to the full cache directories map.
    pub fn all_cache(&self) -> &HashMap<String, Vec<String>> {
        &self.cache_dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_empty() {
        let cf = CredentialFiles::new();
        assert!(cf.is_empty());
    }

    #[test]
    fn test_add_mount() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("github", "/root/.operant/github_token.json");
        assert!(!cf.is_empty());
        let mounts = cf.get_mounts("github");
        assert!(mounts.is_some());
        assert_eq!(mounts.unwrap().len(), 1);
        assert_eq!(mounts.unwrap()[0], "/root/.operant/github_token.json");
    }

    #[test]
    fn test_get_mounts_nonexistent_key() {
        let cf = CredentialFiles::new();
        assert!(cf.get_mounts("nonexistent").is_none());
    }

    #[test]
    fn test_add_cache() {
        let mut cf = CredentialFiles::new();
        cf.add_cache("documents", "/root/.operant/cache/documents");
        assert!(!cf.is_empty());
        let caches = cf.get_cache("documents");
        assert!(caches.is_some());
        assert_eq!(caches.unwrap().len(), 1);
        assert_eq!(caches.unwrap()[0], "/root/.operant/cache/documents");
    }

    #[test]
    fn test_get_cache_nonexistent_key() {
        let cf = CredentialFiles::new();
        assert!(cf.get_cache("nonexistent").is_none());
    }

    #[test]
    fn test_multiple_mounts_same_key() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("docker", "/root/.operant/config.json");
        cf.add_mount("docker", "/root/.operant/auth.json");
        let mounts = cf.get_mounts("docker").unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0], "/root/.operant/config.json");
        assert_eq!(mounts[1], "/root/.operant/auth.json");
    }

    #[test]
    fn test_multiple_cache_same_key() {
        let mut cf = CredentialFiles::new();
        cf.add_cache("images", "/root/.operant/cache/images");
        cf.add_cache("images", "/root/.operant/cache/screenshots");
        let caches = cf.get_cache("images").unwrap();
        assert_eq!(caches.len(), 2);
    }

    #[test]
    fn test_clear() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("github", "/root/.operant/token.json");
        cf.add_cache("docs", "/root/.operant/cache/documents");
        assert!(!cf.is_empty());
        cf.clear();
        assert!(cf.is_empty());
        assert!(cf.get_mounts("github").is_none());
        assert!(cf.get_cache("docs").is_none());
    }

    #[test]
    fn test_is_empty_after_operations() {
        let mut cf = CredentialFiles::new();
        assert!(cf.is_empty());
        cf.add_mount("a", "/p1");
        assert!(!cf.is_empty());
        cf.add_cache("b", "/p2");
        assert!(!cf.is_empty());
        cf.clear();
        assert!(cf.is_empty());
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("docker", "/root/.operant/config.json");
        cf.add_cache("images", "/root/.operant/cache/images");
        cf.add_cache("images", "/root/.operant/cache/screenshots");

        let json = serde_json::to_string(&cf).expect("serialize");
        let deserialized: CredentialFiles = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(
            deserialized.get_mounts("docker").unwrap()[0],
            "/root/.operant/config.json"
        );
        assert_eq!(deserialized.get_cache("images").unwrap().len(), 2);
    }

    #[test]
    fn test_deserialize_empty() {
        let json = r#"{"mounts":{},"cache_dirs":{}}"#;
        let cf: CredentialFiles = serde_json::from_str(json).expect("deserialize");
        assert!(cf.is_empty());
    }

    #[test]
    fn test_separate_mounts_and_cache() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("tool", "/cred/file.json");
        cf.add_cache("tool", "/cache/dir");
        // Same key used for both should not interfere
        assert_eq!(cf.get_mounts("tool").unwrap().len(), 1);
        assert_eq!(cf.get_cache("tool").unwrap().len(), 1);
    }

    #[test]
    fn test_multiple_keys() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("a", "/pa1");
        cf.add_mount("a", "/pa2");
        cf.add_mount("b", "/pb1");
        cf.add_cache("x", "/cx1");
        cf.add_cache("y", "/cy1");
        cf.add_cache("y", "/cy2");

        assert_eq!(cf.get_mounts("a").unwrap().len(), 2);
        assert_eq!(cf.get_mounts("b").unwrap().len(), 1);
        assert_eq!(cf.get_cache("x").unwrap().len(), 1);
        assert_eq!(cf.get_cache("y").unwrap().len(), 2);
    }

    #[test]
    fn test_all_mounts_and_all_cache() {
        let mut cf = CredentialFiles::new();
        cf.add_mount("k1", "/v1");
        cf.add_mount("k2", "/v2");
        cf.add_cache("c1", "/d1");

        assert_eq!(cf.all_mounts().len(), 2);
        assert!(cf.all_mounts().contains_key("k1"));
        assert!(cf.all_mounts().contains_key("k2"));

        assert_eq!(cf.all_cache().len(), 1);
        assert!(cf.all_cache().contains_key("c1"));
    }

    #[test]
    fn test_default_is_empty() {
        let cf = CredentialFiles::default();
        assert!(cf.is_empty());
    }
}

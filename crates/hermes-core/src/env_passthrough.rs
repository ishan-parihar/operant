use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Dynamic environment variable pass-through for subprocess execution.
///
/// Skills that declare `required_environment_variables` in their frontmatter
/// need those vars available in sandboxed execution environments. By default
/// sandboxes strip secrets from the child process environment for security.
/// This struct provides a configurable allow-list so skill-declared vars
/// (and user-configured overrides) pass through.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvPassthrough {
    vars: HashMap<String, String>,
}

impl EnvPassthrough {
    /// Create an empty passthrough registry.
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }

    /// Insert an environment variable into the passthrough allow-list.
    pub fn add(&mut self, key: &str, value: &str) {
        self.vars.insert(key.to_string(), value.to_string());
    }

    /// Remove an environment variable from the passthrough allow-list.
    pub fn remove(&mut self, key: &str) {
        self.vars.remove(key);
    }

    /// Get the value of an environment variable in the passthrough allow-list.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    /// Iterate over all (key, value) pairs in the passthrough allow-list.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Merge passthrough values into a base environment map.
    ///
    /// Returns a new `HashMap` containing all entries from `base_env`
    /// with values from this passthrough allow-list overriding any
    /// matching keys.
    pub fn merge(&self, base_env: &HashMap<String, String>) -> HashMap<String, String> {
        let mut result = base_env.clone();
        for (key, value) in &self.vars {
            result.insert(key.clone(), value.clone());
        }
        result
    }

    /// Load all environment variables whose names start with `prefix` into
    /// the passthrough allow-list. The prefix is **not** stripped from keys.
    ///
    /// For example, `from_env("HERMES_")` would capture `HERMES_HOME`,
    /// `HERMES_API_KEY`, etc. with their full original names.
    pub fn from_env(prefix: &str) -> Self {
        let mut vars = HashMap::new();
        for (key, value) in std::env::vars() {
            if key.starts_with(prefix) {
                vars.insert(key, value);
            }
        }
        Self { vars }
    }

    /// Returns the number of entries in the passthrough allow-list.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Returns `true` if the passthrough allow-list is empty.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Returns a reference to the underlying map.
    pub fn inner(&self) -> &HashMap<String, String> {
        &self.vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let ep = EnvPassthrough::new();
        assert!(ep.is_empty());
        assert_eq!(ep.len(), 0);
    }

    #[test]
    fn test_add_and_get() {
        let mut ep = EnvPassthrough::new();
        ep.add("MY_API_KEY", "sk-1234");
        assert_eq!(ep.get("MY_API_KEY"), Some("sk-1234"));
        assert_eq!(ep.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut ep = EnvPassthrough::new();
        ep.add("MY_KEY", "value");
        assert!(ep.get("MY_KEY").is_some());
        ep.remove("MY_KEY");
        assert!(ep.get("MY_KEY").is_none());
        assert!(ep.is_empty());
    }

    #[test]
    fn test_get_nonexistent() {
        let ep = EnvPassthrough::new();
        assert_eq!(ep.get("NONEXISTENT"), None);
    }

    #[test]
    fn test_overwrite_value() {
        let mut ep = EnvPassthrough::new();
        ep.add("MY_KEY", "first");
        ep.add("MY_KEY", "second");
        assert_eq!(ep.get("MY_KEY"), Some("second"));
    }

    #[test]
    fn test_iter() {
        let mut ep = EnvPassthrough::new();
        ep.add("A", "1");
        ep.add("B", "2");

        let mut pairs: Vec<(&str, &str)> = ep.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));

        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("A", "1"));
        assert_eq!(pairs[1], ("B", "2"));
    }

    #[test]
    fn test_iter_empty() {
        let ep = EnvPassthrough::new();
        assert_eq!(ep.iter().count(), 0);
    }

    #[test]
    fn test_merge_basic() {
        let mut ep = EnvPassthrough::new();
        ep.add("PASSTHROUGH_KEY", "pt-value");

        let mut base = HashMap::new();
        base.insert("BASE_KEY".to_string(), "base-value".to_string());

        let merged = ep.merge(&base);

        assert_eq!(merged.get("BASE_KEY").unwrap(), "base-value");
        assert_eq!(merged.get("PASSTHROUGH_KEY").unwrap(), "pt-value");
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_merge_override() {
        let mut ep = EnvPassthrough::new();
        ep.add("SHARED_KEY", "pt-override");

        let mut base = HashMap::new();
        base.insert("SHARED_KEY".to_string(), "base-value".to_string());

        let merged = ep.merge(&base);

        assert_eq!(merged.get("SHARED_KEY").unwrap(), "pt-override");
    }

    #[test]
    fn test_merge_empty() {
        let ep = EnvPassthrough::new();

        let mut base = HashMap::new();
        base.insert("K".to_string(), "v".to_string());

        let merged = ep.merge(&base);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.get("K").unwrap(), "v");
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut ep = EnvPassthrough::new();
        ep.add("MY_KEY", "my-value");
        ep.add("OTHER_KEY", "other-value");

        let json = serde_json::to_string(&ep).expect("serialize");
        let deserialized: EnvPassthrough = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.get("MY_KEY"), Some("my-value"));
        assert_eq!(deserialized.get("OTHER_KEY"), Some("other-value"));
        assert_eq!(deserialized.len(), 2);
    }

    #[test]
    fn test_default_is_empty() {
        let ep = EnvPassthrough::default();
        assert!(ep.is_empty());
    }

    #[test]
    fn test_inner_returns_underlying_map() {
        let mut ep = EnvPassthrough::new();
        ep.add("K", "v");
        let inner = ep.inner();
        assert_eq!(inner.len(), 1);
        assert_eq!(inner.get("K").unwrap(), "v");
    }
}

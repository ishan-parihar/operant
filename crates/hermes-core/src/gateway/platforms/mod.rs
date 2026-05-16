use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

pub mod discord;
pub mod slack;

use super::PlatformAdapter;
use crate::error::Result;

/// Registry of platform adapters by name.
pub struct PlatformRegistry {
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, adapter: Arc<dyn PlatformAdapter>) {
        self.adapters.insert(name.to_string(), adapter);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn PlatformAdapter>> {
        self.adapters.get(name)
    }

    pub fn get_all(&self) -> Vec<(&String, &Arc<dyn PlatformAdapter>)> {
        self.adapters.iter().collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.adapters.contains_key(name)
    }

    /// Start all registered platform adapters.
    pub async fn start_all(&self) -> Vec<(&str, Result<()>)> {
        let mut results = Vec::new();
        for (name, adapter) in &self.adapters {
            info!("Starting platform adapter: {}", name);
            let result = adapter.start().await;
            results.push((name.as_str(), result));
        }
        results
    }

    /// Stop all registered platform adapters.
    pub async fn stop_all(&self) -> Vec<(&str, Result<()>)> {
        let mut results = Vec::new();
        for (name, adapter) in &self.adapters {
            info!("Stopping platform adapter: {}", name);
            let result = adapter.stop().await;
            results.push((name.as_str(), result));
        }
        results
    }

    /// Number of registered adapters.
    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

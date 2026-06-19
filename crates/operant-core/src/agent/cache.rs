use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info};

use super::OperantAgent;

#[derive(Debug, Clone)]
pub struct AgentCacheConfig {
    pub max_size: usize,
    pub idle_ttl: Duration,
}

impl Default for AgentCacheConfig {
    fn default() -> Self {
        Self {
            max_size: 128,
            idle_ttl: Duration::from_secs(30 * 60),
        }
    }
}

pub struct AgentCacheEntry {
    pub agent: Arc<OperantAgent>,
    pub last_activity: Instant,
    pub model_override: Option<String>,
    pub session_key: String,
}

pub struct AgentCache {
    order: VecDeque<String>,
    entries: HashMap<String, AgentCacheEntry>,
    config: AgentCacheConfig,
}

impl AgentCache {
    pub fn new(config: AgentCacheConfig) -> Self {
        Self {
            order: VecDeque::with_capacity(config.max_size),
            entries: HashMap::with_capacity(config.max_size),
            config,
        }
    }

    pub fn get_or_create<F>(&mut self, session_key: &str, factory: F) -> Arc<OperantAgent>
    where
        F: FnOnce() -> OperantAgent,
    {
        if self.entries.contains_key(session_key) {
            let agent = {
                let entry = self.entries.get_mut(session_key).unwrap();
                entry.last_activity = Instant::now();
                Arc::clone(&entry.agent)
            };
            self.promote(session_key);
            debug!(session = session_key, "Returning cached agent");
            return agent;
        }

        debug!(session = session_key, "Creating new cached agent");
        let agent = Arc::new(factory());

        self.insert(
            session_key.to_string(),
            AgentCacheEntry {
                agent: Arc::clone(&agent),
                last_activity: Instant::now(),
                model_override: None,
                session_key: session_key.to_string(),
            },
        );

        agent
    }

    pub fn remove(&mut self, session_key: &str) -> Option<AgentCacheEntry> {
        self.order.retain(|k| k != session_key);
        self.entries.remove(session_key)
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    pub fn evict_idle(&mut self) -> usize {
        let now = Instant::now();
        let to_evict: Vec<String> = self
            .order
            .iter()
            .filter(|key| {
                self.entries
                    .get(key.as_str())
                    .map(|e| now.duration_since(e.last_activity) > self.config.idle_ttl)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        let count = to_evict.len();
        for key in &to_evict {
            debug!(session = key.as_str(), "Evicting idle cached agent");
            self.entries.remove(key);
        }
        self.order.retain(|k| !to_evict.contains(k));

        if count > 0 {
            info!(count, "Evicted idle cached agents");
        }
        count
    }

    pub fn enforce_capacity(&mut self) -> usize {
        let mut evicted = 0;
        while self.entries.len() > self.config.max_size {
            if let Some(oldest_key) = self.order.pop_front() {
                if self.entries.remove(&oldest_key).is_some() {
                    debug!(session = oldest_key.as_str(), "Evicted LRU cached agent");
                    evicted += 1;
                }
            } else {
                break;
            }
        }
        if evicted > 0 {
            info!(
                evicted,
                cache_size = self.entries.len(),
                max = self.config.max_size,
                "Enforced agent cache capacity"
            );
        }
        evicted
    }

    pub fn session_count(&self) -> usize {
        self.entries.len()
    }

    pub fn set_model_override(&mut self, session_key: &str, model: String) {
        if let Some(entry) = self.entries.get_mut(session_key) {
            debug!(
                session = session_key,
                model = model.as_str(),
                "Setting model override"
            );
            entry.model_override = Some(model);
        }
    }

    pub fn clear_model_override(&mut self, session_key: &str) {
        if let Some(entry) = self.entries.get_mut(session_key) {
            debug!(session = session_key, "Clearing model override");
            entry.model_override = None;
        }
    }

    pub fn get_model_override(&self, session_key: &str) -> Option<String> {
        self.entries
            .get(session_key)
            .and_then(|e| e.model_override.clone())
    }

    pub fn contains(&self, session_key: &str) -> bool {
        self.entries.contains_key(session_key)
    }

    pub fn snapshot(&self) -> Vec<(String, Duration, Option<String>)> {
        let now = Instant::now();
        self.entries
            .iter()
            .map(|(k, e)| {
                (
                    k.clone(),
                    now.duration_since(e.last_activity),
                    e.model_override.clone(),
                )
            })
            .collect()
    }

    fn insert(&mut self, key: String, entry: AgentCacheEntry) {
        if self.entries.contains_key(&key) {
            self.order.retain(|k| k != &key);
        }
        self.entries.insert(key.clone(), entry);
        self.order.push_back(key);
        self.enforce_capacity();
    }

    fn promote(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }
}

pub type SharedAgentCache = Arc<RwLock<AgentCache>>;

pub fn new_shared_cache() -> SharedAgentCache {
    Arc::new(RwLock::new(AgentCache::new(AgentCacheConfig::default())))
}

pub fn new_shared_cache_with_config(config: AgentCacheConfig) -> SharedAgentCache {
    Arc::new(RwLock::new(AgentCache::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AgentCacheConfig {
        AgentCacheConfig {
            max_size: 3,
            idle_ttl: Duration::from_millis(100),
        }
    }

    #[test]
    fn test_config_defaults() {
        let config = AgentCacheConfig::default();
        assert_eq!(config.max_size, 128);
        assert_eq!(config.idle_ttl, Duration::from_secs(30 * 60));
    }

    #[test]
    fn test_empty_cache_operations() {
        let config = test_config();
        let mut cache = AgentCache::new(config);
        assert_eq!(cache.session_count(), 0);
        assert!(!cache.contains("session1"));
        assert!(cache.snapshot().is_empty());
        assert_eq!(cache.evict_idle(), 0);
        assert_eq!(cache.enforce_capacity(), 0);
        cache.clear();
        assert_eq!(cache.session_count(), 0);
    }

    #[test]
    fn test_lru_promotion() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("c".to_string());

        order.retain(|k| k != "a");
        order.push_back("a".to_string());

        assert_eq!(order.front().unwrap(), "b");
        assert_eq!(order.back().unwrap(), "a");
    }

    #[test]
    fn test_lru_eviction_order() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("c".to_string());

        let evicted = order.pop_front().unwrap();
        assert_eq!(evicted, "a");
        assert_eq!(order.len(), 2);
        assert_eq!(order.front().unwrap(), "b");
    }

    #[test]
    fn test_lru_access_promotes_to_back() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("c".to_string());

        order.retain(|k| k != "b");
        order.push_back("b".to_string());

        assert_eq!(order.front().unwrap(), "a");
        assert_eq!(order.back().unwrap(), "b");
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn test_lru_multiple_access_promotes() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("c".to_string());

        for _ in 0..5 {
            order.retain(|k| k != "a");
            order.push_back("a".to_string());
        }

        assert_eq!(order.back().unwrap(), "a");
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn test_lru_interleaved_access_eviction() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("c".to_string());

        order.retain(|k| k != "b");
        order.push_back("b".to_string());

        let evicted = order.pop_front().unwrap();
        assert_eq!(evicted, "a");
    }

    #[test]
    fn test_lru_evict_below_limit() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        assert_eq!(order.len(), 2);
        assert!(order.front().is_some());
    }

    #[test]
    fn test_lru_clear() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.clear();
        assert!(order.is_empty());
    }

    #[tokio::test]
    async fn test_shared_cache_creation() {
        let cache = new_shared_cache();
        assert_eq!(cache.read().await.session_count(), 0);
    }

    #[tokio::test]
    async fn test_shared_cache_with_config() {
        let config = AgentCacheConfig {
            max_size: 5,
            idle_ttl: Duration::from_secs(60),
        };
        let cache = new_shared_cache_with_config(config);
        assert_eq!(cache.read().await.session_count(), 0);
    }

    #[test]
    fn test_lru_interleaved_eviction_order() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("c".to_string());
        order.push_back("d".to_string());

        order.retain(|k| k != "a");
        order.push_back("a".to_string());
        order.retain(|k| k != "c");
        order.push_back("c".to_string());

        let evicted = order.pop_front().unwrap();
        assert_eq!(evicted, "b");
    }

    #[test]
    fn test_lru_single_element() {
        let mut order = VecDeque::new();
        order.push_back("only".to_string());

        order.retain(|k| k != "only");
        order.push_back("only".to_string());

        assert_eq!(order.len(), 1);
        assert_eq!(order.front().unwrap(), "only");
    }

    #[test]
    fn test_lru_duplicate_insert() {
        let mut order = VecDeque::new();
        order.push_back("a".to_string());
        order.push_back("b".to_string());
        order.push_back("a".to_string());

        assert_eq!(order.len(), 3);
        let evicted = order.pop_front().unwrap();
        assert_eq!(evicted, "a");
        assert_eq!(order.pop_front().unwrap(), "b");
        assert_eq!(order.pop_front().unwrap(), "a");
    }
}

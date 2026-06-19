use std::collections::HashMap;
use std::sync::{Arc, RwLock};

type HookFn = Arc<dyn Fn(&str, &str, &serde_json::Value) -> Result<(), String> + Send + Sync>;

pub struct EventHooks {
    hooks: Arc<RwLock<HashMap<String, Vec<(String, HookFn)>>>>,
}

impl EventHooks {
    pub fn new() -> Self {
        Self {
            hooks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&self, event: &str, name: &str, f: HookFn) {
        let mut hooks = self.hooks.write().expect("EventHooks write lock poisoned");
        hooks
            .entry(event.into())
            .or_default()
            .push((name.into(), f));
    }

    pub fn unregister(&self, event: &str, name: &str) {
        let mut hooks = self.hooks.write().expect("EventHooks write lock poisoned");
        if let Some(hooks_list) = hooks.get_mut(event) {
            hooks_list.retain(|(n, _)| n != name);
        }
    }

    pub fn invoke(&self, event: &str, platform: &str, payload: &serde_json::Value) {
        let hooks = self.hooks.read().expect("EventHooks read lock poisoned");
        if let Some(hooks_list) = hooks.get(event) {
            for (name, f) in hooks_list {
                if let Err(e) = f(platform, event, payload) {
                    tracing::error!("Hook '{}' failed for event '{}': {}", name, event, e);
                }
            }
        }
    }

    pub fn list(&self) -> Vec<(String, String)> {
        let hooks = self.hooks.read().expect("EventHooks read lock poisoned");
        let mut result = Vec::new();
        for (event, entries) in hooks.iter() {
            for (name, _) in entries {
                result.push((event.clone(), name.clone()));
            }
        }
        result
    }
}

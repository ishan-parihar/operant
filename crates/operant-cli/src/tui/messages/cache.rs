//! Prepared-frame message cache for efficient transcript rendering.
//!
//! Instead of re-rendering markdown on every frame, we cache rendered
//! `Vec<Line>` per content hash per width. The cache is invalidated when
//! messages change or the terminal is resized.

use ratatui::text::Line;
use std::collections::HashMap;

/// A content-addressed cache of pre-rendered message lines.
///
/// Key insight: markdown rendering is expensive and width-dependent. By caching
/// the rendered output keyed on (content_hash, width), we avoid re-rendering
/// unchanged messages on every frame — the biggest performance win for the
/// transcript viewport.
pub struct MessageCache {
    /// `(content_hash, width) -> rendered lines`
    cache: HashMap<(u64, u16), Vec<Line<'static>>>,
    /// Monotonic version counter — bumped when messages change.
    /// Cache entries from older versions are lazily evicted.
    version: u64,
    /// Version at which each cache entry was created.
    entry_versions: HashMap<(u64, u16), u64>,
    /// Maximum number of cached entries to prevent unbounded growth.
    max_entries: usize,
}

impl MessageCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            version: 0,
            entry_versions: HashMap::new(),
            max_entries: 2048,
        }
    }

    /// Create a cache with a custom maximum entry count.
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(max_entries.min(4096)),
            version: 0,
            entry_versions: HashMap::with_capacity(max_entries.min(4096)),
            max_entries,
        }
    }

    /// Get cached lines or render and cache them.
    ///
    /// `content_hash` should be a fast hash of the raw text content.
    /// `width` is the rendering width in terminal columns.
    /// `render_fn` is called only on cache miss.
    pub fn get_or_render(
        &mut self,
        content_hash: u64,
        width: u16,
        render_fn: impl FnOnce() -> Vec<Line<'static>>,
    ) -> &[Line<'static>] {
        let key = (content_hash, width);
        if !self.cache.contains_key(&key) {
            // Evict oldest entries if at capacity
            if self.cache.len() >= self.max_entries {
                self.gc(0); // evict all entries from versions older than current
            }
            let lines = render_fn();
            self.cache.insert(key, lines);
            self.entry_versions.insert(key, self.version);
        }
        self.cache.get(&key).unwrap()
    }

    /// Invalidate all cached entries. Call when messages change.
    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.entry_versions.clear();
        self.version += 1;
    }

    /// Set the maximum number of cached entries.
    pub fn set_max_entries(&mut self, max: usize) {
        self.max_entries = max;
    }

    /// Maximum number of cached entries.
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Bump the version counter. Entries from old versions are lazily evicted
    /// on the next `gc` call.
    pub fn bump_version(&mut self) {
        self.version += 1;
    }

    /// Evict cache entries from versions older than `keep_versions` ago.
    pub fn gc(&mut self, keep_versions: u64) {
        let cutoff = self.version.saturating_sub(keep_versions);
        let stale: Vec<_> = self
            .entry_versions
            .iter()
            .filter(|(_, v)| **v < cutoff)
            .map(|(k, _)| *k)
            .collect();
        for key in stale {
            self.cache.remove(&key);
            self.entry_versions.remove(&key);
        }
    }

    /// Current version counter.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// Fast, non-crypto hash for content strings. Quality matters more than speed
/// here since we're hashing short-to-medium text snippets on every frame.
pub fn hash_content(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl Default for MessageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    #[test]
    fn cache_miss_renders() {
        let mut cache = MessageCache::new();
        let result = cache.get_or_render(42, 80, || {
            vec![Line::from(vec![Span::styled(
                "hello",
                Style::default().fg(Color::White),
            )])]
        });
        assert_eq!(result.len(), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_hit_skips_render() {
        let mut cache = MessageCache::new();
        let render_count = std::cell::Cell::new(0u32);

        cache.get_or_render(42, 80, || {
            render_count.set(render_count.get() + 1);
            vec![Line::from("first")]
        });

        cache.get_or_render(42, 80, || {
            render_count.set(render_count.get() + 1);
            vec![Line::from("second")]
        });

        assert_eq!(render_count.get(), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn different_widths_get_different_entries() {
        let mut cache = MessageCache::new();
        cache.get_or_render(42, 80, || vec![Line::from("wide")]);
        cache.get_or_render(42, 40, || vec![Line::from("narrow")]);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn invalidate_clears_cache() {
        let mut cache = MessageCache::new();
        cache.get_or_render(42, 80, || vec![Line::from("cached")]);
        assert_eq!(cache.len(), 1);

        cache.invalidate();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.version(), 1);
    }

    #[test]
    fn gc_evicts_old_versions() {
        let mut cache = MessageCache::new();
        cache.get_or_render(1, 80, || vec![Line::from("v0")]);
        cache.bump_version();
        cache.get_or_render(2, 80, || vec![Line::from("v1")]);
        cache.bump_version();
        cache.get_or_render(3, 80, || vec![Line::from("v2")]);

        // Keep only the last 1 version — evict entries from version 0
        cache.gc(1);
        assert_eq!(cache.len(), 2); // v1 and v2 remain
    }

    #[test]
    fn hash_content_deterministic() {
        let h1 = hash_content("hello world");
        let h2 = hash_content("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_content_different_for_different_input() {
        let h1 = hash_content("hello");
        let h2 = hash_content("world");
        assert_ne!(h1, h2);
    }
}

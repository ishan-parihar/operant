// messages/cache.rs — Content hashing for the transcript render path.
//
// (iter-392: `MessageCache` — the prepared-frame content-addressed line cache —
// was deleted here. Nothing ever constructed it; the live per-frame transcript
// caches are the thread-locals in `tui/render/cache.rs`, which key on full
// content equality instead of a pre-hash. `hash_content` survives because
// `tui/mermaid.rs` uses it; the module path is kept so that import resolves.)

/// Fast, non-crypto hash for content strings. Quality matters more than speed
/// here since we're hashing short-to-medium text snippets on every frame.
pub fn hash_content(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

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

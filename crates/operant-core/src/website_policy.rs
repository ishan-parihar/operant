//! Website access policy with fnmatch-style URL blocklisting.
//!
//! Maintains a list of domain/URL patterns and checks URLs against them.
//! Compiled regex patterns are cached with a 30-second TTL to avoid
//! repeated compilation.

use std::cell::RefCell;
use std::time::Instant;

use regex::Regex;

/// Default blocklist patterns loaded by [`WebsitePolicy::new`].
const DEFAULT_BLOCKLIST: &[&str] = &[
    "*.example.com",
    "*.test.com",
    "*.invalid",
    "*.local",
    "*.localhost",
    "*.malware.test",
    "*.phishing.test",
    "*.exploit.test",
];

/// Cache TTL in seconds.
const CACHE_TTL_SECS: u64 = 30;

/// A compiled pattern entry in the cache.
struct CompiledEntry {
    regex: Regex,
    is_path_pattern: bool,
}

/// Website blocklist policy.
///
/// Maintains a list of fnmatch-style domain patterns and caches compiled
/// regex versions with a 30-second TTL.
pub struct WebsitePolicy {
    patterns: Vec<String>,
    cache: RefCell<(Option<Vec<CompiledEntry>>, Instant)>,
}

impl Default for WebsitePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl WebsitePolicy {
    /// Create a new policy pre-loaded with [`DEFAULT_BLOCKLIST`] patterns.
    pub fn new() -> Self {
        let mut policy = Self {
            patterns: Vec::new(),
            cache: RefCell::new((None, Instant::now())),
        };
        for pattern in DEFAULT_BLOCKLIST {
            policy.add_pattern(pattern);
        }
        policy
    }

    /// Create a policy from an explicit list of patterns (no defaults).
    pub fn from_list(patterns: Vec<String>) -> Self {
        Self {
            patterns,
            cache: RefCell::new((None, Instant::now())),
        }
    }

    /// Add a single fnmatch pattern to the blocklist.
    ///
    /// Invalidates the compiled regex cache so the new pattern takes
    /// effect on the next [`is_blocked`] call.
    pub fn add_pattern(&mut self, pattern: &str) {
        self.patterns.push(pattern.to_string());
        *self.cache.borrow_mut() = (None, Instant::now());
    }

    /// Returns `true` if `url` matches any blocklist pattern.
    ///
    /// Extracts the host from the URL and matches it against patterns.
    /// Patterns containing `/` are matched against the full URL instead.
    ///
    /// Compiled regex patterns are cached with a 30-second TTL.
    pub fn is_blocked(&self, url: &str) -> bool {
        let host = extract_host(url);
        if host.is_empty() {
            return false;
        }

        let compiled = self.get_or_compile();
        for entry in compiled.iter() {
            if entry.is_path_pattern {
                if entry.regex.is_match(url) {
                    return true;
                }
            } else if pattern_matches_domain(&host, &entry.regex, &entry.regex.as_str()) {
                return true;
            }
        }
        false
    }

    /// Return the current list of patterns.
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    fn get_or_compile(&self) -> std::cell::Ref<'_, Vec<CompiledEntry>> {
        {
            let cache = self.cache.borrow();
            let (ref cached, ref timestamp) = *cache;
            if cached.is_some() && timestamp.elapsed().as_secs() < CACHE_TTL_SECS {
                return std::cell::Ref::map(cache, |c| c.0.as_ref().unwrap());
            }
        }
        let entries: Vec<CompiledEntry> = self
            .patterns
            .iter()
            .map(|p| CompiledEntry {
                regex: compile_fnmatch(p),
                is_path_pattern: p.contains('/'),
            })
            .collect();
        *self.cache.borrow_mut() = (Some(entries), Instant::now());
        let cache = self.cache.borrow();
        std::cell::Ref::map(cache, |c| c.0.as_ref().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Host extraction from URLs
// ---------------------------------------------------------------------------

/// Extract a normalized (lowercased, stripped) host from a URL string.
fn extract_host(raw_url: &str) -> String {
    let input = raw_url.trim();
    if input.is_empty() {
        return String::new();
    }

    let with_scheme = if input.contains("://") {
        input.to_string()
    } else {
        format!("http://{}", input)
    };

    if let Ok(parsed) = url::Url::parse(&with_scheme) {
        parsed.host_str().unwrap_or("").trim().to_lowercase()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// fnmatch → regex compilation
// ---------------------------------------------------------------------------

/// Convert an fnmatch-style glob to an anchored regex pattern.
fn fnmatch_to_regex(pattern: &str) -> String {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            // Escape regex metacharacters
            '.' | '+' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\' => {
                re.push('\\');
                re.push(ch);
            }
            c => re.push(c),
        }
    }
    re.push('$');
    re
}

/// Compile an fnmatch pattern to a Regex.
///
/// For non-wildcard domain patterns, creates an additional alternation
/// to match subdomains (e.g. `evil.com` also matches `sub.evil.com`).
fn compile_fnmatch(pattern: &str) -> Regex {
    if pattern.contains('/') {
        // Allow any protocol/host prefix — ^px$ becomes ^.*px$ so
        // "https://example.com/secret" matches pattern "example.com/secret/*"
        let inner = fnmatch_to_regex(pattern);
        let relaxed = format!("^.*{}", &inner[1..]);
        Regex::new(&relaxed).expect("invalid fnmatch pattern")
    } else if pattern.starts_with("*.") {
        // *.pattern → matches "pattern" and "sub.pattern"
        let bare = &pattern[2..];
        let pattern_re = fnmatch_to_regex(pattern);
        let bare_re = fnmatch_to_regex(bare);
        let combined = format!("(?:{}|{})", pattern_re, bare_re);
        Regex::new(&combined).expect("invalid fnmatch pattern")
    } else if pattern.contains('*') || pattern.contains('?') {
        Regex::new(&fnmatch_to_regex(pattern)).expect("invalid fnmatch pattern")
    } else {
        // Bare domain — match exact and subdomains
        let escaped = regex::escape(pattern);
        let combined = format!("^(?:{}$|.*\\.{}$)", escaped, escaped);
        Regex::new(&combined).expect("invalid fnmatch pattern")
    }
}

/// Check if `host` matches a domain pattern.
fn pattern_matches_domain(host: &str, compiled: &Regex, _pattern_str: &str) -> bool {
    compiled.is_match(host)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_policy_not_blocked() {
        let policy = WebsitePolicy::from_list(vec![]);
        assert!(!policy.is_blocked("https://example.com"));
    }

    #[test]
    fn test_exact_domain_match() {
        let mut policy = WebsitePolicy::new();
        policy.add_pattern("evil.com");
        assert!(policy.is_blocked("https://evil.com"));
    }

    #[test]
    fn test_subdomain_blocked_by_bare_pattern() {
        let policy = WebsitePolicy::from_list(vec!["evil.com".to_string()]);
        assert!(policy.is_blocked("https://sub.evil.com"));
    }

    #[test]
    fn test_wildcard_domain_match() {
        let policy = WebsitePolicy::from_list(vec!["*.evil.com".to_string()]);
        assert!(policy.is_blocked("https://sub.evil.com"));
    }

    #[test]
    fn test_wildcard_domain_matches_bare() {
        // *.evil.com should also match evil.com itself
        let policy = WebsitePolicy::from_list(vec!["*.evil.com".to_string()]);
        assert!(policy.is_blocked("https://evil.com"));
    }

    #[test]
    fn test_non_blocked_url() {
        let policy = WebsitePolicy::from_list(vec!["evil.com".to_string()]);
        assert!(!policy.is_blocked("https://good.com"));
    }

    #[test]
    fn test_add_pattern_invalidates_cache() {
        let mut policy = WebsitePolicy::new();
        // By default, example.com is blocked (from DEFAULT_BLOCKLIST)
        assert!(policy.is_blocked("https://example.com"));

        // Add a new pattern
        policy.add_pattern("malware.test");
        assert!(policy.is_blocked("https://malware.test"));
    }

    #[test]
    fn test_from_list_no_defaults() {
        let policy = WebsitePolicy::from_list(vec!["only.com".to_string()]);
        assert!(policy.is_blocked("https://only.com"));
        // Should NOT contain defaults
        assert!(!policy.is_blocked("https://example.com"));
    }

    #[test]
    fn test_path_pattern() {
        let policy = WebsitePolicy::from_list(vec!["example.com/secret/*".to_string()]);
        assert!(policy.is_blocked("https://example.com/secret/data"));
        assert!(!policy.is_blocked("https://example.com/public"));
    }

    #[test]
    fn test_extract_host_from_url() {
        assert_eq!(
            extract_host("https://www.example.com/path"),
            "www.example.com"
        );
        assert_eq!(extract_host("http://evil.com:8080"), "evil.com");
    }

    #[test]
    fn test_extract_host_empty() {
        assert_eq!(extract_host(""), "");
        assert_eq!(extract_host("   "), "");
    }

    #[test]
    fn test_patterns_list() {
        let policy = WebsitePolicy::from_list(vec!["a.com".to_string(), "b.com".to_string()]);
        assert_eq!(policy.patterns().len(), 2);
    }

    #[test]
    fn test_default_blocklist_loaded() {
        let policy = WebsitePolicy::new();
        assert!(policy.patterns().len() >= DEFAULT_BLOCKLIST.len());
    }

    #[test]
    fn test_question_mark_wildcard() {
        let policy = WebsitePolicy::from_list(vec!["???.com".to_string()]);
        assert!(policy.is_blocked("https://abc.com"));
        assert!(!policy.is_blocked("https://abcd.com"));
    }

    #[test]
    fn test_cache_ttl() {
        let policy = WebsitePolicy::from_list(vec!["blocked.com".to_string()]);
        // First call compiles and caches
        assert!(policy.is_blocked("https://blocked.com"));
        // Check that cache is now populated
        let cache = policy.cache.borrow();
        assert!(cache.0.is_some());
        assert!(cache.1.elapsed().as_secs() < CACHE_TTL_SECS);
    }
}

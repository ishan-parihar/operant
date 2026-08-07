mod ddg;
mod exa;
mod igs;
mod searxng;
mod tavily;

pub use ddg::DDGProvider;
pub use exa::ExaProvider;
pub use igs::IgsSearchProvider;
pub use searxng::SearXNGProvider;
pub use tavily::TavilyProvider;

use crate::error::Result;
use async_trait::async_trait;
use serde::Serialize;

/// Percent-encode a search query for insertion into a URL query string.
///
/// Uses the `url` crate's form-urlencoded set (spaces → `+`, reserved chars
/// like `&` `?` `#` `=` percent-encoded). The previous naive
/// `split(' ').join("+")` left `&`/`?`/`#`/CJK unencoded, which could inject
/// extra query params or produce a truncated/mangled search URL.
pub(crate) fn urlencode(query: &str) -> String {
    url::form_urlencoded::byte_serialize(query.as_bytes()).collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[async_trait]
pub trait WebSearchProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencode_spaces_become_plus() {
        assert_eq!(urlencode("hello world"), "hello+world");
    }

    #[test]
    fn test_urlencode_reserved_chars_percent_encoded() {
        // Naive space-join encoding left these unencoded, which could inject
        // extra query params or truncate the search URL.
        assert_eq!(urlencode("C++ & Rust"), "C%2B%2B+%26+Rust");
        assert_eq!(urlencode("a?b#c"), "a%3Fb%23c");
        assert_eq!(urlencode("x=y"), "x%3Dy");
    }

    #[test]
    fn test_urlencode_cjk() {
        // Non-ASCII must be percent-encoded (bytes), not passed through raw.
        assert_eq!(urlencode("日本"), "%E6%97%A5%E6%9C%AC");
    }

    #[test]
    fn test_web_search_result_serialization() {
        let result = WebSearchResult {
            title: "Test".to_string(),
            url: "https://example.com".to_string(),
            snippet: "A test result".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Test"));
        assert!(json.contains("example.com"));
    }
}

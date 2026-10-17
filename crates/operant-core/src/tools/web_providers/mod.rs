// (plan 11C1) sourcehound is the only web-search provider — the ddg/exa/
// searxng/tavily alternatives are deleted.
mod sourcehound;

pub use sourcehound::SourcehoundSearchProvider;

use crate::error::Result;
use async_trait::async_trait;
use serde::Serialize;

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

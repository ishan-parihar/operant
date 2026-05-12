mod ddg;
mod exa;
mod searxng;
mod tavily;

pub use ddg::DDGProvider;
pub use exa::ExaProvider;
pub use searxng::SearXNGProvider;
pub use tavily::TavilyProvider;

use async_trait::async_trait;
use serde::Serialize;
use crate::error::Result;

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

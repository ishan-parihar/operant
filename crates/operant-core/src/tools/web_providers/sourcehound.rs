use crate::error::Result;
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use async_trait::async_trait;

/// Web search provider backed by the `sourcehound` MCP server
/// (`sourcehound mcp` → `web.search`).
///
/// Requires the `sourcehound` binary on PATH (or `SOURCEHOUND_BINARY`).
/// sourcehound ships a key-free multi-engine search (DuckDuckGo, Wikipedia,
/// GitHub, HackerNews, StackOverflow) — no API key required. `WebSearchTool`
/// still falls back to the next provider in the chain when sourcehound is
/// unavailable or returns empty output.
pub struct SourcehoundSearchProvider;

#[async_trait]
impl WebSearchProvider for SourcehoundSearchProvider {
    fn name(&self) -> &str {
        "sourcehound"
    }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        crate::tools::sourcehound::web_search_sourcehound(query, num_results).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sourcehound_provider_name() {
        assert_eq!(SourcehoundSearchProvider.name(), "sourcehound");
    }
}

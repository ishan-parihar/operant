use crate::error::Result;
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use async_trait::async_trait;

/// Web search provider backed by the `igs` CLI (`igs web search --format json`).
///
/// Requires the `igs` binary on PATH (or `tools.igs_binary_path`). Search
/// routes through igs's upstream (Tavily/Firecrawl) — when no upstream key
/// is configured igs returns zero results, so `WebSearchTool` falls back to
/// DuckDuckGo on empty output.
pub struct IgsSearchProvider;

#[async_trait]
impl WebSearchProvider for IgsSearchProvider {
    fn name(&self) -> &str {
        "igs"
    }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        crate::tools::igs::web_search_igs(query, num_results).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_igs_provider_name() {
        assert_eq!(IgsSearchProvider.name(), "igs");
    }
}

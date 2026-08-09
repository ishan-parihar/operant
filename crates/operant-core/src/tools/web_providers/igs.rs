use crate::error::Result;
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use async_trait::async_trait;

/// Web search provider backed by the `igs` CLI (`igs web search --format json`).
///
/// Requires the `igs` binary on PATH (or `tools.igs_binary_path`). IGS >= 1.0
/// ships a key-free multi-engine search (DDG, Wikipedia, GitHub, HackerNews,
/// StackOverflow, YouTube) — no API key required. `WebSearchTool` still falls
/// back to DuckDuckGo when IGS is unavailable or returns empty output.
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

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

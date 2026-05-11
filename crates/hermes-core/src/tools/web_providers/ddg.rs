use async_trait::async_trait;
use crate::error::Result;
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use crate::tools::web_tools::parse_ddg_lite_results;

pub struct DDGProvider {
    search_url: String,
    user_agent: String,
}

impl DDGProvider {
    pub fn new(search_url: String, user_agent: String) -> Self {
        Self { search_url, user_agent }
    }
}

#[async_trait]
impl WebSearchProvider for DDGProvider {
    fn name(&self) -> &str { "duckduckgo" }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        let encoded: String = query.split(' ').collect::<Vec<_>>().join("+");
        let url = self.search_url.replace("{query}", &encoded);
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .build()?;
        let resp = client.get(&url).send().await?;
        let html = resp.text().await?;
        let raw = parse_ddg_lite_results(&html, num_results);
        Ok(raw.into_iter().map(|v| WebSearchResult {
            title: v["title"].as_str().unwrap_or("").to_string(),
            url: v["url"].as_str().unwrap_or("").to_string(),
            snippet: v["snippet"].as_str().unwrap_or("").to_string(),
        }).collect())
    }
}

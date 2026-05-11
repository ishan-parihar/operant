use async_trait::async_trait;
use crate::error::{Error, Result};
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};

pub struct ExaProvider {
    api_key: String,
}

impl ExaProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl WebSearchProvider for ExaProvider {
    fn name(&self) -> &str { "exa" }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        let client = reqwest::Client::new();
        let resp = client.post("https://api.exa.ai/search")
            .header("x-api-key", &self.api_key)
            .json(&serde_json::json!({
                "query": query,
                "numResults": num_results.min(20),
            }))
            .send().await?;
        let data: serde_json::Value = resp.json().await?;
        let results = data["results"].as_array().ok_or_else(|| Error::ParseResponse("Exa: missing results".into()))?;
        Ok(results.iter().map(|r| WebSearchResult {
            title: r["title"].as_str().unwrap_or("").to_string(),
            url: r["url"].as_str().unwrap_or("").to_string(),
            snippet: r["text"].as_str().unwrap_or("").to_string(),
        }).collect())
    }
}

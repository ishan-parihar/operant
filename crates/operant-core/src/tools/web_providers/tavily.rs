use crate::error::{Error, Result};
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use async_trait::async_trait;

pub struct TavilyProvider {
    api_key: String,
}

impl TavilyProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl WebSearchProvider for TavilyProvider {
    fn name(&self) -> &str {
        "tavily"
    }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.tavily.com/search")
            .json(&serde_json::json!({
                "api_key": self.api_key,
                "query": query,
                "max_results": num_results.min(20),
            }))
            .send()
            .await?;
        let data: serde_json::Value = resp.json().await?;
        let results = data["results"]
            .as_array()
            .ok_or_else(|| Error::ParseResponse("Tavily: missing results".into()))?;
        Ok(results
            .iter()
            .map(|r| WebSearchResult {
                title: r["title"].as_str().unwrap_or("").to_string(),
                url: r["url"].as_str().unwrap_or("").to_string(),
                snippet: r["content"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tavily_provider_name() {
        let provider = TavilyProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "tavily");
    }
}

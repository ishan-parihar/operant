use crate::error::{Error, Result};
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use async_trait::async_trait;

pub struct SearXNGProvider {
    base_url: String,
}

impl SearXNGProvider {
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }
}

#[async_trait]
impl WebSearchProvider for SearXNGProvider {
    fn name(&self) -> &str {
        "searxng"
    }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        let url = format!(
            "{}/search?format=json&q={}",
            self.base_url,
            urlencoding(query)
        );
        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await?;
        let data: serde_json::Value = resp.json().await?;
        let results = data["results"]
            .as_array()
            .ok_or_else(|| Error::ParseResponse("SearXNG: missing results".into()))?;
        Ok(results
            .iter()
            .take(num_results)
            .map(|r| WebSearchResult {
                title: r["title"].as_str().unwrap_or("").to_string(),
                url: r["url"].as_str().unwrap_or("").to_string(),
                snippet: r["content"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }
}

fn urlencoding(s: &str) -> String {
    s.split(' ').collect::<Vec<_>>().join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_searxng_provider_name() {
        let provider = SearXNGProvider::new("https://searxng.example.com".to_string());
        assert_eq!(provider.name(), "searxng");
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("test"), "test");
    }
}

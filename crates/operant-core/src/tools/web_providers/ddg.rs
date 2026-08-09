use crate::error::Result;
use crate::tools::web_providers::{WebSearchProvider, WebSearchResult};
use crate::tools::web_tools::parse_ddg_lite_results;
use async_trait::async_trait;

pub struct DDGProvider {
    search_url: String,
    user_agent: String,
}

impl DDGProvider {
    pub fn new(search_url: String, user_agent: String) -> Self {
        Self {
            search_url,
            user_agent,
        }
    }
}

#[async_trait]
impl WebSearchProvider for DDGProvider {
    fn name(&self) -> &str {
        "duckduckgo"
    }

    async fn search(&self, query: &str, num_results: usize) -> Result<Vec<WebSearchResult>> {
        // Percent-encode the query so `&`/`?`/`#`/CJK in the search term
        // cannot inject extra params or mangle the URL.
        let encoded = super::urlencode(query);
        let url = self.search_url.replace("{query}", &encoded);
        // HTTP/1.1 only: DDG's anomaly heuristics are friendlier to plain
        // HTTP/1.1 clients (the tool's lite.duckduckgo.com endpoint serves
        // result pages to them while anomalying HTTP/2 fingerprints).
        let client = reqwest::Client::builder()
            .user_agent(&self.user_agent)
            .http1_only()
            .build()?;

        // DDG rate-limits / anomaly-blocks by client fingerprint and IP, and
        // the block is transient — one retry after a short backoff catches
        // the good window instead of surfacing an empty page as "0 results".
        let mut raw: Vec<serde_json::Value> = Vec::new();
        for attempt in 0..2 {
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(crate::error::Error::Provider {
                    status: resp.status().as_u16(),
                    body: format!("DuckDuckGo search failed (HTTP {})", resp.status()),
                    retry_after: None,
                });
            }
            let html = resp.text().await?;
            raw = parse_ddg_lite_results(&html, num_results);
            if !raw.is_empty() || attempt == 1 {
                break;
            }
            // Empty/anomaly page — brief backoff, then retry once.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        Ok(raw
            .into_iter()
            .map(|v| WebSearchResult {
                title: v["title"].as_str().unwrap_or("").to_string(),
                url: v["url"].as_str().unwrap_or("").to_string(),
                snippet: v["snippet"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ddg_provider_name() {
        let provider =
            DDGProvider::new("https://example.com".to_string(), "test-agent".to_string());
        assert_eq!(provider.name(), "duckduckgo");
    }
}

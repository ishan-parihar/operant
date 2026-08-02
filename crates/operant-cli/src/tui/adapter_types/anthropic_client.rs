// adapter_types/anthropic_client.rs — Anthropic API client.

pub struct AnthropicClient {
    api_key: Option<String>,
    base_url: Option<String>,
}

impl AnthropicClient {
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        Self { api_key, base_url }
    }

    /// Fetch available models from the Anthropic API.
    ///
    /// Anthropic added a `/v1/models` endpoint in late 2024. It requires the
    /// `x-api-key` and `anthropic-version` headers (NOT `Authorization: Bearer`,
    /// which is the OpenAI-compat pattern). Returns the live list on success;
    /// on any error (network, auth, parse) falls back to a curated 5-model list
    /// so the picker is never empty.
    pub async fn fetch_available_models(&self) -> Vec<String> {
        // Curated fallback — kept up to date with the latest Claude lineup as of
        // 2026-07. Used only if the API call fails (no key, no network, 4xx).
        let fallback = vec![
            "claude-opus-4-6".to_string(),
            "claude-sonnet-4-6".to_string(),
            "claude-sonnet-4-5-20250929".to_string(),
            "claude-3-7-sonnet-20250219".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ];

        let Some(api_key) = &self.api_key else {
            return fallback;
        };

        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");
        let base = base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        let url = format!("{}/v1/models", base);

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
        {
            Ok(c) => c,
            Err(_) => return fallback,
        };

        let resp = client
            .get(&url)
            .header("x-api-key", api_key)
            // anthropic-version is mandatory; pinned to the latest stable date.
            .header("anthropic-version", "2023-06-01")
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(_) => return fallback,
        };

        let status = resp.status();
        if !status.is_success() {
            return fallback;
        }

        let json = match resp.json::<serde_json::Value>().await {
            Ok(j) => j,
            Err(_) => return fallback,
        };

        // Anthropic's response shape: {"data":[{"id":"claude-...","type":"model",...}, ...], "has_more": bool, "first_id": ..., "last_id": ...}
        // Note: Anthropic paginates (limit/after params) but the default first page
        // covers all current production models — pagination is left as a future
        // enhancement if the catalog grows past the page limit.
        let Some(data) = json.get("data").and_then(|d| d.as_array()) else {
            return fallback;
        };

        let mut ids: Vec<String> = data
            .iter()
            .filter_map(|item| item.get("id")?.as_str().map(String::from))
            .collect();

        if ids.is_empty() {
            return fallback;
        }

        // Sort newest-first by created_at if present, otherwise keep API order.
        ids.sort_by(|a, b| {
            let ta = data
                .iter()
                .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(a))
                .and_then(|item| item.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tb = data
                .iter()
                .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(b))
                .and_then(|item| item.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            tb.cmp(ta)
        });

        ids
    }
}

pub async fn fetch_openai_compatible_models_async(api_key: &str, base_url: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    let url = format!("{}/v1/models", base);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();

    let Ok(client) = client else {
        return vec![];
    };

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await;

    let Ok(resp) = response else {
        return vec![];
    };

    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return vec![];
    };

    // Parse OpenAI-format response: {"data": [{"id": "model-name", ...}, ...]}
    let Some(data) = json.get("data").and_then(|d| d.as_array()) else {
        return vec![];
    };

    data.iter()
        .filter_map(|item| item.get("id")?.as_str().map(String::from))
        .collect()
}

// (iter-136: LoadedPlugin struct deleted — single name field, zero callers)


// ---------- AuthStore ----------

#[derive(Debug, Clone)]
pub struct AuthStore {
    pub credentials: std::collections::HashMap<String, StoredCredential>,
}

#[derive(Debug, Clone)]
pub enum StoredCredential {
    ApiKey {
        key: String,
    },
    OAuthToken {
        access: String,
        refresh: String,
        expires: u64,
    },
}

impl AuthStore {
    pub fn load() -> Self {
        let mut credentials = std::collections::HashMap::new();

        // Load from environment variables
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                credentials.insert("anthropic".to_string(), StoredCredential::ApiKey { key });
            }
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                credentials.insert("openai".to_string(), StoredCredential::ApiKey { key });
            }
        }

        // Load from persisted auth file (simple format: {"provider": "key", ...})
        if let Ok(auth_data) = std::fs::read_to_string(Self::auth_path()) {
            if let Ok(saved) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(&auth_data)
            {
                for (provider, key) in saved {
                    if !key.is_empty() {
                        credentials
                            .entry(provider)
                            .or_insert(StoredCredential::ApiKey { key });
                    }
                }
            }
        }

        Self { credentials }
    }

    fn auth_path() -> std::path::PathBuf {
        let dir = Settings::config_dir();
        dir.join("auth.json")
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Settings::config_dir();
        std::fs::create_dir_all(&dir)?;
        let mut map = std::collections::HashMap::new();
        for (provider, cred) in &self.credentials {
            match cred {
                StoredCredential::ApiKey { key } => {
                    map.insert(provider.clone(), key.clone());
                }
                StoredCredential::OAuthToken { access, .. } => {
                    map.insert(provider.clone(), access.clone());
                }
            }
        }
        let json = serde_json::to_string_pretty(&map)?;
        std::fs::write(Self::auth_path(), json)?;
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: StoredCredential) {
        self.credentials.insert(key.to_string(), value);
        let _ = self.save();
    }
    pub fn api_key_for(&self, provider: impl Into<String>) -> Option<String> {
        let provider = provider.into();
        match self.credentials.get(&provider)? {
            StoredCredential::ApiKey { key } => Some(key.clone()),
            _ => None,
        }
    }
    pub fn has_any_key(&self) -> bool {
        self.credentials
            .values()
            .any(|c| matches!(c, StoredCredential::ApiKey { key } if !key.is_empty()))
    }
}

pub use import_config::{
    ImportPaths, ImportSelection, build_import_preview, execute_import, summarize_import_result,
};

// (iter-223: pub mod file_injection { AtFileRef, AtFileIssue, parse_at_refs }
// deleted — zero callers anywhere; the @-file parsing path they supported
// was never wired to a consumer.)

#[derive(Debug, Clone)]
pub struct FreeUpstream {
    pub id: &'static str,
    pub title: &'static str,
    pub default_model: &'static str,
    pub note: &'static str,
    pub key_url: &'static str,
}

pub const FREE_CATALOG: &[FreeUpstream] = &[
    FreeUpstream {
        id: "groq",
        title: "Groq",
        default_model: "llama-3.3-70b-versatile",
        note: "Blazing fast inference",
        key_url: "console.groq.com",
    },
    FreeUpstream {
        id: "cerebras",
        title: "Cerebras",
        default_model: "llama-3.3-70b",
        note: "Ultra-fast wafer-scale",
        key_url: "cloud.cerebras.ai",
    },
    FreeUpstream {
        id: "google",
        title: "Google Gemini",
        default_model: "gemini-2.0-flash",
        note: "Multimodal, generous free tier",
        key_url: "aistudio.google.com",
    },
    FreeUpstream {
        id: "mistral",
        title: "Mistral",
        default_model: "mistral-small-latest",
        note: "Strong coding models",
        key_url: "console.mistral.ai",
    },
    FreeUpstream {
        id: "sambanova",
        title: "SambaNova",
        default_model: "Meta-Llama-3.3-70B-Instruct",
        note: "Fast inference, free tier",
        key_url: "cloud.sambanova.ai",
    },
];

fn reverse_provider_lookup(dev_provider: &str) -> String {
    for provider in crate::provider::PROVIDERS {
        if let Some(mapped) = operant_core::models_dev::provider_to_models_dev(provider.name) {
            if mapped == dev_provider {
                return provider.name.to_string();
            }
        }
    }
    dev_provider.to_string()
}

// ---------- ModelRegistry ----------

#[derive(Clone)]
pub struct ModelRegistry {
    models: std::collections::HashMap<String, Vec<crate::tui::model_picker::ModelEntry>>,
}

#[derive(Debug, Clone)]
pub struct RegistryModelEntry {
    pub info: ModelInfo,
}

#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    pub context_window: u32,
}

impl ModelRegistry {
    pub fn new() -> Self {
        let mut models = std::collections::HashMap::new();
        Self::populate_default_models(&mut models);
        Self { models }
    }

    pub fn load_cache(&mut self, _path: &std::path::Path) {}

    /// Add any missing providers from PROVIDERS without overwriting existing entries.
    #[allow(dead_code)] // Provider defaults initialization
    pub fn ensure_provider_defaults(&mut self) {
        for provider in crate::provider::PROVIDERS {
            if !self.models.contains_key(provider.name) {
                let entries: Vec<crate::tui::model_picker::ModelEntry> = provider
                    .models
                    .iter()
                    .map(|model_id| crate::tui::model_picker::ModelEntry {
                        id: model_id.to_string(),
                        display_name: model_id.to_string(),
                        description: provider.display_name.to_string(),
                        is_current: false,
                    })
                    .collect();
                if !entries.is_empty() {
                    self.models.insert(provider.name.to_string(), entries);
                }
            }
        }
    }

    /// Fetch models from models.dev catalog and merge into the registry.
    /// Uses provider_to_models_dev() mapping to match operant providers to catalog entries.
    pub async fn load_models_dev(&mut self) {
        let (models, _) = match operant_core::models_dev::fetch_models_dev(false).await {
            Ok(r) => r,
            Err(_) => return,
        };

        for model in &models {
            let m_provider = match model.get("provider_id").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => continue,
            };
            let model_id = match model.get("id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => continue,
            };

            let operant_provider = operant_core::models_dev::provider_to_models_dev(
                &reverse_provider_lookup(m_provider),
            )
            .map(|_| reverse_provider_lookup(m_provider))
            .or_else(|| {
                if crate::provider::PROVIDERS
                    .iter()
                    .any(|p| p.name == m_provider)
                {
                    Some(m_provider.to_string())
                } else {
                    None
                }
            });

            let provider_name = match operant_provider {
                Some(p) => p,
                None => continue,
            };

            let context_window = model.get("context_window").and_then(|v| v.as_u64());
            let cost_input = model.get("cost_input").and_then(|v| v.as_f64());
            let cost_output = model.get("cost_output").and_then(|v| v.as_f64());

            let description = match context_window {
                Some(ctx) => {
                    let ctx_str = if ctx >= 1_000_000 {
                        format!("{}M ctx", ctx / 1_000_000)
                    } else {
                        format!("{}K ctx", ctx / 1000)
                    };
                    let cost_str = match (cost_input, cost_output) {
                        (Some(i), Some(o)) => format!("${:.2}/${:.2} per M", i, o),
                        _ => String::new(),
                    };
                    if cost_str.is_empty() {
                        ctx_str
                    } else {
                        format!("{} | {}", ctx_str, cost_str)
                    }
                }
                None => String::new(),
            };

            let entry = crate::tui::model_picker::ModelEntry {
                id: model_id.to_string(),
                display_name: model_id.to_string(),
                description,
                is_current: false,
            };

            let entries = self.models.entry(provider_name).or_default();
            if !entries.iter().any(|e| e.id == model_id) {
                entries.push(entry);
            }
        }
    }

    /// Fetch models from a provider's /v1/models endpoint and merge them into the registry.
    ///
    /// Routes Anthropic through `AnthropicClient::fetch_available_models` (which
    /// uses `x-api-key` + `anthropic-version` headers — the OpenAI-compat
    /// `Authorization: Bearer` pattern does NOT work for Anthropic). All other
    /// providers go through the OpenAI-compat path.
    pub async fn fetch_from_provider_async(
        &mut self,
        provider_id: &str,
        api_key: &str,
        base_url: &str,
    ) {
        let fetched = if provider_id == "anthropic" {
            let client =
                AnthropicClient::new(Some(api_key.to_string()), Some(base_url.to_string()));
            client.fetch_available_models().await
        } else {
            fetch_openai_compatible_models_async(api_key, base_url).await
        };
        if fetched.is_empty() {
            return;
        }

        // De-dup against any cached/catalog entries already present for this provider
        // (models.dev, populate_default_models, prior fetches). Match by id.
        let models = self.models.entry(provider_id.to_string()).or_default();
        let existing: std::collections::HashSet<String> =
            models.iter().map(|m| m.id.clone()).collect();
        for model_id in fetched {
            if existing.contains(&model_id) {
                continue;
            }
            models.push(crate::tui::model_picker::ModelEntry {
                id: model_id.clone(),
                display_name: model_id,
                description: String::new(),
                is_current: false,
            });
        }
    }
    pub fn get(&self, provider: &str, model_id: &str) -> Option<RegistryModelEntry> {
        self.list_by_provider(provider)
            .into_iter()
            .find(|m| m.id == model_id)
            .map(|_| RegistryModelEntry {
                info: ModelInfo::default(),
            })
    }

    pub fn list_visible_by_provider(
        &self,
        provider: &str,
    ) -> Vec<crate::tui::model_picker::ModelEntry> {
        self.list_by_provider(provider)
    }

    pub fn list_by_provider(&self, provider: &str) -> Vec<crate::tui::model_picker::ModelEntry> {
        self.models.get(provider).cloned().unwrap_or_default()
    }

    pub fn best_model_for_provider(&self, provider: &str) -> Option<String> {
        self.list_by_provider(provider)
            .first()
            .map(|m| m.id.clone())
    }

    fn populate_default_models(
        models: &mut std::collections::HashMap<String, Vec<crate::tui::model_picker::ModelEntry>>,
    ) {
        for provider in crate::provider::PROVIDERS {
            let entries: Vec<crate::tui::model_picker::ModelEntry> = provider
                .models
                .iter()
                .map(|model_id| crate::tui::model_picker::ModelEntry {
                    id: model_id.to_string(),
                    display_name: model_id.to_string(),
                    description: provider.display_name.to_string(),
                    is_current: false,
                })
                .collect();
            if !entries.is_empty() {
                models.insert(provider.name.to_string(), entries);
            }
        }
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderId {
    OpencodeGo,
    OpencodeZen,
    Other(String),
}

impl From<ProviderId> for String {
    fn from(pid: ProviderId) -> String {
        match pid {
            ProviderId::OpencodeGo => "opencode-go".to_string(),
            ProviderId::OpencodeZen => "opencode-zen".to_string(),
            ProviderId::Other(s) => s,
        }
    }
}

impl<'a> From<&'a ProviderId> for String {
    fn from(pid: &'a ProviderId) -> String {
        match pid {
            ProviderId::OpencodeGo => "opencode-go".to_string(),
            ProviderId::OpencodeZen => "opencode-zen".to_string(),
            ProviderId::Other(s) => s.clone(),
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderId::OpencodeGo => write!(f, "opencode-go"),
            ProviderId::OpencodeZen => write!(f, "opencode-zen"),
            ProviderId::Other(s) => write!(f, "{}", s),
        }
    }
}

impl std::str::FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencode-go" => Ok(ProviderId::OpencodeGo),
            "opencode-zen" => Ok(ProviderId::OpencodeZen),
            other => Ok(ProviderId::Other(other.to_string())),
        }
    }
}

// (iter-208: pub mod mcp { ... } deleted — stub McpManager that returned
// empty data for /mcp overlay. load_mcp_servers now reads from
// App.core_mcp_manager (the real operant_core::mcp::McpManager).
// McpServerStatus/McpCatalogEntry/McpToolDef were never used outside
// the stub itself.)
//
// (iter-155: pub mod streaming {} deleted — was empty, only a deletion marker.)


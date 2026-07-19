//! Pluggable browser provider system for Operant-RS.
//!
//! Mirrors operant-agent's `CloudBrowserProvider` ABC and `_PROVIDER_REGISTRY`.
//! Selected via `config.browser.provider`:
//!
//! | Value           | Backend                                         |
//! |-----------------|-------------------------------------------------|
//! | `"lightpanda"`  | Local Lightpanda binary (auto-downloaded)       |
//! | `"camofox"`     | Camofox REST API (`CAMOFOX_URL`)                |
//! | `"browserbase"` | Browserbase cloud (`BROWSERBASE_API_KEY`)       |
//! | `"browser-use"` | Browser Use cloud (`BROWSER_USE_API_KEY`)       |
//! | `"firecrawl"`   | Firecrawl scrape API (`FIRECRAWL_API_KEY`)      |

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait BrowserProvider: Send + Sync {
    fn name(&self) -> &str;
    /// True when required credentials / binary are present.
    fn is_configured(&self) -> bool;
    /// Navigate to URL; return page text/snapshot.
    async fn navigate(&self, url: &str) -> Result<String>;
    /// Take a text snapshot of the current page.
    async fn snapshot(&self) -> Result<String>;
    /// Click element identified by selector.
    async fn click(&self, selector: &str) -> Result<String>;
    /// Type text into selector.
    async fn type_text(&self, selector: &str, text: &str) -> Result<String>;
    /// Scroll the page.
    async fn scroll(&self, direction: &str) -> Result<String>;
    /// Generic command dispatch (for providers that speak their own protocol).
    async fn execute(&self, command: &str, args: Value) -> Result<String> {
        match command {
            "navigate" => self.navigate(args["url"].as_str().unwrap_or("")).await,
            "snapshot" => self.snapshot().await,
            "click" => self.click(args["selector"].as_str().unwrap_or("")).await,
            "type" => {
                self.type_text(
                    args["selector"].as_str().unwrap_or(""),
                    args["text"].as_str().unwrap_or(""),
                )
                .await
            }
            "scroll" => {
                self.scroll(args["direction"].as_str().unwrap_or("down"))
                    .await
            }
            _ => Err(Error::Agent(format!(
                "Unknown browser command: {}",
                command
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Lightpanda — local binary, auto-downloaded from GitHub Releases
// ---------------------------------------------------------------------------

pub struct LightpandaProvider;

impl LightpandaProvider {
    async fn ensure_binary(&self) -> Result<std::path::PathBuf> {
        let bin_path = crate::tools::browser_downloader::BrowserDownloader::default_bin_path();
        if bin_path.exists() {
            return Ok(bin_path);
        }
        crate::tools::browser_downloader::BrowserDownloader::download_binary().await
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        let bin = self.ensure_binary().await?;
        let out = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new(&bin).args(args).output(),
        )
        .await
        .map_err(|_| Error::Agent("browser timeout".into()))??;

        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Check for permission denied errors
            if stderr.contains("Permission denied") || stderr.contains("os error 13") {
                Err(Error::Agent(
                    "Lightpanda binary execution failed: Permission denied. \
                     Ensure ~/.operant/bin/browser is executable (chmod +x) or \
                     try setting BROWSER_PROVIDER=camofox in config.toml (requires Docker). \
                     See https://github.com/lightpanda-io/browser/releases for manual installation."
                        .into(),
                ))
            } else {
                Err(Error::Agent(format!("browser error: {}", stderr)))
            }
        }
    }
}

#[async_trait]
impl BrowserProvider for LightpandaProvider {
    fn name(&self) -> &str {
        "lightpanda"
    }
    fn is_configured(&self) -> bool {
        true
    }
    async fn navigate(&self, url: &str) -> Result<String> {
        self.run(&["fetch", "--dump", "markdown", url]).await
    }
    async fn snapshot(&self) -> Result<String> {
        Err(Error::Agent(
            "Lightpanda fetch mode: use navigate(url) to get page content".into(),
        ))
    }
    async fn click(&self, _selector: &str) -> Result<String> {
        Err(Error::Agent(
            "Lightpanda fetch mode does not support click interactions".into(),
        ))
    }
    async fn type_text(&self, _selector: &str, _text: &str) -> Result<String> {
        Err(Error::Agent(
            "Lightpanda fetch mode does not support type interactions".into(),
        ))
    }
    async fn scroll(&self, _direction: &str) -> Result<String> {
        Err(Error::Agent(
            "Lightpanda fetch mode does not support scroll".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Camofox — local anti-detection browser via REST API
// ---------------------------------------------------------------------------

pub struct CamofoxProvider {
    base_url: String,
    client: reqwest::Client,
}

impl Default for CamofoxProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CamofoxProvider {
    pub fn new() -> Self {
        Self {
            base_url: std::env::var("CAMOFOX_URL")
                .unwrap_or_else(|_| "http://localhost:9222".to_string()),
            client: reqwest::Client::new(),
        }
    }

    async fn post(&self, path: &str, body: Value) -> Result<String> {
        let resp = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(resp)
    }
}

#[async_trait]
impl BrowserProvider for CamofoxProvider {
    fn name(&self) -> &str {
        "camofox"
    }
    fn is_configured(&self) -> bool {
        std::env::var("CAMOFOX_URL").is_ok()
    }
    async fn navigate(&self, url: &str) -> Result<String> {
        self.post("/navigate", serde_json::json!({"url": url}))
            .await
    }
    async fn snapshot(&self) -> Result<String> {
        self.post("/snapshot", serde_json::json!({})).await
    }
    async fn click(&self, selector: &str) -> Result<String> {
        self.post("/click", serde_json::json!({"selector": selector}))
            .await
    }
    async fn type_text(&self, selector: &str, text: &str) -> Result<String> {
        self.post(
            "/type",
            serde_json::json!({"selector": selector, "text": text}),
        )
        .await
    }
    async fn scroll(&self, direction: &str) -> Result<String> {
        self.post("/scroll", serde_json::json!({"direction": direction}))
            .await
    }
}

// ---------------------------------------------------------------------------
// Browserbase — cloud browser sessions
// ---------------------------------------------------------------------------

pub struct BrowserbaseProvider {
    api_key: String,
    project_id: String,
    client: reqwest::Client,
}

impl Default for BrowserbaseProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserbaseProvider {
    pub fn new() -> Self {
        Self {
            api_key: std::env::var("BROWSERBASE_API_KEY").unwrap_or_default(),
            project_id: std::env::var("BROWSERBASE_PROJECT_ID").unwrap_or_default(),
            client: reqwest::Client::new(),
        }
    }

    async fn run_task(&self, task: &str, url: Option<&str>) -> Result<String> {
        let mut body = serde_json::json!({"task": task, "projectId": self.project_id});
        if let Some(u) = url {
            body["url"] = u.into();
        }
        let resp = self
            .client
            .post("https://api.browserbase.com/v1/sessions")
            .header("X-BB-API-Key", &self.api_key)
            .json(&body)
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp.to_string())
    }
}

#[async_trait]
impl BrowserProvider for BrowserbaseProvider {
    fn name(&self) -> &str {
        "browserbase"
    }
    fn is_configured(&self) -> bool {
        !self.api_key.is_empty() && !self.project_id.is_empty()
    }
    async fn navigate(&self, url: &str) -> Result<String> {
        self.run_task("navigate", Some(url)).await
    }
    async fn snapshot(&self) -> Result<String> {
        self.run_task("snapshot", None).await
    }
    async fn click(&self, selector: &str) -> Result<String> {
        self.run_task(&format!("click {}", selector), None).await
    }
    async fn type_text(&self, selector: &str, text: &str) -> Result<String> {
        self.run_task(&format!("type '{}' into {}", text, selector), None)
            .await
    }
    async fn scroll(&self, direction: &str) -> Result<String> {
        self.run_task(&format!("scroll {}", direction), None).await
    }
}

// ---------------------------------------------------------------------------
// Browser Use — cloud AI browser agent
// ---------------------------------------------------------------------------

pub struct BrowserUseProvider {
    api_key: String,
    client: reqwest::Client,
}

impl Default for BrowserUseProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserUseProvider {
    pub fn new() -> Self {
        Self {
            api_key: std::env::var("BROWSER_USE_API_KEY").unwrap_or_default(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl BrowserProvider for BrowserUseProvider {
    fn name(&self) -> &str {
        "browser-use"
    }
    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        let resp = self
            .client
            .post("https://api.browser-use.com/api/v1/run-sync")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({"task": format!("Navigate to {}", url)}))
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp["result"].as_str().unwrap_or("").to_string())
    }

    async fn snapshot(&self) -> Result<String> {
        Err(Error::Agent(
            "Browser Use does not support snapshots directly; use navigate".into(),
        ))
    }
    async fn click(&self, selector: &str) -> Result<String> {
        let resp = self
            .client
            .post("https://api.browser-use.com/api/v1/run-sync")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({"task": format!("Click on element: {}", selector)}))
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp["result"].as_str().unwrap_or("").to_string())
    }
    async fn type_text(&self, selector: &str, text: &str) -> Result<String> {
        let resp = self
            .client
            .post("https://api.browser-use.com/api/v1/run-sync")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({"task": format!("Type '{}' into {}", text, selector)}))
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp["result"].as_str().unwrap_or("").to_string())
    }
    async fn scroll(&self, direction: &str) -> Result<String> {
        let resp = self
            .client
            .post("https://api.browser-use.com/api/v1/run-sync")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({"task": format!("Scroll {}", direction)}))
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp["result"].as_str().unwrap_or("").to_string())
    }
}

// ---------------------------------------------------------------------------
// Firecrawl — scrape/crawl API (navigate = scrape URL)
// ---------------------------------------------------------------------------

pub struct FirecrawlProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl Default for FirecrawlProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FirecrawlProvider {
    pub fn new() -> Self {
        Self {
            api_key: std::env::var("FIRECRAWL_API_KEY").unwrap_or_default(),
            base_url: std::env::var("FIRECRAWL_BASE_URL")
                .unwrap_or_else(|_| "https://api.firecrawl.dev".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl BrowserProvider for FirecrawlProvider {
    fn name(&self) -> &str {
        "firecrawl"
    }
    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        let resp = self
            .client
            .post(format!("{}/v1/scrape", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({"url": url, "formats": ["markdown"]}))
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp["data"]["markdown"].as_str().unwrap_or("").to_string())
    }

    async fn snapshot(&self) -> Result<String> {
        Err(Error::Agent(
            "Firecrawl: call navigate(url) to scrape a page".into(),
        ))
    }
    async fn click(&self, _selector: &str) -> Result<String> {
        Err(Error::Agent(
            "Firecrawl does not support interactive commands".into(),
        ))
    }
    async fn type_text(&self, _selector: &str, _text: &str) -> Result<String> {
        Err(Error::Agent(
            "Firecrawl does not support interactive commands".into(),
        ))
    }
    async fn scroll(&self, _direction: &str) -> Result<String> {
        Err(Error::Agent(
            "Firecrawl does not support interactive commands".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub fn build_browser_provider(name: &str) -> std::sync::Arc<dyn BrowserProvider> {
    match name {
        "camofox" => std::sync::Arc::new(CamofoxProvider::new()),
        "browserbase" => std::sync::Arc::new(BrowserbaseProvider::new()),
        "browser-use" | "browser_use" => std::sync::Arc::new(BrowserUseProvider::new()),
        "firecrawl" => std::sync::Arc::new(FirecrawlProvider::new()),
        _ => std::sync::Arc::new(LightpandaProvider), // default: lightpanda
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_lightpanda() {
        let p = build_browser_provider("lightpanda");
        assert_eq!(p.name(), "lightpanda");
    }

    #[test]
    fn test_unknown_falls_back_to_lightpanda() {
        let p = build_browser_provider("unknown");
        assert_eq!(p.name(), "lightpanda");
    }

    #[test]
    fn test_firecrawl_requires_key() {
        let p = FirecrawlProvider::new();
        // Without env var, not configured
        if std::env::var("FIRECRAWL_API_KEY").is_err() {
            assert!(!p.is_configured());
        }
    }

    #[test]
    fn test_browserbase_requires_keys() {
        let p = BrowserbaseProvider::new();
        if std::env::var("BROWSERBASE_API_KEY").is_err() {
            assert!(!p.is_configured());
        }
    }
}

//! Pluggable browser provider system for Operant-RS.
//!
//! Mirrors operant-agent's `CloudBrowserProvider` ABC and `_PROVIDER_REGISTRY`.
//! Selected via `config.browser.provider`:
//!
//! | Value           | Backend                                         |
//! |-----------------|-------------------------------------------------|
//! | `"lightpanda"`  | Local Lightpanda binary (auto-downloaded)       |
//! | `"obscura"`     | Local Obscura binary (shared with IGS; CDP-driven,  |
//! |                 | stealth by default)                              |
//! | `"camofox"`     | Camofox REST API (`CAMOFOX_URL`)                |
//! | `"browserbase"` | Browserbase cloud (`BROWSERBASE_API_KEY`)       |
//! | `"browser-use"` | Browser Use cloud (`BROWSER_USE_API_KEY`)       |
//! | `"firecrawl"`   | Firecrawl scrape API (`FIRECRAWL_API_KEY`)      |

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use crate::config::runtime_config;
use crate::error::{Error, Result};
use dirs;
use reqwest;
use tokio::io::AsyncWriteExt;

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
// Obscura — local binary with CDP server, auto-downloaded from GitHub Releases
// ---------------------------------------------------------------------------

pub struct ObscuraProvider;

impl ObscuraProvider {
    /// Returns the default installation path for the Obscura binary
    /// (the operant-managed copy at `~/.operant/bin/obscura`).
    pub fn default_bin_path() -> std::path::PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".operant")
            .join("bin")
            .join("obscura")
    }

    /// The config directory the IGS integration uses for its own Obscura
    /// binary. Mirrors igs-rust's `config::user_config_dir()` precedence
    /// exactly: `$IGS_CONFIG_DIR` override, else `$XDG_CONFIG_HOME/igs-mcp`
    /// or `~/.config/igs-mcp`.
    fn igs_config_dir() -> std::path::PathBuf {
        if let Ok(dir) = std::env::var("IGS_CONFIG_DIR")
            && !dir.trim().is_empty()
        {
            return std::path::PathBuf::from(dir);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
        std::path::PathBuf::from(xdg).join("igs-mcp")
    }

    /// Resolve the Obscura binary to execute, in order:
    ///
    /// 1. `tools.obscura_binary_path` config override
    /// 2. The binary the IGS integration manages — `$IGS_CONFIG_DIR/bin/obscura`
    ///    or `~/.config/igs-mcp/bin/obscura`. igs-rust's `ObscuraManager`
    ///    (`src/obscura.rs`) hardcodes that location and auto-downloads from
    ///    the same `h4ckf0r0day/obscura` releases operant uses, so reusing it
    ///    keeps the `browser` tool and the IGS web tools (web_search,
    ///    web_scrape, web_extract, web.crawl) on the *exact same* binary —
    ///    one download, no version drift.
    /// 3. The operant-managed copy at `~/.operant/bin/obscura` (fallback for
    ///    machines that never installed IGS).
    ///
    /// Returns the first path that exists; `None` when no binary is installed
    /// (callers then fall back to [`Self::download_binary`], which installs to
    /// the operant-managed copy).
    pub fn resolve_obscura_binary() -> Option<std::path::PathBuf> {
        Self::resolve_obscura_binary_with(runtime_config().tools.obscura_binary_path.as_deref())
    }

    /// Testable core of [`Self::resolve_obscura_binary`]: resolution order is
    /// `config_override` → IGS-managed binary → operant-managed copy.
    fn resolve_obscura_binary_with(
        config_override: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        // 1. Explicit config override (must be a real file, not a directory).
        if let Some(path) = config_override {
            if path.is_file() {
                return Some(path.to_path_buf());
            }
            tracing::warn!(
                path = %path.display(),
                "configured tools.obscura_binary_path does not exist — falling back"
            );
        }
        // 2. IGS-managed binary (shared with the IGS integration).
        let igs_bin = Self::igs_config_dir().join("bin").join("obscura");
        if igs_bin.exists() {
            return Some(igs_bin);
        }
        // 3. Operant-managed copy.
        let own = Self::default_bin_path();
        own.exists().then_some(own)
    }

    /// Where a fresh download is installed. When the IGS config directory
    /// exists (IGS has run at least once), install into its `bin/` so a later
    /// `igs` run finds the same binary and skips its own download — the
    /// single-shared-binary guarantee even on first-run ordering. Otherwise
    /// fall back to the operant-managed copy.
    fn download_target() -> std::path::PathBuf {
        let igs_dir = Self::igs_config_dir();
        if igs_dir.exists() {
            igs_dir.join("bin").join("obscura")
        } else {
            Self::default_bin_path()
        }
    }

    /// Downloads the latest Obscura browser binary for the current platform.
    /// Checks GitHub Releases for the latest binary.
    async fn download_binary() -> Result<std::path::PathBuf> {
        let install_into_igs = Self::igs_config_dir().exists();
        let bin_path = Self::download_target();

        tracing::info!("Fetching latest Obscura browser binary from GitHub Releases…");

        let release_url = "https://api.github.com/repos/h4ckf0r0day/obscura/releases/latest";
        let client = reqwest::Client::builder()
            .user_agent("Operant-RS-Downloader")
            .build()?;

        let release: serde_json::Value = client.get(release_url).send().await?.json().await?;
        let version = release["tag_name"].as_str().unwrap_or("").to_string();

        let assets = release["assets"]
            .as_array()
            .ok_or_else(|| Error::Agent("No assets found in Obscura release".into()))?;

        let asset = Self::find_matching_asset(assets)?;
        tracing::info!(
            "Downloading asset: {}",
            asset["name"].as_str().unwrap_or("unknown")
        );

        let response = client
            .get(asset["browser_download_url"].as_str().unwrap_or(""))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::Agent(format!(
                "Failed to download binary: {}",
                response.status()
            )));
        }

        if let Some(parent) = bin_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::File::create(&bin_path).await?;
        let content = response.bytes().await?;
        file.write_all(&content).await?;
        file.flush().await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&bin_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&bin_path, perms).await?;
        }

        if Self::verify_binary(&bin_path).await.is_err() {
            return Err(Error::Agent(
                "Downloaded binary failed verification".to_string(),
            ));
        }

        // Stamp the version file igs-rust's ObscuraManager reads
        // (`bin/.obscura_version`). Its `ensure_ready()` compares that file to
        // the latest GitHub release and skips downloading when they match — so
        // a binary we installed into its `bin/` dir is reused, not replaced.
        if install_into_igs && !version.is_empty() {
            tokio::fs::write(
                bin_path.with_file_name(".obscura_version"),
                version.as_bytes(),
            )
            .await?;
        }

        tracing::info!(
            "Obscura binary successfully installed to {}",
            bin_path.display()
        );
        Ok(bin_path)
    }

    fn find_matching_asset(assets: &[serde_json::Value]) -> Result<serde_json::Value> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        tracing::debug!("Matching asset for OS: {}, Arch: {}", os, arch);

        // Obscura release naming: obscura-x86_64-linux.tar.gz (standard),
        // obscura-x86_64-linux-stealth.tar.gz (stealth), …
        let (os_pattern, arch_pattern) = match (os, arch) {
            ("linux", "x86_64") => ("linux", "x86_64"),
            ("linux", "aarch64") => ("linux", "aarch64"),
            ("macos", "x86_64") => ("macos", "x86_64"),
            ("macos", "aarch64") => ("macos", "aarch64"),
            ("windows", "x86_64") => ("windows", "x86_64"),
            _ => {
                return Err(Error::Agent(format!(
                    "Unsupported platform: {} on {}",
                    os, arch
                )));
            }
        };

        let matches_platform = |a: &serde_json::Value| {
            let name = a["name"].as_str().unwrap_or("");
            name.contains(os_pattern) && name.contains(arch_pattern) && name.ends_with(".tar.gz")
        };

        // Prefer the stealth build (anti-detection + TLS fingerprinting + tracker
        // blocking) — the default for the CDP browser. Fall back to the standard
        // build when the release only ships non-stealth assets.
        if let Some(stealth) = assets
            .iter()
            .find(|a| matches_platform(a) && a["name"].as_str().unwrap_or("").contains("-stealth"))
        {
            return Ok(stealth.clone());
        }
        if let Some(standard) = assets.iter().find(|a| matches_platform(a)) {
            tracing::warn!(
                "Stealth build unavailable for {}/{}, falling back to the standard build",
                os,
                arch
            );
            return Ok(standard.clone());
        }
        Err(Error::Agent(format!(
            "Could not find matching binary for {} on {}",
            arch, os
        )))
    }

    /// Verifies that the binary exists and can be executed
    pub async fn verify_binary(path: &std::path::Path) -> Result<()> {
        if !path.exists() {
            return Err(Error::Config(format!(
                "Binary not found at {}",
                path.display()
            )));
        }

        let output = tokio::process::Command::new(path)
            .arg("--version")
            .output()
            .await
            .map_err(|e| Error::Agent(format!("Failed to execute binary: {}", e)))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Agent("Binary execution failed".to_string()))
        }
    }

    /// Resolve the shared Obscura binary (reusing the IGS-managed copy when
    /// present), downloading to the operant-managed location only when neither
    /// IGS nor operant has one installed.
    pub async fn ensure_binary() -> Result<std::path::PathBuf> {
        if let Some(bin) = Self::resolve_obscura_binary() {
            return Ok(bin);
        }
        Self::download_binary().await
    }
}

#[async_trait]
impl BrowserProvider for ObscuraProvider {
    fn name(&self) -> &str {
        "obscura"
    }
    fn is_configured(&self) -> bool {
        true // Auto-provisions the shared (stealth) binary on first use
    }
    async fn navigate(&self, url: &str) -> Result<String> {
        let session = crate::obscura_cdp::get_or_start_shared_session().await?;
        session.navigate(url).await
    }
    async fn snapshot(&self) -> Result<String> {
        let session = crate::obscura_cdp::get_or_start_shared_session().await?;
        session.snapshot().await
    }
    async fn click(&self, selector: &str) -> Result<String> {
        let session = crate::obscura_cdp::get_or_start_shared_session().await?;
        session.click(selector).await
    }
    async fn type_text(&self, selector: &str, text: &str) -> Result<String> {
        let session = crate::obscura_cdp::get_or_start_shared_session().await?;
        session.fill(selector, text).await
    }
    async fn scroll(&self, direction: &str) -> Result<String> {
        let session = crate::obscura_cdp::get_or_start_shared_session().await?;
        session.scroll(direction).await
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
        "igs" => std::sync::Arc::new(crate::tools::igs::IgsBrowserProvider),
        "camofox" => std::sync::Arc::new(CamofoxProvider::new()),
        "browserbase" => std::sync::Arc::new(BrowserbaseProvider::new()),
        "browser-use" | "browser_use" => std::sync::Arc::new(BrowserUseProvider::new()),
        "firecrawl" => std::sync::Arc::new(FirecrawlProvider::new()),
        "obscura" => std::sync::Arc::new(ObscuraProvider),
        _ => std::sync::Arc::new(LightpandaProvider), // default: lightpanda
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "operant_obscura_test_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Restores an env var to its prior value on drop, so a panicking
    /// assertion can't leak `IGS_CONFIG_DIR` into other tests in the process.
    struct EnvGuard {
        name: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: test-only env mutation under exclusive lock; the guard
            // is always created while `env_lock()` is held.
            match &self.previous {
                Some(value) => unsafe { std::env::set_var(self.name, value) },
                None => unsafe { std::env::remove_var(self.name) },
            }
        }
    }

    fn set_env_igs_config_dir(dir: &std::path::Path) -> EnvGuard {
        let previous = std::env::var_os("IGS_CONFIG_DIR");
        // SAFETY: test-only env mutation under exclusive lock
        unsafe { std::env::set_var("IGS_CONFIG_DIR", dir) };
        EnvGuard {
            name: "IGS_CONFIG_DIR",
            previous,
        }
    }

    #[test]
    fn find_matching_asset_prefers_stealth_build() {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let name = |suffix: &str| format!("obscura-{arch}-{os}{suffix}.tar.gz");
        let assets = vec![
            serde_json::json!({ "name": name("") }),
            serde_json::json!({ "name": name("-stealth") }),
            serde_json::json!({ "name": name("-no-render") }),
        ];
        let chosen = ObscuraProvider::find_matching_asset(&assets).unwrap();
        assert_eq!(chosen["name"], name("-stealth"));
    }

    #[test]
    fn find_matching_asset_falls_back_to_standard_build() {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let name = |suffix: &str| format!("obscura-{arch}-{os}{suffix}.tar.gz");
        let assets = vec![
            serde_json::json!({ "name": name("") }),
            serde_json::json!({ "name": name("-no-render") }),
        ];
        let chosen = ObscuraProvider::find_matching_asset(&assets).unwrap();
        // No -stealth asset: standard rendering build wins over -no-render.
        assert_eq!(chosen["name"], name(""));
    }

    #[test]
    fn resolve_prefers_config_override() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("override");
        let bin = dir.join("custom-obscura");
        std::fs::write(&bin, b"").unwrap();
        let result = ObscuraProvider::resolve_obscura_binary_with(Some(&bin));
        assert_eq!(result, Some(bin));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_uses_igs_managed_binary_when_present() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("igs");
        let bin = dir.join("bin").join("obscura");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"").unwrap();
        let _env = set_env_igs_config_dir(&dir);
        let result = ObscuraProvider::resolve_obscura_binary_with(None);
        assert_eq!(result, Some(bin));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_ignores_missing_override_then_uses_igs() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("igs2");
        let bin = dir.join("bin").join("obscura");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"").unwrap();
        let missing = dir.join("does-not-exist");
        let _env = set_env_igs_config_dir(&dir);
        let result = ObscuraProvider::resolve_obscura_binary_with(Some(&missing));
        assert_eq!(result, Some(bin));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn download_target_prefers_igs_dir_when_present() {
        let _guard = env_lock().lock().unwrap();
        // IGS config dir exists → downloads land in its bin/ so igs reuses them.
        let dir = temp_dir("dl-igs");
        let _env = set_env_igs_config_dir(&dir);
        assert_eq!(
            ObscuraProvider::download_target(),
            dir.join("bin").join("obscura")
        );
        let _ = std::fs::remove_dir_all(dir);

        // IGS config dir does NOT exist → operant-managed copy (no IGS to share).
        let absent = temp_dir("dl-absent");
        std::fs::remove_dir_all(&absent).unwrap();
        let _env2 = set_env_igs_config_dir(&absent);
        assert_eq!(
            ObscuraProvider::download_target(),
            ObscuraProvider::default_bin_path()
        );
        let _ = std::fs::remove_dir_all(absent);
    }

    #[test]
    fn test_default_is_lightpanda() {
        let p = build_browser_provider("lightpanda");
        assert_eq!(p.name(), "lightpanda");
    }

    #[test]
    fn test_igs_provider_maps_to_igs() {
        let p = build_browser_provider("igs");
        assert_eq!(p.name(), "igs");
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
